//! Derivatives (`der`), from which Taylor series and Jacobians are built.

use crate::builtins::{call, less_than_zero};
use crate::expr::{add, mul, neg, num, pow, sub, E, R};

fn ln(x: E) -> R<E> {
    call("ln", vec![x])
}

/// The derivative of `e` with respect to the variable `v`.
pub fn der(e: &E, v: &str) -> R<E> {
    if !e.mentions(v) {
        return Ok(num(0.0));
    }
    match e {
        E::Sym(_) => Ok(num(1.0)),
        E::Add(ts) => add(ts.iter().map(|t| der(t, v)).collect::<R<_>>()?),
        E::Mul(fs) => {
            let mut terms = Vec::new();
            for i in 0..fs.len() {
                if !fs[i].mentions(v) {
                    continue;
                }
                let mut f = fs.clone();
                f[i] = der(&fs[i], v)?;
                terms.push(mul(f)?);
            }
            add(terms)
        }
        E::Pow(b, x) => {
            let (b, x) = ((**b).clone(), (**x).clone());
            if !x.mentions(v) {
                mul(vec![x.clone(), pow(b.clone(), add(vec![x, num(-1.0)])?)?, der(&b, v)?])
            } else if !b.mentions(v) {
                mul(vec![e.clone(), ln(b)?, der(&x, v)?])
            } else {
                // d(b^x) = b^x (x' ln b + x b'/b)
                let left = mul(vec![der(&x, v)?, ln(b.clone())?])?;
                let right = mul(vec![x, der(&b, v)?, pow(b, num(-1.0))?])?;
                mul(vec![e.clone(), add(vec![left, right])?])
            }
        }
        E::Fn(name, args) => fn_der(e, name, args, v),
        E::If(c, a, b) => Ok(E::If(c.clone(), Box::new(der(a, v)?), Box::new(der(b, v)?))),
        E::Mat(m) => Ok(E::Mat(m.map(|x| der(x, v))?)),
        _ => Err("this has no derivative".into()),
    }
}

fn fn_der(e: &E, name: &str, args: &[E], v: &str) -> R<E> {
    let unknown = || Ok(E::Fn("der".into(), vec![e.clone(), E::Sym(v.into())]));
    if name == "log" && args.len() == 2 && !args[1].mentions(v) {
        let u = args[0].clone();
        return mul(vec![der(&u, v)?, pow(mul(vec![u, ln(args[1].clone())?])?, num(-1.0))?]);
    }
    if name == "atan2" && args.len() == 2 {
        let (y, x) = (args[0].clone(), args[1].clone());
        let top = sub(mul(vec![x.clone(), der(&y, v)?])?, mul(vec![y.clone(), der(&x, v)?])?)?;
        let bottom = add(vec![pow(x, num(2.0))?, pow(y, num(2.0))?])?;
        return mul(vec![top, pow(bottom, num(-1.0))?]);
    }
    if args.len() != 1 {
        return unknown();
    }
    let u = args[0].clone();
    let du = der(&u, v)?;
    let f = |n: &str, x: E| call(n, vec![x]);
    let sq = |x: E| pow(x, num(2.0));
    let inner = match name {
        "sin" => f("cos", u)?,
        "cos" => neg(f("sin", u)?)?,
        "tan" => pow(f("cos", u)?, num(-2.0))?,
        "asin" => pow(sub(num(1.0), sq(u)?)?, num(-0.5))?,
        "acos" => neg(pow(sub(num(1.0), sq(u)?)?, num(-0.5))?)?,
        "atan" => pow(add(vec![num(1.0), sq(u)?])?, num(-1.0))?,
        "sinh" => f("cosh", u)?,
        "cosh" => f("sinh", u)?,
        "tanh" => sub(num(1.0), sq(f("tanh", u)?)?)?,
        "ln" => pow(u, num(-1.0))?,
        "log10" => pow(mul(vec![u, ln(num(10.0))?])?, num(-1.0))?,
        "log2" => pow(mul(vec![u, ln(num(2.0))?])?, num(-1.0))?,
        "exp" => f("exp", u)?,
        "conj" => num(1.0),
        "floor" | "ceil" | "round" | "truncate" | "sign" => num(0.0),
        // |u|' is -u' where u < 0, else u'
        "abs" => {
            return Ok(match less_than_zero(u)? {
                E::Bool(true) => neg(du)?,
                E::Bool(false) => du,
                c => E::If(Box::new(c), Box::new(neg(du.clone())?), Box::new(du)),
            });
        }
        _ => return unknown(),
    };
    mul(vec![inner, du])
}

/// The `n`th derivative.
pub fn der_n(e: &E, v: &str, n: i64) -> R<E> {
    if !(0..=50).contains(&n) {
        return Err("the derivative order must be between 0 and 50".into());
    }
    let mut out = e.clone();
    for _ in 0..n {
        out = der(&out, v)?;
    }
    Ok(out)
}
