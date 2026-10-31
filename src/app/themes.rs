//! Themes: Notey Dark and Light, VS Code and TextMate theme files, and themes
//! downloaded from a vscodethemes.com URL (Settings > Appearance).

use std::path::{Path, PathBuf};

use gpui::AnyElement;

use super::find::{field_editor, field_text, field_theme, set_field_text};
use super::*;
use crate::theme_import::{load_vscode_theme, ImportedTheme};
use crate::theme_marketplace::{self, InstalledTheme};

pub(super) struct ThemeUi {
    url: Entity<EditorView>,
    config: Compartment,
    error: Option<SharedString>,
    downloading: bool,
    /// The themes of the downloaded Marketplace packages.
    installed: Vec<InstalledTheme>,
    _observe: Subscription,
}

impl ThemeUi {
    pub(super) fn new(p: &Palette, cx: &mut Context<NoteyApp>) -> ThemeUi {
        let config = Compartment::new();
        let url = field_editor(&config, p, cx);
        let observe = cx.observe(&url, |_, _, cx| cx.notify());
        ThemeUi {
            url,
            config,
            error: None,
            downloading: false,
            installed: theme_marketplace::installed_themes(),
            _observe: observe,
        }
    }

    pub(super) fn set_palette(&self, p: &Palette, cx: &mut App) {
        let effect = self.config.reconfigure(THEME.of(Arc::new(field_theme(p))));
        self.url.update(cx, |view, cx| view.dispatch_spec(TransactionSpec::new().effect(effect), cx));
    }

    pub(super) fn url_focused(&self, cx: &App) -> bool {
        self.url.read(cx).has_focus()
    }
}

impl NoteyApp {
    /// The colors in use: the imported theme's, or Notey's own.
    pub(super) fn current_palette(&self) -> Palette {
        self.custom_theme.as_ref().map_or_else(|| palette(self.settings.dark), |t| t.palette)
    }

    /// Load the imported theme the settings name, at startup.
    pub(super) fn load_saved_theme(&mut self) {
        if let Some(path) = self.settings.theme_path.clone() {
            match load_vscode_theme(&path) {
                Ok(theme) => self.custom_theme = Some(theme),
                Err(_) => self.settings.theme_path = None,
            }
        }
    }

    pub(super) fn use_builtin_theme(&mut self, dark: bool, cx: &mut Context<Self>) {
        self.custom_theme = None;
        self.update_settings(
            |s| {
                s.dark = dark;
                s.theme_path = None;
            },
            cx,
        );
    }

    fn activate_theme_path(&mut self, path: &Path, cx: &mut Context<Self>) -> Result<String, String> {
        let theme: ImportedTheme = load_vscode_theme(path)?;
        let (name, dark) = (theme.name.clone(), theme.dark);
        self.custom_theme = Some(theme);
        let path = path.to_path_buf();
        self.update_settings(
            |s| {
                s.dark = dark;
                s.theme_path = Some(path);
            },
            cx,
        );
        Ok(name)
    }

    fn activate_and_report(&mut self, path: &Path, cx: &mut Context<Self>) {
        match self.activate_theme_path(path, cx) {
            Ok(name) => self.flash(format!("Theme: {name}"), cx),
            Err(err) => self.flash(format!("The theme could not be loaded: {err}"), cx),
        }
    }

    fn import_theme_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import Theme".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update(cx, |this, cx| {
                    if let Some(path) = paths.first() {
                        this.activate_and_report(path, cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Download the theme a vscodethemes.com URL names, then use it.
    pub(super) fn start_theme_url_import(&mut self, cx: &mut Context<Self>) {
        let url = field_text(&self.theme_ui.url, cx).trim().to_string();
        if url.is_empty() || self.theme_ui.downloading {
            return;
        }
        self.theme_ui.error = None;
        self.theme_ui.downloading = true;
        self.flash("Downloading the VS Code theme…".into(), cx);
        let download = cx.background_executor().spawn(async move { theme_marketplace::download_theme(&url) });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let outcome: Result<PathBuf, String> = download.await;
            let _ = this.update(cx, |this, cx| {
                this.theme_ui.downloading = false;
                let result = outcome.and_then(|path| {
                    this.theme_ui.installed = theme_marketplace::installed_themes();
                    this.activate_theme_path(&path, cx)
                });
                match result {
                    Ok(name) => {
                        set_field_text(&this.theme_ui.url, "", cx);
                        this.flash(format!("Theme: {name}"), cx);
                    }
                    Err(err) => {
                        this.theme_ui.error = Some(err.clone().into());
                        this.flash(format!("The theme import failed: {err}"), cx);
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// The downloaded themes, and an imported theme file in use.
    pub(super) fn imported_theme_choices(&mut self, mut list: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        let active = self.custom_theme.as_ref().map(|t| t.path.clone());
        for (i, theme) in self.theme_ui.installed.clone().into_iter().enumerate() {
            let selected = active.as_deref() == Some(theme.path.as_path());
            let label = format!("{} \u{2014} {}", theme.label, theme.package);
            list = list.child(self.radio(("installed-theme", i), selected, label, cx, move |this, cx| {
                this.activate_and_report(&theme.path, cx)
            }));
        }
        let from_file = active.as_ref().is_some_and(|path| !self.theme_ui.installed.iter().any(|t| &t.path == path));
        if from_file {
            if let Some(name) = self.custom_theme.as_ref().map(|t| t.name.clone()) {
                list = list.child(self.radio("file-theme", true, name, cx, |_, _| {}));
            }
        }
        list
    }

    /// Importing a theme from a file or from vscodethemes.com.
    pub(super) fn theme_import_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        let downloading = self.theme_ui.downloading;
        let import = self
            .text_button("theme-url-import", if downloading { "Downloading…" } else { "Import" })
            .on_click(cx.listener(|this, _, _, cx| this.start_theme_url_import(cx)));
        let body = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div().flex().child(
                    self.text_button("import-theme-file", "Import VS Code Theme File…")
                        .on_click(cx.listener(|this, _, window, cx| this.import_theme_dialog(window, cx))),
                ),
            )
            .child(
                div()
                    .text_color(p.text_weak)
                    .child("Or get a theme from vscodethemes.com: copy the URL of a theme and paste it here."),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(self.hinted_field("theme-url", &self.theme_ui.url, "https://vscodethemes.com/e/publisher.extension/theme", cx))
                    .child(import),
            )
            .children(self.theme_ui.error.clone().map(|e| div().text_color(gpui::rgb(0xF1707B)).child(e)));
        self.card(body.into_any_element()).into_any_element()
    }
}
