//! Find in Files: grep a folder (respecting .gitignore) line by line. Built
//! on fancy-regex (backreferences and look-around, the Boost.Regex-class
//! features Notepad++ users expect), with literal mode funneled through the
//! same path via escaping so case folding is uniform. Carried over from the
//! egui version; the editor's own search is now codemirror-search, and the
//! single-text functions below serve the plugin API.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
    Regex::new(&src).map_err(|e| e.to_string().lines().next().unwrap_or("invalid pattern").to_string())
}

// ---------- single-text search, for the plugin API (char offsets) ----------

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


/// A matching line.
#[derive(Clone, Debug)]
pub struct GrepMatch {
    pub path: PathBuf,
    pub line_no: usize,
    pub preview: String,
}

pub const GREP_MAX_RESULTS: usize = 2000;

/// Walk `root` (respecting .gitignore), grep matching files line by line,
/// and pass the results to `out`, which returns false to stop. `filters`
/// is a semicolon list like "*.rs;*.toml" (empty = all files).
pub fn grep_dir(root: PathBuf, re: Regex, filters: String, cancel: Arc<AtomicBool>, mut out: impl FnMut(GrepMatch) -> bool) {
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
                if !out(GrepMatch { path: path.to_path_buf(), line_no: i + 1, preview }) {
                    return;
                }
                sent += 1;
            }
        }
    }
}

/// Match a file name against simple "*.ext" / "name*" style patterns.
fn matches_filters(path: &Path, pats: &[String]) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    pats.iter().any(|p| {
        let p = p.to_lowercase();
        if let Some(suffix) = p.strip_prefix('*') {
            name.ends_with(suffix)
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
        SearchOpts { regex, match_case: case }
    }

    #[test]
    fn literal_escapes_and_folds_case() {
        let re = compile("A.b(c)", opts(false, false)).unwrap();
        assert!(re.is_match("x a.B(c) y").unwrap());
        assert!(!re.is_match("axb(c)").unwrap());
        assert!(compile("(", opts(true, true)).is_err());
    }

    #[test]
    fn greps_a_folder_with_filters() {
        let dir = std::env::temp_dir().join(format!("notey-grep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "fn main() {}\nlet needle = 1;\n").unwrap();
        std::fs::write(dir.join("b.txt"), "needle in a haystack\n").unwrap();
        std::fs::write(dir.join("c.bin"), b"needle\0binary").unwrap();
        let re = compile("needle", opts(false, true)).unwrap();
        let mut found = Vec::new();
        grep_dir(dir.clone(), re.clone(), String::new(), Arc::default(), |m| {
            found.push((m.path.file_name().unwrap().to_string_lossy().into_owned(), m.line_no));
            true
        });
        found.sort();
        assert_eq!(found, [("a.rs".to_string(), 2), ("b.txt".to_string(), 1)]);
        let mut rs_only = 0;
        grep_dir(dir.clone(), re, "*.RS".into(), Arc::default(), |_| {
            rs_only += 1;
            true
        });
        assert_eq!(rs_only, 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
