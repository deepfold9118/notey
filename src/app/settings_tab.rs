//! The Settings tab: a list of sections on the left and the settings of the
//! selected section on the right.

use gpui::{anchored, deferred, div, prelude::*, px, uniform_list, AnyElement, Context, Corner, SharedString};

use super::find::{field_editor, field_text, field_theme, select_field};
use super::*;
use crate::packages::Kind;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Section {
    Editor,
    Appearance,
    Spellcheck,
    Plugins,
}

impl Section {
    const ALL: [Section; 4] = [Section::Editor, Section::Appearance, Section::Spellcheck, Section::Plugins];

    fn label(self) -> &'static str {
        match self {
            Section::Editor => "Editor",
            Section::Appearance => "Appearance",
            Section::Spellcheck => "Spellcheck",
            Section::Plugins => "Plugins",
        }
    }
}

/// The Settings tab's own state.
pub(super) struct SettingsUi {
    pub(super) section: Section,
    font_menu: bool,
    font_filter: Entity<EditorView>,
    /// Installed font families, loaded when the font menu first opens.
    fonts: Vec<SharedString>,
    config: Compartment,
    _observe: Subscription,
}

impl SettingsUi {
    pub(super) fn new(p: &Palette, cx: &mut Context<NoteyApp>) -> SettingsUi {
        let config = Compartment::new();
        let font_filter = field_editor(&config, p, cx);
        let observe = cx.observe(&font_filter, |_, _, cx| cx.notify());
        SettingsUi { section: Section::Editor, font_menu: false, font_filter, fonts: Vec::new(), config, _observe: observe }
    }

    pub(super) fn set_palette(&self, p: &Palette, cx: &mut App) {
        let effect = self.config.reconfigure(THEME.of(Arc::new(field_theme(p))));
        self.font_filter.update(cx, |view, cx| view.dispatch_spec(TransactionSpec::new().effect(effect), cx));
    }
}

const ROW_H: f32 = 28.0;

impl NoteyApp {
    pub(super) fn settings_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let nav = div()
            .w(px(200.))
            .flex_none()
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
            .bg(p.editor)
            .border_r_1()
            .border_color(p.stroke)
            .child(div().px_2().pt_2().pb_3().text_size(px(20.)).font_weight(gpui::FontWeight::SEMIBOLD).child("Settings"))
            .children(Section::ALL.map(|section| {
                let selected = self.settings_ui.section == section;
                div()
                    .id(section.label())
                    .h(px(32.))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .when(selected, |d| d.bg(p.control_active))
                    .when(!selected, |d| d.hover(|s| s.bg(p.control_hover)))
                    .child(section.label())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_ui.section = section;
                        this.settings_ui.font_menu = false;
                        cx.notify();
                    }))
            }))
            .child(div().flex_1())
            .child(
                self.text_button("reset-settings", "Reset to defaults")
                    .on_click(cx.listener(|this, _, _, cx| this.reset_settings(cx))),
            );
        let section = self.settings_ui.section;
        let content = match section {
            Section::Editor => self.settings_editor(cx),
            Section::Appearance => self.settings_appearance(cx),
            Section::Spellcheck => self.settings_spellcheck(cx),
            Section::Plugins => self.settings_plugins(window, cx),
        };
        div().flex_1().min_h_0().flex().bg(p.editor).child(nav).child(
            div().id("settings-content").flex_1().min_w_0().overflow_y_scroll().child(
                div()
                    .max_w(px(760.))
                    .px_8()
                    .py_6()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().mb_2().text_size(px(20.)).font_weight(gpui::FontWeight::SEMIBOLD).child(section.label()))
                    .child(content),
            ),
        )
    }

    /// Show the Settings tab at a section.
    pub(super) fn open_settings_at(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_ui.section = section;
        self.open_settings(window, cx);
    }

    /// Reset the editor and layout settings. The theme does not change.
    fn reset_settings(&mut self, cx: &mut Context<Self>) {
        self.update_settings(
            |s| {
                let d = Settings::default();
                s.font_size = d.font_size;
                s.line_height = d.line_height;
                s.tab_size = d.tab_size;
                s.line_numbers = d.line_numbers;
                s.ligatures = d.ligatures;
                s.word_wrap = d.word_wrap;
                s.show_menu = d.show_menu;
                s.show_status = d.show_status;
                s.tabs_left = d.tabs_left;
                s.autosave = d.autosave;
                s.autosave_secs = d.autosave_secs;
                s.editor_font = d.editor_font;
            },
            cx,
        );
    }

    fn settings_editor(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let s = self.settings.clone();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child({
                let picker = self.font_picker(cx);
                self.row("Font", "The editor font", picker)
            })
            .child(self.row(
                "Font size",
                "Before zoom (Ctrl+Plus, Ctrl+Minus and Ctrl+0; the status bar shows the zoom)",
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
                    format!("{:.2}×", s.line_height),
                    cx,
                    |s| s.line_height = ((s.line_height - 0.05) * 20.0).round().max(16.0) / 20.0,
                    |s| s.line_height = ((s.line_height + 0.05) * 20.0).round().min(60.0) / 20.0,
                ),
            ))
            .child(self.row(
                "Tab size",
                "Width of a tab, in spaces",
                self.stepper(
                    "tab-size",
                    format!("{} spaces", s.tab_size),
                    cx,
                    |s| s.tab_size = (s.tab_size - 1).max(1),
                    |s| s.tab_size = (s.tab_size + 1).min(16),
                ),
            ))
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
                "Ligatures",
                "Let fonts such as Cascadia Code or Fira Code join characters like -> and != into one symbol",
                self.toggle("ligatures", s.ligatures, cx, |s| s.ligatures = !s.ligatures),
            ))
            .child(self.row(
                "Autosave",
                "Save modified files that have a file name, on a timer",
                self.toggle("autosave", s.autosave, cx, |s| s.autosave = !s.autosave),
            ))
            .when(s.autosave, |d| {
                d.child(self.row(
                    "Autosave interval",
                    "Seconds between autosaves",
                    self.stepper(
                        "autosave-secs",
                        format!("{:.0} s", s.autosave_secs),
                        cx,
                        |s| s.autosave_secs = (s.autosave_secs - 5.0).max(5.0),
                        |s| s.autosave_secs = (s.autosave_secs + 5.0).min(600.0),
                    ),
                ))
            })
            .into_any_element()
    }

    /// The font button, and its menu with a filter field.
    fn font_picker(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let button = div()
            .id("font-button")
            .w(px(240.))
            .h(px(30.))
            .px_3()
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(4.))
            .bg(p.control_hover)
            .border_1()
            .border_color(p.stroke)
            .hover(|s| s.bg(p.control_active))
            .child(div().font_family(self.settings.editor_font.clone()).child(self.settings.editor_font.clone()))
            .child(div().font_family(self.icon_font.clone()).text_size(px(10.)).child("\u{E70D}"))
            .on_click(cx.listener(|this, _, window, cx| {
                this.settings_ui.font_menu = !this.settings_ui.font_menu;
                if this.settings_ui.font_menu {
                    if this.settings_ui.fonts.is_empty() {
                        let mut fonts = cx.text_system().all_font_names();
                        fonts.sort_by_key(|f| f.to_lowercase());
                        fonts.dedup();
                        this.settings_ui.fonts =
                            fonts.into_iter().filter(|f| !f.starts_with('@') && !f.starts_with('.')).map(Into::into).collect();
                    }
                    select_field(&this.settings_ui.font_filter.clone(), window, cx);
                }
                cx.notify();
            }));
        let mut picker = div().relative().child(button);
        if self.settings_ui.font_menu {
            let filter = field_text(&self.settings_ui.font_filter, cx).to_lowercase();
            let fonts: Vec<SharedString> =
                self.settings_ui.fonts.iter().filter(|f| f.to_lowercase().contains(&filter)).cloned().collect();
            let current = self.settings.editor_font.clone();
            let list = uniform_list(
                "font-list",
                fonts.len(),
                cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    let p = this.palette;
                    range
                        .map(|i| {
                            let name = fonts[i].clone();
                            let selected = name.as_ref() == current.as_str();
                            div()
                                .id(i)
                                .h(px(ROW_H))
                                .px_3()
                                .flex()
                                .items_center()
                                .rounded(px(4.))
                                .when(selected, |d| d.bg(p.control_active))
                                .hover(|s| s.bg(p.control_hover))
                                .font_family(name.clone())
                                .child(name.clone())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.settings_ui.font_menu = false;
                                    let name = name.to_string();
                                    this.update_settings(|s| s.editor_font = name, cx);
                                }))
                        })
                        .collect()
                }),
            )
            .h(px(ROW_H * 10.));
            let menu = div()
                .id("font-menu")
                .occlude()
                .w(px(300.))
                .p_1p5()
                .flex()
                .flex_col()
                .gap_1p5()
                .bg(p.popup)
                .border_1()
                .border_color(p.stroke)
                .rounded(px(8.))
                .shadow_lg()
                .child(div().flex().child(self.hinted_field("font-filter", &self.settings_ui.font_filter, "Filter…", cx)))
                .child(list)
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.settings_ui.font_menu = false;
                    cx.notify();
                }));
            picker = picker.child(
                div()
                    .absolute()
                    .top(px(32.))
                    .right_0()
                    .child(deferred(anchored().anchor(Corner::TopRight).snap_to_window_with_margin(px(8.)).child(menu))),
            );
        }
        picker.into_any_element()
    }

    fn settings_appearance(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let s = self.settings.clone();
        let builtin = s.theme_path.is_none();
        let mut themes = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(self.radio("theme-dark", builtin && s.dark, "Notey Dark", cx, |this, cx| this.use_builtin_theme(true, cx)))
            .child(self.radio("theme-light", builtin && !s.dark, "Notey Light", cx, |this, cx| {
                this.use_builtin_theme(false, cx)
            }));
        themes = self.imported_theme_choices(themes, cx);
        let tabs = div()
            .flex()
            .gap_1()
            .child(self.segment("tabs-top", !s.tabs_left, "Top", cx, |s| s.tabs_left = false))
            .child(self.segment("tabs-left", s.tabs_left, "Left", cx, |s| s.tabs_left = true));
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(self.section_heading("Theme"))
            .child(self.card(themes.into_any_element()))
            .child(self.theme_import_card(cx))
            .child(self.section_heading("Layout"))
            .child(self.row("Tabs", "In the title bar, or as a vertical list on the left", tabs.into_any_element()))
            .child(self.row(
                "Show menu bar",
                "When off, the menus open from the Notey icon",
                self.toggle("menu", s.show_menu, cx, |s| s.show_menu = !s.show_menu),
            ))
            .child(self.row(
                "Show status bar",
                if s.tabs_left {
                    "Line and column, characters, zoom, language, line endings and encoding"
                } else {
                    "It also has the Settings gear; without it, use Ctrl+, or Edit > Settings"
                },
                self.toggle("status", s.show_status, cx, |s| s.show_status = !s.show_status),
            ))
            .into_any_element()
    }

    fn settings_spellcheck(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let mut body = div().flex().flex_col().gap_2();
        match self.spell_languages() {
            None => body = body.child(div().text_color(p.text_weak).child("Loading the dictionaries…")),
            Some(langs) if langs.is_empty() => {
                let buttons = div()
                    .flex()
                    .gap_2()
                    .child(
                        self.text_button("install-spellcheck", "Install the Spellcheck feature")
                            .on_click(cx.listener(|this, _, window, cx| this.open_plugins(Kind::Feature, window, cx))),
                    )
                    .child(
                        self.text_button("install-language", "Install a language")
                            .on_click(cx.listener(|this, _, window, cx| this.open_plugins(Kind::Language, window, cx))),
                    );
                body = body.child(self.card(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child("Spellcheck needs the Spellcheck feature and at least one language.")
                        .child(buttons)
                        .into_any_element(),
                ));
            }
            Some(langs) => {
                let list = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(langs.into_iter().map(|l| div().child(format!("• {l}"))))
                    .child(
                        div().flex().child(
                            self.text_button("add-languages", "Add or Remove Languages…")
                                .on_click(cx.listener(|this, _, window, cx| this.open_plugins(Kind::Language, window, cx))),
                        ),
                    );
                body = body
                    .child(self.row(
                        "Check spelling while you type",
                        "Misspelled words get a wavy underline; right-click one for suggestions",
                        self.toggle("spell", self.settings.spellcheck, cx, |s| s.spellcheck = !s.spellcheck),
                    ))
                    .child(self.section_heading("Languages"))
                    .child(self.card(list.into_any_element()));
                let word_list = crate::packages::store_root()
                    .and_then(|r| r.parent().map(|p| p.join("user_dictionary.txt")))
                    .filter(|p| p.is_file());
                if let Some(path) = word_list {
                    body = body.child(self.section_heading("Personal word list")).child(self.row(
                        "Your words",
                        "The words that you added with \u{201c}Add to dictionary\u{201d}",
                        self.text_button("open-word-list", "Open Personal Word List")
                            .on_click(cx.listener(move |this, _, window, cx| this.open_path(&path, window, cx)))
                            .into_any_element(),
                    ));
                }
            }
        }
        body.into_any_element()
    }

    pub(super) fn section_heading(&self, title: &'static str) -> impl IntoElement {
        div().mt_3().text_size(px(14.)).font_weight(gpui::FontWeight::SEMIBOLD).child(title)
    }

    /// A settings card around any content.
    pub(super) fn card(&self, content: AnyElement) -> impl IntoElement {
        let p = self.palette;
        div().px_4().py_3().rounded(px(6.)).bg(p.control).border_1().border_color(p.stroke).child(content)
    }

    /// A settings card: a label and description on the left, a control on the right.
    pub(super) fn row(
        &self,
        label: impl Into<SharedString>,
        description: impl Into<SharedString>,
        control: AnyElement,
    ) -> impl IntoElement {
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
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(label.into())
                    .child(div().text_size(px(12.)).text_color(p.text_weak).child(description.into())),
            )
            .child(control)
    }

    /// A radio button with a label.
    pub(super) fn radio(
        &self,
        id: impl Into<gpui::ElementId>,
        selected: bool,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
        choose: impl Fn(&mut NoteyApp, &mut Context<NoteyApp>) + 'static,
    ) -> impl IntoElement {
        let p = self.palette;
        let dot = div()
            .size(px(18.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .border_1()
            .border_color(if selected { p.accent } else { p.text_weak })
            .when(selected, |d| d.child(div().size(px(8.)).rounded_full().bg(p.accent)));
        div()
            .id(id)
            .py_1()
            .flex()
            .items_center()
            .gap_2()
            .child(dot)
            .child(label.into())
            .on_click(cx.listener(move |this, _, _, cx| choose(this, cx)))
    }

    /// One button of a segmented control.
    fn segment(
        &self,
        id: &'static str,
        on: bool,
        label: &'static str,
        cx: &mut Context<Self>,
        set: impl Fn(&mut Settings) + 'static,
    ) -> impl IntoElement {
        let p = self.palette;
        div()
            .id(id)
            .h(px(28.))
            .px_3()
            .flex()
            .items_center()
            .rounded(px(4.))
            .border_1()
            .when(on, |d| d.bg(p.accent).border_color(p.accent).text_color(p.editor))
            .when(!on, |d| d.bg(p.control_hover).border_color(p.stroke).hover(|s| s.bg(p.control_active)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.update_settings(&set, cx)))
    }

    /// A Fluent toggle switch.
    pub(super) fn switch(&self, id: impl Into<gpui::ElementId>, on: bool) -> gpui::Stateful<gpui::Div> {
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
    }

    /// A switch for a setting.
    fn toggle(&self, id: &'static str, on: bool, cx: &mut Context<Self>, flip: impl Fn(&mut Settings) + 'static) -> AnyElement {
        self.switch(id, on).on_click(cx.listener(move |this, _, _, cx| this.update_settings(&flip, cx))).into_any_element()
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
        let button = |n: usize, glyph: &'static str| {
            div()
                .id((id, n))
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
            .child(button(0, "−").on_click(cx.listener(move |this, _, _, cx| this.update_settings(&down, cx))))
            .child(div().w(px(72.)).flex().justify_center().child(value))
            .child(button(1, "+").on_click(cx.listener(move |this, _, _, cx| this.update_settings(&up, cx))))
            .into_any_element()
    }
}
