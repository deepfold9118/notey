//! The standard library's ordinary functions (the special forms `map`,
//! `sum`, `der` and friends are in eval.rs), and substitution.

use crate::expr::{add, as_mat, mul, num, pow, CmpOp, Mat, E, R};
use crate::matrix;
use crate::num::Num;

pub const FUNCTIONS: [&str; 30] = [
    "abs", "acos", "asin", "atan", "atan2", "ceil", "conj", "cos", "cosh", "dot", "exp", "floor", "inv", "ln", "log",
    "log10", "log2", "max", "min", "mod", "round", "sin", "sinh", "sqrt", "tan", "tanh", "truncate", "det", "transpose",
    "sign",
];

pub fn is_builtin(name: &str) -> bool {
    FUNCTIONS.contains(&name)
}

/// Rounding noise of trig functions (sin(pi) is 1.2e-16).
fn tidy(x: f64) -> f64 {
    if x.abs() < 1e-15 { 0.0 } else { x }
}

fn real(x: f64) -> Option<Num> {
    x.is_finite().then(|| Num::real(tidy(x)))
}

/// Round half to even, as computers do: round(2.5) is 2.
fn round_even(x: f64) -> f64 {
    let f = x.floor();
    let d = x - f;
    if d > 0.5 || (d == 0.5 && f % 2.0 != 0.0) { f + 1.0 } else { f }
}

/// A one-argument function of a number; `None` keeps it unevaluated.
fn unary(name: &str, x: Num) -> Option<Num> {
    let r = x.is_real();
    let v = x.re;
    match name {
        "sin" if r => real(v.sin()),
        "sin" => Some(Num::complex(v.sin() * x.im.cosh(), v.cos() * x.im.sinh()).clean()),
        "cos" if r => real(v.cos()),
        "cos" => Some(Num::complex(v.cos() * x.im.cosh(), -(v.sin() * x.im.sinh())).clean()),
        "tan" if r => real(v.tan()),
        "asin" if r && v.abs() <= 1.0 => real(v.asin()),
        "acos" if r && v.abs() <= 1.0 => real(v.acos()),
        "atan" if r => real(v.atan()),
        "sinh" if r => real(v.sinh()),
        "cosh" if r => real(v.cosh()),
        "tanh" if r => real(v.tanh()),
        "exp" => Some(x.exp()),
        "ln" if !x.is_zero() => Some(x.ln()),
        "log10" if r && v > 0.0 => real(v.log10()),
        "log10" if !x.is_zero() => Some(x.ln().div(Num::real(10f64.ln()))),
        "log2" if r && v > 0.0 => real(v.log2()),
        "log2" if !x.is_zero() => Some(x.ln().div(Num::real(2f64.ln()))),
        "sqrt" => Some(x.sqrt()),
        "abs" => Some(Num::real(x.abs()).with_radix(x.radix)),
        "conj" => Some(x.conj()),
        "floor" if r => Some(Num::real(v.floor()).with_radix(x.radix)),
        "ceil" if r => Some(Num::real(v.ceil()).with_radix(x.radix)),
        "round" if r => Some(Num::real(round_even(v)).with_radix(x.radix)),
        "truncate" if r => Some(Num::real(v.trunc()).with_radix(x.radix)),
        "sign" if r => Some(Num::real(if v > 0.0 { 1.0 } else if v < 0.0 { -1.0 } else { 0.0 })),
        "inv" if !x.is_zero() => Some(x.inv()),
        _ => None,
    }
}

fn arity(name: &str, args: &[E], n: usize) -> R<()> {
    if args.len() != n {
        return Err(format!("{name} takes {n} argument{}", if n == 1 { "" } else { "s" }));
    }
    Ok(())
}

pub fn log_of(x: E, base: E) -> R<E> {
    if let (Some(a), Some(b)) = (x.as_num(), base.as_num()) {
        if a.is_zero() || b.is_zero() || b.is_one() {
            return Err("the logarithm is undefined".into());
        }
        return Ok(E::Num(a.ln().div(b.ln())));
    }
    match x {
        // logarithms split products and bring powers down
        E::Mul(fs) => add(fs.into_iter().map(|f| log_of(f, base.clone())).collect::<R<_>>()?),
        E::Pow(b, k) if k.as_num().is_some_and(|k| k.is_real()) => mul(vec![*k, log_of(*b, base)?]),
        x => Ok(match base.as_num() {
            Some(b) if b.is_e() => E::Fn("ln".into(), vec![x]),
            Some(b) if b.is_real() && b.re == 10.0 => E::Fn("log10".into(), vec![x]),
            Some(b) if b.is_real() && b.re == 2.0 => E::Fn("log2".into(), vec![x]),
            _ => E::Fn("log".into(), vec![x, base]),
        }),
    }
}

/// The complex conjugate; variables are taken to be real.
fn conj(e: &E) -> R<E> {
    Ok(match e {
        E::Num(n) => E::Num(n.conj()),
        E::Add(ts) => add(ts.iter().map(conj).collect::<R<_>>()?)?,
        E::Mul(fs) => mul(fs.iter().map(conj).collect::<R<_>>()?)?,
        E::Pow(b, x) => pow(conj(b)?, conj(x)?)?,
        E::Fn(n, args) => call(n, args.iter().map(conj).collect::<R<_>>()?)?,
        E::Mat(m) => E::Mat(m.map(conj)?),
        e => e.clone(),
    })
}

/// Apply a built-in function to evaluated arguments.
pub fn call(name: &str, args: Vec<E>) -> R<E> {
    match name {
        "log" => {
            if args.len() == 1 {
                return call("log10", args);
            }
            arity(name, &args, 2)?;
            let mut a = args.into_iter();
            return log_of(a.next().unwrap(), a.next().unwrap());
        }
        "atan2" => {
            arity(name, &args, 2)?;
            return match (args[0].as_num(), args[1].as_num()) {
                (Some(y), Some(x)) if y.is_real() && x.is_real() => Ok(E::Num(Num::real(y.re.atan2(x.re)))),
                _ => Ok(E::Fn(name.into(), args)),
            };
        }
        "mod" => {
            arity(name, &args, 2)?;
            return match (args[0].as_num(), args[1].as_num()) {
                (Some(a), Some(b)) if a.is_real() && b.is_real() && b.re != 0.0 => {
                    Ok(E::Num(Num::real(a.re.rem_euclid(b.re)).with_radix(a.radix)))
                }
                _ => Ok(E::Fn(name.into(), args)),
            };
        }
        "min" | "max" => {
            let items = match args.as_slice() {
                [one] => as_mat(one).map(|m| m.data).unwrap_or_else(|| args.clone()),
                _ => args.clone(),
            };
            let nums: Option<Vec<f64>> = items.iter().map(|x| x.as_num().filter(|n| n.is_real()).map(|n| n.re)).collect();
            return match nums {
                Some(v) if !v.is_empty() => {
                    let pick = if name == "min" { v.iter().cloned().fold(f64::INFINITY, f64::min) } else { v.iter().cloned().fold(f64::NEG_INFINITY, f64::max) };
                    Ok(E::Num(Num::real(pick)))
                }
                _ => Ok(E::Fn(name.into(), args)),
            };
        }
        "dot" => {
            arity(name, &args, 2)?;
            let (Some(a), Some(b)) = (as_mat(&args[0]), as_mat(&args[1])) else {
                return Ok(E::Fn(name.into(), args));
            };
            if a.data.len() != b.data.len() {
                return Err("dot needs two lists of the same length".into());
            }
            let terms = a.data.iter().zip(&b.data).map(|(x, y)| mul(vec![x.clone(), y.clone()])).collect::<R<_>>()?;
            return add(terms);
        }
        _ => {}
    }
    if !is_builtin(name) {
        return Ok(E::Fn(name.into(), args));
    }
    arity(name, &args, 1)?;
    let x = args.into_iter().next().unwrap();
    if let Some(m) = as_mat(&x) {
        return match name {
            "abs" | "det" if m.rows == m.cols => matrix::det(&m),
            "inv" => matrix::inverse(&m).map(E::Mat),
            "transpose" => Ok(E::Mat(m.transpose())),
            "det" => Err("only a square matrix has a determinant".into()),
            _ => Ok(E::Mat(m.map(|e| call(name, vec![e.clone()]))?)),
        };
    }
    if let Some(n) = x.as_num() {
        if let Some(r) = unary(name, n) {
            if !r.is_finite() {
                return Err(format!("{name} is undefined there"));
            }
            return Ok(E::Num(r));
        }
        if name == "inv" {
            return Err("division by zero".into());
        }
        return Ok(E::Fn(name.into(), vec![x]));
    }
    match name {
        "sqrt" => pow(x, num(0.5)),
        "inv" => pow(x, num(-1.0)),
        "exp" => pow(E::Num(Num::real(std::f64::consts::E)), x),
        "ln" => log_of(x, E::Num(Num::real(std::f64::consts::E))),
        "log10" => log_of(x, num(10.0)),
        "log2" => log_of(x, num(2.0)),
        "conj" => conj(&x),
        "det" | "transpose" => Ok(E::Fn(name.into(), vec![x])),
        "abs" => match x {
            E::Fn(n, a) if n == "abs" => Ok(E::Fn(n, a)),
            E::Mul(mut f) if matches!(f[0], E::Num(c) if c.is_real()) => {
                let E::Num(c) = f[0] else { unreachable!() };
                f.remove(0);
                let rest = if f.len() == 1 { f.pop().unwrap() } else { E::Mul(f) };
                mul(vec![E::Num(Num::real(c.re.abs())), E::Fn("abs".into(), vec![rest])])
            }
            x => Ok(E::Fn("abs".into(), vec![x])),
        },
        _ => Ok(E::Fn(name.into(), vec![x])),
    }
}

/// `e` with the variable `name` replaced by `value`, simplified again.
pub fn subst(e: &E, name: &str, value: &E) -> R<E> {
    if !e.mentions(name) {
        return Ok(e.clone());
    }
    let s = |x: &E| subst(x, name, value);
    Ok(match e {
        E::Sym(n) if n == name => value.clone(),
        E::Add(ts) => add(ts.iter().map(s).collect::<R<_>>()?)?,
        E::Mul(fs) => mul(fs.iter().map(s).collect::<R<_>>()?)?,
        E::Pow(b, x) => pow(s(b)?, s(x)?)?,
        E::Fn(n, args) => {
            let args = args.iter().map(s).collect::<R<Vec<_>>>()?;
            if n == "der" { E::Fn(n.clone(), args) } else { call(n, args)? }
        }
        E::Mat(m) => E::Mat(m.map(s)?),
        E::If(c, a, b) => {
            let c = s(c)?;
            match crate::expr::truth(&c) {
                Some(true) => s(a)?,
                Some(false) => s(b)?,
                None => E::If(Box::new(c), Box::new(s(a)?), Box::new(s(b)?)),
            }
        }
        E::Cmp(op, a, b) => crate::expr::cmp(*op, s(a)?, s(b)?)?,
        E::And(xs) => {
            let mut acc = E::Bool(true);
            for x in xs {
                acc = crate::expr::and(acc, s(x)?)?;
            }
            acc
        }
        E::Or(xs) => {
            let mut acc = E::Bool(false);
            for x in xs {
                acc = crate::expr::or(acc, s(x)?)?;
            }
            acc
        }
        E::Not(a) => crate::expr::not(s(a)?)?,
        e => e.clone(),
    })
}

/// `a < 0` and friends, for piecewise derivatives.
pub fn less_than_zero(a: E) -> R<E> {
    crate::expr::cmp(CmpOp::Lt, a, num(0.0))
}

pub fn identity(n: usize) -> E {
    E::Mat(Mat::identity(n))
}
