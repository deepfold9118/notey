//! Plugin manager: owns the plugin store, loads what enabled plugins
//! provide (spellcheck dictionaries, file type grammars, feature scripts) on
//! background threads, and draws the Plugins window.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui::{self, Color32, RichText};
use spellbook::Dictionary;

use crate::packages::{self, Kind, Manifest, Store};
use crate::spell::LanguageFiles;
use crate::syntax::Syntaxes;

/// Built-in engine switched on by the Spellcheck feature plugin.
pub const NATIVE_SPELLCHECK: &str = "spellcheck";

type SpellResult = (Vec<(String, Dictionary)>, Vec<String>);
type SyntaxResult = (Syntaxes, Vec<String>);

enum IndexState {
    Idle,
    Loading,
    Error(String),
}

/// What changed, so the app can reload the affected pieces.
#[derive(Default)]
pub struct Poll {
    /// New dictionaries finished loading (empty = spellcheck unavailable).
    pub spell: Option<SpellResult>,
    /// A new grammar set was installed.
    pub syntaxes_installed: bool,
    /// Feature scripts need reloading.
    pub features_changed: bool,
    pub messages: Vec<String>,
}

pub struct PluginManager {
    pub store: Store,
    pub open: bool,
    tab: Kind,
    filter: String,
    index: Option<Vec<Manifest>>,
    index_state: IndexState,
    index_rx: Option<Receiver<Result<Vec<Manifest>, String>>>,
    busy: HashSet<String>,
    jobs_tx: Sender<(String, Result<(), String>)>,
    jobs_rx: Receiver<(String, Result<(), String>)>,
    spell_rx: Option<Receiver<SpellResult>>,
    syntax_rx: Option<Receiver<SyntaxResult>>,
    pending: Poll,
}

impl PluginManager {
    pub fn new() -> Self {
        let mut store = Store::open();
        // the installer ships default plugins beside the exe
        let bundled = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join("plugins")));
        if let Some(bundled) = &bundled {
            store.seed_defaults(bundled);
            let cache = store.root().join("index-cache.json");
            if !cache.exists() {
                let _ = std::fs::copy(bundled.join("index.json"), &cache);
            }
        }
        let index = std::fs::read(store.root().join("index-cache.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| serde_json::from_value::<Vec<Manifest>>(v["plugins"].clone()).ok());
        let (jobs_tx, jobs_rx) = channel();
        Self {
            store,
            open: false,
            tab: Kind::Feature,
            filter: String::new(),
            index,
            index_state: IndexState::Idle,
            index_rx: None,
            busy: HashSet::new(),
            jobs_tx,
            jobs_rx,
            spell_rx: None,
            syntax_rx: None,
            pending: Poll::default(),
        }
    }

    pub fn open_at(&mut self, tab: Kind) {
        self.open = true;
        self.tab = tab;
        if self.index.is_none() || matches!(self.index_state, IndexState::Error(_)) {
            self.refresh_index();
        }
    }

    pub fn user_dictionary_path(&self) -> Option<PathBuf> {
        self.store.root().parent().map(|p| p.join("user_dictionary.txt"))
    }

    // ---------- loading what plugins provide ----------

    pub fn spellcheck_enabled(&self) -> bool {
        self.store.native_enabled(NATIVE_SPELLCHECK)
    }

    /// Load the enabled languages' dictionaries in the background. With the
    /// Spellcheck feature off, reports an empty set immediately.
    pub fn start_spell_load(&mut self) {
        let langs: Vec<LanguageFiles> = if self.spellcheck_enabled() {
            self.store
                .enabled(Kind::Language)
                .filter_map(|p| {
                    let find = |ext: &str| {
                        p.manifest
                            .files
                            .iter()
                            .find(|f| f.path.ends_with(ext))
                            .map(|f| p.file(&f.path))
                    };
                    Some(LanguageFiles {
                        locale: p.manifest.locale.clone()?,
                        aff: find(".aff")?,
                        dic: find(".dic")?,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(crate::spell::load_languages(&langs));
        });
        self.spell_rx = Some(rx);
    }

    /// Build the grammar set from enabled file types in the background,
    /// using an on-disk cache keyed to exactly those plugins.
    pub fn start_syntax_build(&mut self) {
        let mut grammars = Vec::new();
        let mut hasher = DefaultHasher::new();
        let mut enabled: Vec<_> = self.store.enabled(Kind::Filetype).collect();
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
        let cache_dir = self.store.root().join(".cache");
        let _ = std::fs::create_dir_all(&cache_dir);
        let cache = cache_dir.join(format!("syntaxes-{:016x}.bin", hasher.finish()));
        // drop caches for other plugin combinations
        if let Ok(entries) = std::fs::read_dir(&cache_dir) {
            for e in entries.filter_map(Result::ok) {
                if e.path() != cache && e.file_name().to_string_lossy().starts_with("syntaxes-") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let cache = if grammars.is_empty() { None } else { Some(cache) };
            let _ = tx.send(crate::syntax::build(&grammars, cache.as_deref()));
        });
        self.syntax_rx = Some(rx);
    }

    /// Entry scripts of enabled feature plugins, with the permissions each
    /// package grants (shown to the user in the Plugins window).
    pub fn feature_scripts(&self) -> Vec<(PathBuf, Vec<String>)> {
        self.store
            .enabled(Kind::Feature)
            .filter_map(|p| {
                let script = p.file(p.manifest.script.as_ref()?);
                Some((script, p.manifest.capabilities.clone()))
            })
            .collect()
    }

    /// Reload whatever a change to a plugin of `kind` affects.
    fn reload(&mut self, kind: Kind) {
        match kind {
            Kind::Language => self.start_spell_load(),
            Kind::Filetype => self.start_syntax_build(),
            Kind::Feature => {
                self.pending.features_changed = true;
                self.start_spell_load(); // the Spellcheck feature may have toggled
            }
        }
    }

    /// Collect finished background work. Call once per frame.
    pub fn poll(&mut self) -> Poll {
        if let Some(rx) = &self.spell_rx {
            if let Ok((dicts, errors)) = rx.try_recv() {
                for e in &errors {
                    self.pending.messages.push(format!("Spellcheck language failed to load: {e}"));
                }
                self.pending.spell = Some((dicts, errors));
                self.spell_rx = None;
            }
        }
        if let Some(rx) = &self.syntax_rx {
            if let Ok((set, errors)) = rx.try_recv() {
                for e in &errors {
                    self.pending.messages.push(format!("File type failed to load: {e}"));
                }
                crate::syntax::install(set);
                self.pending.syntaxes_installed = true;
                self.syntax_rx = None;
            }
        }
        if let Some(rx) = &self.index_rx {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok(list) => {
                        let json = serde_json::json!({ "schema": 1, "plugins": list });
                        let _ = std::fs::write(
                            self.store.root().join("index-cache.json"),
                            serde_json::to_vec(&json).unwrap_or_default(),
                        );
                        self.index = Some(list);
                        self.index_state = IndexState::Idle;
                    }
                    Err(e) => self.index_state = IndexState::Error(e),
                }
                self.index_rx = None;
            }
        }
        while let Ok((id, result)) = self.jobs_rx.try_recv() {
            self.busy.remove(&id);
            self.store.rescan();
            match result {
                Ok(()) => {
                    if let Some(p) = self.store.get(&id) {
                        let kind = p.manifest.kind;
                        let name = p.manifest.name.clone();
                        self.pending.messages.push(format!("Installed {name}"));
                        self.reload(kind);
                    }
                }
                Err(e) => self.pending.messages.push(format!("Install failed: {e}")),
            }
        }
        std::mem::take(&mut self.pending)
    }

    pub fn is_busy(&self) -> bool {
        !self.busy.is_empty() || self.index_rx.is_some() || self.spell_rx.is_some() || self.syntax_rx.is_some()
    }

    // ---------- registry actions ----------

    fn refresh_index(&mut self) {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(packages::fetch_index());
        });
        self.index_rx = Some(rx);
        self.index_state = IndexState::Loading;
    }

    pub fn install(&mut self, id: &str) {
        let Some(m) = self.index.as_ref().and_then(|i| i.iter().find(|m| m.id == id)).cloned() else {
            return;
        };
        if !self.busy.insert(m.id.clone()) {
            return;
        }
        let root = self.store.root().to_path_buf();
        let tx = self.jobs_tx.clone();
        std::thread::spawn(move || {
            let result = packages::install_from_registry(&root, &m);
            let _ = tx.send((m.id.clone(), result));
        });
    }

    fn uninstall(&mut self, id: &str) {
        let kind = self.store.get(id).map(|p| p.manifest.kind);
        match self.store.uninstall(id) {
            Ok(()) => {
                if let Some(kind) = kind {
                    self.reload(kind);
                }
            }
            Err(e) => self.pending.messages.push(e),
        }
    }

    fn set_enabled(&mut self, id: &str, on: bool) {
        self.store.set_enabled(id, on);
        if let Some(kind) = self.store.get(id).map(|p| p.manifest.kind) {
            self.reload(kind);
        }
    }

    /// A not-yet-installed file type plugin that handles this file.
    pub fn suggestion_for(&self, path: &Path) -> Option<&Manifest> {
        let ext = path.extension()?.to_str()?.to_lowercase();
        self.index.as_ref()?.iter().find(|m| {
            m.kind == Kind::Filetype
                && self.store.get(&m.id).is_none()
                && m.extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext))
        })
    }

    // ---------- window ----------

    pub fn window(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        egui::Window::new("Plugins")
            .id(egui::Id::new("plugins_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([640.0, 480.0])
            .show(ctx, |ui| self.window_body(ui));
        self.open = open;
    }

    fn window_body(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for kind in [Kind::Feature, Kind::Language, Kind::Filetype] {
                let count = self.store.installed.iter().filter(|p| p.manifest.kind == kind).count();
                let label = format!("{} ({count})", kind.label());
                if ui.selectable_label(self.tab == kind, label).clicked() {
                    self.tab = kind;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let loading = matches!(self.index_state, IndexState::Loading);
                if ui
                    .add_enabled(!loading, egui::Button::new("Refresh"))
                    .on_hover_text("Fetch the latest plugin list from the Notey repository")
                    .clicked()
                {
                    self.refresh_index();
                }
                if loading {
                    ui.spinner();
                }
            });
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.filter)
                .hint_text("Search plugins…")
                .desired_width(f32::INFINITY),
        );
        match &self.index_state {
            IndexState::Error(e) => {
                ui.colored_label(
                    Color32::from_rgb(232, 82, 82),
                    format!("Could not reach the plugin repository ({e}). Showing installed plugins."),
                );
            }
            _ if self.index.is_none() => {
                ui.label(RichText::new("Loading the plugin list…").weak());
            }
            _ => {}
        }
        if self.tab == Kind::Language && !self.spellcheck_enabled() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Languages are used by the Spellcheck feature, which is not enabled.")
                        .weak(),
                );
                if ui.small_button("Show Features").clicked() {
                    self.tab = Kind::Feature;
                }
            });
        }
        ui.separator();

        // merge the registry list with installed plugins (incl. ones the
        // registry no longer lists)
        let mut rows: HashMap<String, (Option<Manifest>, bool)> = HashMap::new();
        for m in self.index.iter().flatten().filter(|m| m.kind == self.tab) {
            rows.insert(m.id.clone(), (Some(m.clone()), false));
        }
        for p in self.store.installed.iter().filter(|p| p.manifest.kind == self.tab) {
            rows.entry(p.manifest.id.clone())
                .and_modify(|r| r.1 = true)
                .or_insert((Some(p.manifest.clone()), true));
        }
        let filter = self.filter.to_lowercase();
        let mut rows: Vec<(Manifest, bool)> = rows
            .into_values()
            .filter_map(|(m, inst)| m.map(|m| (m, inst)))
            .filter(|(m, _)| {
                filter.is_empty()
                    || m.name.to_lowercase().contains(&filter)
                    || m.description.to_lowercase().contains(&filter)
                    || m.extensions.iter().any(|e| e.to_lowercase() == filter)
            })
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.name.to_lowercase().cmp(&b.0.name.to_lowercase())));

        let mut action: Option<(String, &'static str)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if rows.is_empty() {
                ui.label(RichText::new("No plugins match.").weak());
            }
            // the controls column (Update, Enabled, Uninstall) gets a fixed
            // width so descriptions wrap before it instead of running under it
            const CONTROLS_W: f32 = 250.0;
            let text_w = (ui.available_width() - CONTROLS_W).max(160.0);
            for (m, installed) in &rows {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(text_w);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&m.name).strong());
                            ui.label(RichText::new(format!("v{}", m.version)).weak().size(11.0));
                            if *installed {
                                ui.label(RichText::new("Installed").size(11.0).color(ui.visuals().hyperlink_color));
                            }
                        });
                        ui.label(RichText::new(&m.description).weak());
                        if !m.extensions.is_empty() {
                            let mut exts: Vec<String> = m.extensions.iter().take(8).map(|e| format!(".{e}")).collect();
                            if m.extensions.len() > 8 {
                                exts.push(format!("+{} more", m.extensions.len() - 8));
                            }
                            ui.label(RichText::new(exts.join(" ")).weak().size(11.0));
                        }
                        let missing: Vec<String> = m
                            .recommends
                            .iter()
                            .filter(|id| self.store.get(id).is_none())
                            .map(|id| self.name_of(id))
                            .collect();
                        if !missing.is_empty() && self.tab == Kind::Filetype {
                            let shown: Vec<&str> = missing.iter().take(4).map(String::as_str).collect();
                            let more = missing.len().saturating_sub(4);
                            let tail = if more > 0 { format!(" and {more} more") } else { String::new() };
                            ui.label(
                                RichText::new(format!("Also highlights embedded code with: {}{tail}", shown.join(", ")))
                                    .weak()
                                    .size(11.0),
                            );
                        }
                        if !m.capabilities.is_empty() {
                            ui.label(
                                RichText::new(format!("Permissions: {}", m.capabilities.join(", ")))
                                    .size(11.0)
                                    .color(Color32::from_rgb(214, 160, 60)),
                            );
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.busy.contains(&m.id) {
                            ui.spinner();
                        } else if *installed {
                            if ui.button("Uninstall").clicked() {
                                action = Some((m.id.clone(), "uninstall"));
                            }
                            let mut on = self.store.is_enabled(&m.id);
                            if ui.checkbox(&mut on, "Enabled").changed() {
                                action = Some((m.id.clone(), if on { "enable" } else { "disable" }));
                            }
                            let newer = self
                                .index
                                .iter()
                                .flatten()
                                .find(|x| x.id == m.id)
                                .zip(self.store.get(&m.id))
                                .is_some_and(|(remote, local)| remote.version != local.manifest.version);
                            if newer && ui.button("Update").clicked() {
                                action = Some((m.id.clone(), "install"));
                            }
                        } else if ui.button("Install").clicked() {
                            action = Some((m.id.clone(), "install"));
                        }
                    });
                });
                ui.separator();
            }
        });
        if let Some((id, what)) = action {
            match what {
                "install" => self.install(&id),
                "uninstall" => self.uninstall(&id),
                "enable" => self.set_enabled(&id, true),
                _ => self.set_enabled(&id, false),
            }
        }
    }

    fn name_of(&self, id: &str) -> String {
        self.index
            .iter()
            .flatten()
            .find(|m| m.id == id)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| id.to_string())
    }
}
