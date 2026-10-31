//! Spellcheck in the editor: a view plugin that underlines the misspelled
//! words in the viewport. The checker comes from the [`SPELL`] facet, so
//! giving editors a new [`SpellHandle`] (after the dictionaries load, or a
//! word is added) redraws the underlines.

use std::sync::{Arc, LazyLock, RwLock};

use codemirror_state::{EditorState, Extension, Facet, FacetSpec};
use codemirror_view::{
    Decoration, DecorationSet, EditorView, MarkSpec, MarkStyle, PluginValue, ViewPlugin, ViewUpdate, Viewport,
};
use gpui::{px, rgb, UnderlineStyle};

use crate::spell::{misspelled, Spell};

/// The shared spell checker, and a counter that changes when its word
/// list does, so editors notice.
#[derive(Clone)]
pub struct SpellHandle {
    pub spell: Arc<RwLock<Spell>>,
    generation: u64,
}

impl PartialEq for SpellHandle {
    fn eq(&self, other: &SpellHandle) -> bool {
        Arc::ptr_eq(&self.spell, &other.spell) && self.generation == other.generation
    }
}

impl SpellHandle {
    pub fn new(spell: Spell) -> SpellHandle {
        SpellHandle { spell: Arc::new(RwLock::new(spell)), generation: 0 }
    }

    /// Accept `word` from now on. Returns the handle to give the editors.
    pub fn add_word(&self, word: &str) -> SpellHandle {
        self.spell.write().unwrap().add_word(word);
        SpellHandle { spell: self.spell.clone(), generation: self.generation + 1 }
    }
}

/// The spell checker for an editor; none turns spellcheck off.
pub static SPELL: LazyLock<Facet<SpellHandle, Option<SpellHandle>>> =
    LazyLock::new(|| Facet::define(FacetSpec::combine(|values: &[SpellHandle]| values.first().cloned()).compare_eq()));

/// Underline misspelled words, checked with `handle`.
pub fn spellcheck(handle: SpellHandle) -> Extension {
    Extension::list([SPELL.of(handle), SPELL_PLUGIN.extension()])
}

struct SpellPlugin {
    decorations: DecorationSet,
}

impl PluginValue for SpellPlugin {
    fn update(&mut self, update: &ViewUpdate) {
        if update.doc_changed()
            || update.viewport_changed()
            || update.state.facet(&SPELL) != update.start_state.facet(&SPELL)
        {
            self.decorations = underlines(&update.state, update.viewport);
        }
    }
}

static SPELL_PLUGIN: LazyLock<ViewPlugin<SpellPlugin>> = LazyLock::new(|| {
    ViewPlugin::define(|view: &EditorView| SpellPlugin { decorations: underlines(view.state(), view.viewport()) })
        .decorations(|p| p.decorations.clone())
});

fn underlines(state: &EditorState, viewport: Viewport) -> DecorationSet {
    let Some(handle) = state.facet(&SPELL) else { return Decoration::none() };
    let spell = handle.spell.read().unwrap();
    if !spell.available() {
        return Decoration::none();
    }
    let mark = Decoration::mark(MarkSpec {
        class: Some("notey-misspelled".into()),
        style: MarkStyle {
            underline: Some(UnderlineStyle { thickness: px(1.0), color: Some(rgb(0xE85252).into()), wavy: true }),
            ..Default::default()
        },
        ..Default::default()
    });
    let doc = state.doc();
    let (first, last) = (doc.line_at(viewport.from).number, doc.line_at(viewport.to).number);
    let mut ranges = Vec::new();
    for n in first..=last {
        let line = doc.line(n);
        for (a, b) in misspelled(&spell, &line.text) {
            ranges.push(mark.range(line.from + a, line.from + b));
        }
    }
    Decoration::set(ranges, false)
}
