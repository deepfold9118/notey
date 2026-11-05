//! A whole document: lines are grouped into blocks (indented lines continue
//! the line above), headings scope definitions, `#@fr-FR` switches the
//! culture, and every `=>` gets its result written after it.
//!
//! Notey runs this in every tab, so it only writes where that is safe:
//! - `.calca` files work exactly like Calca.
//! - Plain text and Markdown get results that say something new (a value
//!   computed from numbers, definitions or functions), never a bare
//!   restatement such as "Monday => Monday".
//! - Code files (and Markdown code blocks) get only concrete values
//!   (numbers, money, matrices), and only after a `=>` that has nothing
//!   after it, or whose result Notey wrote itself while the tab is open.
//!   `Some(x) => 0` in Rust or `x => x * 2` in JavaScript is never touched.

use crate::currency::Rates;
use crate::eval::{uses_knowledge, Env};
use crate::format::show;
use crate::num::Culture;
use crate::parse::{parse_stmt, Stmt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Calca,
    Prose,
    Code,
}

/// What the engine remembers about a tab between calls.
#[derive(Clone, Debug, Default)]
pub struct BufState {
    pub lines: Vec<String>,
    /// Lines whose `=>` result Notey wrote.
    pub owned: Vec<bool>,
    pub paused: bool,
}

#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    /// Replacements in character offsets of the input, in order.
    pub edits: Vec<(usize, usize, String)>,
    pub currency: bool,
    pub needs_rates: bool,
}

/// Larger documents are left alone.
pub const MAX_TEXT: usize = 2 * 1024 * 1024;

fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

fn is_fence(t: &str) -> bool {
    t.starts_with("```") || t.starts_with("~~~")
}

fn normalize(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace() && *c != '*').collect()
}

/// The text before `=>` on a line (to follow a line through edits).
fn lhs_of(line: &str) -> &str {
    line.split("=>").next().unwrap_or("").trim()
}

/// Which lines of `new` continue the owned lines of `old`.
fn map_owned(old: &[String], owned: &[bool], new: &[&str]) -> Vec<bool> {
    let mut out = vec![false; new.len()];
    if old.is_empty() {
        return out;
    }
    let max = old.len().min(new.len());
    let mut p = 0;
    while p < max && old[p] == new[p] {
        p += 1;
    }
    let mut s = 0;
    while s < max - p && old[old.len() - 1 - s] == new[new.len() - 1 - s] {
        s += 1;
    }
    out[..p].copy_from_slice(&owned[..p]);
    for k in 0..s {
        out[new.len() - 1 - k] = owned[old.len() - 1 - k];
    }
    let (om, nm) = (p..old.len() - s, p..new.len() - s);
    if om.len() == nm.len() {
        for (i, j) in om.zip(nm) {
            out[j] = owned[i];
        }
    } else {
        let lhs: std::collections::HashSet<&str> = om.filter(|&i| owned[i]).map(|i| lhs_of(&old[i])).collect();
        if !lhs.is_empty() {
            for j in nm {
                out[j] = lhs.contains(lhs_of(new[j]));
            }
        }
    }
    out
}

/// Blank out `# comments` (a `#` at the start or after a space) so offsets
/// stay the same.
fn mask_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for (k, line) in src.split('\n').enumerate() {
        if k > 0 {
            out.push('\n');
        }
        let b = line.as_bytes();
        let cut = (0..b.len()).find(|&p| b[p] == b'#' && (p == 0 || b[p - 1].is_ascii_whitespace()));
        match cut {
            Some(p) => {
                out.push_str(&line[..p]);
                out.extend(std::iter::repeat_n(' ', line.len() - p));
            }
            None => out.push_str(line),
        }
    }
    out
}

/// Byte ranges of the `;`-separated statements (outside brackets).
fn statements(src: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (k, b) in src.bytes().enumerate() {
        match b {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b';' if depth <= 0 => {
                out.push((start, k));
                start = k + 1;
            }
            _ => {}
        }
    }
    out.push((start, src.len()));
    out
}

fn arrow(seg: &str) -> Option<usize> {
    let b = seg.as_bytes();
    let mut depth = 0i32;
    for k in 0..b.len() {
        match b[k] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'=' if depth <= 0 && b.get(k + 1) == Some(&b'>') && (k == 0 || !matches!(b[k - 1], b'=' | b'<' | b'>' | b'!')) => {
                return Some(k);
            }
            _ => {}
        }
    }
    None
}

struct Run<'a> {
    text: &'a str,
    lines: Vec<&'a str>,
    starts: Vec<usize>,
    mode: Mode,
    env: Env,
    culture: Culture,
    owned: Vec<bool>,
    edits: Vec<(usize, usize, String)>,
}

impl Run<'_> {
    fn line(&self, k: usize) -> &str {
        self.lines[k].trim_end_matches('\r')
    }

    fn block(&mut self, start: usize, end: usize, code: bool) {
        let first = self.line(start);
        let indent = indent_of(first);
        // an unindented line ending in punctuation is prose
        if indent == 0 && !first.contains("=>") && first.trim_end().ends_with(['.', ':', ',', '!', '?', ';']) {
            return;
        }
        let bs = self.starts[start];
        let be = self.starts[end - 1] + self.line(end - 1).len();
        let src = &self.text[bs..be];
        let masked = mask_comments(src);
        for (s, e) in statements(&masked) {
            let seg = &masked[s..e];
            let (lhs_len, region) = match arrow(seg) {
                Some(p) => (p, Some((s + p + 2, e))),
                None => (seg.len(), None),
            };
            let lhs = &seg[..lhs_len];
            if lhs.trim().is_empty() {
                continue;
            }
            let Ok(stmt) = parse_stmt(lhs, &self.culture) else { continue };
            let (ast, value) = match stmt {
                Stmt::Assign(l, r) => {
                    let defined = self.env.assign(&l, &r);
                    if region.is_none() || !defined {
                        continue;
                    }
                    let v = self.env.eval(&r);
                    (r, v)
                }
                Stmt::Expr(a) => {
                    if region.is_none() {
                        continue;
                    }
                    let v = self.env.result(&a);
                    (a, v)
                }
            };
            let Ok(value) = value else { continue };
            let (rs, re) = region.unwrap();
            let result = show(&value, &self.culture);
            let current = src[rs..re].trim();
            let line = start + src[..s + lhs_len].matches('\n').count();
            // a bare unknown is solved for; if nothing solves it, the result
            // is the name itself, which the trivial check catches
            let interesting = self.mode == Mode::Calca || uses_knowledge(&ast, &self.env) || matches!(ast, crate::parse::Ast::Name(_));
            let trivial = self.mode != Mode::Calca && normalize(&result) == normalize(lhs);
            let safe = match (self.mode, code) {
                (Mode::Calca, false) => true,
                (_, false) => interesting && !trivial,
                (_, true) => interesting && !trivial && value.is_concrete() && (current.is_empty() || self.owned[line] || current == result),
            };
            if !safe {
                continue;
            }
            self.owned[line] = true;
            if current != result {
                self.edits.push((bs + rs, bs + re, format!(" {result}")));
            }
        }
    }
}

/// Work out the results of a document. `state` carries what was learned
/// about the tab from one call to the next.
pub fn process(text: &str, mode: Mode, rates: Option<Rates>, state: &mut BufState) -> Outcome {
    let lines: Vec<&str> = text.split('\n').collect();
    if !text.contains("=>") || text.len() > MAX_TEXT || state.paused {
        state.lines = lines.iter().map(|l| l.to_string()).collect();
        state.owned = vec![false; lines.len()];
        return Outcome::default();
    }
    let mut starts = Vec::with_capacity(lines.len());
    let mut off = 0;
    for l in &lines {
        starts.push(off);
        off += l.len() + 1;
    }
    let owned = map_owned(&state.lines, &state.owned, &lines);
    let mut run = Run { text, lines, starts, mode, env: Env::new(rates), culture: Culture::INVARIANT, owned, edits: Vec::new() };
    let prose = mode != Mode::Code;
    let mut fence = false;
    let mut i = 0;
    while i < run.lines.len() {
        let raw = run.line(i);
        let t = raw.trim_start();
        if prose && is_fence(t) {
            fence = !fence;
            i += 1;
            continue;
        }
        if t.is_empty() {
            i += 1;
            continue;
        }
        let indent = indent_of(raw);
        if prose && !fence && indent == 0 && t.starts_with('#') {
            if let Some(tag) = t.strip_prefix("#@") {
                run.culture = Culture::from_tag(tag);
            } else {
                let level = t.chars().take_while(|c| *c == '#').count();
                if level <= 6 && t[level..].chars().next().is_none_or(char::is_whitespace) {
                    run.env.heading(level);
                }
            }
            i += 1;
            continue;
        }
        let mut end = i + 1;
        while end < run.lines.len() {
            let l = run.line(end);
            let lt = l.trim_start();
            if lt.is_empty() || indent_of(l) <= indent || (prose && is_fence(lt)) {
                break;
            }
            end += 1;
        }
        run.block(i, end, mode == Mode::Code || fence);
        i = end;
    }

    // the text after the edits, so the next call can follow the lines
    let mut after = String::with_capacity(text.len());
    let mut at = 0;
    for (s, e, t) in &run.edits {
        after.push_str(&text[at..*s]);
        after.push_str(t);
        at = *e;
    }
    after.push_str(&text[at..]);
    let after_lines: Vec<&str> = after.split('\n').collect();
    let before: Vec<String> = run.lines.iter().map(|l| l.to_string()).collect();
    state.owned = map_owned(&before, &run.owned, &after_lines);
    state.lines = after_lines.iter().map(|l| l.to_string()).collect();

    // byte offsets to character offsets
    let mut edits = Vec::with_capacity(run.edits.len());
    let (mut byte, mut chars) = (0, 0);
    for (s, e, t) in run.edits {
        chars += text[byte..s].chars().count();
        let start = chars;
        chars += text[s..e].chars().count();
        byte = e;
        edits.push((start, chars, t));
    }
    Outcome { edits, currency: run.env.currency.get(), needs_rates: run.env.needs_rates.get() }
}
