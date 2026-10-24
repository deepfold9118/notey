//! The Settings tab: a list of sections on the left and the settings of the
//! selected section on the right.

use super::*;

impl NoteyApp {
    pub(super) fn settings_view(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let p = self.palette();
        egui::Panel::left(egui::Id::new("settings_nav"))
            .resizable(false)
            .exact_size(180.0)
            .frame(
                egui::Frame::new()
                    .fill(p.editor)
                    .inner_margin(egui::Margin::symmetric(4, 8)),
            )
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Settings").size(18.0).strong());
                ui.add_space(8.0);
                for section in SettingsSection::ALL {
                    let selected = self.settings_section == section;
                    let button = egui::Button::selectable(selected, section.label())
                        .min_size(egui::vec2(ui.available_width(), 28.0));
                    if ui.add(button).clicked() {
                        self.settings_section = section;
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    if ui
                        .button("Reset to defaults")
                        .on_hover_text("Reset the editor and appearance settings. The theme does not change.")
                        .clicked()
                    {
                        self.reset_settings(ctx);
                    }
                });
            });
        let heading = |ui: &mut egui::Ui, section: SettingsSection| {
            ui.add_space(8.0);
            ui.label(egui::RichText::new(section.label()).size(18.0).strong());
            ui.add_space(10.0);
        };
        let section = self.settings_section;
        if section == SettingsSection::Plugins {
            // the plugin list has its own scroll area
            heading(ui, section);
            self.settings_plugins(ui);
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("settings_content")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                heading(ui, section);
                match section {
                    SettingsSection::Editor => self.settings_editor(ctx, ui),
                    SettingsSection::Appearance => self.settings_appearance(ctx, ui),
                    SettingsSection::Spellcheck => self.settings_spellcheck(ui),
                    SettingsSection::Plugins => {}
                }
            });
    }

    fn settings_editor(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let fonts: Vec<(String, PathBuf)> = self.ensure_font_list().to_vec();
        let mut font_changed: Option<PathBuf> = None;
        let mut tab_size_changed = false;
        egui::Grid::new("settings_editor_grid")
            .num_columns(2)
            .spacing([16.0, 12.0])
            .show(ui, |ui| {
                ui.label("Font:");
                egui::ComboBox::from_id_salt("font_picker")
                    .selected_text(self.editor_font.clone())
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.font_filter).hint_text("Filter…"));
                        ui.separator();
                        let filter = self.font_filter.to_lowercase();
                        for (name, path) in fonts
                            .iter()
                            .filter(|(n, _)| filter.is_empty() || n.to_lowercase().contains(&filter))
                        {
                            if ui.selectable_label(self.editor_font == *name, name).clicked() {
                                self.editor_font = name.clone();
                                font_changed = Some(path.clone());
                                ui.close();
                            }
                        }
                    });
                ui.end_row();

                ui.label("Font size:");
                ui.add(
                    egui::DragValue::new(&mut self.font_size)
                        .range(6.0..=72.0)
                        .speed(0.5)
                        .suffix(" pt"),
                );
                ui.end_row();

                ui.label("Line height:");
                ui.add(
                    egui::Slider::new(&mut self.line_height, 0.8..=3.0)
                        .step_by(0.05)
                        .suffix("×"),
                );
                ui.end_row();

                ui.label("Tab size:");
                let tab_resp = ui.add(
                    egui::DragValue::new(&mut self.tab_size)
                        .range(1.0..=16.0)
                        .speed(0.2)
                        .fixed_decimals(0)
                        .suffix(" spaces"),
                );
                if tab_resp.changed() {
                    tab_size_changed = true;
                }
                ui.end_row();

                ui.label("Word wrap:");
                ui.checkbox(&mut self.word_wrap, "Wrap long lines");
                ui.end_row();

                ui.label("Line numbers:");
                ui.checkbox(&mut self.show_line_numbers, "Show line numbers");
                ui.end_row();

                ui.label("Ligatures:");
                ui.checkbox(&mut self.ligatures, "Use font ligatures").on_hover_text(
                    "Lets fonts such as Cascadia Code or Fira Code join characters \
                     like -> and != into single symbols. Has no effect on fonts \
                     without ligatures, such as Consolas.",
                );
                ui.end_row();

                ui.label("Autosave:");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.autosave, "Save files every");
                    ui.add_enabled(
                        self.autosave,
                        egui::DragValue::new(&mut self.autosave_secs)
                            .range(5.0..=600.0)
                            .speed(1.0)
                            .fixed_decimals(0)
                            .suffix(" s"),
                    );
                });
                ui.end_row();

                ui.label("Editor:");
                ui.checkbox(&mut self.preview_editor, "Virtualized editor").on_hover_text(
                    "Lays out only visible lines, so large files stay fast, and \
                     adds multi-cursor and column editing. Turn off to use the \
                     classic editor, which shows inline IME composition.",
                );
                ui.end_row();
            });
        ui.add_space(10.0);
        ui.label(
            egui::RichText::new("Zoom: Ctrl+Plus, Ctrl+Minus, and Ctrl+0. The status bar shows the zoom level.")
                .weak(),
        );
        if let Some(path) = font_changed {
            rebuild_fonts(ctx, Some(&path), self.tab_size);
        } else if tab_size_changed {
            let name = self.editor_font.clone();
            let path = self.font_path(&name);
            rebuild_fonts(ctx, path.as_deref(), self.tab_size);
        }
    }

    fn settings_appearance(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Theme").strong());
        ui.add_space(4.0);
        if ui.radio(self.custom_theme.is_none() && self.dark, "Notey Dark").clicked() {
            self.use_builtin_theme(ctx, true);
        }
        if ui.radio(self.custom_theme.is_none() && !self.dark, "Notey Light").clicked() {
            self.use_builtin_theme(ctx, false);
        }
        let active_path = self.custom_theme.as_ref().map(|t| t.path.clone());
        let mut pick: Option<PathBuf> = None;
        for theme in &self.installed_themes {
            let active = active_path.as_deref() == Some(theme.path.as_path());
            if ui.radio(active, format!("{} — {}", theme.label, theme.package)).clicked() {
                pick = Some(theme.path.clone());
            }
        }
        let active_is_installed = active_path
            .as_ref()
            .is_some_and(|path| self.installed_themes.iter().any(|t| &t.path == path));
        if !active_is_installed {
            if let Some(name) = self.custom_theme.as_ref().map(|t| t.name.clone()) {
                let _ = ui.radio(true, name);
            }
        }
        if let Some(path) = pick {
            match self.activate_theme_path(ctx, &path) {
                Ok(name) => self.flash(format!("Theme: {name}")),
                Err(err) => self.flash(format!("Theme could not be loaded: {err}")),
            }
        }
        ui.add_space(6.0);
        if ui.button("Import VS Code Theme File…").clicked() {
            self.import_theme_dialog(ctx);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("Get a theme from");
            ui.hyperlink_to("vscodethemes.com", "https://vscodethemes.com/");
            ui.label(": copy the URL of a theme and paste it here.");
        });
        let downloading = self.theme_download_rx.is_some();
        let mut start_import = false;
        ui.horizontal(|ui| {
            let response = ui.add_enabled(
                !downloading,
                egui::TextEdit::singleline(&mut self.theme_url)
                    .hint_text("https://vscodethemes.com/e/publisher.extension/theme")
                    .desired_width(420.0),
            );
            let can_import = !downloading && !self.theme_url.trim().is_empty();
            if ui
                .add_enabled(
                    can_import,
                    egui::Button::new(if downloading { "Downloading…" } else { "Import" }),
                )
                .clicked()
            {
                start_import = true;
            }
            if downloading {
                ui.spinner();
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && can_import {
                start_import = true;
            }
        });
        if let Some(error) = &self.theme_url_error {
            ui.colored_label(Color32::from_rgb(0xF1, 0x70, 0x7B), error);
        }
        if start_import {
            self.start_theme_url_import();
        }

        ui.add_space(14.0);
        ui.label(egui::RichText::new("Layout").strong());
        ui.add_space(4.0);
        egui::Grid::new("settings_layout_grid")
            .num_columns(2)
            .spacing([16.0, 12.0])
            .show(ui, |ui| {
                ui.label("Tabs:");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.tabs_left, false, "Top (in the title bar)");
                    ui.radio_value(&mut self.tabs_left, true, "Left (vertical list)");
                });
                ui.end_row();

                ui.label("Menu bar:");
                ui.checkbox(&mut self.show_menu, "Show menu bar")
                    .on_hover_text("When off, the menus open from the Notey icon in the title bar");
                ui.end_row();

                ui.label("Status bar:");
                ui.checkbox(&mut self.show_status, "Show status bar");
                ui.end_row();
            });
        if !self.show_status && !self.tabs_left {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "The gear icon is in the status bar. With the status bar off, open Settings with Ctrl+, or Edit > Settings.",
                )
                .weak(),
            );
        }
    }

    fn settings_spellcheck(&mut self, ui: &mut egui::Ui) {
        if !self.spell.available() {
            ui.label("Spellcheck needs the Spellcheck feature and at least one language.");
            ui.add_space(6.0);
            if ui.button("Install the Spellcheck feature").clicked() {
                self.pm.open_at(crate::packages::Kind::Feature);
            }
            if ui.button("Install a language").clicked() {
                self.pm.open_at(crate::packages::Kind::Language);
            }
            return;
        }
        ui.checkbox(&mut self.spell_enabled, "Check spelling while you type");
        ui.add_space(10.0);
        ui.label(egui::RichText::new("Languages").strong());
        ui.add_space(4.0);
        for locale in self.spell.locales() {
            ui.label(format!("• {locale}"));
        }
        ui.add_space(6.0);
        if ui.button("Add or Remove Languages…").clicked() {
            self.pm.open_at(crate::packages::Kind::Language);
        }
        if let Some(path) = self.pm.user_dictionary_path().filter(|p| p.is_file()) {
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Personal word list").strong());
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("The words that you added with \"Add to dictionary\".").weak(),
            );
            if ui.button("Open Personal Word List").clicked() {
                self.open_path(&path);
            }
        }
    }

    fn settings_plugins(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Reload Scripts").clicked() {
                self.plugin_host = PluginHost::load(&self.pm.feature_scripts());
                let n = self.plugin_host.scripts.len();
                self.flash(format!("Loaded {n} script(s)"));
                self.surface_plugin_errors();
            }
            if ui.button("Open Scripts Folder").clicked() {
                if let Some(dir) = PluginHost::scripts_dir() {
                    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                }
            }
        });
        ui.add_space(8.0);
        self.pm.body(ui);
    }

    fn reset_settings(&mut self, ctx: &egui::Context) {
        self.font_size = 15.0;
        self.line_height = 1.0;
        self.tab_size = 4.0;
        self.show_line_numbers = false;
        self.ligatures = true;
        self.word_wrap = true;
        self.show_menu = true;
        self.show_status = true;
        self.tabs_left = false;
        self.autosave = false;
        self.autosave_secs = 30.0;
        self.editor_font = "Consolas".to_string();
        let path = self.font_path("Consolas");
        rebuild_fonts(ctx, path.as_deref(), self.tab_size);
    }
}
