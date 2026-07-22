//! Search engine: regex and literal find/replace over char offsets, plus
//! multi-file grep. Built on fancy-regex (backreferences + lookaround, the
//! Boost.Regex-class features Notepad++ users expect), with literal mode
//! funneled through the same path via escaping so case folding is uniform.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use fancy_regex::Regex;

#[derive(Clone, Copy)]
pub struct SearchOpts {
    pub regex: bool,
    pub match_case: bool,
}

/// Compile the pattern (escaping it first in literal mode).
pub fn compile(pattern: &str, opts: SearchOpts) -> Result<Regex, String> {
    let mut src = String::new();
    if !opts.match_case {
        src.push_str("(?i)");
    }
    if opts.regex {
        src.push_str(pattern);
    } else {
        src.push_str(&fancy_regex::escape(pattern));
    }
    Regex::new(&src).map_err(|e| trim_err(&e.to_string()))
}

fn trim_err(e: &str) -> String {
    e.lines().next().unwrap_or("invalid pattern").to_string()
}

/// All match ranges as char offsets, capped at `cap`.
pub fn find_all(text: &str, re: &Regex, cap: usize) -> Vec<(usize, usize)> {
    let mut byte_ranges: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    while start <= text.len() {
        match re.find_from_pos(text, start) {
            Ok(Some(m)) => {
                byte_ranges.push((m.start(), m.end()));
                if byte_ranges.len() >= cap {
                    break;
                }
                // avoid infinite loops on empty matches
                start = if m.end() > m.start() {
                    m.end()
                } else {
                    next_char_boundary(text, m.end())
                };
            }
            _ => break,
        }
    }
    bytes_to_chars(text, &byte_ranges)
}

fn next_char_boundary(text: &str, b: usize) -> usize {
    let mut b = b + 1;
    while b < text.len() && !text.is_char_boundary(b) {
        b += 1;
    }
    b
}

/// Convert byte ranges (sorted ascending) to char ranges in one pass.
fn bytes_to_chars(text: &str, byte_ranges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut out = Vec::with_capacity(byte_ranges.len());
    let mut flat: Vec<usize> = Vec::with_capacity(byte_ranges.len() * 2);
    for &(a, b) in byte_ranges {
        flat.push(a);
        flat.push(b);
    }
    let mut fi = 0usize;
    let mut chars = 0usize;
    for (bidx, _) in text.char_indices() {
        while fi < flat.len() && flat[fi] == bidx {
            flat[fi] = chars;
            fi += 1;
        }
        chars += 1;
    }
    while fi < flat.len() {
        flat[fi] = chars; // ranges ending at text.len()
        fi += 1;
    }
    for pair in flat.chunks(2) {
        out.push((pair[0], pair[1]));
    }
    out
}

/// Next match as a char range, searching forward (or backward) from
/// `from_char`, optionally wrapping.
pub fn find_next(
    text: &str,
    re: &Regex,
    from_char: usize,
    backwards: bool,
    wrap: bool,
) -> Option<(usize, usize)> {
    let all = find_all(text, re, 100_000);
    if all.is_empty() {
        return None;
    }
    if backwards {
        all.iter()
            .rev()
            .find(|&&(_, e)| e <= from_char)
            .or(if wrap { all.last() } else { None })
            .copied()
    } else {
        all.iter()
            .find(|&&(s, _)| s >= from_char)
            .or(if wrap { all.first() } else { None })
            .copied()
    }
}

/// Replace every match. In regex mode `$1`/`${name}` expand; in literal
/// mode the replacement is taken verbatim. Returns (new_text, count).
pub fn replace_all(
    text: &str,
    re: &Regex,
    replacement: &str,
    regex_mode: bool,
) -> (String, usize) {
    let count = find_all(text, re, usize::MAX).len();
    if count == 0 {
        return (text.to_string(), 0);
    }
    let new = if regex_mode {
        re.replace_all(text, replacement).into_owned()
    } else {
        re.replace_all(text, fancy_regex::NoExpand(replacement))
            .into_owned()
    };
    (new, count)
}

/// Expand the replacement for one specific match (used by "Replace" on the
/// current selection in regex mode).
pub fn expand_one(
    matched_text: &str,
    re: &Regex,
    replacement: &str,
    regex_mode: bool,
) -> String {
    if !regex_mode {
        return replacement.to_string();
    }
    re.replace(matched_text, replacement).into_owned()
}

// ---------- multi-file grep ----------

pub struct GrepMatch {
    pub path: PathBuf,
    pub line_no: usize, // 1-based
    pub preview: String,
}

pub const GREP_MAX_RESULTS: usize = 2000;

/// Walk `root` (respecting .gitignore), grep matching files line by line,
/// and stream results into `out`. `filters` is a semicolon list like
/// "*.rs;*.toml" (empty = all files).
pub fn grep_dir(
    root: PathBuf,
    re: Regex,
    filters: String,
    cancel: Arc<AtomicBool>,
    out: Sender<GrepMatch>,
) {
    let pats: Vec<String> = filters
        .split(';')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && *s != "*" && *s != "*.*")
        .collect();
    let mut sent = 0usize;
    for entry in ignore::WalkBuilder::new(&root).build() {
        if cancel.load(Ordering::Relaxed) || sent >= GREP_MAX_RESULTS {
            return;
        }
        let Ok(entry) = entry else { continue };
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        if !pats.is_empty() && !matches_filters(path, &pats) {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else { continue };
        if bytes.len() > 16 * 1024 * 1024 || bytes[..bytes.len().min(8192)].contains(&0) {
            continue; // huge or binary
        }
        let content = String::from_utf8_lossy(&bytes);
        for (i, line) in content.lines().enumerate() {
            if cancel.load(Ordering::Relaxed) || sent >= GREP_MAX_RESULTS {
                return;
            }
            if matches!(re.is_match(line), Ok(true)) {
                let mut preview: String = line.trim_end().chars().take(200).collect();
                if preview.len() < line.trim_end().len() {
                    preview.push('…');
                }
                if out
                    .send(GrepMatch {
                        path: path.to_path_buf(),
                        line_no: i + 1,
                        preview,
                    })
                    .is_err()
                {
                    return;
                }
                sent += 1;
            }
        }
    }
}

/// Match a file name against simple "*.ext" / "name*" style patterns.
fn matches_filters(path: &Path, pats: &[String]) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    pats.iter().any(|p| {
        let p = p.to_lowercase();
        if let Some(suffix) = p.strip_prefix('*') {
            name.ends_with(&suffix.to_lowercase())
        } else if let Some(prefix) = p.strip_suffix('*') {
            name.starts_with(prefix)
        } else {
            name == p
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(regex: bool, case: bool) -> SearchOpts {
        SearchOpts {
            regex,
            match_case: case,
        }
    }

    #[test]
    fn literal_case_insensitive() {
        let re = compile("Foo", opts(false, false)).unwrap();
        assert_eq!(find_all("foo FOO fOo", &re, 100).len(), 3);
        let re = compile("Foo", opts(false, true)).unwrap();
        assert_eq!(find_all("foo FOO Foo", &re, 100).len(), 1);
    }

    #[test]
    fn literal_escapes_metachars() {
        let re = compile("a.b(c)", opts(false, true)).unwrap();
        assert_eq!(find_all("a.b(c) axb(c)", &re, 100), vec![(0, 6)]);
    }

    #[test]
    fn char_offsets_with_unicode() {
        let re = compile("wörld", opts(false, true)).unwrap();
        // "héllo wörld" — 'é' and 'ö' are multibyte
        assert_eq!(find_all("héllo wörld", &re, 100), vec![(6, 11)]);
    }

    #[test]
    fn regex_with_backreference() {
        let re = compile(r"(\w+) \1", opts(true, true)).unwrap();
        let hits = find_all("go go stop stop mix max", &re, 100);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn find_next_wraps_and_reverses() {
        let re = compile("ab", opts(false, true)).unwrap();
        let text = "ab..ab..ab";
        assert_eq!(find_next(text, &re, 3, false, true), Some((4, 6)));
        assert_eq!(find_next(text, &re, 9, false, true), Some((0, 2))); // wrapped
        assert_eq!(find_next(text, &re, 9, false, false), None);
        assert_eq!(find_next(text, &re, 4, true, true), Some((0, 2)));
        assert_eq!(find_next(text, &re, 0, true, true), Some((8, 10))); // wrapped back
    }

    #[test]
    fn replace_all_with_groups() {
        let re = compile(r"(\d+)-(\d+)", opts(true, true)).unwrap();
        let (out, n) = replace_all("1-2 and 3-4", &re, "$2-$1", true);
        assert_eq!(out, "2-1 and 4-3");
        assert_eq!(n, 2);
        // literal mode: $ stays literal
        let re = compile("x", opts(false, true)).unwrap();
        let (out, n) = replace_all("x", &re, "$1", false);
        assert_eq!(out, "$1");
        assert_eq!(n, 1);
    }

    #[test]
    fn empty_match_terminates() {
        let re = compile(r"\b", opts(true, true)).unwrap();
        let hits = find_all("ab cd", &re, 100);
        assert!(!hits.is_empty() && hits.len() <= 100);
    }
}
