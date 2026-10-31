//! Syntax highlighting: building the grammar set from the enabled File Type
//! plugins, and the language picker in the status bar.

use std::hash::{DefaultHasher, Hash, Hasher};
use gpui::{anchored, deferred, uniform_list, AnyElement, Corner};

use super::chrome::Menu;
use super::find::{field_editor, field_text, field_theme, select_field, set_field_text};
use super::*;
use crate::packages::{Kind, Store};
use crate::syntax;

impl NoteyApp {
    /// Build the grammar set from the enabled File Type plugins in the
    /// background, with an on-disk cache keyed to exactly those plugins
    /// (the same cache the egui version kept).
    pub(super) fn start_syntax_build(&mut self, cx: &mut Context<Self>) {
        let store = Store::open();
        let mut grammars = Vec::new();
        let mut hasher = DefaultHasher::new();
        let mut enabled: Vec<_> = store.enabled(Kind::Filetype).collect();
        enabled.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        for p in &enabled {
            (&p.manifest.id, &p.manifest.version).hash(&mut hasher);
            for f in &p.manifest.files {
                if f.path.ends_with(".sublime-syntax") {
                    f.sha256.hash(&mut hasher);
                    grammars.push(p.file(&f.path));
                }
            }
        }
        let cache_dir = store.root().join(".cache");
        let _ = std::fs::create_dir_all(&cache_dir);
        let cache = cache_dir.join(format!("syntaxes-{:016x}.bin", hasher.finish()));
        // Drop the caches of other plugin combinations.
        if let Ok(entries) = std::fs::read_dir(&cache_dir) {
            for e in entries.filter_map(Result::ok) {
                if e.path() != cache && e.file_name().to_string_lossy().starts_with("syntaxes-") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let building = cx.background_executor().spawn(async move {
            let cache = if grammars.is_empty() { None } else { Some(cache) };
            syntax::build(&grammars, cache.as_deref())
        });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let (set, errors) = building.await;
            syntax::install(set);
            let _ = this.update(cx, |this, cx| {
                if let Some(e) = errors.first() {
                    this.flash(format!("A file type failed to load: {e}"), cx);
                }
                // Files opened before their file type was available.
                for tab in &mut this.tabs {
                    if tab.doc.language.is_none() {
                        if let Some(path) = &tab.doc.path {
                            tab.doc.language = syntax::detect(path);
                        }
                    }
                }
                this.update_settings(|_| {}, cx);
            });
        }));
    }

    pub(super) fn set_language(&mut self, language: Option<String>, cx: &mut Context<Self>) {
        let idx = self.active;
        self.tabs[idx].doc.language = language;
        self.reconfigure_tab(idx, cx);
        self.session_dirty = true;
        cx.notify();
    }
}

/// The language picker's filter field.
pub(super) struct LanguagesUi {
    pub(super) filter: Entity<EditorView>,
    config: Compartment,
    /// Focus the filter at the next render (the picker just opened).
    pub(super) focus_pending: bool,
    _observe: Subscription,
}

impl LanguagesUi {
    pub(super) fn new(p: &Palette, cx: &mut Context<NoteyApp>) -> LanguagesUi {
        let config = Compartment::new();
        let filter = field_editor(&config, p, cx);
        let observe = cx.observe(&filter, |_, _, cx| cx.notify());
        LanguagesUi { filter, config, focus_pending: false, _observe: observe }
    }

    pub(super) fn set_palette(&self, p: &Palette, cx: &mut App) {
        let effect = self.config.reconfigure(THEME.of(Arc::new(field_theme(p))));
        self.filter.update(cx, |view, cx| view.dispatch_spec(TransactionSpec::new().effect(effect), cx));
    }
}

const ROW_H: f32 = 28.0;

impl NoteyApp {
    /// Plain Text and the installed file types that match the filter.
    fn language_choices(&self, cx: &App) -> Vec<Option<String>> {
        let filter = field_text(&self.languages.filter, cx).to_lowercase();
        std::iter::once(None)
            .chain(syntax::global().names.iter().cloned().map(Some))
            .filter(|l| l.as_deref().unwrap_or("Plain Text").to_lowercase().contains(&filter))
            .collect()
    }

    /// Enter in the filter: take the first language that matches.
    pub(super) fn pick_first_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(language) = self.language_choices(cx).into_iter().next() {
            self.close_language_picker(window, cx);
            self.set_language(language, cx);
        }
    }

    pub(super) fn close_language_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_menu = None;
        set_field_text(&self.languages.filter, "", cx);
        self.focus_active(window, cx);
        cx.notify();
    }

    /// The language picker above the status bar item: a filter, the
    /// languages, and a link to more File Type plugins.
    pub(super) fn language_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.open_menu != Some(Menu::Language) {
            return None;
        }
        if std::mem::take(&mut self.languages.focus_pending) {
            select_field(&self.languages.filter.clone(), window, cx);
        }
        let p = self.palette;
        let choices = self.language_choices(cx);
        let current = self.tabs.get(self.active).and_then(|t| t.doc.language.clone());
        let rows = choices.len();
        let list = uniform_list(
            "language-list",
            rows,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let p = this.palette;
                range
                    .map(|i| {
                        let language = choices[i].clone();
                        let selected = language == current;
                        let label = language.clone().unwrap_or_else(|| "Plain Text".into());
                        div()
                            .id(i)
                            .h(px(ROW_H))
                            .px_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded(px(4.))
                            .hover(|s| s.bg(p.control_hover))
                            .child(div().w(px(12.)).text_color(p.accent).child(if selected { "✓" } else { "" }))
                            .child(label)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close_language_picker(window, cx);
                                this.set_language(language.clone(), cx);
                            }))
                    })
                    .collect()
            }),
        )
        .h(px(ROW_H * (rows.clamp(1, 12) as f32)));
        let panel = div()
            .id("language-picker")
            .occlude()
            .w(px(260.))
            .p_1p5()
            .flex()
            .flex_col()
            .gap_1p5()
            .bg(p.popup)
            .border_1()
            .border_color(p.stroke)
            .rounded(px(8.))
            .shadow_lg()
            .child(div().flex().child(self.hinted_field("language-filter", &self.languages.filter, "Filter languages…", cx)))
            .child(list)
            .child(div().h(px(1.)).bg(p.stroke))
            .child(
                div()
                    .id("more-file-types")
                    .h(px(ROW_H))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .hover(|s| s.bg(p.control_hover))
                    .child("More file types…")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_language_picker(window, cx);
                        this.open_plugins(Kind::Filetype, window, cx);
                    })),
            )
            .on_mouse_down_out(cx.listener(|this, ev: &gpui::MouseDownEvent, window, cx| {
                // A click on the status bar item toggles the picker itself.
                let on_status_bar = f32::from(ev.position.y) >= f32::from(window.viewport_size().height) - 26.0;
                if !on_status_bar {
                    this.close_language_picker(window, cx);
                }
            }));
        Some(
            div()
                .absolute()
                .left_0()
                .bottom(gpui::relative(1.))
                .child(deferred(anchored().anchor(Corner::BottomLeft).snap_to_window_with_margin(px(8.)).child(panel)).with_priority(1))
                .into_any_element(),
        )
    }
}
