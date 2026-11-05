//! Money: currency codes and symbols as units (`100 USD`, `$20`, `€5`),
//! and conversion with exchange rates (`100 USD in EUR`).

use std::collections::HashMap;

use crate::builtins::subst;
use crate::expr::{mul, E, R};
use crate::num::Num;

/// Marks a variable as a currency unit.
pub const PREFIX: char = '¤';

/// The currencies the exchange-rate source (the European Central Bank's
/// reference rates) covers.
pub const CODES: [&str; 31] = [
    "AUD", "BGN", "BRL", "CAD", "CHF", "CNY", "CZK", "DKK", "EUR", "GBP", "HKD", "HUF", "IDR", "ILS", "INR", "ISK",
    "JPY", "KRW", "MXN", "MYR", "NOK", "NZD", "PHP", "PLN", "RON", "SEK", "SGD", "THB", "TRY", "USD", "ZAR",
];

pub fn is_code(s: &str) -> bool {
    CODES.contains(&s)
}

/// The currency a symbol stands for.
pub fn symbol_code(c: char) -> Option<&'static str> {
    Some(match c {
        '$' => "USD",
        '€' => "EUR",
        '£' => "GBP",
        '¥' => "JPY",
        '₹' => "INR",
        '₩' => "KRW",
        _ => return None,
    })
}

pub fn unit(code: &str) -> E {
    E::Sym(format!("{PREFIX}{code}"))
}

pub fn is_currency(e: &E) -> bool {
    matches!(e, E::Sym(s) if s.starts_with(PREFIX))
}

/// The currencies in `e`, in first-occurrence order.
pub fn codes_in(e: &E) -> Vec<String> {
    let mut syms = Vec::new();
    e.symbols(&mut syms);
    syms.into_iter().filter_map(|s| s.strip_prefix(PREFIX).map(str::to_string)).collect()
}

/// Exchange rates: units of each currency per unit of the base currency.
#[derive(Clone, Debug, Default)]
pub struct Rates {
    per_base: HashMap<String, f64>,
}

impl Rates {
    /// Rates from the source's JSON: `{"base": "EUR", "rates": {"USD": 1.08, ...}}`.
    pub fn parse(json: &str) -> Option<Rates> {
        let v: serde_json::Value = serde_json::from_str(json).ok()?;
        let base = v.get("base")?.as_str()?.to_string();
        let mut per_base: HashMap<String, f64> = v
            .get("rates")?
            .as_object()?
            .iter()
            .filter_map(|(k, x)| Some((k.clone(), x.as_f64().filter(|r| *r > 0.0)?)))
            .collect();
        per_base.insert(base, 1.0);
        Some(Rates { per_base })
    }

    /// How many units of `to` one unit of `from` is worth.
    pub fn factor(&self, from: &str, to: &str) -> R<f64> {
        let a = self.per_base.get(from).ok_or_else(|| format!("no exchange rate for {from}"))?;
        let b = self.per_base.get(to).ok_or_else(|| format!("no exchange rate for {to}"))?;
        Ok(b / a)
    }
}

/// `e` with every amount of money in `to`.
pub fn convert(e: &E, to: &str, rates: &Rates) -> R<E> {
    let mut out = e.clone();
    for code in codes_in(e) {
        if code == to {
            continue;
        }
        let f = rates.factor(&code, to)?;
        let value = mul(vec![E::Num(Num::real(f)), unit(to)])?;
        out = subst(&out, &format!("{PREFIX}{code}"), &value)?;
    }
    Ok(out)
}
