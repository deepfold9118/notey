//! Reading Calca's syntax: names of several words (`long name`), implicit
//! multiplication (`2x`, `100 USD`), `|x|` for absolute values, matrices
//! (`[1, 2; 3, 4]`), ranges (`0:3`), `if ... then ... else`, `let ... in`,
//! `%`, and currency conversion (`... in EUR`).

use crate::currency;
use crate::expr::{CmpOp, R};
use crate::num::Culture;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(f64, u8),
    Word(String),
    Op(&'static str),
    /// A currency symbol such as `$`, as its code.
    Cur(&'static str),
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    /// Whitespace comes before it.
    space: bool,
}

const KEYWORDS: [&str; 7] = ["if", "then", "else", "let", "in", "true", "false"];

fn is_keyword(w: &str) -> bool {
    KEYWORDS.contains(&w)
}

const OPS2: [&str; 9] = ["=>", "==", "!=", "<=", ">=", "&&", "||", "**", "->"];
const OPS1: &str = "+-*/^()[],;:|&!<>=%?";

fn lex(src: &str, c: &Culture) -> R<Vec<Token>> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut space = false;
    let mut depth = 0i32;
    let digit = |i: usize| chars.get(i).is_some_and(|c| c.is_ascii_digit());
    while i < chars.len() {
        let ch = chars[i];
        if ch.is_whitespace() {
            space = true;
            i += 1;
            continue;
        }
        let start_space = std::mem::take(&mut space);
        let mut push = |tok: Tok| out.push(Token { tok, space: start_space });
        if ch.is_ascii_digit() || (ch == '.' && digit(i + 1)) || (ch == c.decimal && digit(i + 1) && ch != ',') {
            // hexadecimal and binary
            if ch == '0' && matches!(chars.get(i + 1), Some('x' | 'X' | 'b' | 'B')) && chars.get(i + 2).is_some_and(|d| d.is_ascii_hexdigit()) {
                let radix = if matches!(chars[i + 1], 'x' | 'X') { 16 } else { 2 };
                let mut j = i + 2;
                let mut s = String::new();
                while j < chars.len() && (chars[j].is_digit(radix) || chars[j] == '_') {
                    if chars[j] != '_' {
                        s.push(chars[j]);
                    }
                    j += 1;
                }
                let v = u64::from_str_radix(&s, radix).map_err(|_| "bad number")?;
                if v > u32::MAX as u64 {
                    return Err("hexadecimal and binary numbers are limited to 32 bits".into());
                }
                push(Tok::Num(v as f64, radix as u8));
                i = j;
                continue;
            }
            let mut s = String::new();
            let mut j = i;
            while j < chars.len() {
                let d = chars[j];
                if d.is_ascii_digit() {
                    s.push(d);
                    j += 1;
                    continue;
                }
                // thousands separators: a group of exactly three digits
                let group = c.group.unwrap_or(',');
                let groups_ok = if c.group.is_none() { depth == 0 } else { true };
                if d == group && groups_ok && !s.contains('.') && digit(j + 1) && digit(j + 2) && digit(j + 3) && !digit(j + 4) && !s.is_empty() {
                    s.extend(&chars[j + 1..j + 4]);
                    j += 4;
                    continue;
                }
                if (d == c.decimal || d == '.') && !s.contains('.') && digit(j + 1) {
                    s.push('.');
                    j += 1;
                    continue;
                }
                if (d == 'e' || d == 'E') && (digit(j + 1) || (matches!(chars.get(j + 1), Some('+' | '-')) && digit(j + 2))) {
                    s.push('e');
                    s.push(chars[j + 1]);
                    j += 2;
                    while digit(j) {
                        s.push(chars[j]);
                        j += 1;
                    }
                    break;
                }
                break;
            }
            let v: f64 = s.parse().map_err(|_| format!("bad number {s}"))?;
            push(Tok::Num(v, 10));
            i = j;
            continue;
        }
        if ch.is_alphabetic() || ch == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            push(Tok::Word(chars[i..j].iter().collect()));
            i = j;
            continue;
        }
        if let Some(code) = currency::symbol_code(ch) {
            push(Tok::Cur(code));
            i += 1;
            continue;
        }
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        if let Some(op) = OPS2.iter().find(|o| **o == two) {
            push(Tok::Op(match *op {
                "**" => "^",
                op => op,
            }));
            i += 2;
            continue;
        }
        let op = match ch {
            '×' | '·' => Some("*"),
            '÷' => Some("/"),
            '−' => Some("-"),
            '≤' => Some("<="),
            '≥' => Some(">="),
            '≠' => Some("!="),
            ch => OPS1.find(ch).map(|k| &OPS1[k..k + 1]),
        };
        let Some(op) = op else {
            return Err(format!("unexpected '{ch}'"));
        };
        match op {
            "(" | "[" => depth += 1,
            ")" | "]" => depth -= 1,
            _ => {}
        }
        push(Tok::Op(op));
        i += 1;
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Cmp(CmpOp),
    And,
    Or,
    BitAnd,
    BitOr,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Ast {
    Num(f64, u8),
    Name(String),
    Cur(String),
    Bool(bool),
    Call(String, Vec<Arg>),
    Index(Box<Ast>, Vec<Ast>),
    Neg(Box<Ast>),
    Not(Box<Ast>),
    Bin(Op, Box<Ast>, Box<Ast>),
    Pct(Box<Ast>),
    Abs(Box<Ast>),
    Matrix(Vec<Vec<Ast>>),
    If(Box<Ast>, Box<Ast>, Box<Ast>),
    Let(String, Box<Ast>, Box<Ast>),
    Range(Box<Ast>, Box<Ast>),
    Convert(Box<Ast>, String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arg {
    pub name: Option<String>,
    pub value: Ast,
}

/// A statement: an expression, or `lhs = rhs` (a definition or equation).
#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Expr(Ast),
    Assign(Ast, Ast),
}

const MAX_DEPTH: usize = 150;

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    abs_depth: usize,
    no_in: bool,
    depth: usize,
}

fn bin(op: Op, a: Ast, b: Ast) -> Ast {
    Ast::Bin(op, Box::new(a), Box::new(b))
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }
    fn peek_at(&self, k: usize) -> Option<&Token> {
        self.toks.get(self.pos + k)
    }
    fn is_op(&self, op: &str) -> bool {
        matches!(self.peek(), Some(Tok::Op(o)) if *o == op)
    }
    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(x)) if x == w)
    }
    fn eat_op(&mut self, op: &str) -> bool {
        if self.is_op(op) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect_op(&mut self, op: &str) -> R<()> {
        if self.eat_op(op) { Ok(()) } else { Err(format!("expected '{op}'")) }
    }
    fn expect_word(&mut self, w: &str) -> R<()> {
        if self.is_word(w) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("expected '{w}'"))
        }
    }

    fn enter(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err("the expression is nested too deeply".into());
        }
        Ok(())
    }

    fn expr(&mut self) -> R<Ast> {
        self.enter()?;
        let mut e = self.if_let()?;
        while self.is_word("in") && !self.no_in {
            self.pos += 1;
            let code = match self.peek().cloned() {
                Some(Tok::Cur(c)) => c.to_string(),
                Some(Tok::Word(w)) => w,
                _ => return Err("expected a currency after 'in'".into()),
            };
            self.pos += 1;
            e = Ast::Convert(Box::new(e), code);
        }
        self.depth -= 1;
        Ok(e)
    }

    fn if_let(&mut self) -> R<Ast> {
        if self.is_word("if") {
            self.pos += 1;
            let c = self.or()?;
            self.expect_word("then")?;
            let a = self.or()?;
            self.expect_word("else")?;
            let b = self.if_let()?;
            return Ok(Ast::If(Box::new(c), Box::new(a), Box::new(b)));
        }
        if self.is_word("let") {
            self.pos += 1;
            let name = self.name()?;
            self.expect_op("=")?;
            let saved = std::mem::replace(&mut self.no_in, true);
            let value = self.if_let();
            self.no_in = saved;
            let value = value?;
            self.expect_word("in")?;
            let body = self.if_let()?;
            return Ok(Ast::Let(name, Box::new(value), Box::new(body)));
        }
        self.or()
    }

    fn or(&mut self) -> R<Ast> {
        let mut e = self.and()?;
        while self.eat_op("||") {
            e = bin(Op::Or, e, self.and()?);
        }
        Ok(e)
    }

    fn and(&mut self) -> R<Ast> {
        let mut e = self.comparison()?;
        while self.eat_op("&&") {
            e = bin(Op::And, e, self.comparison()?);
        }
        Ok(e)
    }

    fn comparison(&mut self) -> R<Ast> {
        let e = self.bit_or()?;
        let op = match self.peek() {
            Some(Tok::Op("==")) => CmpOp::Eq,
            Some(Tok::Op("!=")) => CmpOp::Ne,
            Some(Tok::Op("<")) => CmpOp::Lt,
            Some(Tok::Op("<=")) => CmpOp::Le,
            Some(Tok::Op(">")) => CmpOp::Gt,
            Some(Tok::Op(">=")) => CmpOp::Ge,
            _ => return Ok(e),
        };
        self.pos += 1;
        Ok(bin(Op::Cmp(op), e, self.bit_or()?))
    }

    fn bit_or(&mut self) -> R<Ast> {
        let mut e = self.bit_and()?;
        // inside |...| a bar closes the absolute value instead
        while self.abs_depth == 0 && self.eat_op("|") {
            e = bin(Op::BitOr, e, self.bit_and()?);
        }
        Ok(e)
    }

    fn bit_and(&mut self) -> R<Ast> {
        let mut e = self.range()?;
        while self.eat_op("&") {
            e = bin(Op::BitAnd, e, self.range()?);
        }
        Ok(e)
    }

    fn range(&mut self) -> R<Ast> {
        let e = self.sum()?;
        if self.eat_op(":") {
            return Ok(Ast::Range(Box::new(e), Box::new(self.sum()?)));
        }
        Ok(e)
    }

    fn sum(&mut self) -> R<Ast> {
        let mut e = self.product()?;
        loop {
            if self.eat_op("+") {
                e = bin(Op::Add, e, self.product()?);
            } else if self.eat_op("-") {
                e = bin(Op::Sub, e, self.product()?);
            } else {
                return Ok(e);
            }
        }
    }

    /// Whether the next token multiplies what came before without a `*`:
    /// after a number or a closing bracket, a name, currency or `(`.
    fn implicit(&self) -> bool {
        let Some(prev) = self.pos.checked_sub(1).and_then(|p| self.toks.get(p)) else { return false };
        let after_value = matches!(prev.tok, Tok::Num(..) | Tok::Op(")") | Tok::Op("]") | Tok::Op("%") | Tok::Cur(_));
        let starts = match self.peek() {
            Some(Tok::Word(w)) => !is_keyword(w),
            Some(Tok::Cur(_)) | Some(Tok::Op("(")) => true,
            _ => false,
        };
        after_value && starts
    }

    fn product(&mut self) -> R<Ast> {
        let mut e = self.unary()?;
        loop {
            if self.eat_op("*") {
                e = bin(Op::Mul, e, self.unary()?);
            } else if self.eat_op("/") {
                e = bin(Op::Div, e, self.unary()?);
            } else if self.implicit() {
                e = bin(Op::Mul, e, self.unary()?);
            } else {
                return Ok(e);
            }
        }
    }

    fn unary(&mut self) -> R<Ast> {
        self.enter()?;
        let e = if self.eat_op("-") {
            Ast::Neg(Box::new(self.unary()?))
        } else if self.eat_op("+") {
            self.unary()?
        } else if self.eat_op("!") {
            Ast::Not(Box::new(self.unary()?))
        } else {
            self.power()?
        };
        self.depth -= 1;
        Ok(e)
    }

    fn power(&mut self) -> R<Ast> {
        let base = self.postfix()?;
        if self.eat_op("^") {
            return Ok(bin(Op::Pow, base, self.unary()?));
        }
        Ok(base)
    }

    fn postfix(&mut self) -> R<Ast> {
        let mut e = self.primary()?;
        loop {
            if self.eat_op("%") {
                e = Ast::Pct(Box::new(e));
            } else if self.is_op("[") && !self.toks[self.pos].space {
                self.pos += 1;
                let saved = std::mem::replace(&mut self.abs_depth, 0);
                let mut idx = vec![self.expr()?];
                while self.eat_op(",") {
                    idx.push(self.expr()?);
                }
                self.abs_depth = saved;
                self.expect_op("]")?;
                e = Ast::Index(Box::new(e), idx);
            } else {
                return Ok(e);
            }
        }
    }

    /// A name of one or more words.
    fn name(&mut self) -> R<String> {
        let Some(Tok::Word(w)) = self.peek().cloned() else { return Err("expected a name".into()) };
        if is_keyword(&w) {
            return Err(format!("'{w}' is a keyword"));
        }
        self.pos += 1;
        let mut name = w;
        while let Some(Token { tok: Tok::Word(w), space: true }) = self.peek_at(0) {
            if is_keyword(w) {
                break;
            }
            name.push(' ');
            name.push_str(w);
            self.pos += 1;
        }
        Ok(name)
    }

    fn args(&mut self) -> R<Vec<Arg>> {
        let saved = std::mem::replace(&mut self.abs_depth, 0);
        let saved_in = std::mem::replace(&mut self.no_in, false);
        let mut args = Vec::new();
        if !self.is_op(")") {
            loop {
                // a named argument: name = value
                let start = self.pos;
                let mut name = None;
                if matches!(self.peek(), Some(Tok::Word(w)) if !is_keyword(w)) {
                    let n = self.name()?;
                    if self.eat_op("=") {
                        name = Some(n);
                    } else {
                        self.pos = start;
                    }
                }
                args.push(Arg { name, value: self.expr()? });
                if !self.eat_op(",") {
                    break;
                }
            }
        }
        self.abs_depth = saved;
        self.no_in = saved_in;
        self.expect_op(")")?;
        Ok(args)
    }

    fn primary(&mut self) -> R<Ast> {
        let Some(tok) = self.peek().cloned() else { return Err("unexpected end".into()) };
        match tok {
            Tok::Num(v, radix) => {
                self.pos += 1;
                let n = Ast::Num(v, radix);
                // 2x, 0.5t^2, 1/2i: a number and a name bind tightly
                match self.peek_at(0) {
                    Some(Token { tok: Tok::Word(w), space: false }) if !is_keyword(w) => Ok(bin(Op::Mul, n, self.power()?)),
                    Some(Token { tok: Tok::Cur(c), space: false }) => {
                        let c = c.to_string();
                        self.pos += 1;
                        Ok(bin(Op::Mul, n, Ast::Cur(c)))
                    }
                    _ => Ok(n),
                }
            }
            Tok::Cur(c) => {
                self.pos += 1;
                // $100
                if matches!(self.peek(), Some(Tok::Num(..))) {
                    let amount = self.postfix()?;
                    return Ok(bin(Op::Mul, amount, Ast::Cur(c.to_string())));
                }
                Ok(Ast::Cur(c.to_string()))
            }
            Tok::Word(w) => match w.as_str() {
                "true" => {
                    self.pos += 1;
                    Ok(Ast::Bool(true))
                }
                "false" => {
                    self.pos += 1;
                    Ok(Ast::Bool(false))
                }
                "if" | "let" => self.if_let(),
                w if is_keyword(w) => Err(format!("unexpected '{w}'")),
                _ => {
                    let name = self.name()?;
                    if self.eat_op("(") {
                        return Ok(Ast::Call(name, self.args()?));
                    }
                    Ok(Ast::Name(name))
                }
            },
            Tok::Op("(") => {
                self.pos += 1;
                let saved = std::mem::replace(&mut self.abs_depth, 0);
                let saved_in = std::mem::replace(&mut self.no_in, false);
                let e = self.expr();
                self.abs_depth = saved;
                self.no_in = saved_in;
                let e = e?;
                self.expect_op(")")?;
                Ok(e)
            }
            Tok::Op("[") => {
                self.pos += 1;
                let saved = std::mem::replace(&mut self.abs_depth, 0);
                let saved_in = std::mem::replace(&mut self.no_in, false);
                let mut rows = vec![Vec::new()];
                if !self.is_op("]") {
                    loop {
                        rows.last_mut().unwrap().push(self.expr()?);
                        if self.eat_op(",") {
                            continue;
                        }
                        if self.eat_op(";") {
                            rows.push(Vec::new());
                            continue;
                        }
                        break;
                    }
                }
                self.abs_depth = saved;
                self.no_in = saved_in;
                self.expect_op("]")?;
                if rows.iter().any(Vec::is_empty) {
                    return Err("empty matrix row".into());
                }
                Ok(Ast::Matrix(rows))
            }
            Tok::Op("|") => {
                self.pos += 1;
                self.abs_depth += 1;
                let e = self.expr();
                self.abs_depth -= 1;
                let e = e?;
                self.expect_op("|")?;
                Ok(Ast::Abs(Box::new(e)))
            }
            Tok::Op(op) => Err(format!("unexpected '{op}'")),
        }
    }
}

fn parse_tokens(toks: Vec<Token>) -> R<Ast> {
    if toks.is_empty() {
        return Err("nothing to calculate".into());
    }
    let mut p = Parser { toks, pos: 0, abs_depth: 0, no_in: false, depth: 0 };
    let e = p.expr()?;
    if p.pos < p.toks.len() {
        return Err("unexpected text after the expression".into());
    }
    Ok(e)
}

pub fn parse_expr(src: &str, c: &Culture) -> R<Ast> {
    parse_tokens(lex(src, c)?)
}

/// Parse a statement: `expr`, or `lhs = rhs` (the `=` outside brackets).
pub fn parse_stmt(src: &str, c: &Culture) -> R<Stmt> {
    let toks = lex(src, c)?;
    let mut depth = 0i32;
    let mut split = None;
    for (k, t) in toks.iter().enumerate() {
        match t.tok {
            Tok::Op("(") | Tok::Op("[") => depth += 1,
            Tok::Op(")") | Tok::Op("]") => depth -= 1,
            // the first `=`; a later one belongs to a `let`
            Tok::Op("=") if depth == 0 && split.is_none() => split = Some(k),
            Tok::Op("?") => return Err("a search".into()),
            _ => {}
        }
    }
    // `let x = ... in` has its own `=`
    if matches!(toks.first(), Some(Token { tok: Tok::Word(w), .. }) if w == "let") {
        split = None;
    }
    match split {
        None => Ok(Stmt::Expr(parse_tokens(toks)?)),
        Some(k) => {
            let rhs = toks[k + 1..].to_vec();
            let mut lhs = toks;
            lhs.truncate(k);
            Ok(Stmt::Assign(parse_tokens(lhs)?, parse_tokens(rhs)?))
        }
    }
}
