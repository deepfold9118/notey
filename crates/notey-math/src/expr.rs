//! Symbolic expressions and the constructors that keep them simplified:
//! sums collect like terms, products collect powers of the same base, and
//! a number or monomial times a sum is multiplied out (Calca shows
//! `y * (1 + 10%)` as `1.1y`).

use crate::format::{factor_order, key, term_order};
use crate::matrix;
use crate::num::Num;

pub type R<T> = Result<T, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub fn symbol(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
        }
    }
    /// The operator with its sides swapped (a < b is b > a).
    pub fn flip(self) -> CmpOp {
        match self {
            CmpOp::Lt => CmpOp::Gt,
            CmpOp::Le => CmpOp::Ge,
            CmpOp::Gt => CmpOp::Lt,
            CmpOp::Ge => CmpOp::Le,
            op => op,
        }
    }
    /// The opposite test (!(a < b) is a >= b).
    pub fn negate(self) -> CmpOp {
        match self {
            CmpOp::Eq => CmpOp::Ne,
            CmpOp::Ne => CmpOp::Eq,
            CmpOp::Lt => CmpOp::Ge,
            CmpOp::Le => CmpOp::Gt,
            CmpOp::Gt => CmpOp::Le,
            CmpOp::Ge => CmpOp::Lt,
        }
    }
    fn test(self, a: f64, b: f64) -> bool {
        match self {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::Ge => a >= b,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum E {
    Num(Num),
    Bool(bool),
    Sym(String),
    Add(Vec<E>),
    /// A product; a numeric coefficient, if any, comes first.
    Mul(Vec<E>),
    Pow(Box<E>, Box<E>),
    /// A function that could not be evaluated further: sin(x), f(x), |x|.
    Fn(String, Vec<E>),
    Mat(Mat),
    Range(i64, i64),
    If(Box<E>, Box<E>, Box<E>),
    Cmp(CmpOp, Box<E>, Box<E>),
    And(Vec<E>),
    Or(Vec<E>),
    Not(Box<E>),
}

/// A matrix, row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct Mat {
    pub rows: usize,
    pub cols: usize,
    pub data: Vec<E>,
}

/// The most elements a range or matrix may have.
pub const MAX_ELEMENTS: usize = 100_000;
/// The most terms an expansion may make.
const MAX_TERMS: usize = 5_000;

impl Mat {
    pub fn new(rows: usize, cols: usize, data: Vec<E>) -> Mat {
        Mat { rows, cols, data }
    }
    pub fn get(&self, r: usize, c: usize) -> &E {
        &self.data[r * self.cols + c]
    }
    pub fn map(&self, mut f: impl FnMut(&E) -> R<E>) -> R<Mat> {
        Ok(Mat { rows: self.rows, cols: self.cols, data: self.data.iter().map(&mut f).collect::<R<_>>()? })
    }
    pub fn transpose(&self) -> Mat {
        let mut data = Vec::with_capacity(self.data.len());
        for c in 0..self.cols {
            for r in 0..self.rows {
                data.push(self.get(r, c).clone());
            }
        }
        Mat { rows: self.cols, cols: self.rows, data }
    }
    pub fn identity(n: usize) -> Mat {
        let data = (0..n * n).map(|k| num(if k / n == k % n { 1.0 } else { 0.0 })).collect();
        Mat { rows: n, cols: n, data }
    }
    /// Elements in column-major order, the order of `m[i]`.
    pub fn column_major(&self) -> Vec<E> {
        self.transpose().data
    }
}

pub fn num(x: f64) -> E {
    E::Num(Num::real(x))
}

impl E {
    pub fn as_num(&self) -> Option<Num> {
        match self {
            E::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        self.as_num().and_then(|n| n.as_int())
    }
    pub fn is_zero(&self) -> bool {
        self.as_num().is_some_and(|n| n.is_zero())
    }
    pub fn is_one(&self) -> bool {
        self.as_num().is_some_and(|n| n.is_one())
    }
    pub fn key(&self) -> String {
        key(self)
    }
    /// A value with no unknowns: numbers, truth values, and matrices and
    /// ranges of them, or an amount of money.
    pub fn is_concrete(&self) -> bool {
        match self {
            E::Num(_) | E::Bool(_) | E::Range(..) => true,
            E::Mat(m) => m.data.iter().all(E::is_concrete),
            E::Mul(f) => f.len() == 2 && matches!(f[0], E::Num(_)) && crate::currency::is_currency(&f[1]),
            E::Sym(_) => crate::currency::is_currency(self),
            E::Add(t) => t.iter().all(E::is_concrete),
            _ => false,
        }
    }
    /// Whether the variable `v` occurs.
    pub fn mentions(&self, v: &str) -> bool {
        self.count(v) > 0
    }
    /// How often the variable `v` occurs.
    pub fn count(&self, v: &str) -> usize {
        match self {
            E::Sym(s) => usize::from(s == v),
            E::Num(_) | E::Bool(_) | E::Range(..) => 0,
            E::Add(xs) | E::Mul(xs) | E::Fn(_, xs) | E::And(xs) | E::Or(xs) => xs.iter().map(|x| x.count(v)).sum(),
            E::Pow(a, b) | E::Cmp(_, a, b) => a.count(v) + b.count(v),
            E::Mat(m) => m.data.iter().map(|x| x.count(v)).sum(),
            E::If(c, a, b) => c.count(v) + a.count(v) + b.count(v),
            E::Not(a) => a.count(v),
        }
    }
    /// The variables, in first-occurrence order.
    pub fn symbols(&self, out: &mut Vec<String>) {
        match self {
            E::Sym(s) => {
                if !out.contains(s) {
                    out.push(s.clone());
                }
            }
            E::Num(_) | E::Bool(_) | E::Range(..) => {}
            E::Add(xs) | E::Mul(xs) | E::Fn(_, xs) | E::And(xs) | E::Or(xs) => xs.iter().for_each(|x| x.symbols(out)),
            E::Pow(a, b) | E::Cmp(_, a, b) => {
                a.symbols(out);
                b.symbols(out);
            }
            E::Mat(m) => m.data.iter().for_each(|x| x.symbols(out)),
            E::If(c, a, b) => {
                c.symbols(out);
                a.symbols(out);
                b.symbols(out);
            }
            E::Not(a) => a.symbols(out),
        }
    }
    fn is_logic(&self) -> bool {
        matches!(self, E::Bool(_) | E::Cmp(..) | E::And(_) | E::Or(_) | E::Not(_))
    }
}

pub fn range_mat(a: i64, b: i64) -> R<Mat> {
    let len = (a - b).unsigned_abs() as usize + 1;
    if len > MAX_ELEMENTS {
        return Err("the range is too long".into());
    }
    let data: Vec<E> = if a <= b { (a..=b).map(|k| num(k as f64)).collect() } else { (b..=a).rev().map(|k| num(k as f64)).collect() };
    Ok(Mat { rows: 1, cols: len, data })
}

/// A matrix from a matrix or range value.
pub fn as_mat(e: &E) -> Option<Mat> {
    match e {
        E::Mat(m) => Some(m.clone()),
        E::Range(a, b) => range_mat(*a, *b).ok(),
        _ => None,
    }
}

fn no_logic(xs: &[E], what: &str) -> R<()> {
    if xs.iter().any(E::is_logic) {
        return Err(format!("cannot {what} a truth value"));
    }
    Ok(())
}

/// Split a term into its numeric coefficient and the rest.
pub fn split_coeff(t: E) -> (Num, E) {
    match t {
        E::Num(n) => (n, num(1.0)),
        E::Mul(mut f) if matches!(f.first(), Some(E::Num(_))) => {
            let E::Num(c) = f.remove(0) else { unreachable!() };
            let rest = if f.len() == 1 { f.pop().unwrap() } else { E::Mul(f) };
            (c, rest)
        }
        t => (Num::ONE, t),
    }
}

/// `c * rest` without multiplying out again.
pub fn with_coeff(c: Num, rest: E) -> E {
    if c.is_one() {
        return rest;
    }
    match rest {
        E::Num(n) => E::Num(c.mul(n)),
        E::Mul(mut f) => {
            f.insert(0, E::Num(c));
            E::Mul(f)
        }
        rest => E::Mul(vec![E::Num(c), rest]),
    }
}

pub fn add(terms: Vec<E>) -> R<E> {
    let mut flat = Vec::with_capacity(terms.len());
    for t in terms {
        match t {
            E::Add(v) => flat.extend(v),
            E::Range(a, b) => flat.push(E::Mat(range_mat(a, b)?)),
            t => flat.push(t),
        }
    }
    if flat.len() > MAX_TERMS {
        return Err("the expression is too large".into());
    }
    if flat.iter().any(|t| matches!(t, E::Mat(_))) {
        return elementwise(flat, add);
    }
    no_logic(&flat, "add")?;
    let mut constant = Num::ZERO;
    let mut scale = 0f64;
    let mut have_constant = false;
    // (key of the rest, coefficient, largest coefficient added, rest)
    let mut groups: Vec<(String, Num, f64, E)> = Vec::new();
    for t in flat {
        if let E::Num(x) = t {
            constant = constant.add(x);
            scale = scale.max(x.abs());
            have_constant = true;
            continue;
        }
        let (c, rest) = split_coeff(t);
        let k = rest.key();
        match groups.iter_mut().find(|g| g.0 == k) {
            Some(g) => {
                g.1 = g.1.add(c);
                g.2 = g.2.max(c.abs());
            }
            None => groups.push((k, c, c.abs(), rest)),
        }
    }
    let mut out = Vec::new();
    for (_, c, big, rest) in groups {
        if c.is_zero() || c.abs() <= big * 1e-12 {
            continue;
        }
        out.push(with_coeff(c, rest));
    }
    if have_constant && constant.abs() <= scale * 1e-12 {
        constant = Num { re: 0.0, im: 0.0, radix: constant.radix };
    }
    if !constant.is_zero() || out.is_empty() {
        out.push(E::Num(constant.clean()));
    }
    if out.len() == 1 {
        return Ok(out.pop().unwrap());
    }
    out.sort_by(term_order);
    Ok(E::Add(out))
}

pub fn mul(factors: Vec<E>) -> R<E> {
    mul_depth(factors, 0)
}

fn mul_depth(factors: Vec<E>, depth: usize) -> R<E> {
    if depth > 12 {
        return Err("the expression is too complex".into());
    }
    let mut flat = Vec::with_capacity(factors.len());
    for f in factors {
        match f {
            E::Mul(v) => flat.extend(v),
            E::Range(a, b) => flat.push(E::Mat(range_mat(a, b)?)),
            f => flat.push(f),
        }
    }
    if flat.iter().any(|f| matches!(f, E::Mat(_))) {
        return mul_mats(flat);
    }
    no_logic(&flat, "multiply")?;
    let mut c = Num::ONE;
    // (key of the base, base, exponent)
    let mut bases: Vec<(String, E, E)> = Vec::new();
    for f in flat {
        match f {
            E::Num(x) => c = c.mul(x),
            f => {
                let (b, e) = match f {
                    E::Pow(b, e) => (*b, *e),
                    f => (f, num(1.0)),
                };
                let k = b.key();
                match bases.iter_mut().find(|g| g.0 == k) {
                    Some(g) => g.2 = add(vec![g.2.clone(), e])?,
                    None => bases.push((k, b, e)),
                }
            }
        }
    }
    if c.is_zero() {
        return Ok(E::Num(Num { re: 0.0, im: 0.0, radix: c.radix }));
    }
    let mut rebuilt = Vec::with_capacity(bases.len());
    let mut again = false;
    for (_, b, e) in bases {
        let p = if e.is_one() { b } else { pow(b, e)? };
        match p {
            E::Num(x) => c = c.mul(x),
            p @ E::Mul(_) => {
                again = true;
                rebuilt.push(p);
            }
            p => rebuilt.push(p),
        }
    }
    if again {
        rebuilt.insert(0, E::Num(c));
        return mul_depth(rebuilt, depth + 1);
    }
    let c = c.clean();
    let sums = rebuilt.iter().filter(|f| matches!(f, E::Add(_))).count();
    if sums == 1 && (rebuilt.len() > 1 || !c.is_one()) {
        let i = rebuilt.iter().position(|f| matches!(f, E::Add(_))).unwrap();
        let E::Add(terms) = rebuilt.remove(i) else { unreachable!() };
        let mut out = Vec::with_capacity(terms.len());
        for t in terms {
            let mut fs = rebuilt.clone();
            fs.push(t);
            fs.push(E::Num(c));
            out.push(mul_depth(fs, depth + 1)?);
        }
        return add(out);
    }
    if rebuilt.is_empty() {
        return Ok(E::Num(c));
    }
    rebuilt.sort_by(factor_order);
    if c.is_one() && rebuilt.len() == 1 {
        return Ok(rebuilt.pop().unwrap());
    }
    if !c.is_one() {
        rebuilt.insert(0, E::Num(c));
    }
    Ok(E::Mul(rebuilt))
}

/// Apply `f` element by element; plain values go with every element.
fn elementwise(items: Vec<E>, f: fn(Vec<E>) -> R<E>) -> R<E> {
    let shape = items.iter().find_map(|t| match t {
        E::Mat(m) => Some((m.rows, m.cols)),
        _ => None,
    });
    let Some((rows, cols)) = shape else { return f(items) };
    for t in &items {
        if let E::Mat(m) = t {
            if (m.rows, m.cols) != (rows, cols) {
                return Err(format!("the matrices are {}x{} and {}x{}", rows, cols, m.rows, m.cols));
            }
        }
    }
    let mut data = Vec::with_capacity(rows * cols);
    for k in 0..rows * cols {
        let args = items
            .iter()
            .map(|t| match t {
                E::Mat(m) => m.data[k].clone(),
                t => t.clone(),
            })
            .collect();
        data.push(f(args)?);
    }
    Ok(E::Mat(Mat { rows, cols, data }))
}

/// Products with matrices: matrix times matrix is matrix multiplication,
/// anything else scales every element.
fn mul_mats(flat: Vec<E>) -> R<E> {
    let mut acc = num(1.0);
    for f in flat {
        acc = match (acc, f) {
            (E::Mat(a), E::Mat(b)) => E::Mat(matmul(&a, &b)?),
            (E::Mat(a), s) => E::Mat(a.map(|x| mul(vec![x.clone(), s.clone()]))?),
            (s, E::Mat(b)) => E::Mat(b.map(|x| mul(vec![s.clone(), x.clone()]))?),
            (a, b) => mul(vec![a, b])?,
        };
    }
    Ok(acc)
}

pub fn matmul(a: &Mat, b: &Mat) -> R<Mat> {
    if a.cols != b.rows {
        return Err(format!("cannot multiply a {}x{} matrix by a {}x{} matrix", a.rows, a.cols, b.rows, b.cols));
    }
    let mut data = Vec::with_capacity(a.rows * b.cols);
    for r in 0..a.rows {
        for c in 0..b.cols {
            let terms = (0..a.cols).map(|k| mul(vec![a.get(r, k).clone(), b.get(k, c).clone()])).collect::<R<_>>()?;
            data.push(add(terms)?);
        }
    }
    Ok(Mat { rows: a.rows, cols: b.cols, data })
}

pub fn pow(b: E, e: E) -> R<E> {
    match (&b, &e) {
        (E::Range(x, y), _) => return pow(E::Mat(range_mat(*x, *y)?), e),
        (E::Mat(m), E::Sym(s)) if s == "T" => return Ok(E::Mat(m.transpose())),
        (E::Mat(m), E::Num(_)) => {
            let k = e.as_int().ok_or("a matrix power must be a whole number")?;
            return matrix::power(m, k).map(E::Mat);
        }
        (E::Mat(_), _) => return Err("a matrix power must be a whole number, or T".into()),
        (_, E::Mat(_)) => return elementwise(vec![b, e], |v| pow(v[0].clone(), v[1].clone())),
        (E::Num(x), E::Num(y)) => {
            let r = x.pow(*y);
            if !r.is_finite() {
                return Err("division by zero".into());
            }
            return Ok(E::Num(r));
        }
        _ => {}
    }
    no_logic(&[b.clone(), e.clone()], "raise")?;
    if let Some(y) = e.as_num() {
        if y.is_zero() {
            return Ok(num(1.0));
        }
        if y.is_one() {
            return Ok(b);
        }
    }
    if let Some(x) = b.as_num() {
        if x.is_one() {
            return Ok(num(1.0));
        }
        if x.is_zero() && e.as_num().is_some_and(|y| y.is_real() && y.re > 0.0) {
            return Ok(num(0.0));
        }
    }
    let whole = e.as_int().is_some();
    match b {
        E::Pow(bb, be) if whole => pow(*bb, mul(vec![*be, e])?),
        E::Mul(fs) if whole => mul(fs.into_iter().map(|f| pow(f, e.clone())).collect::<R<_>>()?),
        E::Mul(mut fs) if e.as_num().is_some_and(|y| y.is_real()) && fs[0].as_num().is_some_and(|c| c.is_real() && c.re > 0.0) => {
            let c = fs.remove(0);
            let rest = if fs.len() == 1 { fs.pop().unwrap() } else { E::Mul(fs) };
            mul(vec![pow(c, e.clone())?, E::Pow(Box::new(rest), Box::new(e))])
        }
        b => Ok(E::Pow(Box::new(b), Box::new(e))),
    }
}

pub fn neg(a: E) -> R<E> {
    mul(vec![num(-1.0), a])
}

pub fn sub(a: E, b: E) -> R<E> {
    add(vec![a, neg(b)?])
}

pub fn div(a: E, b: E) -> R<E> {
    if matches!(a, E::Mat(_) | E::Range(..)) || matches!(b, E::Mat(_) | E::Range(..)) {
        let a = as_mat(&a).map_or(a, E::Mat);
        let b = as_mat(&b).map_or(b, E::Mat);
        return elementwise(vec![a, b], |v| div(v[0].clone(), v[1].clone()));
    }
    if b.is_zero() {
        return Err("division by zero".into());
    }
    if let (Some(x), Some(y)) = (a.as_num(), b.as_num()) {
        return Ok(E::Num(x.div(y)));
    }
    mul(vec![a, pow(b, num(-1.0))?])
}

/// Whether a value counts as true: `None` when it is not known.
pub fn truth(e: &E) -> Option<bool> {
    match e {
        E::Bool(b) => Some(*b),
        E::Num(n) => Some(!n.is_zero()),
        _ => None,
    }
}

pub fn cmp(op: CmpOp, a: E, b: E) -> R<E> {
    if matches!(a, E::Mat(_)) || matches!(b, E::Mat(_)) {
        let f = move |v: Vec<E>| cmp(op, v[0].clone(), v[1].clone());
        return elementwise_with(vec![a, b], f);
    }
    match (&a, &b) {
        (E::Num(x), E::Num(y)) => {
            if x.is_real() && y.is_real() {
                return Ok(E::Bool(op.test(x.re, y.re)));
            }
            return match op {
                CmpOp::Eq => Ok(E::Bool(x.re == y.re && x.im == y.im)),
                CmpOp::Ne => Ok(E::Bool(x.re != y.re || x.im != y.im)),
                _ => Err("complex numbers have no order".into()),
            };
        }
        (E::Bool(x), E::Bool(y)) if matches!(op, CmpOp::Eq | CmpOp::Ne) => {
            return Ok(E::Bool((x == y) == (op == CmpOp::Eq)));
        }
        _ => {}
    }
    // a + c op k  ->  x op (k - c) / coefficient, as Calca shows if-conditions
    if let Some(k) = b.as_num().filter(|k| k.is_real()) {
        let (c, term) = match &a {
            E::Add(t) if t.len() == 2 && t[1].as_num().is_some_and(|c| c.is_real()) => (t[1].as_num().unwrap(), t[0].clone()),
            t => (Num::ZERO, t.clone()),
        };
        let (coeff, x) = split_coeff(term);
        if coeff.is_real() && !coeff.is_zero() && (!c.is_zero() || !coeff.is_one()) {
            let rhs = E::Num(k.add(c.neg()).div(coeff));
            let op = if coeff.re < 0.0 { op.flip() } else { op };
            return Ok(E::Cmp(op, Box::new(x), Box::new(rhs)));
        }
    }
    Ok(E::Cmp(op, Box::new(a), Box::new(b)))
}

fn elementwise_with(items: Vec<E>, f: impl Fn(Vec<E>) -> R<E>) -> R<E> {
    let (rows, cols) = items
        .iter()
        .find_map(|t| match t {
            E::Mat(m) => Some((m.rows, m.cols)),
            _ => None,
        })
        .unwrap();
    let mut data = Vec::with_capacity(rows * cols);
    for k in 0..rows * cols {
        let mut args = Vec::with_capacity(items.len());
        for t in &items {
            args.push(match t {
                E::Mat(m) if (m.rows, m.cols) == (rows, cols) => m.data[k].clone(),
                E::Mat(m) => return Err(format!("the matrices are {}x{} and {}x{}", rows, cols, m.rows, m.cols)),
                t => t.clone(),
            });
        }
        data.push(f(args)?);
    }
    Ok(E::Mat(Mat { rows, cols, data }))
}

/// `a && b`: like Calca, a false first value is the result, else the second.
pub fn and(a: E, b: E) -> R<E> {
    match truth(&a) {
        Some(false) => Ok(a),
        Some(true) => Ok(b),
        None => match truth(&b) {
            Some(true) if matches!(b, E::Bool(_)) => Ok(a),
            Some(false) if matches!(b, E::Bool(_)) => Ok(b),
            _ => {
                let mut v = Vec::new();
                for x in [a, b] {
                    match x {
                        E::And(xs) => v.extend(xs),
                        x => v.push(x),
                    }
                }
                Ok(E::And(v))
            }
        },
    }
}

/// `a || b`: a true first value is the result, else the second.
pub fn or(a: E, b: E) -> R<E> {
    match truth(&a) {
        Some(true) => Ok(a),
        Some(false) => Ok(b),
        None => match truth(&b) {
            Some(false) if matches!(b, E::Bool(_)) => Ok(a),
            Some(true) if matches!(b, E::Bool(_)) => Ok(b),
            _ => {
                let mut v = Vec::new();
                for x in [a, b] {
                    match x {
                        E::Or(xs) => v.extend(xs),
                        x => v.push(x),
                    }
                }
                Ok(E::Or(v))
            }
        },
    }
}

pub fn not(a: E) -> R<E> {
    Ok(match a {
        E::Bool(b) => E::Bool(!b),
        E::Num(n) => E::Bool(n.is_zero()),
        E::Not(x) => *x,
        E::Cmp(op, x, y) => E::Cmp(op.negate(), x, y),
        E::And(xs) => {
            let mut out = Vec::new();
            for x in xs {
                match not(x)? {
                    E::Or(v) => out.extend(v),
                    x => out.push(x),
                }
            }
            E::Or(out)
        }
        E::Or(xs) => {
            let mut out = Vec::new();
            for x in xs {
                match not(x)? {
                    E::And(v) => out.extend(v),
                    x => out.push(x),
                }
            }
            E::And(out)
        }
        E::Mat(m) => E::Mat(m.map(|x| not(x.clone()))?),
        a => E::Not(Box::new(a)),
    })
}

/// Bitwise `&` and `|` on 32-bit whole numbers.
pub fn bits(and: bool, a: E, b: E) -> R<E> {
    let (Some(x), Some(y)) = (a.as_num(), b.as_num()) else {
        return Err("bit operations need whole numbers".into());
    };
    let (Some(i), Some(j)) = (x.as_int(), y.as_int()) else {
        return Err("bit operations need whole numbers".into());
    };
    let (i, j) = (i as u32, j as u32);
    let r = if and { i & j } else { i | j };
    let radix = if x.radix != 10 { x.radix } else { y.radix };
    Ok(E::Num(Num::real(r as f64).with_radix(radix)))
}

/// Multiply out products of sums and whole powers of sums (for solving).
pub fn expand(e: &E) -> R<E> {
    match e {
        E::Add(ts) => add(ts.iter().map(expand).collect::<R<_>>()?),
        E::Mul(fs) => {
            let mut acc = vec![num(1.0)];
            for f in fs {
                let f = expand(f)?;
                acc = cross(&acc, &terms_of(&f))?;
            }
            add(acc)
        }
        E::Pow(b, x) if matches!(**b, E::Add(_)) && x.as_int().is_some_and(|k| (2..=12).contains(&k)) => {
            let b = expand(b)?;
            let parts = terms_of(&b);
            let mut acc = vec![num(1.0)];
            for _ in 0..x.as_int().unwrap() {
                acc = cross(&acc, &parts)?;
            }
            add(acc)
        }
        e => Ok(e.clone()),
    }
}

pub fn terms_of(e: &E) -> Vec<E> {
    match e {
        E::Add(ts) => ts.clone(),
        e => vec![e.clone()],
    }
}

fn cross(a: &[E], b: &[E]) -> R<Vec<E>> {
    if a.len() * b.len() > MAX_TERMS {
        return Err("the expression is too large to expand".into());
    }
    let mut out = Vec::with_capacity(a.len() * b.len());
    for x in a {
        for y in b {
            out.extend(terms_of(&mul(vec![x.clone(), y.clone()])?));
        }
    }
    Ok(out)
}
