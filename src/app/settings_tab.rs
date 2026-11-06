//! The Settings tab: a list of sections on the left and the settings of the
//! selected section on the right.

use gpui::{anchored, deferred, div, prelude::*, px, uniform_list, AnyElement, Context, Corner, SharedString};

use std::collections::HashMap;

use super::find::{field_editor, field_text, field_theme, select_field, set_field_text};
use super::*;
use crate::packages::Kind;
use crate::plugin_settings::{self, Kind as SettingKind, Setting};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Section {
    Editor,
    Appearance,
    Spellcheck,
    Plugins,
    Updates,
    /// The settings of the plugin script at this index.
    Plugin(usize),
}

impl Section {
    pub(super) const ALL: [Section; 5] =
        [Section::Editor, Section::Appearance, Section::Spellcheck, Section::Plugins, Section::Updates];

    pub(super) fn label(self) -> &'static str {
        match self {
            Section::Editor => "Editor",
            Section::Appearance => "Appearance",
            Section::Spellcheck => "Spellcheck",
            Section::Plugins => "Plugins",
            Section::Updates => "Updates",
            Section::Plugin(_) => "Plugin",
        }
    }
}

/// The width of the section list.
const NAV_W: f32 = 200.;

/// The Settings tab's own state.
pub(super) struct SettingsUi {
    pub(super) section: Section,
    font_menu: bool,
    font_filter: Entity<EditorView>,
    /// Installed font families, loaded when the font menu first opens.
    fonts: Vec<SharedString>,
    config: Compartment,
    _observe: Subscription,
    /// Choices that plugin scripts listed, by (script, setting); listed
    /// again each time their section opens.
    plugin_choices: HashMap<(usize, String), Vec<(String, String)>>,
    /// Text fields of plugin settings, by (script, setting).
    plugin_fields: HashMap<(usize, String), Entity<EditorView>>,
    plugin_subs: Vec<Subscription>,
    /// The open drop-down of a plugin setting.
    open_choice: Option<(usize, String)>,
}

impl SettingsUi {
    pub(super) fn new(p: &Palette, cx: &mut Context<NoteyApp>) -> SettingsUi {
        let config = Compartment::new();
        let font_filter = field_editor(&config, p, cx);
        let observe = cx.observe(&font_filter, |_, _, cx| cx.notify());
        SettingsUi {
            section: Section::Editor,
            font_menu: false,
            font_filter,
            fonts: Vec::new(),
            config,
            _observe: observe,
            plugin_choices: HashMap::new(),
            plugin_fields: HashMap::new(),
            plugin_subs: Vec::new(),
            open_choice: None,
        }
    }

    pub(super) fn set_palette(&self, p: &Palette, cx: &mut App) {
        for view in std::iter::once(&self.font_filter).chain(self.plugin_fields.values()) {
            let effect = self.config.reconfigure(THEME.of(Arc::new(field_theme(p))));
            view.update(cx, |view, cx| view.dispatch_spec(TransactionSpec::new().effect(effect), cx));
        }
    }

    /// The scripts were reloaded: their indices may have changed.
    pub(super) fn forget_plugins(&mut self) {
        self.plugin_choices.clear();
        self.plugin_fields.clear();
        self.plugin_subs.clear();
        self.open_choice = None;
    }
}

const ROW_H: f32 = 28.0;

impl NoteyApp {
    pub(super) fn settings_page(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let nav = div()
            .w(px(NAV_W))
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
            // A section for each enabled plugin that has settings.
            .children(self.plugin_sections().into_iter().map(|(i, name)| {
                let section = Section::Plugin(i);
                let selected = self.settings_ui.section == section;
                div()
                    .id(("plugin-section", i))
                    .h(px(32.))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .when(selected, |d| d.bg(p.control_active))
                    .when(!selected, |d| d.hover(|s| s.bg(p.control_hover)))
                    .child(name)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_ui.section = section;
                        this.settings_ui.font_menu = false;
                        this.settings_ui.open_choice = None;
                        // look again for what is installed
                        this.settings_ui.plugin_choices.retain(|(s, _), _| *s != i);
                        cx.notify();
                    }))
            }))
            .child(div().flex_1())
            .child(
                self.text_button("reset-settings", "Reset to defaults")
                    .on_click(cx.listener(|this, _, _, cx| this.reset_settings(cx))),
            );
        // the room beside the navigation (and the tab list on the left)
        let left = if self.settings.tabs_left { self.settings.tabs_left_width + 1. } else { 0. };
        let content_w = (f32::from(window.viewport_size().width) - NAV_W - left).clamp(0., 760.);
        let section = self.settings_ui.section;
        let content = match section {
            Section::Editor => self.settings_editor(cx),
            Section::Appearance => self.settings_appearance(cx),
            Section::Spellcheck => self.settings_spellcheck(cx),
            Section::Plugins => self.settings_plugins(window, cx),
            Section::Updates => self.settings_updates(cx),
            Section::Plugin(i) => self.settings_plugin(i, cx),
        };
        let title: SharedString = match section {
            Section::Plugin(i) => self.scripting.host.scripts.get(i).map_or_else(String::new, |s| s.name.clone()).into(),
            s => s.label().into(),
        };
        div().flex_1().min_h_0().flex().bg(p.editor).child(nav).child(
            div().id("settings-content").flex_1().min_w_0().flex().flex_col().overflow_y_scroll().child(
                // A definite width (not max_w), so that the column's height
                // comes from text wrapped at the width it is drawn at, and
                // the scroll reaches the last row.
                div()
                    .w(px(content_w))
                    .px_8()
                    .py_6()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().mb_2().text_size(px(20.)).font_weight(gpui::FontWeight::SEMIBOLD).child(title))
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

    fn settings_updates(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.settings;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(self.row(
                "Check for updates automatically",
                "Once a day, look for a newer release of Notey on GitHub and offer to install it",
                self.toggle("check-updates", s.check_updates, cx, |s| s.check_updates = !s.check_updates),
            ))
            .child(self.row(
                format!("Notey {}", crate::update::current_version()),
                "The version you are using",
                self.text_button("check-now", "Check Now")
                    .on_click(cx.listener(|this, _, window, cx| this.check_for_update(true, window, cx)))
                    .into_any_element(),
            ))
            .into_any_element()
    }

    /// The enabled plugin scripts with settings: (script index, name).
    pub(super) fn plugin_sections(&self) -> Vec<(usize, SharedString)> {
        self.scripting
            .host
            .scripts
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.settings.is_empty())
            .map(|(i, s)| (i, s.name.clone().into()))
            .collect()
    }

    /// The choices of a plugin's drop-down setting, listing them from the
    /// script when they aren't known yet.
    pub(super) fn plugin_setting_choices(&mut self, i: usize, s: &Setting, cx: &mut Context<Self>) -> Vec<(String, String)> {
        let Some(f) = &s.choices_fn else { return s.choices.clone() };
        let key = (i, s.id.clone());
        if let Some(list) = self.settings_ui.plugin_choices.get(&key) {
            return list.clone();
        }
        let list = match self.script_snapshot(cx) {
            Some(ctx) => self.scripting.host.choices(i, f, ctx),
            None => Vec::new(),
        };
        self.surface_plugin_errors(cx);
        self.settings_ui.plugin_choices.insert(key, list.clone());
        list
    }

    /// A plugin's Settings section: its settings, as it declares them.
    fn settings_plugin(&mut self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(settings) = self.scripting.host.scripts.get(i).map(|s| s.settings.clone()) else {
            return div().into_any_element();
        };
        let values = self.scripting.host.setting_values(i);
        let mut body = div().flex().flex_col().gap_2();
        for s in settings.iter().filter(|s| plugin_settings::visible(s, &values)) {
            let value = values.get(&s.id).cloned().unwrap_or_default();
            let control = match s.kind {
                SettingKind::Toggle => {
                    let on = value == "true";
                    let id = s.id.clone();
                    self.switch(SharedString::from(format!("plugin-toggle-{i}-{}", s.id)), on)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.scripting.host.set_setting(i, &id, if on { "false" } else { "true" });
                            cx.notify();
                        }))
                        .into_any_element()
                }
                SettingKind::Choice => self.plugin_choice(i, s, &value, cx),
                SettingKind::Text | SettingKind::Path => self.plugin_text(i, s, &value, cx),
            };
            body = body.child(self.row(s.title.clone(), s.description.clone(), control));
        }
        body.into_any_element()
    }

    /// A drop-down for a plugin setting.
    fn plugin_choice(&mut self, i: usize, s: &Setting, value: &str, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let choices = self.plugin_setting_choices(i, s, cx);
        let label = choices.iter().find(|(v, _)| v == value).map_or_else(|| value.to_string(), |(_, l)| l.clone());
        let key = (i, s.id.clone());
        let open = self.settings_ui.open_choice.as_ref() == Some(&key);
        let button = div()
            .id(SharedString::from(format!("plugin-choice-{i}-{}", s.id)))
            .min_w(px(240.))
            .h(px(30.))
            .px_3()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .rounded(px(4.))
            .bg(p.control_hover)
            .border_1()
            .border_color(p.stroke)
            .hover(|s| s.bg(p.control_active))
            .child(label)
            .child(div().font_family(self.icon_font.clone()).text_size(px(10.)).child("\u{E70D}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings_ui.open_choice = if open { None } else { Some(key.clone()) };
                cx.notify();
            }));
        let mut picker = div().relative().child(button);
        if open {
            let items = choices.into_iter().enumerate().map(|(n, (v, l))| {
                let id = s.id.clone();
                let selected = v == value;
                div()
                    .id(("plugin-choice-item", n))
                    .h(px(ROW_H))
                    .px_3()
                    .flex()
                    .items_center()
                    .rounded(px(4.))
                    .when(selected, |d| d.bg(p.control_active))
                    .hover(|s| s.bg(p.control_hover))
                    .child(l)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_ui.open_choice = None;
                        this.scripting.host.set_setting(i, &id, &v);
                        cx.notify();
                    }))
            });
            let menu = div()
                .id("plugin-choice-menu")
                .occlude()
                .min_w(px(260.))
                .p_1p5()
                .flex()
                .flex_col()
                .gap_0p5()
                .bg(p.popup)
                .border_1()
                .border_color(p.stroke)
                .rounded(px(8.))
                .shadow_lg()
                .children(items)
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.settings_ui.open_choice = None;
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

    /// A text field for a plugin setting, with Browse for a path. Each
    /// change is saved.
    fn plugin_text(&mut self, i: usize, s: &Setting, value: &str, cx: &mut Context<Self>) -> AnyElement {
        let key = (i, s.id.clone());
        let field = match self.settings_ui.plugin_fields.get(&key) {
            Some(f) => f.clone(),
            None => {
                let f = field_editor(&self.settings_ui.config, &self.palette, cx);
                set_field_text(&f, value, cx);
                // the cursor at the end, not the text selected
                f.update(cx, |view, cx| {
                    let end = view.state().doc().len();
                    view.dispatch_spec(TransactionSpec::new().selection(codemirror_state::EditorSelection::single(end, end)), cx);
                });
                let id = s.id.clone();
                let sub = cx.observe(&f, move |this, f, cx| {
                    let text = field_text(&f, cx).replace(['\r', '\n'], "");
                    if this.scripting.host.setting_values(i).get(&id) != Some(&text) {
                        this.scripting.host.set_setting(i, &id, &text);
                    }
                });
                self.settings_ui.plugin_subs.push(sub);
                self.settings_ui.plugin_fields.insert(key, f.clone());
                f
            }
        };
        let mut row = div()
            .w(px(340.))
            .flex()
            .items_center()
            .gap_2()
            .child(self.field(SharedString::from(format!("plugin-field-{i}-{}", s.id)), &field, cx));
        if s.kind == SettingKind::Path {
            row = row.child(
                self.text_button(SharedString::from(format!("plugin-browse-{i}-{}", s.id)), "Browse\u{2026}")
                    .on_click(cx.listener(move |_, _, window, cx| {
                        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
                            files: true,
                            directories: false,
                            multiple: false,
                            prompt: None,
                        });
                        let field = field.clone();
                        cx.spawn_in(window, async move |this, cx| {
                            if let Ok(Ok(Some(paths))) = picked.await {
                                if let Some(path) = paths.first() {
                                    let _ = this.update(cx, |_, cx| set_field_text(&field, &path.display().to_string(), cx));
                                }
                            }
                        })
                        .detach();
                    })),
            );
        }
        row.into_any_element()
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
