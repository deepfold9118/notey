//! Text styles that plugins put on ranges of a buffer (API 3): a text color
//! and a background color. Notey keeps them in memory only. They are not
//! written to the file.
//!
//! The runs live in a state field of the editor, in byte offsets, and move
//! with each change: text typed inside a run gets its colors, text typed at
//! the edge of a run does not. They are drawn as mark decorations above the
//! syntax colors.

use std::sync::LazyLock;

use codemirror_state::{
    ChangeDesc, Extension, Prec, StateEffectType, StateField, StateFieldSpec, Transaction,
};
use codemirror_view::{Decoration, DecorationSet, DecorationSource, MarkSpec, MarkStyle, DECORATIONS};
use gpui::Hsla;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    pub start: usize,
    pub end: usize,
    pub fg: Option<Hsla>,
    pub bg: Option<Hsla>,
}

/// What to do with one property of the styled range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    Keep,
    Set(Hsla),
    Clear,
}

impl Change {
    fn over(self, old: Option<Hsla>) -> Option<Hsla> {
        match self {
            Change::Keep => old,
            Change::Set(c) => Some(c),
            Change::Clear => None,
        }
    }
}

/// Set or clear the colors of the byte range `from..to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StyleChange {
    pub from: usize,
    pub to: usize,
    pub fg: Change,
    pub bg: Change,
}

/// The effect that changes text styles.
pub static STYLE_RANGE: LazyLock<StateEffectType<StyleChange>> = LazyLock::new(|| {
    StateEffectType::define_with_map(|c: &StyleChange, ch: &ChangeDesc| {
        Some(StyleChange { from: ch.map_pos(c.from, 1), to: ch.map_pos(c.to, -1), ..*c })
    })
});

/// The runs: sorted, with no overlaps, and each with a color.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Styles {
    runs: Vec<Run>,
}

impl Styles {
    #[cfg(test)]
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// Set or clear the colors of `a..b`.
    pub fn apply(&mut self, a: usize, b: usize, fg: Change, bg: Change) {
        let (a, b) = (a.min(b), a.max(b));
        if a == b {
            return;
        }
        let mut out = Vec::with_capacity(self.runs.len() + 3);
        let mut pos = a; // the first position in a..b that no run covers
        for r in self.runs.drain(..) {
            if r.end <= a || r.start >= b {
                out.push(r);
                continue;
            }
            if r.start < a {
                out.push(Run { end: a, ..r });
            }
            let s = r.start.max(a);
            if s > pos {
                out.push(Run { start: pos, end: s, fg: fg.over(None), bg: bg.over(None) });
            }
            let e = r.end.min(b);
            out.push(Run { start: s, end: e, fg: fg.over(r.fg), bg: bg.over(r.bg) });
            pos = e;
            if r.end > b {
                out.push(Run { start: b, ..r });
            }
        }
        if pos < b {
            out.push(Run { start: pos, end: b, fg: fg.over(None), bg: bg.over(None) });
        }
        out.sort_by_key(|r| r.start);
        self.runs = normalize(out);
    }

    /// Move the runs through a change.
    fn map(&self, changes: &ChangeDesc) -> Styles {
        let runs = self
            .runs
            .iter()
            .map(|r| Run { start: changes.map_pos(r.start, 1), end: changes.map_pos(r.end, -1), ..*r })
            .collect();
        Styles { runs: normalize(runs) }
    }

    fn decorations(&self) -> DecorationSet {
        let ranges = self
            .runs
            .iter()
            .map(|r| {
                Decoration::mark(MarkSpec {
                    class: Some("notey-text-style".into()),
                    style: MarkStyle { color: r.fg, background: r.bg, ..Default::default() },
                    ..Default::default()
                })
                .range(r.start, r.end)
            })
            .collect();
        Decoration::set(ranges, false)
    }
}

/// Remove empty and colorless runs, and join neighbors with the same colors.
fn normalize(runs: Vec<Run>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::with_capacity(runs.len());
    for r in runs {
        if r.start >= r.end || (r.fg.is_none() && r.bg.is_none()) {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end == r.start && last.fg == r.fg && last.bg == r.bg => last.end = r.end,
            _ => out.push(r),
        }
    }
    out
}

static STYLES: LazyLock<StateField<Styles>> = LazyLock::new(|| {
    StateField::define(
        StateFieldSpec::new(
            |_| Styles::default(),
            |styles: &Styles, tr: &Transaction| {
                let mut styles = if tr.doc_changed() { styles.map(tr.changes().desc()) } else { styles.clone() };
                for effect in tr.effects() {
                    if let Some(c) = effect.value(&STYLE_RANGE) {
                        styles.apply(c.from, c.to, c.fg, c.bg);
                    }
                }
                styles
            },
        )
        .compare_eq()
        // Low precedence: as in CodeMirror, a lower-precedence mark is inner,
        // so its color wins over the syntax colors.
        .provide(|field| Prec::low(DECORATIONS.from(field, |s: &Styles| DecorationSource::Set(s.decorations())))),
    )
});

/// Keep plugin text styles in the editor.
pub fn text_styles() -> Extension {
    Extension::from(&*STYLES)
}

#[cfg(test)]
/// The text styles of a state.
pub fn styles_of(state: &codemirror_state::EditorState) -> Option<&Styles> {
    state.field_opt(&STYLES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codemirror_state::{ChangeSpec, EditorState, EditorStateConfig, TransactionSpec};

    fn red() -> Hsla {
        gpui::rgb(0xcd3131).into()
    }
    fn blue() -> Hsla {
        gpui::rgb(0x2472c8).into()
    }

    fn state(doc: &str) -> EditorState {
        EditorState::create(EditorStateConfig::new().doc(doc).extensions(text_styles()))
    }

    fn style(s: &EditorState, from: usize, to: usize, fg: Change, bg: Change) -> EditorState {
        s.update(TransactionSpec::new().effect(STYLE_RANGE.of(StyleChange { from, to, fg, bg }))).state().clone()
    }

    fn runs(s: &EditorState) -> Vec<(usize, usize)> {
        styles_of(s).unwrap().runs().iter().map(|r| (r.start, r.end)).collect()
    }

    #[test]
    fn colors_split_and_merge() {
        let s = style(&state("hello world"), 0, 5, Change::Set(red()), Change::Keep);
        let s = style(&s, 3, 8, Change::Keep, Change::Set(blue()));
        let r = styles_of(&s).unwrap().runs();
        assert_eq!(r.len(), 3);
        assert_eq!((r[0].start, r[0].end, r[0].fg, r[0].bg), (0, 3, Some(red()), None));
        assert_eq!((r[1].start, r[1].end, r[1].fg, r[1].bg), (3, 5, Some(red()), Some(blue())));
        assert_eq!((r[2].start, r[2].end, r[2].fg, r[2].bg), (5, 8, None, Some(blue())));
        let s = style(&s, 0, 11, Change::Clear, Change::Clear);
        assert!(runs(&s).is_empty());
    }

    #[test]
    fn runs_move_with_edits() {
        let s = style(&state("abc def ghi"), 4, 7, Change::Set(red()), Change::Keep);
        // Typing before the run moves it.
        let s = s.update(TransactionSpec::new().changes(ChangeSpec::insert(0, "XX"))).state().clone();
        assert_eq!(runs(&s), [(6, 9)]);
        // Typing inside it grows it; at its edges it does not.
        let s = s.update(TransactionSpec::new().changes(ChangeSpec::insert(7, "-"))).state().clone();
        assert_eq!(runs(&s), [(6, 10)]);
        let s = s.update(TransactionSpec::new().changes(ChangeSpec::insert(6, "<"))).state().clone();
        let s = s.update(TransactionSpec::new().changes(ChangeSpec::insert(11, ">"))).state().clone();
        assert_eq!(runs(&s), [(7, 11)]);
        // Deleting it removes it.
        let s = s.update(TransactionSpec::new().changes(ChangeSpec::delete(5, 12))).state().clone();
        assert!(runs(&s).is_empty());
    }
}
