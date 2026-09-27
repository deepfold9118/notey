//! Syntax highlighting for both editors, built on syntect.
//!
//! Grammars are not compiled into Notey: each file type is a plugin that
//! ships Sublime `.sublime-syntax` files. The app builds a `SyntaxSet` from
//! the enabled file type plugins (in the background, with an on-disk cache)
//! and swaps it in with [`install`]; a generation counter lets highlight
//! caches notice the change.
//!
//! Highlighting is virtualization-friendly: `HlCache` snapshots the parser
//! state at every line boundary, so only lines up to the visible bottom are
//! ever parsed, and an edit invalidates from the edited line down (never
//! above). Colors come from syntect themes mapped to the Fluent dark/light
//! look; bold/italic style flags are deliberately ignored so glyph geometry
//! stays identical to the unhighlighted layout (hit-testing relies on that).

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use eframe::egui::Color32;
use syntect::highlighting::{
    HighlightState, Highlighter, HighlightIterator, Theme, ThemeSet,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxDefinition, SyntaxSet, SyntaxSetBuilder};

pub struct Syntaxes {
    pub set: SyntaxSet,
    /// Sorted human-visible syntax names.
    pub names: Vec<String>,
}

impl Syntaxes {
    fn from_set(set: SyntaxSet) -> Self {
        let mut names: Vec<String> = set
            .syntaxes()
            .iter()
            .filter(|s| !s.hidden && s.name != "Plain Text")
            .map(|s| s.name.clone())
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names.dedup();
        Self { set, names }
    }

    /// Plain text only: what the app has before any file type is enabled.
    pub fn plain() -> Self {
        let mut b = SyntaxSetBuilder::new();
        b.add_plain_text_syntax();
        Self::from_set(b.build())
    }
}

pub struct Themes {
    pub dark: Theme,
    pub light: Theme,
}

pub fn themes() -> &'static Themes {
    static THEMES: OnceLock<Themes> = OnceLock::new();
    THEMES.get_or_init(|| {
        let mut set = ThemeSet::load_defaults();
        Themes {
            dark: set.themes.remove("base16-eighties.dark").unwrap_or_default(),
            light: set.themes.remove("InspiredGitHub").unwrap_or_default(),
        }
    })
}

static CURRENT: RwLock<Option<Arc<Syntaxes>>> = RwLock::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The grammar set built from the enabled file type plugins.
pub fn global() -> Arc<Syntaxes> {
    if let Some(s) = CURRENT.read().unwrap().as_ref() {
        return Arc::clone(s);
    }
    let mut w = CURRENT.write().unwrap();
    Arc::clone(w.get_or_insert_with(|| Arc::new(Syntaxes::plain())))
}

/// Bumped whenever [`install`] swaps the grammar set.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

pub fn install(s: Syntaxes) {
    *CURRENT.write().unwrap() = Some(Arc::new(s));
    GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// Build a grammar set from `.sublime-syntax` files. `cache` is a file path
/// keyed to exactly this set of grammars; a valid cache skips the (slow)
/// YAML parsing. Grammars that fail to parse are skipped and reported.
pub fn build(grammars: &[PathBuf], cache: Option<&Path>) -> (Syntaxes, Vec<String>) {
    if let Some(path) = cache {
        if let Ok(set) = syntect::dumps::from_dump_file::<SyntaxSet, _>(path) {
            return (Syntaxes::from_set(set), Vec::new());
        }
    }
    let mut errors = Vec::new();
    let mut b = SyntaxSetBuilder::new();
    b.add_plain_text_syntax();
    for path in grammars {
        let parsed = std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|src| SyntaxDefinition::load_from_str(&src, true, None).map_err(|e| e.to_string()));
        match parsed {
            Ok(def) => b.add(def),
            Err(e) => errors.push(format!(
                "{}: {e}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
    }
    let set = b.build();
    if let Some(path) = cache {
        let _ = syntect::dumps::dump_to_file(&set, path);
    }
    (Syntaxes::from_set(set), errors)
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
    /// Grammar-set generation the cached spans were computed with.
    generation: u64,
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
            generation: generation(),
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
        let current = generation();
        if self.generation != current {
            self.invalidate_all(); // file type plugins changed
            self.generation = current;
        }
        let g = global();
        let Some(sr) = g.set.find_syntax_by_name(&self.syntax) else {
            return;
        };
        let theme = imported_theme.unwrap_or(if dark { &themes().dark } else { &themes().light });
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
pub(crate) mod tests {
    use super::*;

    /// Install the repo's official file type plugins (once per test run).
    pub(crate) fn install_official() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/filetypes");
            let mut grammars = Vec::new();
            for dir in std::fs::read_dir(root).unwrap().filter_map(Result::ok) {
                for f in std::fs::read_dir(dir.path()).unwrap().filter_map(Result::ok) {
                    if f.path().extension().is_some_and(|e| e == "sublime-syntax") {
                        grammars.push(f.path());
                    }
                }
            }
            let (set, errors) = build(&grammars, None);
            assert!(errors.is_empty(), "{errors:?}");
            install(set);
        });
    }

    #[test]
    fn detects_common_extensions() {
        install_official();
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
        install_official();
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
        install_official();
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
        install_official();
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
        install_official();
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
