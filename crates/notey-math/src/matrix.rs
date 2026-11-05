//! Determinants, inverses and powers of square matrices. Numeric matrices
//! use Gaussian elimination; symbolic ones (up to 6x6) use cofactors.

use crate::expr::{add, matmul, mul, num, pow, Mat, E, R};
use crate::num::Num;

const MAX_SYMBOLIC: usize = 6;

fn numeric(m: &Mat) -> Option<Vec<Num>> {
    m.data.iter().map(E::as_num).collect()
}

fn square(m: &Mat) -> R<usize> {
    if m.rows != m.cols {
        return Err(format!("a {}x{} matrix is not square", m.rows, m.cols));
    }
    Ok(m.rows)
}

fn minor(m: &Mat, row: usize, col: usize) -> Mat {
    let mut data = Vec::with_capacity((m.rows - 1) * (m.cols - 1));
    for r in 0..m.rows {
        for c in 0..m.cols {
            if r != row && c != col {
                data.push(m.get(r, c).clone());
            }
        }
    }
    Mat::new(m.rows - 1, m.cols - 1, data)
}

pub fn det(m: &Mat) -> R<E> {
    let n = square(m)?;
    if n == 0 {
        return Ok(num(1.0));
    }
    if let Some(mut a) = numeric(m) {
        let mut d = Num::ONE;
        for col in 0..n {
            let pivot = (col..n).max_by(|&x, &y| a[x * n + col].abs().total_cmp(&a[y * n + col].abs())).unwrap();
            if a[pivot * n + col].abs() == 0.0 {
                return Ok(num(0.0));
            }
            if pivot != col {
                for c in 0..n {
                    a.swap(pivot * n + c, col * n + c);
                }
                d = d.neg();
            }
            let p = a[col * n + col];
            d = d.mul(p);
            for r in col + 1..n {
                let f = a[r * n + col].div(p);
                for c in col..n {
                    a[r * n + c] = a[r * n + c].add(f.mul(a[col * n + c]).neg());
                }
            }
        }
        return Ok(E::Num(d.clean()));
    }
    if n > MAX_SYMBOLIC {
        return Err("the matrix is too large for symbolic math".into());
    }
    laplace(m)
}

fn laplace(m: &Mat) -> R<E> {
    if m.rows == 1 {
        return Ok(m.data[0].clone());
    }
    let mut terms = Vec::with_capacity(m.cols);
    for c in 0..m.cols {
        let x = m.get(0, c);
        if x.is_zero() {
            continue;
        }
        let sign = if c % 2 == 0 { 1.0 } else { -1.0 };
        terms.push(mul(vec![num(sign), x.clone(), laplace(&minor(m, 0, c))?])?);
    }
    add(terms)
}

pub fn inverse(m: &Mat) -> R<Mat> {
    let n = square(m)?;
    if let Some(mut a) = numeric(m) {
        let mut inv: Vec<Num> = (0..n * n).map(|k| if k / n == k % n { Num::ONE } else { Num::ZERO }).collect();
        for col in 0..n {
            let pivot = (col..n).max_by(|&x, &y| a[x * n + col].abs().total_cmp(&a[y * n + col].abs())).unwrap();
            if a[pivot * n + col].abs() < 1e-300 {
                return Err("the matrix has no inverse".into());
            }
            for c in 0..n {
                a.swap(pivot * n + c, col * n + c);
                inv.swap(pivot * n + c, col * n + c);
            }
            let p = a[col * n + col];
            for c in 0..n {
                a[col * n + c] = a[col * n + c].div(p);
                inv[col * n + c] = inv[col * n + c].div(p);
            }
            for r in 0..n {
                if r == col {
                    continue;
                }
                let f = a[r * n + col];
                if f.is_zero() {
                    continue;
                }
                for c in 0..n {
                    a[r * n + c] = a[r * n + c].add(f.mul(a[col * n + c]).neg());
                    inv[r * n + c] = inv[r * n + c].add(f.mul(inv[col * n + c]).neg());
                }
            }
        }
        return Ok(Mat::new(n, n, inv.into_iter().map(|x| E::Num(snap(x.clean()))).collect()));
    }
    if n > MAX_SYMBOLIC {
        return Err("the matrix is too large for a symbolic inverse".into());
    }
    let d = det(m)?;
    if d.is_zero() {
        return Err("the matrix has no inverse".into());
    }
    let dinv = pow(d, num(-1.0))?;
    let mut data = vec![num(0.0); n * n];
    for r in 0..n {
        for c in 0..n {
            let sign = if (r + c) % 2 == 0 { 1.0 } else { -1.0 };
            let cof = if n == 1 { num(1.0) } else { laplace(&minor(m, r, c))? };
            // the inverse is the transposed cofactors over the determinant
            data[c * n + r] = mul(vec![num(sign), cof, dinv.clone()])?;
        }
    }
    Ok(Mat::new(n, n, data))
}

/// Rounding noise from elimination (1e-17 where 0 belongs).
fn snap(x: Num) -> Num {
    let r = |v: f64| if v.abs() < 1e-13 { 0.0 } else { v };
    Num::complex(r(x.re), r(x.im))
}

pub fn power(m: &Mat, k: i64) -> R<Mat> {
    if k == 1 {
        return Ok(m.clone());
    }
    let n = square(m)?;
    if k == 0 {
        return Ok(Mat::identity(n));
    }
    let base = if k < 0 { inverse(m)? } else { m.clone() };
    let k = k.unsigned_abs();
    if k > 1000 {
        return Err("the power is too large".into());
    }
    let mut acc = base.clone();
    for _ in 1..k {
        acc = matmul(&acc, &base)?;
    }
    Ok(acc)
}
