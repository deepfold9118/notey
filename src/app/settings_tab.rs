//! The Settings tab.

use gpui::{div, prelude::*, px, AnyElement, Context, SharedString};

use super::*;

impl NoteyApp {
    pub(super) fn settings_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let s = &self.settings;
        let spell_status = match self.spell_languages() {
            None => "Underline misspelled words. Loading the dictionaries…".to_string(),
            Some(langs) if langs.is_empty() => {
                "Underline misspelled words. Needs the Spellcheck plugin and a Language plugin".to_string()
            }
            Some(langs) => format!("Underline misspelled words: {}", langs.join(", ")),
        };
        div()
            .id("settings")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .bg(p.editor)
            .child(
                div()
                    .max_w(px(760.))
                    .mx_auto()
                    .px_8()
                    .py_6()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_size(px(26.)).font_weight(gpui::FontWeight::SEMIBOLD).mb_2().child("Settings"))
                    .child(self.section("Appearance"))
                    .child(self.row(
                        "Dark theme",
                        "Use the Fluent dark colors",
                        self.toggle("dark", s.dark, cx, |s| s.dark = !s.dark),
                    ))
                    .child(self.row(
                        "Show menu bar",
                        "When off, the menus open from the Notey icon",
                        self.toggle("menu", s.show_menu, cx, |s| s.show_menu = !s.show_menu),
                    ))
                    .child(self.row(
                        "Show status bar",
                        "Line and column, characters, zoom, line endings and encoding",
                        self.toggle("status", s.show_status, cx, |s| s.show_status = !s.show_status),
                    ))
                    .child(self.section("Editor"))
                    .child(self.row(
                        "Word wrap",
                        "Wrap long lines at the window edge",
                        self.toggle("wrap", s.word_wrap, cx, |s| s.word_wrap = !s.word_wrap),
                    ))
                    .child(self.row(
                        "Line numbers",
                        "Show line numbers beside the text",
                        self.toggle("numbers", s.line_numbers, cx, |s| s.line_numbers = !s.line_numbers),
                    ))
                    .child(self.row(
                        "Spellcheck",
                        spell_status,
                        self.toggle("spell", s.spellcheck, cx, |s| s.spellcheck = !s.spellcheck),
                    ))
                    .child(self.row(
                        "Font size",
                        format!("{} with {}", s.editor_font, if s.zoom == 100.0 { "no zoom".into() } else { format!("{:.0}% zoom", s.zoom) }),
                        self.stepper(
                            "font-size",
                            format!("{:.0} px", s.font_size),
                            cx,
                            |s| s.font_size = (s.font_size - 1.0).max(6.0),
                            |s| s.font_size = (s.font_size + 1.0).min(72.0),
                        ),
                    ))
                    .child(self.row(
                        "Line height",
                        "Space between lines",
                        self.stepper(
                            "line-height",
                            format!("{:.1}×", s.line_height),
                            cx,
                            |s| s.line_height = ((s.line_height - 0.1) * 10.0).round().max(8.0) / 10.0,
                            |s| s.line_height = ((s.line_height + 0.1) * 10.0).round().min(30.0) / 10.0,
                        ),
                    ))
                    .child(self.row(
                        "Tab size",
                        "Width of a tab, in spaces",
                        self.stepper(
                            "tab-size",
                            s.tab_size.to_string(),
                            cx,
                            |s| s.tab_size = (s.tab_size - 1).max(1),
                            |s| s.tab_size = (s.tab_size + 1).min(16),
                        ),
                    )),
            )
    }

    fn section(&self, title: &'static str) -> impl IntoElement {
        div().mt_4().mb_1().text_size(px(15.)).font_weight(gpui::FontWeight::SEMIBOLD).child(title)
    }

    /// A settings card: a label and description on the left, a control on the right.
    fn row(&self, label: &'static str, description: impl Into<SharedString>, control: AnyElement) -> impl IntoElement {
        let p = self.palette;
        div()
            .px_4()
            .py_3()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .rounded(px(6.))
            .bg(p.control)
            .border_1()
            .border_color(p.stroke)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(label)
                    .child(div().text_size(px(12.)).text_color(p.text_weak).child(description.into())),
            )
            .child(control)
    }

    /// A Fluent toggle switch.
    fn toggle(
        &self,
        id: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        flip: impl Fn(&mut Settings) + 'static,
    ) -> AnyElement {
        let p = self.palette;
        let knob = div().size(px(12.)).rounded_full().bg(if on { p.editor } else { p.text_weak });
        div()
            .id(id)
            .w(px(40.))
            .h(px(20.))
            .flex_none()
            .flex()
            .items_center()
            .px(px(3.))
            .rounded_full()
            .when(on, |d| d.bg(p.accent).justify_end())
            .when(!on, |d| d.border_1().border_color(p.text_weak))
            .child(knob)
            .on_click(cx.listener(move |this, _, _, cx| this.update_settings(&flip, cx)))
            .into_any_element()
    }

    /// A value with − and + buttons.
    fn stepper(
        &self,
        id: &'static str,
        value: String,
        cx: &mut Context<Self>,
        down: impl Fn(&mut Settings) + 'static,
        up: impl Fn(&mut Settings) + 'static,
    ) -> AnyElement {
        let p = self.palette;
        let button = |suffix: &'static str, glyph: &'static str| {
            div()
                .id((id, if suffix == "down" { 0usize } else { 1 }))
                .size(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .bg(p.control_hover)
                .hover(|s| s.bg(p.control_active))
                .child(glyph)
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .child(button("down", "−").on_click(cx.listener(move |this, _, _, cx| this.update_settings(&down, cx))))
            .child(div().w(px(56.)).flex().justify_center().child(value))
            .child(button("up", "+").on_click(cx.listener(move |this, _, _, cx| this.update_settings(&up, cx))))
            .into_any_element()
    }
}
