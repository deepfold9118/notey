//! Solving an equation for a variable: polynomials up to degree two
//! exactly (degree three and up numerically), a variable that occurs once
//! by undoing what is done to it, and otherwise the closest form.

use crate::builtins::{call, log_of};
use crate::expr::{add, expand, mul, neg, num, pow, sub, terms_of, CmpOp, Mat, E, R};
use crate::num::Num;

/// The coefficients of `e` as a polynomial in `v`, lowest degree first.
fn poly(e: &E, v: &str) -> Option<Vec<E>> {
    let mut coeffs: Vec<Vec<E>> = Vec::new();
    for t in terms_of(e) {
        let (deg, c) = term_degree(&t, v)?;
        if deg > 12 {
            return None;
        }
        if coeffs.len() <= deg {
            coeffs.resize(deg + 1, Vec::new());
        }
        coeffs[deg].push(c);
    }
    coeffs.into_iter().map(|c| add(c).ok()).collect()
}

fn term_degree(t: &E, v: &str) -> Option<(usize, E)> {
    if !t.mentions(v) {
        return Some((0, t.clone()));
    }
    let one = |f: &E| -> Option<usize> {
        match f {
            E::Sym(s) if s == v => Some(1),
            E::Pow(b, k) if matches!(&**b, E::Sym(s) if s == v) => k.as_int().filter(|k| *k >= 0).map(|k| k as usize),
            _ => None,
        }
    };
    match t {
        E::Mul(fs) => {
            let mut deg = 0;
            let mut rest = Vec::new();
            for f in fs {
                if f.mentions(v) {
                    deg += one(f)?;
                } else {
                    rest.push(f.clone());
                }
            }
            Some((deg, mul(rest).ok()?))
        }
        t => Some((one(t)?, num(1.0))),
    }
}

fn recip(x: E) -> R<E> {
    pow(x, num(-1.0))
}

/// Solve `l = r` for `v`.
pub fn solve(l: &E, r: &E, v: &str) -> R<E> {
    let d = expand(&sub(l.clone(), r.clone())?)?;
    if let Some(mut c) = poly(&d, v) {
        while c.len() > 1 && c.last().is_some_and(E::is_zero) {
            c.pop();
        }
        match c.len() - 1 {
            0 => return Err(format!("{v} cancels out")),
            1 => return mul(vec![neg(c[0].clone())?, recip(c[1].clone())?]),
            2 => {
                // a positive leading coefficient, as Calca writes the roots
                if crate::format::leads_negative(&c[2]) {
                    c = c.into_iter().map(neg).collect::<R<_>>()?;
                }
                return quadratic(&c[2], &c[1], &c[0]);
            }
            _ => {
                if let Some(nums) = c.iter().map(E::as_num).collect::<Option<Vec<_>>>() {
                    let roots = numeric_roots(&nums);
                    return Ok(E::Mat(Mat::new(1, roots.len(), roots.into_iter().map(E::Num).collect())));
                }
            }
        }
    }
    if l.count(v) + r.count(v) == 1 {
        let (target, other) = if l.mentions(v) { (l.clone(), r.clone()) } else { (r.clone(), l.clone()) };
        if let Ok(x) = isolate(target, other, v) {
            return Ok(x);
        }
    }
    // closest form: terms with v on the left, the rest on the right
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for t in terms_of(&d) {
        if t.mentions(v) {
            left.push(t);
        } else {
            right.push(neg(t)?);
        }
    }
    Ok(E::Cmp(CmpOp::Eq, Box::new(add(left)?), Box::new(add(right)?)))
}

fn quadratic(a: &E, b: &E, c: &E) -> R<E> {
    let disc = add(vec![pow(b.clone(), num(2.0))?, mul(vec![num(-4.0), a.clone(), c.clone()])?])?;
    let over = recip(mul(vec![num(2.0), a.clone()])?)?;
    let nb = neg(b.clone())?;
    if disc.is_zero() {
        return mul(vec![nb, over]);
    }
    let root = pow(disc, num(0.5))?;
    let x1 = mul(vec![add(vec![nb.clone(), root.clone()])?, over.clone()])?;
    let x2 = mul(vec![add(vec![nb, neg(root)?])?, over])?;
    Ok(E::Mat(Mat::new(1, 2, vec![x1, x2])))
}

/// All roots of a numeric polynomial (coefficients lowest degree first),
/// by the Durand-Kerner method.
fn numeric_roots(c: &[Num]) -> Vec<Num> {
    let n = c.len() - 1;
    let lead = c[n];
    let monic: Vec<Num> = c.iter().map(|x| x.div(lead)).collect();
    let eval = |z: Num| monic.iter().rev().fold(Num::ZERO, |acc, k| acc.mul(z).add(*k));
    let seed = Num::complex(0.4, 0.9);
    let mut roots: Vec<Num> = (0..n).map(|k| seed.pow(Num::real(k as f64))).collect();
    for _ in 0..1000 {
        let mut moved = 0f64;
        for i in 0..n {
            let mut den = Num::ONE;
            for j in 0..n {
                if i != j {
                    den = den.mul(roots[i].add(roots[j].neg()));
                }
            }
            let step = eval(roots[i]).div(den);
            roots[i] = roots[i].add(step.neg());
            moved = moved.max(step.abs());
        }
        if moved < 1e-14 {
            break;
        }
    }
    let mut roots: Vec<Num> = roots
        .into_iter()
        .map(|z| {
            let im = if z.im.abs() < 1e-9 * z.re.abs().max(1.0) { 0.0 } else { z.im };
            Num::complex(z.re, im)
        })
        .collect();
    roots.sort_by(|a, b| a.re.total_cmp(&b.re).then(a.im.total_cmp(&b.im)));
    roots
}

/// Undo, one step at a time, what is done to the only occurrence of `v`.
fn isolate(mut target: E, mut other: E, v: &str) -> R<E> {
    for _ in 0..64 {
        target = match target {
            E::Sym(s) if s == v => return Ok(other),
            E::Add(ts) => {
                let (inside, rest): (Vec<E>, Vec<E>) = ts.into_iter().partition(|t| t.mentions(v));
                other = sub(other, add(rest)?)?;
                inside.into_iter().next().ok_or("lost the variable")?
            }
            E::Mul(fs) => {
                let (inside, rest): (Vec<E>, Vec<E>) = fs.into_iter().partition(|t| t.mentions(v));
                other = mul(vec![other, recip(mul(rest)?)?])?;
                inside.into_iter().next().ok_or("lost the variable")?
            }
            E::Pow(b, x) if b.mentions(v) => {
                other = pow(other, recip(*x)?)?;
                *b
            }
            E::Pow(b, x) => {
                other = log_of(other, *b)?;
                *x
            }
            E::Fn(name, mut args) if args.len() == 1 => {
                let inverse = match name.as_str() {
                    "sin" => "asin",
                    "cos" => "acos",
                    "tan" => "atan",
                    "asin" => "sin",
                    "acos" => "cos",
                    "atan" => "tan",
                    "ln" => "exp",
                    "exp" => "ln",
                    "log10" => {
                        other = pow(num(10.0), other)?;
                        target = args.pop().unwrap();
                        continue;
                    }
                    "log2" => {
                        other = pow(num(2.0), other)?;
                        target = args.pop().unwrap();
                        continue;
                    }
                    _ => return Err("cannot undo this function".into()),
                };
                other = call(inverse, vec![other])?;
                args.pop().unwrap()
            }
            E::Fn(name, mut args) if name == "log" && args.len() == 2 && !args[1].mentions(v) => {
                let base = args.pop().unwrap();
                other = pow(base, other)?;
                args.pop().unwrap()
            }
            _ => return Err("cannot isolate the variable".into()),
        };
    }
    Err("cannot isolate the variable".into())
}
