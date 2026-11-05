//! Writing expressions: the display form (Calca's style: `1.1y`,
//! `-2x/y^2`, `0.5*sqrt(a)/b`, `2i + 1`, `[1, 2; 3, 4]`), and an exact
//! key used to compare and order terms.

use std::cmp::Ordering;

use crate::currency;
use crate::expr::{split_coeff, E};
use crate::num::{fmt_num, Culture, Num};

/// An exact, unambiguous form of `e`, for comparing expressions.
pub fn key(e: &E) -> String {
    let mut s = String::new();
    write_key(e, &mut s);
    s
}

fn write_key(e: &E, s: &mut String) {
    let list = |s: &mut String, tag: &str, xs: &[E]| {
        s.push('(');
        s.push_str(tag);
        for x in xs {
            s.push(' ');
            write_key(x, s);
        }
        s.push(')');
    };
    match e {
        E::Num(n) => s.push_str(&format!("#{:?},{:?}", n.re, n.im)),
        E::Bool(b) => s.push_str(if *b { "#t" } else { "#f" }),
        E::Sym(n) => s.push_str(n),
        E::Add(xs) => list(s, "+", xs),
        E::Mul(xs) => list(s, "*", xs),
        E::Pow(a, b) => list(s, "^", &[(**a).clone(), (**b).clone()]),
        E::Fn(n, xs) => list(s, &format!("f:{n}"), xs),
        E::Mat(m) => list(s, &format!("m{}x{}", m.rows, m.cols), &m.data),
        E::Range(a, b) => s.push_str(&format!("(r {a} {b})")),
        E::If(c, a, b) => list(s, "if", &[(**c).clone(), (**a).clone(), (**b).clone()]),
        E::Cmp(op, a, b) => list(s, op.symbol(), &[(**a).clone(), (**b).clone()]),
        E::And(xs) => list(s, "&&", xs),
        E::Or(xs) => list(s, "||", xs),
        E::Not(a) => list(s, "!", &[(**a).clone()]),
    }
}

fn base_of(e: &E) -> &E {
    match e {
        E::Pow(b, _) => b,
        e => e,
    }
}

fn class(e: &E) -> u8 {
    match base_of(e) {
        E::Sym(s) if s.starts_with(currency::PREFIX) => 4,
        E::Sym(_) => 0,
        E::Fn(..) => 1,
        E::Add(_) => 2,
        _ => 3,
    }
}

/// The order of factors in a product: variables (by name, so `a*x^2`),
/// then functions, then sums; money last.
pub fn factor_order(a: &E, b: &E) -> Ordering {
    class(a).cmp(&class(b)).then_with(|| key(base_of(a)).cmp(&key(base_of(b)))).then_with(|| key(a).cmp(&key(b)))
}

fn degree_in(t: &E, v: &str) -> f64 {
    let one = |f: &E| match f {
        E::Sym(s) if s == v => 1.0,
        E::Pow(b, e) if matches!(&**b, E::Sym(s) if s == v) => e.as_num().map_or(0.0, |n| n.re),
        _ => 0.0,
    };
    match t {
        E::Mul(fs) => fs.iter().map(one).sum(),
        t => one(t),
    }
}

/// The order of terms in a sum: by their first variable, then by its power
/// (`x - 0.5x^2 + 1`); numbers last.
pub fn term_order(a: &E, b: &E) -> Ordering {
    let info = |t: &E| {
        let mut syms = Vec::new();
        t.symbols(&mut syms);
        syms.sort();
        let count = syms.len();
        let first = syms.into_iter().next().unwrap_or_default();
        let d = degree_in(t, &first);
        (matches!(t, E::Num(_)), first, d, count)
    };
    let (ca, fa, da, na) = info(a);
    let (cb, fb, db, nb) = info(b);
    ca.cmp(&cb)
        .then_with(|| fa.cmp(&fb))
        .then_with(|| da.partial_cmp(&db).unwrap_or(Ordering::Equal))
        .then_with(|| na.cmp(&nb))
        .then_with(|| key(&split_coeff(a.clone()).1).cmp(&key(&split_coeff(b.clone()).1)))
        .then_with(|| key(a).cmp(&key(b)))
}

/// Whether `e` is written with a leading minus.
pub fn leads_negative(e: &E) -> bool {
    is_negative(e)
}

/// Writes expressions for display.
pub struct Printer {
    pub culture: Culture,
}

// precedence levels, loosest first
const P_IF: u8 = 0;
const P_OR: u8 = 1;
const P_AND: u8 = 2;
const P_CMP: u8 = 3;
const P_RANGE: u8 = 4;
const P_ADD: u8 = 5;
const P_MUL: u8 = 6;
const P_POW: u8 = 7;
const P_ATOM: u8 = 9;

fn negative(n: &Num) -> bool {
    if n.is_real() { n.re < 0.0 } else { n.re == 0.0 && n.im < 0.0 }
}

/// Whether a term is written with a leading minus.
fn is_negative(t: &E) -> bool {
    match t {
        E::Num(n) => negative(n),
        E::Mul(f) => matches!(&f[0], E::Num(n) if negative(n)),
        _ => false,
    }
}

fn negated(t: &E) -> E {
    match t {
        E::Num(n) => E::Num(n.neg()),
        E::Mul(f) => {
            let mut f = f.clone();
            if let E::Num(n) = f[0] {
                if n.neg().is_one() {
                    f.remove(0);
                } else {
                    f[0] = E::Num(n.neg());
                }
            }
            if f.len() == 1 { f.pop().unwrap() } else { E::Mul(f) }
        }
        t => t.clone(),
    }
}

fn prec(e: &E) -> u8 {
    match e {
        E::If(..) => P_IF,
        E::Or(_) => P_OR,
        E::And(_) => P_AND,
        E::Cmp(..) => P_CMP,
        E::Range(..) => P_RANGE,
        E::Add(_) => P_ADD,
        E::Num(n) if !n.is_real() && n.re != 0.0 => P_ADD,
        E::Num(n) if negative(n) => P_ADD,
        E::Num(n) if !n.is_real() => P_MUL,
        E::Mul(f) if is_negative(e) || f.iter().any(|x| matches!(x, E::Pow(_, p) if p.as_num().is_some_and(|n| n.is_real() && n.re < 0.0))) => {
            if is_negative(e) { P_ADD } else { P_MUL }
        }
        E::Mul(_) => P_MUL,
        E::Pow(b, x) if x.as_num().is_some_and(|n| n.is_real() && n.re < 0.0) && !matches!(**b, E::Mat(_)) => P_MUL,
        E::Pow(..) => P_POW,
        _ => P_ATOM,
    }
}

impl Printer {
    pub fn new(culture: Culture) -> Self {
        Printer { culture }
    }

    pub fn show(&self, e: &E) -> String {
        self.expr(e)
    }

    fn wrap(&self, e: &E, min: u8) -> String {
        let s = self.expr(e);
        if prec(e) < min { format!("({s})") } else { s }
    }

    fn number(&self, n: Num) -> String {
        fmt_num(n, &self.culture, None)
    }

    fn expr(&self, e: &E) -> String {
        match e {
            E::Num(n) => self.number(*n),
            E::Bool(b) => (if *b { "true" } else { "false" }).into(),
            E::Sym(s) => match s.strip_prefix(currency::PREFIX) {
                Some(code) => format!("{} {code}", fmt_num(Num::ONE, &self.culture, Some(2))),
                None => s.clone(),
            },
            E::Add(ts) => self.sum(ts),
            E::Mul(fs) => self.product(fs),
            E::Pow(b, x) => self.power(b, x),
            E::Fn(name, args) if name == "abs" && args.len() == 1 => format!("|{}|", self.expr(&args[0])),
            E::Fn(name, args) => {
                let args: Vec<String> = args.iter().map(|a| self.expr(a)).collect();
                format!("{name}({})", args.join(", "))
            }
            E::Mat(m) => {
                let rows: Vec<String> = (0..m.rows)
                    .map(|r| (0..m.cols).map(|c| self.expr(m.get(r, c))).collect::<Vec<_>>().join(", "))
                    .collect();
                format!("[{}]", rows.join("; "))
            }
            E::Range(a, b) => format!("{a}:{b}"),
            E::If(c, a, b) => format!("if {} then {} else {}", self.expr(c), self.expr(a), self.expr(b)),
            E::Cmp(op, a, b) => format!("{} {} {}", self.wrap(a, P_RANGE), op.symbol(), self.wrap(b, P_RANGE)),
            E::And(xs) => xs.iter().map(|x| self.wrap(x, P_RANGE)).collect::<Vec<_>>().join(" && "),
            E::Or(xs) => xs.iter().map(|x| self.wrap(x, P_RANGE)).collect::<Vec<_>>().join(" || "),
            E::Not(a) => format!("!{}", self.wrap(a, P_ATOM)),
        }
    }

    fn sum(&self, ts: &[E]) -> String {
        // a complex constant is written as two terms: 3i + 2
        let mut terms = Vec::new();
        for t in ts {
            match t {
                E::Num(n) if !n.is_real() && n.re != 0.0 => {
                    terms.push(E::Num(Num::complex(0.0, n.im)));
                    terms.push(E::Num(Num::real(n.re)));
                }
                t => terms.push(t.clone()),
            }
        }
        let mut s = String::new();
        for (i, t) in terms.iter().enumerate() {
            if i == 0 {
                s.push_str(&self.wrap(t, P_ADD));
            } else if is_negative(t) {
                s.push_str(" - ");
                s.push_str(&self.wrap(&negated(t), P_MUL));
            } else {
                s.push_str(" + ");
                s.push_str(&self.wrap(t, P_ADD + 1));
            }
        }
        s
    }

    fn product(&self, fs: &[E]) -> String {
        let mut coeff = Num::ONE;
        let mut top: Vec<&E> = Vec::new();
        let mut bottom: Vec<E> = Vec::new();
        let mut money: Option<&str> = None;
        for f in fs {
            match f {
                E::Num(n) => coeff = coeff.mul(*n),
                E::Sym(s) if s.starts_with(currency::PREFIX) && money.is_none() => money = Some(&s[currency::PREFIX.len_utf8()..]),
                E::Pow(b, x) if x.as_num().is_some_and(|n| n.is_real() && n.re < 0.0) && !matches!(**b, E::Mat(_)) => {
                    let n = x.as_num().unwrap().neg();
                    bottom.push(if n.is_one() { (**b).clone() } else { E::Pow(b.clone(), Box::new(E::Num(n))) });
                }
                f => top.push(f),
            }
        }
        let places = money.map(|_| 2);
        let coeff_text = |c: Num| fmt_num(c, &self.culture, places);
        let mut s = String::new();
        if top.is_empty() {
            if money.is_some() || !bottom.is_empty() || !coeff.is_one() {
                s.push_str(&if !coeff.is_real() && coeff.re != 0.0 { format!("({})", coeff_text(coeff)) } else { coeff_text(coeff) });
            }
        } else {
            let first = self.wrap(top[0], P_MUL);
            if coeff.is_one() && money.is_none() {
            } else if coeff.neg().is_one() && money.is_none() {
                s.push('-');
            } else {
                let c = coeff_text(coeff);
                let complex = !coeff.is_real();
                let c = if complex && coeff.re != 0.0 { format!("({c})") } else { c };
                let glue = !complex && coeff.radix == 10 && !c.contains('e') && class(top[0]) == 0;
                s.push_str(&c);
                if !glue {
                    s.push('*');
                }
            }
            s.push_str(&first);
            for f in &top[1..] {
                s.push('*');
                s.push_str(&self.wrap(f, P_MUL));
            }
        }
        if !bottom.is_empty() {
            if s.is_empty() {
                s.push('1');
            }
            s.push('/');
            if bottom.len() == 1 {
                s.push_str(&self.wrap(&bottom[0], P_POW));
            } else {
                let parts: Vec<String> = bottom.iter().map(|b| self.wrap(b, P_MUL)).collect();
                s.push_str(&format!("({})", parts.join("*")));
            }
        }
        if let Some(code) = money {
            if s.is_empty() {
                s.push_str(&coeff_text(coeff));
            }
            s.push(' ');
            s.push_str(code);
        }
        s
    }

    fn power(&self, b: &E, x: &E) -> String {
        if let Some(n) = x.as_num().filter(|n| n.is_real()) {
            if n.re == 0.5 {
                return format!("sqrt({})", self.expr(b));
            }
            if n.re < 0.0 && !matches!(b, E::Mat(_)) {
                return self.product(&[E::Pow(Box::new(b.clone()), Box::new(x.clone()))]);
            }
        }
        let base = match b {
            E::Num(n) if n.is_e() => "e".to_string(),
            b => self.wrap(b, P_ATOM),
        };
        let exp = match x {
            E::Num(n) if n.is_real() => self.number(*n),
            x => self.wrap(x, P_ATOM),
        };
        format!("{base}^{exp}")
    }
}

pub fn show(e: &E, culture: &Culture) -> String {
    Printer::new(*culture).show(e)
}
