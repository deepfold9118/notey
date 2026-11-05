//! Evaluating parsed expressions with the document's definitions, the
//! special forms (`map`, `reduce`, `sum`, `prod`, `der`, `taylor`,
//! `jacobian`), and solving for an undefined variable.

use std::cell::Cell;
use std::collections::HashMap;

use crate::builtins::{self, is_builtin, subst};
use crate::calculus::{der, der_n};
use crate::currency::{self, Rates};
use crate::expr::{self, add, as_mat, div, mul, neg, num, pow, range_mat, sub, truth, Mat, E, MAX_ELEMENTS, R};
use crate::matrix;
use crate::num::Num;
use crate::parse::{Arg, Ast, Op};
use crate::solve::solve;

#[derive(Clone, Debug)]
pub struct Def {
    pub params: Vec<String>,
    /// The parameters were written out (`f(x) = ...`).
    pub explicit: bool,
    pub body: Ast,
}

/// Something the document stated, for solving: a definition or an equation.
#[derive(Clone, Debug)]
pub enum Fact {
    Def(String, Ast),
    Eq(Ast, Ast),
}

/// Definitions under one heading (and its children).
#[derive(Debug, Default)]
pub struct Scope {
    pub level: usize,
    pub defs: HashMap<String, Def>,
    pub facts: Vec<Fact>,
}

pub struct Env {
    scopes: Vec<Scope>,
    pub rates: Option<Rates>,
    /// Money was used.
    pub currency: Cell<bool>,
    /// A conversion needed exchange rates that are not loaded.
    pub needs_rates: Cell<bool>,
}

const SPECIAL: [&str; 7] = ["map", "reduce", "sum", "prod", "der", "taylor", "jacobian"];
const MAX_DEPTH: usize = 250;

fn is_constant(n: &str) -> bool {
    matches!(n, "pi" | "π" | "e" | "i")
}

impl Env {
    pub fn new(rates: Option<Rates>) -> Env {
        Env { scopes: vec![Scope::default()], rates, currency: Cell::new(false), needs_rates: Cell::new(false) }
    }

    /// A heading of `level` starts a section: definitions of sections at
    /// the same or a deeper level are no longer visible.
    pub fn heading(&mut self, level: usize) {
        while self.scopes.len() > 1 && self.scopes.last().unwrap().level >= level {
            self.scopes.pop();
        }
        self.scopes.push(Scope { level, ..Scope::default() });
    }

    pub fn def(&self, name: &str) -> Option<&Def> {
        self.scopes.iter().rev().find_map(|s| s.defs.get(name))
    }

    fn top(&mut self) -> &mut Scope {
        self.scopes.last_mut().unwrap()
    }

    /// `lhs = rhs`: a definition when it names something new, else an
    /// equation (definitions cannot be redefined). Returns whether it was
    /// a definition.
    pub fn assign(&mut self, lhs: &Ast, rhs: &Ast) -> bool {
        let (name, params) = match lhs {
            Ast::Name(n) => (n.clone(), None),
            Ast::Call(n, args) if args.iter().all(|a| a.name.is_none() && matches!(a.value, Ast::Name(_))) => {
                let params = args.iter().map(|a| if let Ast::Name(p) = &a.value { p.clone() } else { unreachable!() }).collect();
                (n.clone(), Some(params))
            }
            _ => {
                self.top().facts.push(Fact::Eq(lhs.clone(), rhs.clone()));
                return false;
            }
        };
        if self.def(&name).is_some() {
            self.top().facts.push(Fact::Eq(lhs.clone(), rhs.clone()));
            return false;
        }
        let explicit = params.is_some();
        let params = params.unwrap_or_else(|| {
            let ctx = Ctx::new(self);
            ctx.free_names(rhs).into_iter().filter(|p| *p != name).collect()
        });
        self.top().defs.insert(name.clone(), Def { params, explicit, body: rhs.clone() });
        self.top().facts.push(Fact::Def(name, rhs.clone()));
        true
    }

    pub fn eval(&self, ast: &Ast) -> R<E> {
        Ctx::new(self).eval(ast)
    }

    /// The value of `ast` for a `=>`: an undefined variable on its own is
    /// solved for, and a sum of different currencies is converted to the
    /// first one.
    pub fn result(&self, ast: &Ast) -> R<E> {
        let mut ctx = Ctx::new(self);
        let value = match ast {
            Ast::Name(n) if ctx.is_free(n) => self.solve_for(n)?,
            ast => ctx.eval(ast)?,
        };
        let codes = currency::codes_in(&value);
        if codes.len() > 1 {
            let first = first_currency(ast, self).unwrap_or_else(|| codes[0].clone());
            return self.convert(&value, &first);
        }
        Ok(value)
    }

    fn convert(&self, value: &E, code: &str) -> R<E> {
        self.currency.set(true);
        if !currency::is_code(code) {
            return Err(format!("{code} is not a currency"));
        }
        match &self.rates {
            Some(r) => currency::convert(value, code, r),
            None => {
                self.needs_rates.set(true);
                Err("exchange rates are not loaded yet".into())
            }
        }
    }

    fn facts(&self) -> impl Iterator<Item = &Fact> {
        self.scopes.iter().rev().flat_map(|s| s.facts.iter().rev())
    }

    /// Solve for an undefined variable with the latest equation or
    /// definition that uses it.
    fn solve_for(&self, v: &str) -> R<E> {
        let mut ctx = Ctx::new(self);
        for fact in self.facts() {
            match fact {
                Fact::Def(name, body) if mentions(body, v) => {
                    let r = ctx.eval(body)?;
                    return solve(&E::Sym(name.clone()), &r, v);
                }
                Fact::Eq(l, r) if mentions(l, v) || mentions(r, v) => {
                    if let Some(x) = ctx.matrix_solve(l, r, v)? {
                        return Ok(x);
                    }
                    return solve(&ctx.eval(l)?, &ctx.eval(r)?, v);
                }
                _ => {}
            }
        }
        Ok(E::Sym(v.to_string()))
    }
}

/// The first currency written in `ast`, or in the definitions it uses.
fn first_currency(ast: &Ast, env: &Env) -> Option<String> {
    first_currency_in(ast, env, 0)
}

fn first_currency_in(ast: &Ast, env: &Env, depth: usize) -> Option<String> {
    let mut nodes = Vec::new();
    walk(ast, &mut |a| {
        if matches!(a, Ast::Cur(_) | Ast::Name(_) | Ast::Call(..)) {
            nodes.push(a.clone());
        }
    });
    for a in nodes {
        match &a {
            Ast::Cur(c) => return Some(c.clone()),
            Ast::Name(n) | Ast::Call(n, _) => match env.def(n) {
                Some(def) if depth < 16 => {
                    if let Some(c) = first_currency_in(&def.body, env, depth + 1) {
                        return Some(c);
                    }
                }
                None if currency::is_code(n) => return Some(n.clone()),
                _ => {}
            },
            _ => {}
        }
    }
    None
}

fn walk(ast: &Ast, f: &mut dyn FnMut(&Ast)) {
    f(ast);
    match ast {
        Ast::Call(_, args) => args.iter().for_each(|a| walk(&a.value, f)),
        Ast::Index(b, idx) => {
            walk(b, f);
            idx.iter().for_each(|i| walk(i, f));
        }
        Ast::Neg(a) | Ast::Not(a) | Ast::Pct(a) | Ast::Abs(a) | Ast::Convert(a, _) => walk(a, f),
        Ast::Bin(_, a, b) | Ast::Range(a, b) | Ast::Let(_, a, b) => {
            walk(a, f);
            walk(b, f);
        }
        Ast::Matrix(rows) => rows.iter().flatten().for_each(|x| walk(x, f)),
        Ast::If(c, a, b) => {
            walk(c, f);
            walk(a, f);
            walk(b, f);
        }
        Ast::Num(..) | Ast::Name(_) | Ast::Cur(_) | Ast::Bool(_) => {}
    }
}

fn mentions(ast: &Ast, v: &str) -> bool {
    let mut found = false;
    walk(ast, &mut |a| found |= matches!(a, Ast::Name(n) if n == v));
    found
}

/// Whether evaluating `ast` uses anything known: a number, a definition, a
/// constant, money or a function. If not, its value can only be the same
/// unknowns rearranged.
pub fn uses_knowledge(ast: &Ast, env: &Env) -> bool {
    let mut known = false;
    walk(ast, &mut |a| {
        known |= match a {
            Ast::Num(..) | Ast::Bool(_) | Ast::Cur(_) | Ast::Matrix(_) | Ast::Range(..) | Ast::Pct(_) | Ast::Convert(..) => true,
            Ast::Name(n) => env.def(n).is_some() || is_constant(n) || currency::is_code(n),
            Ast::Call(n, _) => env.def(n).is_some() || is_builtin(n) || SPECIAL.contains(&n.as_str()),
            Ast::Bin(Op::Cmp(_) | Op::And | Op::Or | Op::BitAnd | Op::BitOr, ..) => true,
            _ => false,
        }
    });
    known
}

/// A function value, for `map` and `reduce`.
enum FnVal<'a> {
    User(String),
    Builtin(String),
    Lambda(&'a Ast, Vec<String>),
    Unknown(String),
}

struct Ctx<'e> {
    env: &'e Env,
    locals: Vec<(String, E)>,
    /// Definitions being evaluated (a self-reference stays a variable).
    busy: Vec<String>,
    depth: Cell<usize>,
}

impl<'e> Ctx<'e> {
    fn new(env: &'e Env) -> Self {
        Ctx { env, locals: Vec::new(), busy: Vec::new(), depth: Cell::new(0) }
    }

    fn local(&self, n: &str) -> Option<&E> {
        self.locals.iter().rev().find(|(k, _)| k == n).map(|(_, v)| v)
    }

    /// Whether `n` names nothing: not a local, definition, constant or
    /// currency.
    fn is_free(&self, n: &str) -> bool {
        self.local(n).is_none() && self.env.def(n).is_none() && !is_constant(n) && !currency::is_code(n)
    }

    /// The undefined names in `ast`, in order of first occurrence.
    fn free_names(&self, ast: &Ast) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        self.collect_free(ast, &mut Vec::new(), &mut out);
        out
    }

    fn collect_free(&self, ast: &Ast, bound: &mut Vec<String>, out: &mut Vec<String>) {
        match ast {
            Ast::Name(n) => {
                if self.is_free(n) && !bound.contains(n) && !out.contains(n) && n != "T" {
                    out.push(n.clone());
                }
            }
            Ast::Let(n, v, body) => {
                self.collect_free(v, bound, out);
                bound.push(n.clone());
                self.collect_free(body, bound, out);
                bound.pop();
            }
            Ast::Call(_, args) => args.iter().for_each(|a| self.collect_free(&a.value, bound, out)),
            Ast::Index(b, idx) => {
                self.collect_free(b, bound, out);
                idx.iter().for_each(|i| self.collect_free(i, bound, out));
            }
            Ast::Neg(a) | Ast::Not(a) | Ast::Pct(a) | Ast::Abs(a) | Ast::Convert(a, _) => self.collect_free(a, bound, out),
            Ast::Bin(_, a, b) | Ast::Range(a, b) => {
                self.collect_free(a, bound, out);
                self.collect_free(b, bound, out);
            }
            Ast::Matrix(rows) => rows.iter().flatten().for_each(|x| self.collect_free(x, bound, out)),
            Ast::If(c, a, b) => {
                self.collect_free(c, bound, out);
                self.collect_free(a, bound, out);
                self.collect_free(b, bound, out);
            }
            Ast::Num(..) | Ast::Cur(_) | Ast::Bool(_) => {}
        }
    }

    fn eval(&mut self, ast: &Ast) -> R<E> {
        let d = self.depth.get() + 1;
        if d > MAX_DEPTH {
            return Err("the calculation goes too deep".into());
        }
        self.depth.set(d);
        let r = self.eval_inner(ast);
        self.depth.set(d - 1);
        r
    }

    fn eval_inner(&mut self, ast: &Ast) -> R<E> {
        match ast {
            Ast::Num(v, radix) => Ok(E::Num(Num::real(*v).with_radix(*radix))),
            Ast::Bool(b) => Ok(E::Bool(*b)),
            Ast::Name(n) => self.name(n),
            Ast::Cur(c) => {
                self.env.currency.set(true);
                Ok(currency::unit(c))
            }
            Ast::Call(n, args) => self.call(n, args),
            Ast::Index(b, idx) => {
                let b = self.eval(b)?;
                let idx = idx.iter().map(|i| self.eval(i)).collect::<R<Vec<_>>>()?;
                index(&b, &idx)
            }
            Ast::Neg(a) => neg(self.eval(a)?),
            Ast::Not(a) => expr::not(self.eval(a)?),
            Ast::Bin(op, a, b) => {
                let (a, b) = (self.eval(a)?, self.eval(b)?);
                match op {
                    Op::Add => add(vec![a, b]),
                    Op::Sub => sub(a, b),
                    Op::Mul => mul(vec![a, b]),
                    Op::Div => div(a, b),
                    Op::Pow => pow(a, b),
                    Op::Cmp(c) => expr::cmp(*c, a, b),
                    Op::And => expr::and(a, b),
                    Op::Or => expr::or(a, b),
                    Op::BitAnd => expr::bits(true, a, b),
                    Op::BitOr => expr::bits(false, a, b),
                }
            }
            Ast::Pct(a) => mul(vec![self.eval(a)?, num(0.01)]),
            Ast::Abs(a) => builtins::call("abs", vec![self.eval(a)?]),
            Ast::Matrix(rows) => {
                let mut vals = Vec::new();
                for row in rows {
                    vals.push(row.iter().map(|x| self.eval(x)).collect::<R<Vec<_>>>()?);
                }
                matrix_literal(vals)
            }
            Ast::If(c, a, b) => {
                let c = self.eval(c)?;
                match truth(&c) {
                    Some(true) => self.eval(a),
                    Some(false) => self.eval(b),
                    None => Ok(E::If(Box::new(c), Box::new(self.eval(a)?), Box::new(self.eval(b)?))),
                }
            }
            Ast::Let(n, v, body) => {
                let v = self.eval(v)?;
                self.locals.push((n.clone(), v));
                let r = self.eval(body);
                self.locals.pop();
                r
            }
            Ast::Range(a, b) => {
                let (a, b) = (self.eval(a)?, self.eval(b)?);
                match (a.as_int(), b.as_int()) {
                    (Some(a), Some(b)) => Ok(E::Range(a, b)),
                    _ => Err("a range needs whole numbers".into()),
                }
            }
            Ast::Convert(x, code) => {
                let v = self.eval(x)?;
                self.env.convert(&v, code)
            }
        }
    }

    fn name(&mut self, n: &str) -> R<E> {
        if let Some(v) = self.local(n) {
            return Ok(v.clone());
        }
        if self.busy.iter().any(|b| b == n) {
            return Ok(E::Sym(n.to_string()));
        }
        if let Some(def) = self.env.def(n) {
            // a function's parameters stay variables; other names resolve
            let locals = if def.explicit { def.params.iter().map(|p| (p.clone(), E::Sym(p.clone()))).collect() } else { Vec::new() };
            let saved = std::mem::replace(&mut self.locals, locals);
            self.busy.push(n.to_string());
            let r = self.eval(&def.body);
            self.busy.pop();
            self.locals = saved;
            return r;
        }
        Ok(match n {
            "pi" | "π" => num(std::f64::consts::PI),
            "e" => num(std::f64::consts::E),
            "i" => E::Num(Num::I),
            n if currency::is_code(n) => {
                self.env.currency.set(true);
                currency::unit(n)
            }
            n => E::Sym(n.to_string()),
        })
    }

    fn call(&mut self, n: &str, args: &[Arg]) -> R<E> {
        if self.env.def(n).is_none() && SPECIAL.contains(&n) {
            return self.special(n, args);
        }
        if self.env.def(n).is_some() {
            let mut positional = Vec::new();
            let mut named = Vec::new();
            for a in args {
                let v = self.eval(&a.value)?;
                match &a.name {
                    Some(k) => named.push((k.clone(), v)),
                    None => positional.push(v),
                }
            }
            return self.call_user(n, positional, named);
        }
        if args.iter().any(|a| a.name.is_some()) {
            return Err(format!("{n} has no named parameters"));
        }
        let vals = args.iter().map(|a| self.eval(&a.value)).collect::<R<Vec<_>>>()?;
        if is_builtin(n) {
            return builtins::call(n, vals);
        }
        Ok(E::Fn(n.to_string(), vals))
    }

    fn call_user(&mut self, n: &str, positional: Vec<E>, named: Vec<(String, E)>) -> R<E> {
        let def = self.env.def(n).unwrap();
        if positional.len() > def.params.len() {
            return Err(format!("{n} takes {} argument{}", def.params.len(), if def.params.len() == 1 { "" } else { "s" }));
        }
        let mut locals: Vec<(String, E)> = def.params.iter().cloned().zip(positional).collect();
        for (k, v) in named {
            if !def.params.contains(&k) {
                return Err(format!("{n} has no parameter {k}"));
            }
            locals.retain(|(p, _)| *p != k);
            locals.push((k, v));
        }
        if def.explicit {
            for p in &def.params {
                if !locals.iter().any(|(k, _)| k == p) {
                    locals.insert(0, (p.clone(), E::Sym(p.clone())));
                }
            }
        }
        let saved = std::mem::replace(&mut self.locals, locals);
        let r = self.eval(&def.body);
        self.locals = saved;
        r
    }

    fn fn_val<'a>(&self, arg: &'a Ast, arity: usize) -> FnVal<'a> {
        if let Ast::Name(n) = arg {
            if self.local(n).is_none() {
                if self.env.def(n).is_some_and(|d| !d.params.is_empty()) {
                    return FnVal::User(n.clone());
                }
                if is_builtin(n) {
                    return FnVal::Builtin(n.clone());
                }
                if self.is_free(n) {
                    return FnVal::Unknown(n.clone());
                }
            }
        }
        let mut params = self.free_names(arg);
        params.truncate(arity);
        FnVal::Lambda(arg, params)
    }

    fn apply(&mut self, f: &FnVal, args: Vec<E>) -> R<E> {
        match f {
            FnVal::User(n) => self.call_user(n, args, Vec::new()),
            FnVal::Builtin(n) => builtins::call(n, args),
            FnVal::Unknown(n) => Ok(E::Fn(n.clone(), args)),
            FnVal::Lambda(ast, params) => {
                let k = self.locals.len();
                self.locals.extend(params.iter().cloned().zip(args));
                let r = self.eval(ast);
                self.locals.truncate(k);
                r
            }
        }
    }

    fn list(&mut self, ast: &Ast) -> R<Mat> {
        let v = self.eval(ast)?;
        as_mat(&v).ok_or_else(|| "expected a list or a range".to_string())
    }

    /// The expression a function argument stands for, and its variable:
    /// a definition's body, or the expression itself.
    fn target(&mut self, arg: &Ast) -> R<(E, Option<String>)> {
        if let Ast::Name(n) = arg {
            if let Some(def) = self.env.def(n) {
                let var = if def.explicit { def.params.first().cloned() } else { self.free_names(&def.body).into_iter().next() };
                return Ok((self.name(n)?, var));
            }
        }
        let var = self.free_names(arg).into_iter().next();
        Ok((self.eval(arg)?, var))
    }

    fn var_arg(arg: &Arg) -> R<String> {
        match (&arg.name, &arg.value) {
            (None, Ast::Name(v)) => Ok(v.clone()),
            _ => Err("expected a variable name".into()),
        }
    }

    fn special(&mut self, n: &str, args: &[Arg]) -> R<E> {
        match n {
            "map" => self.map(args),
            "reduce" => self.reduce(args),
            "sum" | "prod" => self.sum_prod(n == "sum", args),
            "der" => {
                let first = args.first().ok_or("der needs a function")?;
                let (e, default) = self.target(&first.value)?;
                let v = match args.get(1) {
                    Some(a) => Self::var_arg(a)?,
                    None => default.ok_or("der needs a variable")?,
                };
                let k = match args.get(2) {
                    Some(a) => self.eval(&a.value)?.as_int().ok_or("the derivative order must be a whole number")?,
                    None => 1,
                };
                der_n(&e, &v, k)
            }
            "taylor" => {
                let first = args.first().ok_or("taylor needs a function")?;
                let (e, default) = self.target(&first.value)?;
                let (v, at) = match args.get(1) {
                    Some(Arg { name: Some(v), value }) => (v.clone(), self.eval(value)?),
                    Some(a) => (Self::var_arg(a)?, num(0.0)),
                    None => (default.ok_or("taylor needs a variable")?, num(0.0)),
                };
                let order = match args.get(2) {
                    Some(a) => self.eval(&a.value)?.as_int().ok_or("the order must be a whole number")?,
                    None => 3,
                };
                if !(0..=30).contains(&order) {
                    return Err("the order must be between 0 and 30".into());
                }
                let mut terms = Vec::new();
                let mut d = e;
                let mut factorial = 1.0;
                let offset = sub(E::Sym(v.clone()), at.clone())?;
                for k in 0..=order {
                    if k > 0 {
                        d = der(&d, &v)?;
                        factorial *= k as f64;
                    }
                    let value = subst(&d, &v, &at)?;
                    terms.push(mul(vec![value, num(1.0 / factorial), pow(offset.clone(), num(k as f64))?])?);
                }
                add(terms)
            }
            "jacobian" => {
                let mut funcs: Vec<E> = Vec::new();
                let mut vars: Vec<String> = Vec::new();
                let mut free: Vec<String> = Vec::new();
                for (k, a) in args.iter().enumerate() {
                    if k > 0 && (a.name.is_none() && matches!(&a.value, Ast::Name(v) if self.is_free(v)) || !vars.is_empty()) {
                        vars.push(Self::var_arg(a)?);
                        continue;
                    }
                    for v in self.free_names(&a.value) {
                        if !free.contains(&v) {
                            free.push(v);
                        }
                    }
                    match self.eval(&a.value)? {
                        E::Mat(m) => funcs.extend(m.data),
                        e => funcs.push(e),
                    }
                }
                if vars.is_empty() {
                    vars = free;
                }
                let mut data = Vec::with_capacity(funcs.len() * vars.len());
                for f in &funcs {
                    for v in &vars {
                        data.push(der(f, v)?);
                    }
                }
                Ok(E::Mat(Mat::new(funcs.len(), vars.len(), data)))
            }
            _ => unreachable!(),
        }
    }

    fn map(&mut self, args: &[Arg]) -> R<E> {
        let (first, rest) = args.split_first().ok_or("map needs a function and a list")?;
        if rest.is_empty() {
            return Err("map needs a list".into());
        }
        let named: Vec<String> = rest.iter().filter_map(|a| a.name.clone()).collect();
        let lists = rest.iter().map(|a| self.list(&a.value)).collect::<R<Vec<_>>>()?;
        let f = if named.len() == rest.len() {
            match self.fn_val(&first.value, rest.len()) {
                FnVal::Lambda(ast, _) => FnVal::Lambda(ast, named),
                f => f,
            }
        } else {
            self.fn_val(&first.value, rest.len())
        };
        let shape = &lists[0];
        let mut data = Vec::with_capacity(shape.data.len());
        for k in 0..shape.data.len() {
            let elems = lists.iter().map(|l| l.data.get(k).cloned().ok_or("the lists have different lengths")).collect::<Result<Vec<_>, _>>()?;
            data.push(self.apply(&f, elems)?);
        }
        Ok(E::Mat(Mat::new(shape.rows, shape.cols, data)))
    }

    fn reduce(&mut self, args: &[Arg]) -> R<E> {
        if args.len() < 2 {
            return Err("reduce needs a function and a list".into());
        }
        let f = self.fn_val(&args[0].value, 2);
        let items = self.list(&args[1].value)?.data;
        let mut items = items.into_iter();
        let mut acc = match args.get(2) {
            Some(a) => self.eval(&a.value)?,
            None => items.next().ok_or("reduce needs a list that is not empty")?,
        };
        for x in items {
            acc = self.apply(&f, vec![acc, x])?;
        }
        Ok(acc)
    }

    fn sum_prod(&mut self, is_sum: bool, args: &[Arg]) -> R<E> {
        let what = if is_sum { "sum" } else { "prod" };
        if args.len() != 2 {
            return Err(format!("{what} needs a function and a range"));
        }
        let (f_arg, spec) = (&args[0].value, &args[1]);
        let values = self.list(&spec.value)?.data;
        if values.len() > MAX_ELEMENTS {
            return Err("the range is too long".into());
        }
        let mut terms = Vec::with_capacity(values.len());
        match &spec.name {
            Some(v) => {
                // sum(sq, x=1:5): the function's body, summed over x
                let body = match f_arg {
                    Ast::Name(n) => self.env.def(n).map(|d| d.body.clone()),
                    _ => None,
                };
                let body = body.as_ref().unwrap_or(f_arg);
                for x in values {
                    let k = self.locals.len();
                    self.locals.push((v.clone(), x));
                    let r = self.eval(body);
                    self.locals.truncate(k);
                    terms.push(r?);
                }
            }
            None => {
                let f = self.fn_val(f_arg, 1);
                for x in values {
                    terms.push(match &f {
                        // sum(x, c): x is the variable itself
                        FnVal::Unknown(_) => x,
                        f => self.apply(f, vec![x])?,
                    });
                }
            }
        }
        if is_sum { add(terms) } else { mul(terms) }
    }

    /// `M*x = B` (or `x*M = B`) for a matrix `M`: x is M^-1 B.
    fn matrix_solve(&self, l: &Ast, r: &Ast, v: &str) -> R<Option<E>> {
        let (l, r) = if mentions(l, v) { (l, r) } else { (r, l) };
        let Ast::Bin(Op::Mul, a, b) = l else { return Ok(None) };
        let mut ctx = Ctx::new(self.env);
        let (other, left) = match (&**a, &**b) {
            (m, Ast::Name(x)) if x == v && !mentions(m, v) => (m, true),
            (Ast::Name(x), m) if x == v && !mentions(m, v) => (m, false),
            _ => return Ok(None),
        };
        let (E::Mat(m), E::Mat(rhs)) = (ctx.eval(other)?, ctx.eval(r)?) else { return Ok(None) };
        let inv = matrix::inverse(&m)?;
        let x = if left { expr::matmul(&inv, &rhs)? } else { expr::matmul(&rhs, &inv)? };
        Ok(Some(E::Mat(x)))
    }
}

/// A matrix from evaluated rows; `[[1, 2], [3, 4]]` also works.
fn matrix_literal(rows: Vec<Vec<E>>) -> R<E> {
    if rows.len() == 1 && rows[0].iter().all(|x| matches!(x, E::Mat(m) if m.rows == 1)) && rows[0].len() > 1 {
        let inner: Vec<Mat> = rows[0].iter().map(|x| if let E::Mat(m) = x { m.clone() } else { unreachable!() }).collect();
        let cols = inner[0].cols;
        if inner.iter().all(|m| m.cols == cols) {
            let n = inner.len();
            return Ok(E::Mat(Mat::new(n, cols, inner.into_iter().flat_map(|m| m.data).collect())));
        }
    }
    let cols = rows[0].len();
    if rows.iter().any(|r| r.len() != cols) {
        return Err("the rows of the matrix have different lengths".into());
    }
    if rows.iter().flatten().any(|x| matches!(x, E::Mat(_) | E::Range(..))) {
        return Err("a matrix cannot hold matrices".into());
    }
    let n = rows.len();
    Ok(E::Mat(Mat::new(n, cols, rows.into_iter().flatten().collect())))
}

fn indices(e: &E, len: usize) -> R<Vec<usize>> {
    let check = |k: i64| -> R<usize> {
        if k < 0 || k as usize >= len {
            return Err(format!("index {k} is out of range"));
        }
        Ok(k as usize)
    };
    match e {
        E::Range(a, b) => range_mat(*a, *b)?.data.iter().map(|x| check(x.as_int().unwrap())).collect(),
        e => Ok(vec![check(e.as_int().ok_or("an index must be a whole number")?)?]),
    }
}

/// `m[i]` (column-major, from 0) and `m[row, col]`, with ranges for parts.
fn index(base: &E, idx: &[E]) -> R<E> {
    let m = as_mat(base).ok_or("only a matrix can be indexed")?;
    match idx {
        [i] => {
            let flat = m.column_major();
            let ks = indices(i, flat.len())?;
            if ks.len() == 1 && !matches!(i, E::Range(..)) {
                return Ok(flat[ks[0]].clone());
            }
            Ok(E::Mat(Mat::new(1, ks.len(), ks.into_iter().map(|k| flat[k].clone()).collect())))
        }
        [r, c] => {
            let rs = indices(r, m.rows)?;
            let cs = indices(c, m.cols)?;
            if rs.len() == 1 && cs.len() == 1 && !matches!(r, E::Range(..)) && !matches!(c, E::Range(..)) {
                return Ok(m.get(rs[0], cs[0]).clone());
            }
            let mut data = Vec::with_capacity(rs.len() * cs.len());
            for &r in &rs {
                for &c in &cs {
                    data.push(m.get(r, c).clone());
                }
            }
            Ok(E::Mat(Mat::new(rs.len(), cs.len(), data)))
        }
        _ => Err("use m[index] or m[row, column]".into()),
    }
}
