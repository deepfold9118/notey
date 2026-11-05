//! Numbers: complex doubles that remember whether they were written in
//! hexadecimal or binary, and their display in a culture.

/// A complex number. `radix` is 16 or 2 for a number written as `0x..` or
/// `0b..` (and results computed from one), else 10.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Num {
    pub re: f64,
    pub im: f64,
    pub radix: u8,
}

/// Values this close to zero, relative to the other part or operand, are
/// rounding noise (sin(pi), sqrt(i)^2).
const NOISE: f64 = 1e-12;

#[allow(clippy::should_implement_trait)] // Copy values with their own radix rules
impl Num {
    pub const ZERO: Num = Num { re: 0.0, im: 0.0, radix: 10 };
    pub const ONE: Num = Num { re: 1.0, im: 0.0, radix: 10 };
    pub const I: Num = Num { re: 0.0, im: 1.0, radix: 10 };

    pub fn real(x: f64) -> Num {
        Num { re: x, im: 0.0, radix: 10 }
    }
    pub fn complex(re: f64, im: f64) -> Num {
        Num { re, im, radix: 10 }
    }
    pub fn with_radix(mut self, radix: u8) -> Num {
        self.radix = radix;
        self
    }
    fn radix_of(a: Num, b: Num) -> u8 {
        if a.radix != 10 { a.radix } else { b.radix }
    }
    pub fn is_real(&self) -> bool {
        self.im == 0.0
    }
    pub fn is_zero(&self) -> bool {
        self.re == 0.0 && self.im == 0.0
    }
    pub fn is_one(&self) -> bool {
        self.re == 1.0 && self.im == 0.0
    }
    pub fn is_finite(&self) -> bool {
        self.re.is_finite() && self.im.is_finite()
    }
    pub fn is_int(&self) -> bool {
        self.is_real() && self.re.is_finite() && self.re.fract() == 0.0
    }
    pub fn as_int(&self) -> Option<i64> {
        (self.is_int() && self.re.abs() < 9.0e15).then_some(self.re as i64)
    }
    pub fn add(self, o: Num) -> Num {
        Num { re: self.re + o.re, im: self.im + o.im, radix: Self::radix_of(self, o) }
    }
    pub fn neg(self) -> Num {
        Num { re: -self.re, im: -self.im, radix: self.radix }
    }
    pub fn mul(self, o: Num) -> Num {
        let radix = Self::radix_of(self, o);
        if self.is_real() && o.is_real() {
            return Num { re: self.re * o.re, im: 0.0, radix };
        }
        Num { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re, radix }.clean()
    }
    pub fn inv(self) -> Num {
        if self.is_real() {
            return Num { re: 1.0 / self.re, im: 0.0, radix: self.radix };
        }
        let d = self.re * self.re + self.im * self.im;
        Num { re: self.re / d, im: -self.im / d, radix: self.radix }
    }
    pub fn div(self, o: Num) -> Num {
        if self.is_real() && o.is_real() {
            return Num { re: self.re / o.re, im: 0.0, radix: Self::radix_of(self, o) };
        }
        self.mul(o.inv())
    }
    pub fn abs(self) -> f64 {
        if self.is_real() { self.re.abs() } else { self.re.hypot(self.im) }
    }
    pub fn arg(self) -> f64 {
        self.im.atan2(self.re)
    }
    pub fn conj(self) -> Num {
        Num { re: self.re, im: -self.im, radix: self.radix }
    }
    pub fn exp(self) -> Num {
        if self.is_real() {
            return Num::real(self.re.exp());
        }
        let r = self.re.exp();
        Num::complex(r * self.im.cos(), r * self.im.sin()).clean()
    }
    pub fn ln(self) -> Num {
        if self.is_real() && self.re > 0.0 {
            return Num::real(self.re.ln());
        }
        Num::complex(self.abs().ln(), self.arg()).clean()
    }
    pub fn sqrt(self) -> Num {
        if self.is_real() {
            return if self.re >= 0.0 {
                Num::real(self.re.sqrt())
            } else {
                Num::complex(0.0, (-self.re).sqrt())
            };
        }
        let r = self.abs();
        let re = ((r + self.re) / 2.0).sqrt();
        let im = ((r - self.re) / 2.0).sqrt().copysign(self.im);
        Num::complex(re, im).clean()
    }
    /// Whole powers by squaring, which keeps i^43 exact.
    fn powi(self, n: i64) -> Num {
        if n < 0 {
            return self.powi(-n).inv();
        }
        let (mut result, mut base, mut n) = (Num::ONE, self, n);
        while n > 0 {
            if n & 1 == 1 {
                result = result.mul(base);
            }
            base = base.mul(base);
            n >>= 1;
        }
        result
    }
    pub fn pow(self, e: Num) -> Num {
        let radix = self.radix;
        let out = if self.is_real() && e.is_real() && (self.re >= 0.0 || e.is_int()) {
            Num::real(self.re.powf(e.re))
        } else if let Some(n) = e.as_int().filter(|n| n.abs() < 1 << 20) {
            self.powi(n)
        } else if self.is_zero() {
            Num::ZERO
        } else {
            self.ln().mul(e).exp()
        };
        out.with_radix(radix).clean()
    }
    /// Drop a part that is rounding noise next to the other.
    pub fn clean(mut self) -> Num {
        if self.im != 0.0 && self.im.abs() <= NOISE * self.re.abs() {
            self.im = 0.0;
        }
        if self.re != 0.0 && self.re.abs() <= NOISE * self.im.abs() {
            self.re = 0.0;
        }
        self
    }
    pub fn is_e(&self) -> bool {
        self.is_real() && self.re == std::f64::consts::E
    }
}

/// How numbers are written: the decimal mark and the thousands separator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Culture {
    pub decimal: char,
    pub group: Option<char>,
}

impl Culture {
    pub const INVARIANT: Culture = Culture { decimal: '.', group: None };

    /// A culture from a tag such as "fr-FR" (`#@fr-FR`). An empty or unknown
    /// tag gives the invariant culture.
    pub fn from_tag(tag: &str) -> Culture {
        let tag = tag.trim().to_ascii_lowercase();
        let lang = tag.split(['-', '_']).next().unwrap_or("");
        let region = tag.split(['-', '_']).nth(1).unwrap_or("");
        let (decimal, group) = match lang {
            "en" | "ja" | "zh" | "ko" | "th" | "he" | "ga" | "ms" | "hi" => ('.', Some(',')),
            "fr" | "ru" | "pl" | "sv" | "fi" | "nb" | "no" | "cs" | "sk" | "uk" | "hu" | "bg" | "lt" | "lv" | "et" => (',', Some(' ')),
            "de" if region == "ch" => ('.', Some('\'')),
            "de" | "es" | "it" | "pt" | "nl" | "da" | "tr" | "id" | "ro" | "el" | "sl" | "hr" | "sr" | "vi" => (',', Some('.')),
            _ => return Culture::INVARIANT,
        };
        Culture { decimal, group }
    }
}

fn strip_zeros(s: &str) -> &str {
    if s.contains('.') && !s.contains('e') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// `s` (Rust's format, '.' decimal) in the culture's marks.
fn localize(s: &str, c: &Culture) -> String {
    let (sign, body) = s.strip_prefix('-').map_or(("", s), |b| ("-", b));
    let (int, rest) = match body.find(['.', 'e']) {
        Some(i) => body.split_at(i),
        None => (body, ""),
    };
    let mut out = String::from(sign);
    match c.group {
        Some(g) if int.len() > 3 && !rest.contains('e') => {
            for (i, ch) in int.chars().enumerate() {
                if i > 0 && (int.len() - i) % 3 == 0 {
                    out.push(g);
                }
                out.push(ch);
            }
        }
        _ => out.push_str(int),
    }
    out.extend(rest.chars().map(|ch| if ch == '.' { c.decimal } else { ch }));
    out
}

/// A real number for display: at most four decimals (as Calca shows them),
/// or `places` decimals exactly.
pub fn fmt_real(x: f64, c: &Culture, places: Option<usize>) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "infinity".into() } else { "-infinity".into() };
    }
    let x = if x == 0.0 { 0.0 } else { x };
    if let Some(p) = places {
        return localize(&format!("{x:.p$}"), c);
    }
    let a = x.abs();
    let s = if a == 0.0 {
        "0".to_string()
    } else if !(1e-9..1e15).contains(&a) {
        let s = format!("{x:.4e}");
        match s.split_once('e') {
            Some((m, e)) => format!("{}e{e}", strip_zeros(m)),
            None => s,
        }
    } else if a < 0.00005 {
        let digits = (4 - a.log10().floor() as i32 - 1).max(0) as usize;
        strip_zeros(&format!("{x:.digits$}")).to_string()
    } else {
        strip_zeros(&format!("{x:.4}")).to_string()
    };
    localize(&s, c)
}

/// A number for display: complex numbers as `2i + 1` (Calca's order), and
/// hexadecimal or binary numbers as they were written.
pub fn fmt_num(n: Num, c: &Culture, places: Option<usize>) -> String {
    if n.is_real() {
        if let Some(v) = n.as_int().filter(|v| *v > 0 && *v < 1 << 31) {
            match n.radix {
                16 => return format!("0x{v:X}"),
                2 => return format!("0b{v:b}"),
                _ => {}
            }
        }
        return fmt_real(n.re, c, places);
    }
    let imag = match n.im {
        1.0 => "i".to_string(),
        -1.0 => "-i".to_string(),
        im => format!("{}i", fmt_real(im, c, places)),
    };
    if n.re == 0.0 {
        return imag;
    }
    let re = fmt_real(n.re.abs(), c, places);
    if n.re < 0.0 { format!("{imag} - {re}") } else { format!("{imag} + {re}") }
}
