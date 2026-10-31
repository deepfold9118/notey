//! Syntax highlighting in the editor: a view plugin that keeps a syntect
//! parse cache for the document (see [`HlCache`]) and colors the visible
//! lines with mark decorations. The [`SYNTAX`] facet names the grammar; a
//! new value (another language, the light or dark theme, a rebuilt grammar
//! set) starts the cache over.

use std::sync::LazyLock;

use codemirror_state::{EditorState, Extension, Facet, FacetSpec};
use codemirror_view::{
    Decoration, DecorationSet, EditorView, MarkSpec, MarkStyle, PluginValue, ViewPlugin, ViewUpdate, Viewport,
};

use crate::syntax::HlCache;

/// What an editor highlights with.
#[derive(Clone, Debug, PartialEq)]
pub struct SyntaxSpec {
    /// The syntect grammar name, such as "Rust".
    pub name: String,
    pub dark: bool,
    /// [`crate::syntax::generation`] when the spec was made, so editors
    /// notice a rebuilt grammar set.
    pub generation: u64,
}

/// The grammar of an editor; none means plain text.
pub static SYNTAX: LazyLock<Facet<SyntaxSpec, Option<SyntaxSpec>>> =
    LazyLock::new(|| Facet::define(FacetSpec::combine(|values: &[SyntaxSpec]| values.first().cloned()).compare_eq()));

/// Highlight the document with the grammar `spec` names.
pub fn highlighting(spec: SyntaxSpec) -> Extension {
    Extension::list([SYNTAX.of(spec), HIGHLIGHTER.extension()])
}

struct Highlighter {
    spec: Option<SyntaxSpec>,
    cache: Option<HlCache>,
    decorations: DecorationSet,
}

impl Highlighter {
    fn new(state: &EditorState, viewport: Viewport) -> Highlighter {
        let spec = state.facet(&SYNTAX).clone();
        let cache = spec.as_ref().map(|s| HlCache::new(s.name.clone()));
        let mut h = Highlighter { spec, cache, decorations: Decoration::none() };
        h.decorations = h.colors(state, viewport);
        h
    }

    /// Parse up to the end of the viewport, and color its lines.
    fn colors(&mut self, state: &EditorState, viewport: Viewport) -> DecorationSet {
        let (Some(spec), Some(cache)) = (&self.spec, &mut self.cache) else { return Decoration::none() };
        let doc = state.doc();
        let (first, last) = (doc.line_at(viewport.from).number, doc.line_at(viewport.to).number);
        cache.ensure(last, doc.lines(), spec.dark, None, |i| format!("{}\n", doc.line(i + 1).text));
        let mut ranges = Vec::new();
        let mut marks: Vec<(gpui::Hsla, Decoration)> = Vec::new();
        for n in first..=last {
            let Some(Some(spans)) = cache.spans.get(n - 1) else { continue };
            let mut pos = doc.line(n).from;
            for &(color, len) in spans {
                let len = len as usize;
                if len > 0 {
                    let mark = match marks.iter().find(|(c, _)| *c == color) {
                        Some((_, m)) => m.clone(),
                        None => {
                            let m = Decoration::mark(MarkSpec {
                                style: MarkStyle { color: Some(color), ..Default::default() },
                                ..Default::default()
                            });
                            marks.push((color, m.clone()));
                            m
                        }
                    };
                    ranges.push(mark.range(pos, pos + len));
                }
                pos += len;
            }
        }
        Decoration::set(ranges, false)
    }
}

impl PluginValue for Highlighter {
    fn update(&mut self, update: &ViewUpdate) {
        let spec = update.state.facet(&SYNTAX).clone();
        let respec = spec != self.spec;
        if respec {
            self.cache = spec.as_ref().map(|s| HlCache::new(s.name.clone()));
            self.spec = spec;
        } else if update.doc_changed() {
            // Lines above the first change keep their parse state.
            let mut first = usize::MAX;
            update.changes.iter_changed_ranges(|_, _, from_b, _| first = first.min(from_b), false);
            if let (Some(cache), true) = (&mut self.cache, first != usize::MAX) {
                let doc = update.state.doc();
                cache.invalidate_from(doc.line_at(first).number - 1, doc.lines());
            }
        }
        if respec || update.doc_changed() || update.viewport_changed() {
            self.decorations = self.colors(&update.state, update.viewport);
        }
    }
}

static HIGHLIGHTER: LazyLock<ViewPlugin<Highlighter>> = LazyLock::new(|| {
    ViewPlugin::define(|view: &EditorView| Highlighter::new(view.state(), view.viewport()))
        .decorations(|h| h.decorations.clone())
});
