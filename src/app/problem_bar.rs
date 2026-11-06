//! The problem bar: one line above the status bar that shows the problem
//! nearest the cursor, which plugins report (API 7). It is there only
//! while the active tab has problems. The arrows step through them, and
//! clicking the message goes to the problem. Hiding it lasts until a
//! problem that was not there before shows up.

use codemirror_state::{EditorSelection, TransactionSpec};
use gpui::prelude::*;
use gpui::{div, px, AnyElement, App, Context, SharedString, Window};

use super::NoteyApp;
use crate::problems::{problems_of, Severity};

/// A problem as the bar shows it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BarProblem {
    pub from: usize,
    pub severity: Severity,
    pub message: String,
    pub line: usize,
    pub col: usize,
}

/// The problem nearest `head`: the one at the cursor or after it, else the
/// last one.
fn nearest(problems: &[BarProblem], to_of: &[usize], head: usize) -> usize {
    to_of.iter().position(|&to| to >= head).unwrap_or(problems.len().saturating_sub(1))
}

impl NoteyApp {
    /// The bar problems of the active tab, and which one is current.
    pub(super) fn bar_problems(&self, cx: &App) -> Option<(Vec<BarProblem>, usize)> {
        if self.settings_active {
            return None;
        }
        let tab = self.tabs.get(self.active)?;
        let state = tab.state(cx);
        let all = problems_of(state)?.all();
        let doc = state.doc();
        let (mut list, mut ends) = (Vec::new(), Vec::new());
        for p in all.into_iter().filter(|p| p.bar) {
            let line = doc.line_at(p.from);
            list.push(BarProblem {
                from: p.from,
                severity: p.severity,
                message: p.message.clone(),
                line: line.number,
                col: line.text[..p.from - line.from].chars().count() + 1,
            });
            ends.push(p.to);
        }
        if list.is_empty() {
            return None;
        }
        let current = nearest(&list, &ends, state.selection().main().head());
        Some((list, current))
    }

    /// What the bar says about a problem.
    pub(super) fn problem_text(&self, p: &BarProblem) -> String {
        format!("Line {}, col {}: {}", p.line, p.col, p.message)
    }

    /// The bar, if the active tab has problems and the bar is not hidden.
    pub(super) fn problem_bar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (list, current) = self.bar_problems(cx)?;
        let tab = self.tabs.get_mut(self.active)?;
        if let Some(hidden) = &tab.problems_hidden {
            if list.iter().all(|p| hidden.contains(&p.message)) {
                return None;
            }
            // a new problem brings the bar back
            tab.problems_hidden = None;
        }
        let p = self.palette;
        let problem = &list[current];
        let (glyph, color) = match problem.severity {
            Severity::Error => ("\u{E783}", p.error),
            Severity::Hint => ("\u{E7BA}", p.warning),
        };
        let from = problem.from;
        let prev = list[(current + list.len() - 1) % list.len()].from;
        let next = list[(current + 1) % list.len()].from;
        let many = list.len() > 1;
        let icon = |id: &'static str, glyph: &'static str, tip: &'static str| {
            div()
                .id(id)
                .size(px(22.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .font_family(self.icon_font.clone())
                .text_size(px(10.))
                .hover(|s| s.bg(p.control_hover))
                .child(glyph)
                .tooltip(crate::prompt::tooltip(tip))
        };
        let message: SharedString = self.problem_text(problem).into();
        Some(
            div()
                .h(px(26.))
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .px_2()
                .border_t_1()
                .border_color(p.stroke)
                .bg(p.chrome)
                .text_size(px(12.))
                .child(div().flex_none().font_family(self.icon_font.clone()).text_size(px(12.)).text_color(color).child(glyph))
                .child(
                    div()
                        .id("problem-message")
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .px_1()
                        .rounded(px(4.))
                        .text_color(p.text)
                        .hover(|s| s.bg(p.control_hover))
                        .child(message.clone())
                        .tooltip(crate::prompt::tooltip(message))
                        .on_click(cx.listener(move |this, _, window, cx| this.go_to_problem(from, window, cx))),
                )
                .when(many, |d| {
                    d.child(div().flex_none().text_color(p.text_weak).child(format!("{} of {}", current + 1, list.len())))
                        .child(
                            icon("problem-prev", "\u{E70E}", "Previous problem")
                                .on_click(cx.listener(move |this, _, window, cx| this.go_to_problem(prev, window, cx))),
                        )
                        .child(
                            icon("problem-next", "\u{E70D}", "Next problem")
                                .on_click(cx.listener(move |this, _, window, cx| this.go_to_problem(next, window, cx))),
                        )
                })
                .child(
                    icon("problem-hide", "\u{E8BB}", "Hide until a new problem shows up")
                        .on_click(cx.listener(|this, _, _, cx| this.hide_problems(cx))),
                )
                .into_any_element(),
        )
    }

    /// Put the cursor on a problem (a byte offset in the active tab).
    pub(super) fn go_to_problem(&mut self, from: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let view = tab.view.clone();
        view.update(cx, |view, cx| {
            let at = from.min(view.state().doc().len());
            view.dispatch_spec(TransactionSpec::new().selection(EditorSelection::single(at, at)).scroll_into_view(), cx);
        });
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Hide the bar for the active tab until a new problem shows up.
    pub(super) fn hide_problems(&mut self, cx: &mut Context<Self>) {
        let messages = self.bar_problems(cx).map(|(list, _)| list.into_iter().map(|p| p.message).collect());
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.problems_hidden = messages;
        }
        cx.notify();
    }

    /// Step to the previous or next problem (for screen readers).
    pub(super) fn step_problem(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((list, current)) = self.bar_problems(cx) {
            let i = if forward { (current + 1) % list.len() } else { (current + list.len() - 1) % list.len() };
            self.go_to_problem(list[i].from, window, cx);
        }
    }

    /// Whether the bar is showing (for screen readers).
    pub(super) fn problem_bar_shown(&self, cx: &App) -> Option<(Vec<BarProblem>, usize)> {
        let (list, current) = self.bar_problems(cx)?;
        let tab = self.tabs.get(self.active)?;
        match &tab.problems_hidden {
            Some(hidden) if list.iter().all(|p| hidden.contains(&p.message)) => None,
            _ => Some((list, current)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(from: usize) -> BarProblem {
        BarProblem { from, severity: Severity::Error, message: String::new(), line: 1, col: 1 }
    }

    #[test]
    fn the_current_problem_is_at_or_after_the_cursor() {
        let list = [at(2), at(10), at(20)];
        let ends = [3, 11, 21];
        assert_eq!(nearest(&list, &ends, 0), 0);
        assert_eq!(nearest(&list, &ends, 3), 0);
        assert_eq!(nearest(&list, &ends, 4), 1);
        assert_eq!(nearest(&list, &ends, 30), 2);
    }
}
