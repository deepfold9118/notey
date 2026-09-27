//! Syntax highlighting for both editors, built on syntect + two-face.
//!
//! Highlighting is virtualization-friendly: `HlCache` snapshots the parser
//! state at every line boundary, so only lines up to the visible bottom are
//! ever parsed, and an edit invalidates from the edited line down (never
//! above). Colors come from syntect themes mapped to the Fluent dark/light
//! look; bold/italic style flags are deliberately ignored so glyph geometry
//! stays identical to the unhighlighted layout (hit-testing relies on that).

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::OnceLock;

use eframe::egui::Color32;
use syntect::highlighting::{
    HighlightState, Highlighter, HighlightIterator, Theme, ThemeSet,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};

pub struct Syntaxes {
    pub set: SyntaxSet,
    pub theme_dark: Theme,
    pub theme_light: Theme,
    /// Sorted human-visible syntax names.
    pub names: Vec<String>,
}

static GLOBAL: OnceLock<Syntaxes> = OnceLock::new();

pub fn global() -> &'static Syntaxes {
    GLOBAL.get_or_init(|| {
        let set = two_face::syntax::extra_newlines();
        let mut themes = ThemeSet::load_defaults();
        let theme_dark = themes
            .themes
            .remove("base16-eighties.dark")
            .unwrap_or_else(|| Theme::default());
        let theme_light = themes
            .themes
            .remove("InspiredGitHub")
            .unwrap_or_else(|| Theme::default());
        let mut names: Vec<String> = set
            .syntaxes()
            .iter()
            .filter(|s| !s.hidden)
            .map(|s| s.name.clone())
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        Syntaxes {
            set,
            theme_dark,
            theme_light,
            names,
        }
    })
}

/// Guess a syntax name from a file path's extension (None = plain text).
pub fn detect(path: &std::path::Path) -> Option<String> {
    let g = global();
    let ext = path.extension()?.to_str()?;
    let sr = g
        .set
        .find_syntax_by_extension(ext)
        .or_else(|| g.set.find_syntax_by_extension(&ext.to_lowercase()))?;
    if sr.name == "Plain Text" {
        None
    } else {
        Some(sr.name.clone())
    }
}

fn to_color32(c: syntect::highlighting::Color) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// Per-document highlight cache: color spans per line as (color, byte_len)
/// runs, plus parser-state snapshots at each line boundary.
pub struct HlCache {
    pub syntax: String,
    source_hash: Option<u64>,
    theme_key: Option<u8>,
    /// states[i] = (parse, highlight) state BEFORE line i. len >= 1 once used.
    states: Vec<(ParseState, HighlightState)>,
    /// spans[i] = color runs for line i (byte lengths sum to the line's
    /// byte length). None = not computed yet.
    pub spans: Vec<Option<Vec<(Color32, u32)>>>,
}

impl HlCache {
    pub fn new(syntax: String) -> Self {
        Self {
            syntax,
            source_hash: None,
            theme_key: None,
            states: Vec::new(),
            spans: Vec::new(),
        }
    }

    /// Invalidate cached spans when an editor cannot report the first line
    /// changed by an edit (egui's TextEdit only reports that text changed).
    pub fn sync_source(&mut self, text: &str) {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        if self.source_hash != Some(hash) {
            self.invalidate_all();
            self.source_hash = Some(hash);
        }
    }

    /// An edit touched `first_line`; line count is now `n_lines`.
    pub fn invalidate_from(&mut self, first_line: usize, n_lines: usize) {
        self.states.truncate(first_line + 1);
        self.spans.resize(n_lines, None);
        for s in self.spans.iter_mut().skip(first_line) {
            *s = None;
        }
    }

    pub fn invalidate_all(&mut self) {
        self.states.clear();
        self.spans.clear();
    }

    /// Compute spans for lines `0..upto` that are missing. `line_at` must
    /// return the line's text WITH a trailing '\n'.
    pub fn ensure(
        &mut self,
        upto: usize,
        n_lines: usize,
        dark: bool,
        imported_theme: Option<&Theme>,
        mut line_at: impl FnMut(usize) -> String,
    ) {
        let theme_key = if imported_theme.is_some() {
            2
        } else if dark {
            1
        } else {
            0
        };
        if self.theme_key != Some(theme_key) {
            self.invalidate_all();
            self.theme_key = Some(theme_key);
        }
        let g = global();
        let Some(sr) = g.set.find_syntax_by_name(&self.syntax) else {
            return;
        };
        let theme = imported_theme.unwrap_or(if dark { &g.theme_dark } else { &g.theme_light });
        let highlighter = Highlighter::new(theme);
        if self.spans.len() != n_lines {
            self.spans.resize(n_lines, None);
        }
        if self.states.is_empty() {
            self.states.push((
                ParseState::new(sr),
                HighlightState::new(&highlighter, ScopeStack::new()),
            ));
        }
        let upto = upto.min(n_lines);
        // parse forward from the last valid boundary
        while self.states.len() <= upto {
            let i = self.states.len() - 1;
            let line = line_at(i);
            let (mut ps, mut hs) = self.states[i].clone();
            let ops = ps.parse_line(&line, &g.set).unwrap_or_default();
            let iter = HighlightIterator::new(&mut hs, &ops[..], &line, &highlighter);
            let mut spans: Vec<(Color32, u32)> = iter
                .map(|(style, piece)| (to_color32(style.foreground), piece.len() as u32))
                .collect();
            // drop the trailing '\n' byte from the final run
            if let Some(last) = spans.last_mut() {
                if last.1 > 0 {
                    last.1 -= 1;
                }
                if last.1 == 0 {
                    spans.pop();
                }
            }
            self.spans[i] = Some(spans);
            self.states.push((ps, hs));
        }
        // fill any holes below the valid boundary (shouldn't happen, but
        // stay defensive against resize artifacts)
        for i in 0..upto {
            if self.spans[i].is_none() && i + 1 < self.states.len() {
                let line = line_at(i);
                let (mut ps, mut hs) = self.states[i].clone();
                let ops = ps.parse_line(&line, &g.set).unwrap_or_default();
                let iter =
                    HighlightIterator::new(&mut hs, &ops[..], &line, &highlighter);
                let mut spans: Vec<(Color32, u32)> = iter
                    .map(|(style, piece)| {
                        (to_color32(style.foreground), piece.len() as u32)
                    })
                    .collect();
                if let Some(last) = spans.last_mut() {
                    if last.1 > 0 {
                        last.1 -= 1;
                    }
                    if last.1 == 0 {
                        spans.pop();
                    }
                }
                self.spans[i] = Some(spans);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_extensions() {
        assert_eq!(
            detect(std::path::Path::new("x/main.rs")).as_deref(),
            Some("Rust")
        );
        assert_eq!(
            detect(std::path::Path::new("a.json")).as_deref(),
            Some("JSON")
        );
        assert!(detect(std::path::Path::new("notes.txt")).is_none());
        assert!(detect(std::path::Path::new("noext")).is_none());
    }

    #[test]
    fn highlights_rust_line_spans_match_bytes() {
        let mut hl = HlCache::new("Rust".to_string());
        let lines = ["fn main() {", "    let x = 1; // hi", "}"];
        hl.ensure(3, 3, true, None, |i| format!("{}\n", lines[i]));
        for (i, line) in lines.iter().enumerate() {
            let spans = hl.spans[i].as_ref().expect("spans computed");
            let total: u32 = spans.iter().map(|s| s.1).sum();
            assert_eq!(total as usize, line.len(), "line {i} span bytes");
        }
    }

    #[test]
    fn invalidation_truncates_states() {
        let mut hl = HlCache::new("Rust".to_string());
        let lines = ["fn a() {}", "fn b() {}", "fn c() {}"];
        hl.ensure(3, 3, true, None, |i| format!("{}\n", lines[i]));
        assert_eq!(hl.states.len(), 4);
        hl.invalidate_from(1, 3);
        assert_eq!(hl.states.len(), 2);
        assert!(hl.spans[0].is_some());
        assert!(hl.spans[1].is_none());
        assert!(hl.spans[2].is_none());
        // re-ensure recomputes
        hl.ensure(3, 3, true, None, |i| format!("{}\n", lines[i]));
        assert!(hl.spans.iter().all(|s| s.is_some()));
    }

    #[test]
    fn source_change_invalidates_cached_spans() {
        let mut hl = HlCache::new("Rust".to_string());
        hl.sync_source("fn a() {}");
        hl.ensure(1, 1, true, None, |_| "fn a() {}\n".to_string());
        assert_eq!(hl.states.len(), 2);

        hl.sync_source("fn a() {}");
        assert_eq!(hl.states.len(), 2);

        hl.sync_source("fn longer_name() {}");
        assert!(hl.states.is_empty());
        assert!(hl.spans.is_empty());
    }

    #[test]
    fn imported_theme_colors_are_used() {
        let mut imported = Theme::default();
        imported.settings.foreground = Some(syntect::highlighting::Color {
            r: 0x12,
            g: 0x34,
            b: 0x56,
            a: 0xff,
        });
        imported.settings.background = Some(syntect::highlighting::Color::BLACK);
        let mut hl = HlCache::new("Rust".to_string());
        hl.ensure(1, 1, true, Some(&imported), |_| "plain_name\n".to_string());
        let spans = hl.spans[0].as_ref().unwrap();
        assert!(spans.iter().all(|(color, _)| {
            *color == Color32::from_rgb(0x12, 0x34, 0x56)
        }));
    }
}
