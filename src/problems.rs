//! Problems that plugins report on ranges of a buffer (API 7): a syntax
//! error or a hint that something might be wrong. They are drawn as wavy
//! underlines, and the problem bar shows the one nearest the cursor.
//!
//! Each plugin's problems replace that plugin's earlier ones. They live in
//! a state field of the editor, in byte offsets, and move with each change
//! until the plugin checks the text again.

use std::sync::LazyLock;

use codemirror_state::{ChangeDesc, EditorState, Extension, StateEffectType, StateField, StateFieldSpec, Transaction};
use codemirror_view::{Decoration, DecorationSet, DecorationSource, MarkSpec, MarkStyle, DECORATIONS};
use gpui::{px, Hsla, UnderlineStyle};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Hint,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub from: usize,
    pub to: usize,
    pub severity: Severity,
    pub message: String,
    /// Shown in the problem bar (a plugin can keep hints to underlines).
    pub bar: bool,
    /// The underline color, from the theme.
    pub color: Hsla,
}

/// Replace the problems of one plugin (by script name).
#[derive(Clone, Debug, PartialEq)]
pub struct SetProblems {
    pub source: String,
    pub problems: Vec<Problem>,
}

pub static SET_PROBLEMS: LazyLock<StateEffectType<SetProblems>> = LazyLock::new(|| {
    StateEffectType::define_with_map(|s: &SetProblems, ch: &ChangeDesc| {
        Some(SetProblems { source: s.source.clone(), problems: map(&s.problems, ch) })
    })
});

/// Each plugin's problems, sorted by position.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Problems {
    sources: Vec<(String, Vec<Problem>)>,
}

impl Problems {
    /// All problems, sorted by position, errors first at the same place.
    pub fn all(&self) -> Vec<&Problem> {
        let mut out: Vec<&Problem> = self.sources.iter().flat_map(|(_, p)| p.iter()).collect();
        out.sort_by_key(|p| (p.from, p.severity, p.to));
        out
    }

    fn set(&mut self, source: &str, problems: Vec<Problem>) {
        self.sources.retain(|(s, _)| s != source);
        if !problems.is_empty() {
            self.sources.push((source.to_string(), problems));
        }
    }

    fn decorations(&self) -> DecorationSet {
        let ranges: Vec<_> = self
            .all()
            .into_iter()
            .filter(|p| p.from < p.to)
            .map(|p| {
                Decoration::mark(MarkSpec {
                    class: Some("notey-problem".into()),
                    style: MarkStyle {
                        underline: Some(UnderlineStyle { thickness: px(1.0), color: Some(p.color), wavy: true }),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .range(p.from, p.to)
            })
            .collect();
        Decoration::set(ranges, true)
    }
}

/// Move problems through a change. A problem whose text was deleted goes.
fn map(problems: &[Problem], changes: &ChangeDesc) -> Vec<Problem> {
    problems
        .iter()
        .filter_map(|p| {
            let (from, to) = (changes.map_pos(p.from, 1), changes.map_pos(p.to, -1));
            (from < to).then(|| Problem { from, to, ..p.clone() })
        })
        .collect()
}

static PROBLEMS: LazyLock<StateField<Problems>> = LazyLock::new(|| {
    StateField::define(
        StateFieldSpec::new(
            |_| Problems::default(),
            |problems: &Problems, tr: &Transaction| {
                let mut out = if tr.doc_changed() {
                    let ch = tr.changes().desc();
                    Problems { sources: problems.sources.iter().map(|(s, p)| (s.clone(), map(p, ch))).collect() }
                } else {
                    problems.clone()
                };
                for effect in tr.effects() {
                    if let Some(s) = effect.value(&SET_PROBLEMS) {
                        out.set(&s.source, s.problems.clone());
                    }
                }
                out
            },
        )
        .compare_eq()
        .provide(|field| DECORATIONS.from(field, |p: &Problems| DecorationSource::Set(p.decorations()))),
    )
});

/// Keep plugin problems in the editor.
pub fn problems() -> Extension {
    Extension::from(&*PROBLEMS)
}

/// The problems of a state.
pub fn problems_of(state: &EditorState) -> Option<&Problems> {
    state.field_opt(&PROBLEMS)
}

/// A problem's byte range made visible: an empty range covers the next
/// character, or the one before it at the end of a line or the text.
pub fn visible_range(text: &str, from: usize, to: usize) -> (usize, usize) {
    let (from, to) = (from.min(text.len()), to.min(text.len()));
    if from < to {
        return (from, to);
    }
    match text[from..].chars().next() {
        Some(c) if c != '\n' && c != '\r' => (from, from + c.len_utf8()),
        _ => match text[..from].chars().next_back() {
            Some(c) if c != '\n' => (from - c.len_utf8(), from),
            _ => (from, from),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codemirror_state::{ChangeSpec, EditorStateConfig, TransactionSpec};

    fn problem(from: usize, to: usize, message: &str) -> Problem {
        Problem { from, to, severity: Severity::Error, message: message.into(), bar: true, color: gpui::rgb(0xF14C4C).into() }
    }

    fn set(s: &EditorState, source: &str, problems: Vec<Problem>) -> EditorState {
        s.update(TransactionSpec::new().effect(SET_PROBLEMS.of(SetProblems { source: source.into(), problems })))
            .state()
            .clone()
    }

    fn ranges(s: &EditorState) -> Vec<(usize, usize)> {
        problems_of(s).unwrap().all().iter().map(|p| (p.from, p.to)).collect()
    }

    #[test]
    fn problems_replace_per_plugin_and_move_with_edits() {
        let s = EditorState::create(EditorStateConfig::new().doc("{\"a\": 1,}").extensions(problems()));
        let s = set(&s, "check", vec![problem(8, 9, "trailing comma")]);
        let s = set(&s, "other", vec![problem(0, 1, "x")]);
        assert_eq!(ranges(&s), [(0, 1), (8, 9)]);
        let s = set(&s, "check", vec![]);
        assert_eq!(ranges(&s), [(0, 1)]);
        let s = set(&s, "check", vec![problem(8, 9, "trailing comma")]);
        let s = s.update(TransactionSpec::new().changes(ChangeSpec::insert(0, "  "))).state().clone();
        assert_eq!(ranges(&s), [(2, 3), (10, 11)]);
        // Deleting a problem's text removes it.
        let s = s.update(TransactionSpec::new().changes(ChangeSpec::delete(9, 11))).state().clone();
        assert_eq!(ranges(&s), [(2, 3)]);
    }

    #[test]
    fn empty_ranges_cover_a_character() {
        assert_eq!(visible_range("ab\ncd", 1, 1), (1, 2));
        assert_eq!(visible_range("ab\ncd", 2, 2), (1, 2));
        assert_eq!(visible_range("ab\ncd", 5, 5), (4, 5));
        assert_eq!(visible_range("", 0, 0), (0, 0));
        assert_eq!(visible_range("é", 2, 2), (0, 2));
    }
}
