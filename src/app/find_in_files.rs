//! Find in Files (Ctrl+Shift+F): a panel below the editor with the search
//! form and the results, which open at their line when clicked.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use codemirror_state::EditorSelection;
use codemirror_view::{scroll_target, ScrollStrategy, ScrollTarget, SCROLL_INTO_VIEW};
use gpui::{uniform_list, AnyElement};

use super::find::{field_editor, field_text, field_theme, select_field, set_field_text};
use super::*;
use crate::search::{self, GrepMatch, SearchOpts, GREP_MAX_RESULTS};

actions!(notey, [FindInFiles]);

const ROW_H: f32 = 22.0;

pub(super) struct FifPanel {
    open: bool,
    pattern: Entity<EditorView>,
    folder: Entity<EditorView>,
    filter: Entity<EditorView>,
    match_case: bool,
    regex: bool,
    config: Compartment,
    error: Option<SharedString>,
    results: Vec<GrepMatch>,
    /// The folder the results are relative to.
    root: PathBuf,
    /// The pattern of the last search, for the summary.
    searched: String,
    running: bool,
    cancel: Arc<AtomicBool>,
    _observers: Vec<Subscription>,
}

impl FifPanel {
    pub(super) fn new(p: &Palette, cx: &mut Context<NoteyApp>) -> FifPanel {
        let config = Compartment::new();
        let (pattern, folder, filter) =
            (field_editor(&config, p, cx), field_editor(&config, p, cx), field_editor(&config, p, cx));
        let observers = [&pattern, &folder, &filter].map(|v| cx.observe(v, |_, _, cx| cx.notify())).into();
        FifPanel {
            open: false,
            pattern,
            folder,
            filter,
            match_case: false,
            regex: false,
            config,
            error: None,
            results: Vec::new(),
            root: PathBuf::new(),
            searched: String::new(),
            running: false,
            cancel: Arc::default(),
            _observers: observers,
        }
    }

    pub(super) fn set_palette(&self, p: &Palette, cx: &mut App) {
        let effect = self.config.reconfigure(THEME.of(Arc::new(field_theme(p))));
        for view in [&self.pattern, &self.folder, &self.filter] {
            let effect = effect.clone();
            view.update(cx, |view, cx| view.dispatch_spec(TransactionSpec::new().effect(effect), cx));
        }
    }

    /// Whether one of the panel's fields has the keyboard focus.
    pub(super) fn focused(&self, cx: &App) -> bool {
        [&self.pattern, &self.folder, &self.filter].iter().any(|v| v.read(cx).has_focus())
    }

    /// Move the focus to the next field.
    pub(super) fn next_field(&self, window: &mut Window, cx: &mut App) {
        let fields = [&self.pattern, &self.folder, &self.filter];
        let i = fields.iter().position(|v| v.read(cx).has_focus()).map_or(0, |i| (i + 1) % 3);
        select_field(&fields[i].clone(), window, cx);
    }
}

impl NoteyApp {
    pub(super) fn open_find_in_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fif = &mut self.fif;
        fif.open = true;
        if field_text(&fif.folder, cx).trim().is_empty() {
            let dir = self.tabs.get(self.active).and_then(|t| t.doc.path.as_ref()?.parent().map(|d| d.to_path_buf()));
            if let Some(dir) = dir {
                set_field_text(&fif.folder, &dir.display().to_string(), cx);
            }
        }
        // Start from the selected text, as Find does.
        if let Some(tab) = self.tabs.get(self.active).filter(|_| !self.settings_active) {
            let sel = tab.state(cx).selection().main();
            if !sel.is_empty() && sel.to - sel.from <= 100 {
                let text = tab.state(cx).slice_doc(sel.from, sel.to);
                if !text.contains('\n') {
                    set_field_text(&fif.pattern, &text, cx);
                }
            }
        }
        select_field(&fif.pattern.clone(), window, cx);
        cx.notify();
    }

    pub(super) fn close_find_in_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.fif.open = false;
        self.fif.cancel.store(true, Ordering::Relaxed);
        self.focus_active(window, cx);
        cx.notify();
    }

    pub(super) fn start_find_in_files(&mut self, cx: &mut Context<Self>) {
        let fif = &mut self.fif;
        fif.error = None;
        let pattern = field_text(&fif.pattern, cx);
        if pattern.is_empty() {
            return;
        }
        let re = match search::compile(&pattern, SearchOpts { regex: fif.regex, match_case: fif.match_case }) {
            Ok(re) => re,
            Err(e) => {
                fif.error = Some(e.into());
                cx.notify();
                return;
            }
        };
        let root = PathBuf::from(field_text(&fif.folder, cx).trim());
        if !root.is_dir() {
            fif.error = Some("That folder does not exist".into());
            cx.notify();
            return;
        }
        // Stop a search that is still running.
        fif.cancel.store(true, Ordering::Relaxed);
        fif.cancel = Arc::default();
        let cancel = fif.cancel.clone();
        let filters = field_text(&fif.filter, cx);
        fif.results.clear();
        fif.root = root.clone();
        fif.searched = pattern;
        fif.running = true;
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        std::thread::spawn(move || {
            search::grep_dir(root, re, filters, cancel, |m| tx.unbounded_send(m).is_ok());
        });
        self._tasks.push(cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                // Take what has arrived since, so the panel redraws less often.
                let mut batch = vec![first];
                while let Ok(m) = rx.try_recv() {
                    batch.push(m);
                }
                if this.update(cx, |this, cx| {
                    this.fif.results.extend(batch);
                    cx.notify();
                }).is_err() {
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.fif.running = false;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn browse_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update(cx, |this, cx| {
                    if let Some(dir) = paths.first() {
                        set_field_text(&this.fif.folder, &dir.display().to_string(), cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Open a result's file at its line.
    fn open_result(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = self.fif.results.get(i).cloned() else { return };
        self.open_path(&m.path, window, cx);
        let Some(tab) = self.tabs.get(self.active).filter(|t| t.doc.path.as_deref() == Some(&*m.path)) else { return };
        let doc = tab.state(cx).doc();
        let line = doc.line(m.line_no.clamp(1, doc.lines()));
        let selection = EditorSelection::single(line.from, line.to);
        let target = ScrollTarget { y: ScrollStrategy::Center, ..scroll_target(selection.main()) };
        tab.view.update(cx, |view, cx| {
            view.dispatch_spec(TransactionSpec::new().selection(selection).effect(SCROLL_INTO_VIEW.of(target)), cx)
        });
    }

    pub(super) fn find_in_files_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.fif.open {
            return None;
        }
        let p = self.palette;
        let fif = &self.fif;
        let option = |id: &'static str, label: &'static str, on: bool, flip: fn(&mut FifPanel)| {
            self.icon_button(id, label, on).text_size(px(12.)).on_click(cx.listener(move |this, _, _, cx| {
                flip(&mut this.fif);
                cx.notify();
            }))
        };
        let label = |text: &'static str| div().w(px(64.)).flex_none().text_color(p.text_weak).child(text);
        let form = div()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(label("Find"))
                    .child(self.field("fif-pattern", &fif.pattern, cx))
                    .child(option("fif-case", "Aa", fif.match_case, |f| f.match_case = !f.match_case))
                    .child(option("fif-regex", ".*", fif.regex, |f| f.regex = !f.regex))
                    .child(
                        self.text_button("fif-search", if fif.running { "Stop" } else { "Search" }).on_click(
                            cx.listener(|this, _, _, cx| {
                                if this.fif.running {
                                    this.fif.cancel.store(true, Ordering::Relaxed);
                                } else {
                                    this.start_find_in_files(cx);
                                }
                            }),
                        ),
                    )
                    .child(
                        self.icon_button("fif-close", "\u{E8BB}", false)
                            .font_family(self.icon_font.clone())
                            .text_size(px(10.))
                            .on_click(cx.listener(|this, _, window, cx| this.close_find_in_files(window, cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(label("In folder"))
                    .child(self.field("fif-folder", &fif.folder, cx))
                    .child(
                        self.text_button("fif-browse", "Browse…")
                            .on_click(cx.listener(|this, _, window, cx| this.browse_folder(window, cx))),
                    )
                    .child(label("Files"))
                    .child(div().w(px(160.)).flex_none().flex().child(self.field("fif-filter", &fif.filter, cx))),
            );
        let summary: SharedString = if let Some(e) = &fif.error {
            e.clone()
        } else if fif.searched.is_empty() {
            "Enter text to find, and a folder. Files: a list such as *.rs;*.toml (empty for all).".into()
        } else {
            let n = fif.results.len();
            let more = if n >= GREP_MAX_RESULTS { " (the first ones)" } else { "" };
            let state = if fif.running { ", searching…" } else { "" };
            format!("{n} result{}{more} for \u{201c}{}\u{201d}{state}", if n == 1 { "" } else { "s" }, fif.searched).into()
        };
        let font = self.settings.editor_font.clone();
        let list = uniform_list(
            "fif-results",
            fif.results.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let p = this.palette;
                range
                    .map(|i| {
                        let m = &this.fif.results[i];
                        let rel = m.path.strip_prefix(&this.fif.root).unwrap_or(&m.path).display().to_string();
                        div()
                            .id(i)
                            .h(px(ROW_H))
                            .px_2()
                            .flex()
                            .items_center()
                            .gap_3()
                            .rounded(px(4.))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .hover(|s| s.bg(p.control_hover))
                            .child(div().flex_none().text_color(p.accent).child(format!("{rel}:{}", m.line_no)))
                            .child(div().font_family(font.clone()).child(m.preview.clone()))
                            .on_click(cx.listener(move |this, _, window, cx| this.open_result(i, window, cx)))
                    })
                    .collect()
            }),
        )
        .h(px(ROW_H * 9.))
        .w_full();
        Some(
            div()
                .id("find-in-files")
                .flex_none()
                .p_2()
                .flex()
                .flex_col()
                .gap_1p5()
                .border_t_1()
                .border_color(p.stroke)
                .bg(p.chrome)
                .text_size(px(12.))
                .child(form)
                .child(
                    div()
                        .text_color(if fif.error.is_some() { gpui::rgb(0xE85252).into() } else { p.text_weak })
                        .child(summary),
                )
                .child(list)
                .into_any_element(),
        )
    }
}
