//! The plugin manager (Settings > Plugins): install, update, turn on or off
//! and remove plugins from the official registry, in three lists (Features,
//! Languages, File Types). Downloads run in the background; a change
//! reloads what the plugin provides.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use gpui::AnyElement;

use super::find::{field_editor, field_text, field_theme};
use super::settings_tab::Section;
use super::*;
use crate::packages::{self, Kind, Manifest, Store};

enum IndexState {
    Idle,
    Loading,
    Error(SharedString),
}

pub(super) struct PluginsUi {
    pub(super) store: Store,
    tab: Kind,
    filter: Entity<EditorView>,
    config: Compartment,
    /// The registry's plugin list (from the cache until refreshed).
    index: Option<Vec<Manifest>>,
    index_state: IndexState,
    /// Plugins being installed or updated.
    busy: HashSet<String>,
    _observe: Subscription,
}

impl PluginsUi {
    pub(super) fn new(p: &Palette, cx: &mut Context<NoteyApp>) -> PluginsUi {
        let mut store = Store::open();
        // The installer ships default plugins beside the exe.
        let bundled = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.join("plugins")));
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
        let config = Compartment::new();
        let filter = field_editor(&config, p, cx);
        let observe = cx.observe(&filter, |_, _, cx| cx.notify());
        PluginsUi {
            store,
            tab: Kind::Feature,
            filter,
            config,
            index,
            index_state: IndexState::Idle,
            busy: HashSet::new(),
            _observe: observe,
        }
    }

    pub(super) fn set_palette(&self, p: &Palette, cx: &mut App) {
        let effect = self.config.reconfigure(THEME.of(Arc::new(field_theme(p))));
        self.filter.update(cx, |view, cx| view.dispatch_spec(TransactionSpec::new().effect(effect), cx));
    }

    /// A File Type plugin, not installed, that would highlight this file.
    pub(super) fn suggestion_for(&self, path: &Path) -> Option<&Manifest> {
        let ext = path.extension()?.to_str()?.to_lowercase();
        self.index.as_ref()?.iter().find(|m| {
            m.kind == Kind::Filetype
                && self.store.get(&m.id).is_none()
                && m.extensions.iter().any(|e| e.eq_ignore_ascii_case(&ext))
        })
    }

    fn name_of(&self, id: &str) -> String {
        self.index.iter().flatten().find(|m| m.id == id).map(|m| m.name.clone()).unwrap_or_else(|| id.to_string())
    }
}

impl NoteyApp {
    /// Show Settings > Plugins at a list.
    pub(super) fn open_plugins(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        self.plugins.tab = kind;
        if self.plugins.index.is_none() || matches!(self.plugins.index_state, IndexState::Error(_)) {
            self.refresh_index(cx);
        }
        self.open_settings_at(Section::Plugins, window, cx);
    }

    /// Fetch the plugin list from the registry.
    fn refresh_index(&mut self, cx: &mut Context<Self>) {
        self.plugins.index_state = IndexState::Loading;
        let fetch = cx.background_executor().spawn(async { packages::fetch_index() });
        self._tasks.push(cx.spawn(async move |this, cx| {
            let result = fetch.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(list) => {
                        let json = serde_json::json!({ "schema": 1, "plugins": list });
                        let _ = std::fs::write(
                            this.plugins.store.root().join("index-cache.json"),
                            serde_json::to_vec(&json).unwrap_or_default(),
                        );
                        this.plugins.index = Some(list);
                        this.plugins.index_state = IndexState::Idle;
                    }
                    Err(e) => this.plugins.index_state = IndexState::Error(e.into()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Install (or update) a plugin from the registry.
    pub(super) fn install_plugin(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(m) = self.plugins.index.as_ref().and_then(|i| i.iter().find(|m| m.id == id)).cloned() else { return };
        if !self.plugins.busy.insert(m.id.clone()) {
            return;
        }
        let root = self.plugins.store.root().to_path_buf();
        let job = {
            let m = m.clone();
            cx.background_executor().spawn(async move { packages::install_from_registry(&root, &m) })
        };
        self._tasks.push(cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.plugins.busy.remove(&m.id);
                this.plugins.store.rescan();
                match result {
                    Ok(()) => {
                        this.flash(format!("Installed {}", m.name), cx);
                        this.reload_plugin_kind(m.kind, cx);
                    }
                    Err(e) => this.flash(format!("Install failed: {e}"), cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn uninstall_plugin(&mut self, id: &str, cx: &mut Context<Self>) {
        let kind = self.plugins.store.get(id).map(|p| p.manifest.kind);
        match self.plugins.store.uninstall(id) {
            Ok(()) => {
                if let Some(kind) = kind {
                    self.reload_plugin_kind(kind, cx);
                }
            }
            Err(e) => self.flash(e, cx),
        }
        cx.notify();
    }

    pub(super) fn set_plugin_enabled(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        self.plugins.store.set_enabled(id, on);
        if let Some(kind) = self.plugins.store.get(id).map(|p| p.manifest.kind) {
            self.reload_plugin_kind(kind, cx);
        }
        cx.notify();
    }

    /// Reload what a change to a plugin of `kind` affects.
    fn reload_plugin_kind(&mut self, kind: Kind, cx: &mut Context<Self>) {
        match kind {
            Kind::Language => self.start_spell_load(cx),
            Kind::Filetype => self.start_syntax_build(cx),
            Kind::Feature => {
                // The Spellcheck feature may have changed.
                self.start_spell_load(cx);
                self.reload_scripts(false, cx);
            }
        }
    }

    pub(super) fn settings_plugins(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette;
        if self.plugins.index.is_none() && matches!(self.plugins.index_state, IndexState::Idle) {
            self.refresh_index(cx);
        }
        let tabs = div().flex().gap_1().children([Kind::Feature, Kind::Language, Kind::Filetype].map(|kind| {
            let count = self.plugins.store.installed.iter().filter(|p| p.manifest.kind == kind).count();
            let on = self.plugins.tab == kind;
            div()
                .id(kind.label())
                .h(px(28.))
                .px_3()
                .flex()
                .items_center()
                .rounded(px(4.))
                .border_1()
                .when(on, |d| d.bg(p.accent).border_color(p.accent).text_color(p.editor))
                .when(!on, |d| d.bg(p.control_hover).border_color(p.stroke).hover(|s| s.bg(p.control_active)))
                .child(format!("{} ({count})", kind.label()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.plugins.tab = kind;
                    cx.notify();
                }))
        }));
        let loading = matches!(self.plugins.index_state, IndexState::Loading);
        let toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .child(tabs)
            .child(div().flex_1())
            .child(
                self.text_button("reload-scripts", "Reload Scripts")
                    .on_click(cx.listener(|this, _, _, cx| this.reload_scripts(true, cx))),
            )
            .child(
                self.text_button("scripts-folder", "Open Scripts Folder")
                    .on_click(cx.listener(|_, _, _, _| {
                        if let Some(dir) = crate::plugins::PluginHost::scripts_dir() {
                            let _ = std::fs::create_dir_all(&dir);
                            let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                        }
                    })),
            )
            .child(
                self.text_button("refresh-plugins", if loading { "Refreshing…" } else { "Refresh" })
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_index(cx))),
            );
        let mut notes = div().flex().flex_col().gap_1().text_color(p.text_weak);
        match &self.plugins.index_state {
            IndexState::Error(e) => {
                notes = notes.child(
                    div()
                        .text_color(gpui::rgb(0xE85252))
                        .child(format!("Could not reach the plugin repository ({e}). Showing installed plugins.")),
                )
            }
            _ if self.plugins.index.is_none() => notes = notes.child("Loading the plugin list…"),
            _ => {}
        }
        let tab = self.plugins.tab;
        if tab == Kind::Language && !self.plugins.store.native_enabled("spellcheck") {
            notes = notes.child("Languages are used by the Spellcheck feature, which is not enabled.");
        }

        // The registry's list merged with the installed plugins (including
        // ones the registry no longer lists).
        let mut rows: HashMap<String, (Manifest, bool)> = HashMap::new();
        for m in self.plugins.index.iter().flatten().filter(|m| m.kind == tab) {
            rows.insert(m.id.clone(), (m.clone(), false));
        }
        for p in self.plugins.store.installed.iter().filter(|p| p.manifest.kind == tab) {
            rows.entry(p.manifest.id.clone()).and_modify(|r| r.1 = true).or_insert((p.manifest.clone(), true));
        }
        let filter = field_text(&self.plugins.filter, cx).to_lowercase();
        let mut rows: Vec<(Manifest, bool)> = rows
            .into_values()
            .filter(|(m, _)| {
                filter.is_empty()
                    || m.name.to_lowercase().contains(&filter)
                    || m.description.to_lowercase().contains(&filter)
                    || m.extensions.iter().any(|e| e.to_lowercase() == filter)
            })
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.name.to_lowercase().cmp(&b.0.name.to_lowercase())));

        let mut list = div().flex().flex_col().gap_2();
        if rows.is_empty() {
            list = list.child(div().text_color(p.text_weak).child("No plugins match."));
        }
        for (i, (m, installed)) in rows.into_iter().enumerate() {
            list = list.child(self.plugin_row(i, m, installed, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(toolbar)
            .child(div().flex().child(self.hinted_field("plugin-filter", &self.plugins.filter, "Search plugins…", cx)))
            .child(notes)
            .child(list)
            .into_any_element()
    }

    fn plugin_row(&self, i: usize, m: Manifest, installed: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette;
        let small = |text: String| div().text_size(px(11.)).text_color(p.text_weak).child(text);
        let mut info = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(m.name.clone()))
                    .child(small(format!("v{}", m.version)))
                    .when(installed, |d| d.child(div().text_size(px(11.)).text_color(p.accent).child("Installed"))),
            )
            .child(div().text_color(p.text_weak).child(m.description.clone()));
        if !m.extensions.is_empty() {
            let mut exts: Vec<String> = m.extensions.iter().take(8).map(|e| format!(".{e}")).collect();
            if m.extensions.len() > 8 {
                exts.push(format!("+{} more", m.extensions.len() - 8));
            }
            info = info.child(small(exts.join(" ")));
        }
        let missing: Vec<String> =
            m.recommends.iter().filter(|id| self.plugins.store.get(id).is_none()).map(|id| self.plugins.name_of(id)).collect();
        if !missing.is_empty() && m.kind == Kind::Filetype {
            let shown: Vec<&str> = missing.iter().take(4).map(String::as_str).collect();
            let more = missing.len().saturating_sub(4);
            let tail = if more > 0 { format!(" and {more} more") } else { String::new() };
            info = info.child(small(format!("Also highlights embedded code with: {}{tail}", shown.join(", "))));
        }
        if !m.capabilities.is_empty() {
            info = info.child(
                div()
                    .text_size(px(11.))
                    .text_color(gpui::rgb(0xD6A03C))
                    .child(format!("Permissions: {}", m.capabilities.join(", "))),
            );
        }
        let mut controls = div().flex().flex_none().items_center().gap_2();
        let id = m.id.clone();
        if self.plugins.busy.contains(&m.id) {
            controls = controls.child(div().text_color(p.text_weak).child("Installing…"));
        } else if installed {
            let newer = self
                .plugins
                .index
                .iter()
                .flatten()
                .find(|x| x.id == m.id)
                .zip(self.plugins.store.get(&m.id))
                .is_some_and(|(remote, local)| remote.version != local.manifest.version);
            if newer {
                let id = id.clone();
                controls = controls.child(
                    self.text_button(("update", i), "Update")
                        .on_click(cx.listener(move |this, _, _, cx| this.install_plugin(&id, cx))),
                );
            }
            let on = self.plugins.store.is_enabled(&m.id);
            let toggle_id = id.clone();
            controls = controls
                .child(
                    self.switch(("enabled", i), on)
                        .on_click(cx.listener(move |this, _, _, cx| this.set_plugin_enabled(&toggle_id, !on, cx))),
                )
                .child(
                    self.text_button(("uninstall", i), "Uninstall")
                        .on_click(cx.listener(move |this, _, _, cx| this.uninstall_plugin(&id, cx))),
                );
        } else {
            controls = controls.child(
                self.text_button(("install", i), "Install")
                    .on_click(cx.listener(move |this, _, _, cx| this.install_plugin(&id, cx))),
            );
        }
        div()
            .px_4()
            .py_3()
            .flex()
            .items_center()
            .gap_4()
            .rounded(px(6.))
            .bg(p.control)
            .border_1()
            .border_color(p.stroke)
            .child(info)
            .child(controls)
    }
}
