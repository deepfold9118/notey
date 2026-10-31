//! Notey's dialogs (`window.prompt`), drawn in the window in the Fluent
//! style, as the egui version did, rather than as Windows task dialogs.
//! The first button is the default (Enter), the last one cancels (Escape).

use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{
    div, prelude::*, px, App, Context, EventEmitter, FocusHandle, Focusable, KeyDownEvent, PromptButton, PromptHandle,
    PromptLevel, PromptResponse, RenderablePromptHandle, SharedString, Window,
};

use crate::theme::palette;

static DARK: AtomicBool = AtomicBool::new(true);

/// Follow the theme setting.
pub fn set_dark(dark: bool) {
    DARK.store(dark, Ordering::Relaxed);
}

/// Use the Fluent prompt for every dialog.
pub fn init(cx: &mut App) {
    cx.set_prompt_builder(build);
}

fn build(
    _level: PromptLevel,
    message: &str,
    detail: Option<&str>,
    actions: &[PromptButton],
    handle: PromptHandle,
    window: &mut Window,
    cx: &mut App,
) -> RenderablePromptHandle {
    let view = cx.new(|cx| FluentPrompt {
        message: message.to_string().into(),
        detail: detail.map(|d| d.to_string().into()),
        actions: actions.iter().map(|a| a.label().clone()).collect(),
        focus: cx.focus_handle(),
    });
    // The prompt draws with the window, which must draw again to show it.
    window.refresh();
    handle.with_view(view, window, cx)
}

struct FluentPrompt {
    message: SharedString,
    detail: Option<SharedString>,
    actions: Vec<SharedString>,
    focus: FocusHandle,
}

impl EventEmitter<PromptResponse> for FluentPrompt {}

impl Focusable for FluentPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for FluentPrompt {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(DARK.load(Ordering::Relaxed));
        let last = self.actions.len().saturating_sub(1);
        let buttons = self.actions.iter().enumerate().map(|(i, label)| {
            let primary = i == 0;
            div()
                .id(i)
                .min_w(px(96.))
                .h(px(32.))
                .px_4()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .when(primary, |d| d.bg(p.accent).border_color(p.accent).text_color(p.editor))
                .when(!primary, |d| d.bg(p.control_hover).border_color(p.stroke).hover(|s| s.bg(p.control_active)))
                .child(label.clone())
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(PromptResponse(i))))
        });
        let dialog = div()
            .w(px(420.))
            .flex()
            .flex_col()
            .rounded(px(8.))
            .overflow_hidden()
            .bg(p.popup)
            .border_1()
            .border_color(p.stroke)
            .shadow_lg()
            .child(
                div()
                    .p_6()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_size(px(16.)).font_weight(gpui::FontWeight::SEMIBOLD).child(self.message.clone()))
                    .children(self.detail.clone().map(|d| div().text_color(p.text_weak).child(d))),
            )
            .child(div().px_6().py_4().flex().justify_end().gap_2().bg(p.chrome).children(buttons));
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(move |_, event: &KeyDownEvent, _, cx| match event.keystroke.key.as_str() {
                "enter" => cx.emit(PromptResponse(0)),
                "escape" => cx.emit(PromptResponse(last)),
                _ => {}
            }))
            .size_full()
            .absolute()
            .top_0()
            .left_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::hsla(0., 0., 0., 0.4))
            .text_color(p.text)
            .text_size(px(13.))
            .font_family("Segoe UI")
            .occlude()
            .child(dialog)
    }
}
