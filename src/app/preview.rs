//! The Markdown preview of a tab: off, rendered (in place of the editor), or
//! split (beside the editor). The Markdown Tools plugin switches it.

use std::collections::HashMap;

use gpui::AnyElement;

use super::*;
use crate::markdown::{self, MdStyle};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Preview {
    Rendered,
    Split,
}

impl NoteyApp {
    pub(super) fn preview_mode(&self, doc_id: u64) -> &'static str {
        match self.previews.get(&doc_id) {
            Some(Preview::Rendered) => "rendered",
            Some(Preview::Split) => "split",
            None => "off",
        }
    }

    pub(super) fn set_preview_mode(&mut self, mode: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.get(self.active).map(|t| t.doc.id) else { return };
        match mode {
            "rendered" => {
                self.previews.insert(id, Preview::Rendered);
                self.focus_active(window, cx);
            }
            "split" => {
                self.previews.insert(id, Preview::Split);
                self.focus_active(window, cx);
            }
            _ => {
                self.previews.remove(&id);
                self.focus_active(window, cx);
            }
        }
        cx.notify();
    }

    /// Flip the active tab between the editor and the rendered Markdown.
    pub(super) fn toggle_markdown_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        if tab.doc.language.as_deref() != Some("Markdown") {
            self.flash("Not a Markdown tab".into(), cx);
            return;
        }
        let mode = if self.previews.get(&tab.doc.id) == Some(&Preview::Rendered) { "off" } else { "rendered" };
        self.set_preview_mode(mode, window, cx);
    }

    /// The active tab's preview, if it has one, and whether it replaces
    /// the editor.
    pub(super) fn markdown_view(&self, cx: &App) -> Option<(AnyElement, bool)> {
        let tab = self.tabs.get(self.active)?;
        let mode = *self.previews.get(&tab.doc.id)?;
        let p = self.palette;
        let style = MdStyle {
            text: p.text,
            weak: p.text_weak,
            accent: p.accent,
            code_bg: p.control_hover,
            stroke: p.stroke,
            mono: self.settings.editor_font.clone().into(),
            font_size: self.settings.zoomed_font_size(),
        };
        let text = tab.state(cx).doc().to_string();
        let base = tab.doc.path.as_ref().and_then(|p| p.parent());
        let view = div()
            .id(("md-preview", tab.doc.id))
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .bg(p.editor)
            .when(mode == Preview::Split, |d| d.border_l_1().border_color(p.stroke))
            .child(div().max_w(px(900.)).mx_auto().px_8().py_6().child(markdown::render(&text, base, &style)))
            .into_any_element();
        Some((view, mode == Preview::Rendered))
    }
}

/// Previews by document id.
pub(super) type Previews = HashMap<u64, Preview>;
