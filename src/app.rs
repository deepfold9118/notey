use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, FontFamily, FontId, Key, KeyboardShortcut, Modifiers, TextFormat,
};
use eframe::egui::text::{CCursor, CCursorRange, LayoutJob};

use std::collections::HashMap;

use crate::doc::{Document, Encoding, LineEnding};
use crate::editor::{self, EditorState};
use crate::ipc::{clean_path, Inbox};
use crate::plugins::{self, HostAction, PluginHost, ScriptCtx};
use crate::search;
use crate::session::{self, Session, SessionTab};
use crate::spell::{self, Spell};
use crate::theme;

// ---------- shortcuts ----------
const SC_NEW_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::N);
const SC_NEW_WINDOW: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::N);
const SC_OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::O);
const SC_SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::S);
const SC_SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::S);
const SC_CLOSE_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::W);
const SC_PRINT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::P);
const SC_FIND: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::F);
const SC_FIND_FILES: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::F);
const SC_REPLACE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::H);
const SC_GOTO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::G);
const SC_FIND_NEXT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F3);
const SC_FIND_PREV: KeyboardShortcut = KeyboardShortcut::new(Modifiers::SHIFT, Key::F3);
const SC_TIME_DATE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F5);
const SC_ZOOM_IN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Plus);
const SC_ZOOM_IN2: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Equals);
const SC_ZOOM_OUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Minus);
const SC_ZOOM_RESET: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Num0);
const SC_NEXT_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Tab);
const SC_PREV_TAB: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::Tab);
const SC_PREFS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Comma);

const EDITOR_FONT_KEY: &str = "editor_font";

fn editor_family() -> FontFamily {
    FontFamily::Name("editor".into())
}

enum PendingClose {
    Tab(usize),
    App,
}

pub struct NoteyApp {
    docs: Vec<Document>,
    active: usize,
    next_doc_id: u64,
    next_untitled: usize,

    spell: Spell,
    spell_enabled: bool,

    dark: bool,
    custom_theme: Option<theme::ImportedTheme>,
    installed_themes: Vec<crate::theme_marketplace::InstalledTheme>,
    theme_url_open: bool,
    theme_url: String,
    theme_url_error: Option<String>,
    theme_download_rx: Option<std::sync::mpsc::Receiver<Result<PathBuf, String>>>,
    word_wrap: bool,
    show_status: bool,
    /// Always show the classic menu bar row. Off by default — the menus
    /// live under the title-bar icon instead.
    show_menu: bool,
    /// Use the virtualized preview editor instead of egui's TextEdit.
    preview_editor: bool,
    editor_states: HashMap<u64, EditorState>,
    /// Tabs currently showing the rendered Markdown view instead of the
    /// editor (session-transient, keyed by document id).
    md_rendered: std::collections::HashSet<u64>,
    /// Tabs showing a rendered preview beside the editor.
    md_split: std::collections::HashSet<u64>,
    /// Plugin status bar items: (key, text, tooltip), in insertion order.
    status_items: Vec<(String, String, String)>,
    /// Throttled change tracking for plugin events.
    text_changed_pending: bool,
    last_text_event: f64,
    last_selection: Option<(u64, (usize, usize))>,
    /// Text typed last frame, delivered as an `input` event this frame
    /// (after the editor has inserted it).
    pending_input: String,
    md_cache: egui_commonmark::CommonMarkCache,
    font_size: f32,
    editor_font: String,
    line_height: f32, // multiplier, 1.0 = font default
    tab_size: f32,    // in space widths
    show_line_numbers: bool,
    ligatures: bool,
    zoom: f32, // percent

    prefs_open: bool,
    font_list: Option<Vec<(String, PathBuf)>>,
    font_filter: String,

    find_open: bool,
    show_replace: bool,
    find_text: String,
    replace_text: String,
    match_case: bool,
    wrap_around: bool,
    use_regex: bool,
    regex_error: Option<String>,
    focus_find: bool,
    search_marks: Vec<(usize, usize)>,
    marks_doc: u64,

    goto_open: bool,
    goto_line: String,
    about_open: bool,
    lang_filter: String,

    // Find in Files
    fif_open: bool,
    fif_pattern: String,
    fif_dir: String,
    fif_filter: String,
    fif_regex: bool,
    fif_case: bool,
    fif_error: Option<String>,
    fif_results: Vec<search::GrepMatch>,
    fif_rx: Option<std::sync::mpsc::Receiver<search::GrepMatch>>,
    fif_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    fif_panel: bool,

    recent_files: Vec<PathBuf>,
    last_disk_check: f64,
    reload_confirm: Option<u64>,
    autosave: bool,
    autosave_secs: f32,
    last_autosave: f64,

    pending_events: Vec<egui::Event>,
    focus_editor: bool,
    /// The window had OS focus in the previous frame.
    window_focused: bool,
    close_confirm: Option<PendingClose>,
    allow_quit: bool,

    // word under the last right-click: (char_start, char_end, word)
    ctx_word: Option<(usize, usize, String)>,
    ctx_suggestions: Vec<String>,
    status_msg: Option<(String, f64)>,
    last_title: String,
    inbox: Inbox,
    /// Owns the single-instance socket and the saved session. Secondary
    /// windows neither restore nor overwrite the session.
    is_primary: bool,
    icon_tex: Option<egui::TextureHandle>,

    plugin_host: PluginHost,
    pm: crate::plugin_manager::PluginManager,
    pending_plugin_cmds: Vec<(usize, String)>,
    queued_plugin_events: Vec<(String, Option<String>)>,
    plugin_alerts: Vec<(String, String)>,
    ready_fired: bool,
    last_active_doc: u64,
}

impl NoteyApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        paths: Vec<PathBuf>,
        inbox: Inbox,
        is_primary: bool,
    ) -> Self {
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        // Accessibility normally switches on only when a screen reader or
        // other UI Automation client queries the window; this forces it on
        // for testing that the widget tree stays valid.
        if std::env::var_os("NOTEY_ACCESSKIT").is_some() {
            cc.egui_ctx.enable_accesskit();
        }

        let mut app = Self {
            docs: Vec::new(),
            active: 0,
            next_doc_id: 1,
            next_untitled: 1,
            spell: Spell::empty(),
            spell_enabled: true,
            dark: true,
            custom_theme: None,
            installed_themes: Vec::new(),
            theme_url_open: false,
            theme_url: String::new(),
            theme_url_error: None,
            theme_download_rx: None,
            word_wrap: true,
            show_status: true,
            show_menu: false,
            preview_editor: true,
            editor_states: HashMap::new(),
            md_rendered: std::collections::HashSet::new(),
            md_split: std::collections::HashSet::new(),
            status_items: Vec::new(),
            text_changed_pending: false,
            last_text_event: 0.0,
            last_selection: None,
            pending_input: String::new(),
            md_cache: egui_commonmark::CommonMarkCache::default(),
            font_size: 15.0,
            editor_font: "Consolas".to_string(),
            line_height: 1.0,
            tab_size: 4.0,
            show_line_numbers: false,
            ligatures: true,
            zoom: 100.0,
            prefs_open: false,
            font_list: None,
            font_filter: String::new(),
            find_open: false,
            show_replace: false,
            find_text: String::new(),
            replace_text: String::new(),
            match_case: false,
            wrap_around: true,
            use_regex: false,
            regex_error: None,
            focus_find: false,
            search_marks: Vec::new(),
            marks_doc: 0,
            goto_open: false,
            goto_line: String::new(),
            about_open: false,
            lang_filter: String::new(),
            fif_open: false,
            fif_pattern: String::new(),
            fif_dir: String::new(),
            fif_filter: String::new(),
            fif_regex: false,
            fif_case: false,
            fif_error: None,
            fif_results: Vec::new(),
            fif_rx: None,
            fif_cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            fif_panel: false,
            recent_files: Vec::new(),
            last_disk_check: 0.0,
            reload_confirm: None,
            autosave: false,
            autosave_secs: 30.0,
            last_autosave: 0.0,
            pending_events: Vec::new(),
            focus_editor: true,
            window_focused: false,
            close_confirm: None,
            allow_quit: false,
            ctx_word: None,
            ctx_suggestions: Vec::new(),
            status_msg: None,
            last_title: String::new(),
            inbox,
            is_primary,
            icon_tex: None,
            plugin_host: PluginHost::default(),
            pm: crate::plugin_manager::PluginManager::new(),
            pending_plugin_cmds: Vec::new(),
            queued_plugin_events: Vec::new(),
            plugin_alerts: Vec::new(),
            ready_fired: false,
            last_active_doc: 0,
        };

        PluginHost::seed_examples();
        PluginHost::remove_legacy_markdown_script();
        app.plugin_host = PluginHost::load(&app.pm.feature_scripts());
        // dictionaries and grammars come from plugins; load them off-thread
        app.pm.start_spell_load();
        app.pm.start_syntax_build();
        app.installed_themes = crate::theme_marketplace::installed_themes();

        if let Some(storage) = cc.storage {
            let get_bool = |k: &str, d: bool| {
                storage.get_string(k).map(|v| v == "1").unwrap_or(d)
            };
            app.dark = get_bool("dark", true);
            app.word_wrap = get_bool("word_wrap", true);
            app.show_status = get_bool("show_status", true);
            app.spell_enabled = get_bool("spell", true);
            app.show_line_numbers = get_bool("line_numbers", false);
            app.ligatures = get_bool("ligatures", true);
            app.show_menu = get_bool("show_menu", false);
            // New key: the virtualized editor became the default in 0.4.0, so
            // the "off" saved while it was opt-in is deliberately ignored once.
            app.preview_editor = get_bool("editor_virtualized", true);
            if let Some(v) = storage.get_string("font_size").and_then(|v| v.parse().ok()) {
                app.font_size = v;
            }
            if let Some(v) = storage.get_string("zoom").and_then(|v| v.parse().ok()) {
                app.zoom = v;
            }
            if let Some(v) = storage.get_string("line_height").and_then(|v| v.parse().ok()) {
                app.line_height = v;
            }
            if let Some(v) = storage.get_string("tab_size").and_then(|v| v.parse().ok()) {
                app.tab_size = v;
            }
            if let Some(v) = storage.get_string("editor_font") {
                if !v.is_empty() {
                    app.editor_font = v;
                }
            }
            app.autosave = get_bool("autosave", false);
            if let Some(v) = storage.get_string("autosave_secs").and_then(|v| v.parse().ok())
            {
                app.autosave_secs = v;
            }
            if let Some(v) = storage.get_string("recent_files") {
                if let Ok(list) = serde_json::from_str::<Vec<String>>(&v) {
                    app.recent_files = list.into_iter().map(PathBuf::from).collect();
                }
            }
            if let Some(path) = storage.get_string("theme_path").filter(|path| !path.is_empty()) {
                match theme::load_vscode_theme(PathBuf::from(path).as_path()) {
                    Ok(imported) => {
                        app.dark = imported.dark;
                        app.custom_theme = Some(imported);
                    }
                    Err(err) => {
                        app.status_msg =
                            Some((format!("Theme could not be restored: {err}"), 0.0));
                    }
                }
            }
        }

        // point the "editor" font family at the saved font choice
        let font_name = app.editor_font.clone();
        let path = app.font_path(&font_name);
        rebuild_fonts(&cc.egui_ctx, path.as_deref(), app.tab_size);

        theme::apply_palette(&cc.egui_ctx, app.dark, app.palette());

        // decode the app icon for the custom title bar
        if let Ok(img) = image::load_from_memory(include_bytes!("../notey_icon.ico")) {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let color_img = egui::ColorImage::from_rgba_unmultiplied(
                [w as usize, h as usize],
                rgba.as_raw(),
            );
            app.icon_tex = Some(cc.egui_ctx.load_texture(
                "app_icon",
                color_img,
                egui::TextureOptions::LINEAR,
            ));
        }

        // the first window restores the tabs left open last time
        if is_primary {
            app.restore_session();
        }

        // open files passed on the command line
        for p in &paths {
            app.open_path(p);
        }
        if app.docs.is_empty() {
            app.new_tab();
        }
        app
    }

    // ---------- session (unsaved tabs survive app close) ----------

    fn restore_session(&mut self) {
        let Some(session) = session::load() else { return };
        for tab in session.tabs {
            let id = self.next_doc_id;
            self.next_doc_id += 1;
            let mut doc = match (&tab.path, tab.dirty_text) {
                (Some(path), Some(text)) => Document::restore_file(id, path, text),
                (Some(path), None) => match Document::open(id, path) {
                    Ok(doc) => doc,
                    Err(_) => continue, // file vanished since last session
                },
                (None, Some(text)) => {
                    let doc = Document::restore_untitled(id, tab.untitled_n, text);
                    self.next_untitled = self.next_untitled.max(tab.untitled_n + 1);
                    doc
                }
                (None, None) => continue,
            };
            if tab.language.is_some() {
                doc.language = tab.language.clone();
            }
            self.docs.push(doc);
        }
        if !self.docs.is_empty() {
            self.active = session.active.min(self.docs.len() - 1);
        }
    }

    fn session_snapshot(&self) -> Session {
        let mut tabs = Vec::new();
        let mut active = 0;
        for (i, d) in self.docs.iter().enumerate() {
            // skip pristine empty untitled tabs
            if d.path.is_none() && d.text.is_empty() {
                continue;
            }
            if i == self.active {
                active = tabs.len();
            }
            tabs.push(SessionTab {
                path: d.path.clone(),
                untitled_n: d.untitled_n,
                dirty_text: if d.modified() {
                    Some(d.text.clone())
                } else {
                    None
                },
                language: d.language.clone(),
            });
        }
        Session { active, tabs }
    }

    // ---------- fonts ----------

    fn ensure_font_list(&mut self) -> &[(String, PathBuf)] {
        if self.font_list.is_none() {
            self.font_list = Some(system_fonts());
        }
        self.font_list.as_deref().unwrap()
    }

    fn font_path(&mut self, name: &str) -> Option<PathBuf> {
        let name = name.to_string();
        self.ensure_font_list()
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, p)| p.clone())
    }

    // ---------- document management ----------

    fn new_tab(&mut self) {
        let doc = Document::new(self.next_doc_id, self.next_untitled);
        self.next_doc_id += 1;
        self.next_untitled += 1;
        self.docs.push(doc);
        self.active = self.docs.len() - 1;
        self.focus_editor = true;
    }

    fn editor_id(&self) -> egui::Id {
        egui::Id::new(("editor", self.docs[self.active].id))
    }

    fn open_dialog(&mut self) {
        let files = rfd::FileDialog::new()
            .add_filter("Text Documents (*.txt)", &["txt"])
            .add_filter("All Files", &["*"])
            .pick_files();
        if let Some(files) = files {
            for path in files {
                self.open_path(&path);
            }
        }
    }

    fn note_recent(&mut self, path: &std::path::Path) {
        let p = clean_path(path.to_path_buf());
        self.recent_files.retain(|r| r != &p);
        self.recent_files.insert(0, p);
        self.recent_files.truncate(10);
    }

    /// Open `path` as a tab (or focus its tab if already open).
    fn open_path(&mut self, path: &std::path::Path) {
        let path = clean_path(path.to_path_buf());
        if let Some(i) = self
            .docs
            .iter()
            .position(|d| d.path.as_deref() == Some(&*path))
        {
            self.active = i;
            self.focus_editor = true;
            self.note_recent(&path);
            return;
        }
        let id = self.next_doc_id;
        self.next_doc_id += 1;
        match Document::open(id, &path) {
            Ok(doc) => {
                // replace a pristine untitled tab
                if self.docs.len() == 1
                    && self.docs[0].path.is_none()
                    && self.docs[0].text.is_empty()
                {
                    self.docs.clear();
                }
                self.docs.push(doc);
                self.active = self.docs.len() - 1;
                self.focus_editor = true;
                self.note_recent(&path);
                self.queued_plugin_events.push((
                    "buffer_opened".to_string(),
                    Some(path.display().to_string()),
                ));
            }
            Err(e) => self.flash(format!("Could not open {}: {e}", path.display())),
        }
    }

    fn save_doc(&mut self, ctx: &egui::Context, idx: usize) -> bool {
        if self.docs[idx].path.is_none() {
            return self.save_doc_as(ctx, idx);
        }
        // scripts may mutate the buffer before it hits disk
        if idx == self.active {
            self.fire_event(ctx, "before_save", None);
        }
        match self.docs[idx].save() {
            Ok(()) => {
                let t = self.docs[idx].title();
                self.flash(format!("Saved {t}"));
                if idx == self.active {
                    let path = self.docs[idx].path.as_ref().map(|p| p.display().to_string());
                    self.queued_plugin_events
                        .push(("after_save".to_string(), path));
                }
                true
            }
            Err(e) => {
                self.flash(format!("Save failed: {e}"));
                false
            }
        }
    }

    fn save_doc_as(&mut self, ctx: &egui::Context, idx: usize) -> bool {
        let suggested = self.docs[idx].title();
        let picked = rfd::FileDialog::new()
            .add_filter("Text Documents (*.txt)", &["txt"])
            .add_filter("All Files", &["*"])
            .set_file_name(format!("{suggested}.txt"))
            .save_file();
        if let Some(path) = picked {
            self.docs[idx].path = Some(path.clone());
            self.note_recent(&path);
            self.save_doc(ctx, idx)
        } else {
            false
        }
    }

    fn request_close_tab(&mut self, idx: usize) {
        if self.docs[idx].modified() {
            self.close_confirm = Some(PendingClose::Tab(idx));
        } else {
            self.close_tab(idx);
        }
    }

    fn close_tab(&mut self, idx: usize) {
        self.docs.remove(idx);
        if self.docs.is_empty() {
            self.new_tab();
        }
        if self.active >= self.docs.len() {
            self.active = self.docs.len() - 1;
        }
    }

    fn print_doc(&mut self, ctx: &egui::Context) {
        let doc = &self.docs[self.active];
        let path = if let Some(p) = &doc.path {
            if doc.modified() {
                self.flash("Save the file before printing".into());
                return;
            }
            p.clone()
        } else {
            self.flash("Save the file before printing".into());
            return;
        };
        let _ = ctx; // reserved
        let quoted = path.display().to_string().replace('\'', "''");
        let result = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &format!("Start-Process -FilePath '{quoted}' -Verb Print"),
            ])
            .spawn();
        match result {
            Ok(_) => self.flash("Sent to printer".into()),
            Err(e) => self.flash(format!("Print failed: {e}")),
        }
    }

    fn new_window(&mut self) {
        if let Ok(exe) = std::env::current_exe() {
            let _ = std::process::Command::new(exe).spawn();
        }
    }

    fn flash(&mut self, msg: String) {
        self.status_msg = Some((msg, 0.0));
    }

    fn palette(&self) -> theme::Palette {
        self.custom_theme
            .as_ref()
            .map(|theme| theme.palette)
            .unwrap_or_else(|| theme::palette(self.dark))
    }

    fn apply_current_theme(&mut self, ctx: &egui::Context) {
        theme::apply_palette(ctx, self.dark, self.palette());
        for state in self.editor_states.values_mut() {
            if let Some(cache) = &mut state.hl {
                cache.invalidate_all();
            }
        }
    }

    fn use_builtin_theme(&mut self, ctx: &egui::Context, dark: bool) {
        self.custom_theme = None;
        self.dark = dark;
        self.apply_current_theme(ctx);
    }

    fn import_theme_dialog(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("VS Code color theme", &["json", "jsonc"])
            .add_filter("TextMate theme", &["tmTheme"])
            .pick_file()
        else {
            return;
        };
        match self.activate_theme_path(ctx, &path) {
            Ok(name) => self.flash(format!("Theme: {name}")),
            Err(err) => self.flash(format!("Theme import failed: {err}")),
        }
    }

    fn activate_theme_path(
        &mut self,
        ctx: &egui::Context,
        path: &std::path::Path,
    ) -> Result<String, String> {
        let imported = theme::load_vscode_theme(path)?;
        let name = imported.name.clone();
        self.dark = imported.dark;
        self.custom_theme = Some(imported);
        self.apply_current_theme(ctx);
        Ok(name)
    }

    fn start_theme_url_import(&mut self) {
        let url = self.theme_url.trim().to_string();
        if url.is_empty() || self.theme_download_rx.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.theme_url_error = None;
        self.theme_download_rx = Some(rx);
        self.flash("Downloading VS Code theme…".into());
        std::thread::spawn(move || {
            let _ = tx.send(crate::theme_marketplace::download_theme(&url));
        });
    }

    fn poll_theme_url_import(&mut self, ctx: &egui::Context) {
        let outcome = self.theme_download_rx.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Some(Err("the theme download stopped unexpectedly".into()))
            }
        });
        let Some(outcome) = outcome else {
            if self.theme_download_rx.is_some() {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            return;
        };
        self.theme_download_rx = None;
        match outcome {
            Ok(path) => {
                self.installed_themes = crate::theme_marketplace::installed_themes();
                match self.activate_theme_path(ctx, &path) {
                    Ok(name) => {
                        self.theme_url.clear();
                        self.theme_url_error = None;
                        self.theme_url_open = false;
                        self.flash(format!("Theme: {name}"));
                    }
                    Err(err) => {
                        self.theme_url_error = Some(err.clone());
                        self.flash(format!("Theme import failed: {err}"));
                    }
                }
            }
            Err(err) => {
                self.theme_url_error = Some(err.clone());
                self.flash(format!("Theme import failed: {err}"));
            }
        }
    }

    // ---------- plugins ----------

    fn script_snapshot(&self, ctx: &egui::Context) -> ScriptCtx {
        let doc = &self.docs[self.active];
        let sel = self.cursor_range(ctx).unwrap_or((0, 0));
        ScriptCtx {
            text: doc.text.clone(),
            sel,
            text_dirty: false,
            sel_dirty: false,
            buffers: self
                .docs
                .iter()
                .enumerate()
                .map(|(i, d)| plugins::BufInfo {
                    id: d.id,
                    title: d.title(),
                    path: d.path.as_ref().map(|p| p.display().to_string()),
                    modified: d.modified(),
                    active: i == self.active,
                })
                .collect(),
            active_id: doc.id,
            language: doc.language.clone(),
            config_dir: PluginHost::scripts_dir()
                .and_then(|d| d.parent().map(|p| p.display().to_string()))
                .unwrap_or_default(),
            actions: Vec::new(),
            status: None,
            preview: self.preview_mode(doc.id).to_string(),
            script_name: String::new(),
            capabilities: Vec::new(),
        }
    }

    fn preview_mode(&self, id: u64) -> &'static str {
        if self.md_rendered.contains(&id) {
            "rendered"
        } else if self.md_split.contains(&id) {
            "split"
        } else {
            "off"
        }
    }

    fn set_preview_mode(&mut self, mode: &str) {
        let id = self.docs[self.active].id;
        self.md_rendered.remove(&id);
        self.md_split.remove(&id);
        match mode {
            "rendered" => {
                self.md_rendered.insert(id);
            }
            "split" => {
                self.md_split.insert(id);
                self.focus_editor = true;
            }
            _ => self.focus_editor = true,
        }
    }

    /// Apply a script's new text as the smallest single replacement, so
    /// undo, scroll position and other cursors are disturbed as little as
    /// possible.
    fn apply_text_diff(&mut self, ctx: &egui::Context, new_text: &str) {
        let old: Vec<char> = self.docs[self.active].text.chars().collect();
        let new: Vec<char> = new_text.chars().collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let max_suffix = old.len().min(new.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(max_suffix)
            .take_while(|(a, b)| a == b)
            .count();
        let insert: String = new[prefix..new.len() - suffix].iter().collect();
        self.edit_text(ctx, (prefix, old.len() - suffix), &insert);
    }

    fn apply_script_result(&mut self, ctx: &egui::Context, out: ScriptCtx) {
        if out.text_dirty {
            self.apply_text_diff(ctx, &out.text);
        }
        if out.sel_dirty {
            self.set_cursor(ctx, out.sel.0, out.sel.1);
        }
        if let Some(msg) = out.status {
            self.flash(msg);
        }
        for action in out.actions {
            match action {
                HostAction::OpenFile(p) => self.open_path(&p),
                HostAction::NewTab => self.new_tab(),
                HostAction::SaveActive => {
                    self.save_doc(ctx, self.active);
                }
                HostAction::ActivateBuffer(id) => {
                    if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                        self.active = i;
                        self.focus_editor = true;
                    }
                }
                HostAction::CloseBuffer(id) => {
                    if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                        self.request_close_tab(i);
                    }
                }
                HostAction::Alert(title, body) => self.plugin_alerts.push((title, body)),
                HostAction::SetClipboard(s) => {
                    if let Ok(mut c) = arboard::Clipboard::new() {
                        let _ = c.set_text(s);
                    }
                }
                HostAction::SetLanguage(lang) => {
                    let valid = match &lang {
                        None => Some(None),
                        Some(name) => crate::syntax::global()
                            .set
                            .find_syntax_by_name(name)
                            .map(|sr| Some(sr.name.clone())),
                    };
                    match valid {
                        Some(l) => self.docs[self.active].language = l,
                        None => self.flash(format!(
                            "Unknown language: {}",
                            lang.unwrap_or_default()
                        )),
                    }
                }
                HostAction::ToggleMarkdownPreview => {
                    self.toggle_markdown_preview();
                }
                HostAction::SetPreview(mode) => self.set_preview_mode(&mode),
                HostAction::OpenPath(path) => {
                    // default app for the file (e.g. a browser for .html)
                    let _ = std::process::Command::new("explorer").arg(&path).spawn();
                }
                HostAction::SetStatusItem(key, text, tooltip) => {
                    match self.status_items.iter_mut().find(|(k, _, _)| *k == key) {
                        Some(item) => {
                            item.1 = text;
                            item.2 = tooltip;
                        }
                        None => self.status_items.push((key, text, tooltip)),
                    }
                }
                HostAction::ClearStatusItem(key) => {
                    self.status_items.retain(|(k, _, _)| *k != key);
                }
            }
        }
    }

    /// Flip the active tab between the raw editor and the rendered
    /// Markdown view. Only meaningful for Markdown tabs.
    fn toggle_markdown_preview(&mut self) {
        let doc = &self.docs[self.active];
        if doc.language.as_deref() != Some("Markdown") {
            self.flash("Not a Markdown tab".to_string());
            return;
        }
        let id = doc.id;
        if self.md_rendered.contains(&id) {
            self.md_rendered.remove(&id);
            self.focus_editor = true;
        } else {
            self.md_rendered.insert(id);
            self.md_split.remove(&id);
        }
    }

    fn run_plugin_command(&mut self, ctx: &egui::Context, script_idx: usize, cmd_id: &str) {
        if script_idx >= self.plugin_host.scripts.len() {
            return;
        }
        let snapshot = self.script_snapshot(ctx);
        let out = self.plugin_host.run_command(script_idx, cmd_id, snapshot);
        self.apply_script_result(ctx, out);
        self.surface_plugin_errors();
    }

    fn fire_event(&mut self, ctx: &egui::Context, kind: &str, path: Option<String>) {
        let subscribed: Vec<usize> = self
            .plugin_host
            .scripts
            .iter()
            .enumerate()
            .filter(|(_, s)| s.events.contains(kind))
            .map(|(i, _)| i)
            .collect();
        for idx in subscribed {
            let snapshot = self.script_snapshot(ctx);
            let mut event = rhai::Map::new();
            event.insert("kind".into(), rhai::Dynamic::from(kind.to_string()));
            event.insert(
                "buffer_id".into(),
                rhai::Dynamic::from(self.docs[self.active].id as i64),
            );
            if let Some(p) = &path {
                // `input` events carry the typed text; the rest carry a path
                let key = if kind == "input" { "text" } else { "path" };
                event.insert(key.into(), rhai::Dynamic::from(p.clone()));
            }
            let out = self.plugin_host.run_event(idx, event, snapshot);
            self.apply_script_result(ctx, out);
        }
        self.surface_plugin_errors();
    }

    fn surface_plugin_errors(&mut self) {
        if let Some(err) = self.plugin_host.errors.pop() {
            self.plugin_host.errors.clear();
            self.flash(format!("Plugin error: {err}"));
        }
    }

    // ---------- cursor / text helpers ----------

    fn cursor_range(&self, ctx: &egui::Context) -> Option<(usize, usize)> {
        if self.preview_editor {
            let id = self.docs[self.active].id;
            return self.editor_states.get(&id).map(|s| s.selection());
        }
        let state = egui::text_edit::TextEditState::load(ctx, self.editor_id())?;
        let range = state.cursor.char_range()?;
        let (a, b) = (
            usize::from(range.primary.index),
            usize::from(range.secondary.index),
        );
        Some((a.min(b), a.max(b)))
    }

    fn set_cursor(&mut self, ctx: &egui::Context, start: usize, end: usize) {
        if self.preview_editor {
            let id = self.docs[self.active].id;
            self.editor_states
                .entry(id)
                .or_default()
                .set_selection(start, end);
            self.focus_editor = true;
            return;
        }
        let id = self.editor_id();
        let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(start), CCursor::new(end))));
        state.store(ctx, id);
        self.focus_drawn_editor(ctx);
    }

    /// Focus the editor, but only if it will be drawn this frame. Focusing
    /// an id with no widget behind it (e.g. while the tab shows the rendered
    /// Markdown view) makes egui report a phantom focused node, which
    /// crashes accessibility clients (AccessKit validates the focus).
    fn focus_drawn_editor(&self, ctx: &egui::Context) {
        let doc_id = self.docs[self.active].id;
        if !self.md_rendered.contains(&doc_id) {
            let id = self.editor_id();
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }

    fn insert_at_cursor(&mut self, ctx: &egui::Context, s: &str) {
        let range = self.cursor_range(ctx).unwrap_or_else(|| {
            let n = self.docs[self.active].text.chars().count();
            (n, n)
        });
        self.edit_text(ctx, range, s);
    }

    fn insert_time_date(&mut self, ctx: &egui::Context) {
        let now = chrono::Local::now();
        let stamp = now.format("%-I:%M %p %-m/%-d/%Y").to_string();
        self.insert_at_cursor(ctx, &stamp);
    }

    // ---------- find / replace ----------

    /// Compile the Find pattern, surfacing errors in the Find window.
    fn compiled_pattern(&mut self) -> Option<fancy_regex::Regex> {
        let opts = search::SearchOpts {
            regex: self.use_regex,
            match_case: self.match_case,
        };
        match search::compile(&self.find_text, opts) {
            Ok(re) => {
                self.regex_error = None;
                Some(re)
            }
            Err(e) => {
                self.regex_error = Some(e);
                None
            }
        }
    }

    fn find_next(&mut self, ctx: &egui::Context, backwards: bool) {
        if self.find_text.is_empty() {
            return;
        }
        let Some(re) = self.compiled_pattern() else { return };
        let text = self.docs[self.active].text.clone();
        let (sel_start, sel_end) = self.cursor_range(ctx).unwrap_or((0, 0));
        let from = if backwards { sel_start } else { sel_end };
        match search::find_next(&text, &re, from, backwards, self.wrap_around) {
            Some((a, b)) => self.set_cursor(ctx, a, b),
            None => self.flash(format!("Cannot find \"{}\"", self.find_text)),
        }
    }

    fn replace_current(&mut self, ctx: &egui::Context) {
        if self.find_text.is_empty() {
            return;
        }
        let Some(re) = self.compiled_pattern() else { return };
        if let Some((a, b)) = self.cursor_range(ctx) {
            if a != b {
                let text = self.docs[self.active].text.clone();
                // selection must be exactly one match of the pattern
                if search::find_next(&text, &re, a, false, false) == Some((a, b)) {
                    let sb = char_to_byte(&text, a);
                    let eb = char_to_byte(&text, b);
                    let replacement = search::expand_one(
                        &text[sb..eb],
                        &re,
                        &self.replace_text,
                        self.use_regex,
                    );
                    self.edit_text(ctx, (a, b), &replacement);
                }
            }
        }
        self.find_next(ctx, false);
    }

    fn replace_all(&mut self, ctx: &egui::Context) {
        if self.find_text.is_empty() {
            return;
        }
        let Some(re) = self.compiled_pattern() else { return };
        let text = self.docs[self.active].text.clone();
        let (out, count) = search::replace_all(&text, &re, &self.replace_text, self.use_regex);
        if count > 0 {
            let old_n = text.chars().count();
            self.edit_text(ctx, (0, old_n), &out);
            self.set_cursor(ctx, 0, 0);
        }
        self.flash(format!("Replaced {count} occurrence(s)"));
    }

    fn mark_all(&mut self, _ctx: &egui::Context) {
        if self.find_text.is_empty() {
            self.search_marks.clear();
            return;
        }
        let Some(re) = self.compiled_pattern() else { return };
        let doc = &self.docs[self.active];
        self.search_marks = search::find_all(&doc.text, &re, 5000);
        self.marks_doc = doc.id;
        self.flash(format!("Marked {} match(es)", self.search_marks.len()));
    }

    fn goto_line_n(&mut self, ctx: &egui::Context, n: usize) {
        let text = self.docs[self.active].text.clone();
        if n <= 1 {
            self.set_cursor(ctx, 0, 0);
            return;
        }
        let mut line = 1usize;
        for (i, ch) in text.chars().enumerate() {
            if ch == '\n' {
                line += 1;
                if line == n {
                    self.set_cursor(ctx, i + 1, i + 1);
                    return;
                }
            }
        }
        let end = text.chars().count();
        self.set_cursor(ctx, end, end);
    }

    fn goto_line(&mut self, ctx: &egui::Context) {
        if let Ok(n) = self.goto_line.trim().parse::<usize>() {
            self.goto_line_n(ctx, n);
            self.goto_open = false;
        }
    }

    // ---------- Find in Files ----------

    fn open_fif(&mut self) {
        if self.fif_dir.trim().is_empty() {
            if let Some(dir) = self.docs[self.active]
                .path
                .as_ref()
                .and_then(|p| p.parent())
            {
                self.fif_dir = dir.display().to_string();
            }
        }
        self.fif_open = true;
    }

    fn start_find_in_files(&mut self) {
        self.fif_error = None;
        if self.fif_pattern.is_empty() {
            return;
        }
        let opts = search::SearchOpts {
            regex: self.fif_regex,
            match_case: self.fif_case,
        };
        let re = match search::compile(&self.fif_pattern, opts) {
            Ok(re) => re,
            Err(e) => {
                self.fif_error = Some(e);
                return;
            }
        };
        let root = PathBuf::from(self.fif_dir.trim());
        if !root.is_dir() {
            self.fif_error = Some("Folder does not exist".to_string());
            return;
        }
        // cancel any previous run
        self.fif_cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.fif_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel = self.fif_cancel.clone();
        let filters = self.fif_filter.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            search::grep_dir(root, re, filters, cancel, tx);
        });
        self.fif_rx = Some(rx);
        self.fif_results.clear();
        self.fif_panel = true;
    }

    fn fif_window(&mut self, ctx: &egui::Context) {
        if !self.fif_open {
            return;
        }
        let mut open = self.fif_open;
        egui::Window::new("Find in Files")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("fif_grid").num_columns(2).show(ui, |ui| {
                    ui.label("Find what:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.fif_pattern)
                            .desired_width(260.0),
                    );
                    ui.end_row();
                    ui.label("In folder:");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.fif_dir)
                                .desired_width(220.0),
                        );
                        if ui.button("…").clicked() {
                            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                self.fif_dir = dir.display().to_string();
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("Filters:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.fif_filter)
                            .hint_text("*.rs;*.toml (empty = all)")
                            .desired_width(260.0),
                    );
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.fif_case, "Match case");
                    ui.checkbox(&mut self.fif_regex, "Regex");
                });
                if let Some(err) = &self.fif_error {
                    ui.colored_label(Color32::from_rgb(232, 82, 82), err.clone());
                }
                ui.separator();
                if ui.button("Search").clicked() {
                    self.start_find_in_files();
                }
            });
        self.fif_open = open;
    }

    fn fif_results_panel(&mut self, root: &mut egui::Ui, ctx: &egui::Context) {
        if !self.fif_panel {
            return;
        }
        let p = self.palette();
        let mut open_req: Option<(PathBuf, usize)> = None;
        egui::Panel::bottom(egui::Id::new("fif_results"))
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.chrome)
                    .inner_margin(egui::Margin::symmetric(8, 6)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    let running = self.fif_rx.is_some();
                    let count = self.fif_results.len();
                    ui.label(format!(
                        "{count} result(s) for \"{}\"{}",
                        self.fif_pattern,
                        if running { " — searching…" } else { "" }
                    ));
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui.small_button("Close").clicked() {
                                self.fif_panel = false;
                                self.fif_cancel
                                    .store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                            if running && ui.small_button("Stop").clicked() {
                                self.fif_cancel
                                    .store(true, std::sync::atomic::Ordering::Relaxed);
                            }
                        },
                    );
                });
                ui.separator();
                let root_dir = PathBuf::from(self.fif_dir.trim());
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for m in &self.fif_results {
                            let rel = m
                                .path
                                .strip_prefix(&root_dir)
                                .unwrap_or(&m.path)
                                .display();
                            let label = format!("{rel}:{}  {}", m.line_no, m.preview);
                            if ui
                                .selectable_label(false, egui::RichText::new(label).monospace().size(12.0))
                                .clicked()
                            {
                                open_req = Some((m.path.clone(), m.line_no));
                            }
                        }
                    });
            });
        if let Some((path, line)) = open_req {
            self.open_path(&path);
            self.goto_line_n(ctx, line);
        }
    }

    // ---------- UI ----------

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let mut zoom_in = false;
        let mut zoom_out = false;
        let mut zoom_reset = false;
        let mut new_tab = false;
        let mut new_window = false;
        let mut open = false;
        let mut save = false;
        let mut save_as = false;
        let mut close_tab = false;
        let mut print = false;
        let mut find = false;
        let mut find_files = false;
        let mut replace = false;
        let mut goto = false;
        let mut find_next = false;
        let mut find_prev = false;
        let mut time_date = false;
        let mut next_tab = false;
        let mut prev_tab = false;
        let mut prefs = false;

        ctx.input_mut(|i| {
            new_window = i.consume_shortcut(&SC_NEW_WINDOW);
            new_tab = i.consume_shortcut(&SC_NEW_TAB);
            open = i.consume_shortcut(&SC_OPEN);
            save_as = i.consume_shortcut(&SC_SAVE_AS);
            save = i.consume_shortcut(&SC_SAVE);
            close_tab = i.consume_shortcut(&SC_CLOSE_TAB);
            print = i.consume_shortcut(&SC_PRINT);
            find_files = i.consume_shortcut(&SC_FIND_FILES);
            find = i.consume_shortcut(&SC_FIND);
            replace = i.consume_shortcut(&SC_REPLACE);
            goto = i.consume_shortcut(&SC_GOTO);
            find_prev = i.consume_shortcut(&SC_FIND_PREV);
            find_next = i.consume_shortcut(&SC_FIND_NEXT);
            time_date = i.consume_shortcut(&SC_TIME_DATE);
            zoom_in = i.consume_shortcut(&SC_ZOOM_IN) || i.consume_shortcut(&SC_ZOOM_IN2);
            zoom_out = i.consume_shortcut(&SC_ZOOM_OUT);
            zoom_reset = i.consume_shortcut(&SC_ZOOM_RESET);
            prev_tab = i.consume_shortcut(&SC_PREV_TAB);
            next_tab = i.consume_shortcut(&SC_NEXT_TAB);
            prefs = i.consume_shortcut(&SC_PREFS);
        });

        if new_window {
            self.new_window();
        }
        if new_tab {
            self.new_tab();
        }
        if open {
            self.open_dialog();
        }
        if save {
            self.save_doc(ctx, self.active);
        }
        if save_as {
            self.save_doc_as(ctx, self.active);
        }
        if close_tab {
            self.request_close_tab(self.active);
        }
        if print {
            self.print_doc(ctx);
        }
        if find {
            self.find_open = true;
            self.show_replace = false;
            self.focus_find = true;
        }
        if find_files {
            self.open_fif();
        }
        if replace {
            self.find_open = true;
            self.show_replace = true;
            self.focus_find = true;
        }
        if goto {
            self.goto_open = true;
        }
        if find_next {
            self.find_next(ctx, false);
        }
        if find_prev {
            self.find_next(ctx, true);
        }
        if time_date {
            self.insert_time_date(ctx);
        }
        if zoom_in {
            self.zoom = (self.zoom + 10.0).min(500.0);
        }
        if zoom_out {
            self.zoom = (self.zoom - 10.0).max(10.0);
        }
        if zoom_reset {
            self.zoom = 100.0;
        }
        if next_tab && self.docs.len() > 1 {
            self.active = (self.active + 1) % self.docs.len();
        }
        if prev_tab && self.docs.len() > 1 {
            self.active = (self.active + self.docs.len() - 1) % self.docs.len();
        }
        if prefs {
            self.prefs_open = true;
        }
    }

    /// Consume manifest-declared plugin shortcuts. Must run BEFORE the
    /// built-in shortcuts: egui's `matches_logically` ignores extra pressed
    /// modifiers, so Ctrl+Alt+S would otherwise be swallowed by Ctrl+S.
    /// Offer unmodified key presses (e.g. Enter) to scripts that registered
    /// a key hook; a script returning true consumes the key.
    fn handle_plugin_keys(&mut self, ctx: &egui::Context) {
        if !ctx.memory(|m| m.has_focus(self.editor_id())) {
            return;
        }
        let language = self.docs[self.active].language.clone();
        let hooks: Vec<(usize, Key)> = self
            .plugin_host
            .scripts
            .iter()
            .enumerate()
            .flat_map(|(i, s)| {
                let language = language.clone();
                s.keys
                    .iter()
                    .filter(move |(_, w)| w.matches(language.as_deref(), true))
                    .map(move |(k, _)| (i, *k))
            })
            .collect();
        for (i, key) in hooks {
            let pressed = ctx.input(|inp| inp.modifiers.is_none() && inp.key_pressed(key));
            if !pressed {
                continue;
            }
            let snapshot = self.script_snapshot(ctx);
            let (out, handled) = self.plugin_host.run_key(i, key.name(), snapshot);
            if handled {
                ctx.input_mut(|inp| inp.consume_key(Modifiers::NONE, key));
            }
            self.apply_script_result(ctx, out);
            self.surface_plugin_errors();
            if handled {
                break;
            }
        }
    }

    fn handle_plugin_shortcuts(&mut self, ctx: &egui::Context) {
        let language = self.docs[self.active].language.clone();
        let focused = ctx.memory(|m| m.has_focus(self.editor_id()));
        let plugin_shortcuts: Vec<(usize, String, KeyboardShortcut)> = self
            .plugin_host
            .scripts
            .iter()
            .enumerate()
            .flat_map(|(i, s)| {
                let language = language.clone();
                s.commands.iter().filter_map(move |c| {
                    c.when
                        .matches(language.as_deref(), focused)
                        .then_some(())
                        .and(c.shortcut.map(|sc| (i, c.id.clone(), sc)))
                })
            })
            .collect();
        for (i, id, sc) in plugin_shortcuts {
            if ctx.input_mut(|inp| inp.consume_shortcut(&sc)) {
                self.pending_plugin_cmds.push((i, id));
            }
        }
    }

    fn menu_bar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        // flat, Fluent-style top-level menu buttons
        ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
        egui::MenuBar::new().ui(ui, |ui| {
            self.menus(ctx, ui);
        });
    }

    /// The File/Edit/View/Help menus — hosted either in the menu bar row or
    /// in the popup under the title-bar icon.
    fn menus(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        {
            ui.menu_button("File", |ui| {
                if menu_item(ui, "New Tab", "Ctrl+N") {
                    self.new_tab();
                }
                if menu_item(ui, "New Window", "Ctrl+Shift+N") {
                    self.new_window();
                }
                if menu_item(ui, "Open…", "Ctrl+O") {
                    self.open_dialog();
                }
                ui.menu_button("Open Recent", |ui| {
                    ui.set_min_width(180.0);
                    if self.recent_files.is_empty() {
                        ui.label(egui::RichText::new("(empty)").weak());
                    }
                    let mut open_req: Option<PathBuf> = None;
                    for p in &self.recent_files {
                        let name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| p.display().to_string());
                        if ui
                            .button(name)
                            .on_hover_text(p.display().to_string())
                            .clicked()
                        {
                            open_req = Some(p.clone());
                            ui.close();
                        }
                    }
                    if !self.recent_files.is_empty() {
                        ui.separator();
                        if ui.button("Clear Recently Opened").clicked() {
                            self.recent_files.clear();
                            ui.close();
                        }
                    }
                    if let Some(p) = open_req {
                        self.open_path(&p);
                    }
                });
                if menu_item(ui, "Save", "Ctrl+S") {
                    self.save_doc(ctx, self.active);
                }
                if menu_item(ui, "Save As…", "Ctrl+Shift+S") {
                    self.save_doc_as(ctx, self.active);
                }
                ui.separator();
                if menu_item(ui, "Print…", "Ctrl+P") {
                    self.print_doc(ctx);
                }
                ui.separator();
                if menu_item(ui, "Close Tab", "Ctrl+W") {
                    self.request_close_tab(self.active);
                }
                if ui.button("Exit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if menu_item(ui, "Undo", "Ctrl+Z") {
                    self.pending_events.push(key_event(Key::Z, Modifiers::CTRL));
                }
                if menu_item(ui, "Redo", "Ctrl+Y") {
                    self.pending_events.push(key_event(Key::Y, Modifiers::CTRL));
                }
                ui.separator();
                if menu_item(ui, "Cut", "Ctrl+X") {
                    self.pending_events.push(egui::Event::Cut);
                }
                if menu_item(ui, "Copy", "Ctrl+C") {
                    self.pending_events.push(egui::Event::Copy);
                }
                if menu_item(ui, "Paste", "Ctrl+V") {
                    if let Some(s) = clipboard_text() {
                        self.pending_events.push(egui::Event::Paste(s));
                    }
                }
                if menu_item(ui, "Delete", "Del") {
                    self.pending_events.push(key_event(Key::Delete, Modifiers::NONE));
                }
                ui.separator();
                if menu_item(ui, "Find…", "Ctrl+F") {
                    self.find_open = true;
                    self.show_replace = false;
                    self.focus_find = true;
                }
                if menu_item(ui, "Find Next", "F3") {
                    self.find_next(ctx, false);
                }
                if menu_item(ui, "Find Previous", "Shift+F3") {
                    self.find_next(ctx, true);
                }
                if menu_item(ui, "Replace…", "Ctrl+H") {
                    self.find_open = true;
                    self.show_replace = true;
                    self.focus_find = true;
                }
                if menu_item(ui, "Find in Files…", "Ctrl+Shift+F") {
                    self.open_fif();
                }
                if menu_item(ui, "Go To…", "Ctrl+G") {
                    self.goto_open = true;
                }
                ui.separator();
                if menu_item(ui, "Select All", "Ctrl+A") {
                    let n = self.docs[self.active].text.chars().count();
                    self.set_cursor(ctx, 0, n);
                }
                if menu_item(ui, "Time/Date", "F5") {
                    self.insert_time_date(ctx);
                }
                ui.separator();
                if menu_item(ui, "Preferences…", "Ctrl+,") {
                    self.prefs_open = true;
                }
            });
            ui.menu_button("View", |ui| {
                ui.menu_button("Zoom", |ui| {
                    if menu_item(ui, "Zoom In", "Ctrl+Plus") {
                        self.zoom = (self.zoom + 10.0).min(500.0);
                    }
                    if menu_item(ui, "Zoom Out", "Ctrl+Minus") {
                        self.zoom = (self.zoom - 10.0).max(10.0);
                    }
                    if menu_item(ui, "Restore Default Zoom", "Ctrl+0") {
                        self.zoom = 100.0;
                    }
                });
                ui.checkbox(&mut self.show_status, "Status Bar");
                let mut spell_on = self.spell_enabled;
                let spell_resp = ui
                    .add_enabled(self.spell.available(), egui::Checkbox::new(&mut spell_on, "Spellcheck"));
                if spell_resp.changed() {
                    self.spell_enabled = spell_on;
                }
                if self.spell.available() {
                    spell_resp.on_hover_text(format!("Languages: {}", self.spell.locales().join(", ")));
                } else if spell_resp
                    .on_disabled_hover_text("Install the Spellcheck feature and a language from Plugins")
                    .clicked()
                {
                    self.pm.open_at(crate::packages::Kind::Feature);
                }
                ui.separator();
                ui.menu_button("Theme", |ui| {
                    if ui
                        .selectable_label(self.custom_theme.is_none() && self.dark, "Notey Dark")
                        .clicked()
                    {
                        self.use_builtin_theme(ctx, true);
                        ui.close();
                    }
                    if ui
                        .selectable_label(self.custom_theme.is_none() && !self.dark, "Notey Light")
                        .clicked()
                    {
                        self.use_builtin_theme(ctx, false);
                        ui.close();
                    }
                    let active_theme_path = self
                        .custom_theme
                        .as_ref()
                        .map(|theme| theme.path.clone());
                    let active_is_installed = active_theme_path.as_ref().is_some_and(|path| {
                        self.installed_themes
                            .iter()
                            .any(|theme| theme.path.as_path() == path.as_path())
                    });
                    let mut installed_pick = None;
                    if !self.installed_themes.is_empty() {
                        ui.menu_button("Installed Themes", |ui| {
                            for theme in &self.installed_themes {
                                let active = active_theme_path
                                    .as_ref()
                                    .is_some_and(|path| path.as_path() == theme.path.as_path());
                                let label = format!("{} — {}", theme.label, theme.package);
                                if ui.selectable_label(active, label).clicked() {
                                    installed_pick = Some(theme.path.clone());
                                    ui.close();
                                }
                            }
                        });
                    }
                    if let Some(path) = installed_pick {
                        match self.activate_theme_path(ctx, &path) {
                            Ok(name) => self.flash(format!("Theme: {name}")),
                            Err(err) => self.flash(format!("Theme could not be loaded: {err}")),
                        }
                        ui.close();
                    }
                    if !active_is_installed {
                        if let Some(name) = self.custom_theme.as_ref().map(|theme| theme.name.clone()) {
                            let _ = ui.selectable_label(true, name);
                        }
                    }
                    ui.separator();
                    if ui.button("Import VS Code Theme…").clicked() {
                        ui.close();
                        self.import_theme_dialog(ctx);
                    }
                    if ui.button("Get Theme from vscodethemes.com…").clicked() {
                        self.theme_url_error = None;
                        self.theme_url_open = true;
                        ui.close();
                    }
                });
            });
            ui.menu_button("Language", |ui| {
                self.language_picker_contents(ui);
            });
            ui.menu_button("Plugins", |ui| {
                if ui.button("Manage Plugins…").clicked() {
                    self.pm.open_at(crate::packages::Kind::Feature);
                    ui.close();
                }
                ui.separator();
                let mut run: Option<(usize, String)> = None;
                let language = self.docs[self.active].language.clone();
                for (i, script) in self.plugin_host.scripts.iter().enumerate() {
                    for cmd in &script.commands {
                        // focus conditions don't apply to menu clicks
                        let available = cmd.when.matches(language.as_deref(), true);
                        let btn = if cmd.shortcut_text.is_empty() {
                            ui.add_enabled(available, egui::Button::new(&cmd.title))
                        } else {
                            ui.add_enabled(
                                available,
                                egui::Button::new(&cmd.title)
                                    .shortcut_text(&cmd.shortcut_text),
                            )
                        };
                        if btn
                            .on_hover_text(format!(
                                "{} — {}",
                                script.name,
                                script.path.display()
                            ))
                            .clicked()
                        {
                            run = Some((i, cmd.id.clone()));
                            self.focus_editor = true;
                        }
                    }
                }
                if !self.plugin_host.scripts.is_empty() {
                    ui.separator();
                }
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
                if let Some((i, id)) = run {
                    self.pending_plugin_cmds.push((i, id));
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("About Notey").clicked() {
                    self.about_open = true;
                }
            });
        }
    }

    /// Custom Fluent title bar: app icon, Win11-style tabs, "+", a drag
    /// region, and min/max/close caption buttons.
    fn title_bar(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let p = self.palette();
        let bar = ui.max_rect();
        let painter = ui.painter().clone();
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));

        // ---- caption buttons (fixed to the right edge) ----
        const BTN_W: f32 = 46.0;
        let caption_x0 = bar.right() - 3.0 * BTN_W;
        let glyph_stroke = egui::Stroke::new(1.2, p.text);
        for (k, idx) in ["min", "max", "close"].iter().zip(0..3) {
            let rect = egui::Rect::from_min_size(
                egui::pos2(caption_x0 + idx as f32 * BTN_W, bar.top()),
                egui::vec2(BTN_W, bar.height()),
            );
            let resp = ui.interact(rect, egui::Id::new(("caption", *k)), egui::Sense::click());
            let is_close = *k == "close";
            if resp.hovered() {
                let fill = if is_close { p.close_hover } else { p.control_hover };
                painter.rect_filled(rect, 0.0, fill);
            }
            let g = if resp.hovered() && is_close {
                egui::Stroke::new(1.2, Color32::WHITE)
            } else {
                glyph_stroke
            };
            let c = rect.center();
            match *k {
                "min" => {
                    painter.line_segment(
                        [c + egui::vec2(-5.0, 0.5), c + egui::vec2(5.0, 0.5)],
                        g,
                    );
                }
                "max" => {
                    if maximized {
                        // restore: two offset squares
                        let r1 = egui::Rect::from_center_size(
                            c + egui::vec2(-1.5, 1.5),
                            egui::vec2(8.0, 8.0),
                        );
                        painter.rect_stroke(r1, 2.0, g, egui::StrokeKind::Middle);
                        painter.line_segment(
                            [c + egui::vec2(-1.5, -2.5), c + egui::vec2(2.5, -2.5)],
                            g,
                        );
                        painter.line_segment(
                            [c + egui::vec2(4.5, -2.5), c + egui::vec2(4.5, 1.5)],
                            g,
                        );
                    } else {
                        let r = egui::Rect::from_center_size(c, egui::vec2(9.0, 9.0));
                        painter.rect_stroke(r, 2.0, g, egui::StrokeKind::Middle);
                    }
                }
                _ => {
                    painter.line_segment(
                        [c + egui::vec2(-4.5, -4.5), c + egui::vec2(4.5, 4.5)],
                        g,
                    );
                    painter.line_segment(
                        [c + egui::vec2(-4.5, 4.5), c + egui::vec2(4.5, -4.5)],
                        g,
                    );
                }
            }
            if resp.clicked() {
                match *k {
                    "min" => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                    "max" => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized))
                    }
                    _ => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                }
            }
        }

        // ---- app icon (doubles as the menu button when the bar is hidden) ----
        let mut x = bar.left() + 8.0;
        if let Some(tex) = self.icon_tex.clone() {
            let hit_rect = egui::Rect::from_center_size(
                egui::pos2(x + 14.0, bar.center().y),
                egui::vec2(28.0, 28.0),
            );
            let icon_rect =
                egui::Rect::from_center_size(hit_rect.center(), egui::vec2(16.0, 16.0));
            if self.show_menu {
                painter.image(
                    tex.id(),
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                let resp =
                    ui.interact(hit_rect, egui::Id::new("icon_menu"), egui::Sense::click());
                if resp.hovered() {
                    painter.rect_filled(hit_rect, 4.0, p.tab_hover);
                }
                painter.image(
                    tex.id(),
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
                egui::Popup::menu(&resp)
                    .anchor(egui::PopupAnchor::Position(egui::pos2(
                        hit_rect.left(),
                        hit_rect.bottom() + 4.0,
                    )))
                    .show(|ui| {
                        ui.set_min_width(160.0);
                        self.menus(ctx, ui);
                    });
            }
            x += 30.0;
        }

        // ---- tabs ----
        let mut close_request: Option<usize> = None;
        let mut activate: Option<usize> = None;
        let plus_w = 30.0;
        let tabs_avail = (caption_x0 - 60.0 - plus_w - x).max(60.0);
        let n = self.docs.len().max(1) as f32;
        let tab_w = (tabs_avail / n).clamp(60.0, 200.0);
        let tab_font = FontId::new(13.0, FontFamily::Proportional);
        for (i, doc) in self.docs.iter().enumerate() {
            let rect = egui::Rect::from_min_size(
                egui::pos2(x, bar.top() + 6.0),
                egui::vec2(tab_w - 4.0, bar.height() - 12.0),
            );
            x += tab_w;
            let resp = ui.interact(rect, egui::Id::new(("tab", doc.id)), egui::Sense::click());
            let active = i == self.active;
            if active {
                painter.rect_filled(rect, 6.0, p.control_active);
                painter.rect_stroke(
                    rect,
                    6.0,
                    egui::Stroke::new(1.0, p.stroke),
                    egui::StrokeKind::Inside,
                );
            } else if resp.hovered() {
                painter.rect_filled(rect, 6.0, p.tab_hover);
            }

            // close button (shown on active or hovered tabs, if wide enough)
            let show_close = (active || resp.hovered()) && tab_w > 80.0;
            let close_rect = egui::Rect::from_center_size(
                egui::pos2(rect.right() - 14.0, rect.center().y),
                egui::vec2(18.0, 18.0),
            );
            let text_right = if show_close {
                close_rect.left() - 2.0
            } else {
                rect.right() - 8.0
            };

            // title (truncated)
            let label = if doc.modified() {
                format!("• {}", doc.title())
            } else {
                doc.title()
            };
            let text_color = if active { p.text } else { p.text_weak };
            let mut job =
                egui::text::LayoutJob::simple_singleline(label, tab_font.clone(), text_color);
            job.wrap.max_width = (text_right - rect.left() - 10.0).max(10.0);
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            let galley = painter.layout_job(job);
            painter.galley(
                egui::pos2(
                    rect.left() + 10.0,
                    rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                text_color,
            );

            if show_close {
                let close_resp = ui.interact(
                    close_rect,
                    egui::Id::new(("tab_close", doc.id)),
                    egui::Sense::click(),
                );
                if close_resp.hovered() {
                    painter.rect_filled(close_rect, 4.0, p.control_hover);
                }
                let cc = close_rect.center();
                let cg = egui::Stroke::new(1.1, text_color);
                painter.line_segment(
                    [cc + egui::vec2(-3.5, -3.5), cc + egui::vec2(3.5, 3.5)],
                    cg,
                );
                painter.line_segment(
                    [cc + egui::vec2(-3.5, 3.5), cc + egui::vec2(3.5, -3.5)],
                    cg,
                );
                if close_resp.clicked() {
                    close_request = Some(i);
                }
            }
            if resp.clicked() && close_request.is_none() {
                activate = Some(i);
            }
            if resp.middle_clicked() {
                close_request = Some(i);
            }
        }

        // ---- "+" new tab ----
        let plus_rect = egui::Rect::from_min_size(
            egui::pos2(x + 2.0, bar.top() + 8.0),
            egui::vec2(26.0, bar.height() - 16.0),
        );
        let plus_resp = ui.interact(plus_rect, egui::Id::new("tab_plus"), egui::Sense::click());
        if plus_resp.hovered() {
            painter.rect_filled(plus_rect, 4.0, p.tab_hover);
        }
        let pc = plus_rect.center();
        let pg = egui::Stroke::new(1.2, p.text);
        painter.line_segment([pc + egui::vec2(-4.5, 0.0), pc + egui::vec2(4.5, 0.0)], pg);
        painter.line_segment([pc + egui::vec2(0.0, -4.5), pc + egui::vec2(0.0, 4.5)], pg);
        if plus_resp.clicked() {
            self.new_tab();
        }
        plus_resp.on_hover_text("New tab (Ctrl+N)");

        // ---- drag region (everything between "+" and caption buttons) ----
        let drag_rect = egui::Rect::from_min_max(
            egui::pos2(plus_rect.right() + 2.0, bar.top()),
            egui::pos2(caption_x0, bar.bottom()),
        );
        let drag_resp = ui.interact(
            drag_rect,
            egui::Id::new("titlebar_drag"),
            egui::Sense::click_and_drag(),
        );
        if drag_resp.double_clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        } else if drag_resp.is_pointer_button_down_on()
            && ctx.input(|input| input.pointer.primary_pressed())
        {
            // Hand the press to the OS immediately. `drag_started()` waits
            // for egui's movement threshold, which leaves the window behind
            // the pointer by that initial dead-zone distance.
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        if let Some(i) = activate {
            self.active = i;
            self.focus_editor = true;
        }
        if let Some(i) = close_request {
            self.request_close_tab(i);
        }
    }

    /// Edge-resize support for the frameless window.
    fn handle_resize_edges(&self, ctx: &egui::Context) {
        use egui::{CursorIcon, ViewportCommand};
        use egui::viewport::ResizeDirection;
        if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
            return;
        }
        let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else {
            return;
        };
        let r = ctx.content_rect();
        const B: f32 = 5.0;
        let left = pos.x - r.left() < B;
        let right = r.right() - pos.x < B;
        let top = pos.y - r.top() < B;
        let bottom = r.bottom() - pos.y < B;
        let dir = match (left, right, top, bottom) {
            (true, _, true, _) => Some(ResizeDirection::NorthWest),
            (_, true, true, _) => Some(ResizeDirection::NorthEast),
            (true, _, _, true) => Some(ResizeDirection::SouthWest),
            (_, true, _, true) => Some(ResizeDirection::SouthEast),
            (true, ..) => Some(ResizeDirection::West),
            (_, true, ..) => Some(ResizeDirection::East),
            (_, _, true, _) => Some(ResizeDirection::North),
            (_, _, _, true) => Some(ResizeDirection::South),
            _ => None,
        };
        let Some(dir) = dir else { return };
        let icon = match dir {
            ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
            ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
            ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
            ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
        };
        ctx.output_mut(|o| o.cursor_icon = icon);
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }

    fn editor(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        self.keep_editor_focus(ctx);
        // rendered Markdown view (toggled by the bundled Markdown Preview
        // plugin) replaces whichever editor is active for this tab
        let active_id = self.docs[self.active].id;
        if self.md_rendered.contains(&active_id) {
            if self.docs[self.active].language.as_deref() == Some("Markdown") {
                self.markdown_view(ui);
                return;
            }
            // language changed away from Markdown: fall back to the editor
            self.md_rendered.remove(&active_id);
        }
        if self.md_split.contains(&active_id) {
            let text = self.docs[self.active].text.clone();
            let fill = self.palette().editor;
            egui::Panel::right(egui::Id::new(("md_split", active_id)))
                .resizable(true)
                .default_size(ui.available_width() * 0.5)
                .min_size(160.0)
                .frame(egui::Frame::new().fill(fill).inner_margin(egui::Margin {
                    left: 14,
                    right: 8,
                    top: 4,
                    bottom: 4,
                }))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("md_split_scroll", active_id))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            egui_commonmark::CommonMarkViewer::new().show(ui, &mut self.md_cache, &text);
                        });
                });
        }
        if self.preview_editor {
            self.preview_editor_ui(ctx, ui);
            return;
        }
        let font = FontId::new(self.font_size * self.zoom / 100.0, editor_family());
        let line_height = if (self.line_height - 1.0).abs() < 0.01 {
            None
        } else {
            let base = ui.fonts_mut(|f| f.row_height(&font));
            Some(base * self.line_height)
        };
        let text_color = ui.visuals().text_color();
        let gutter_bg = self.palette().chrome;
        let spell_on = self.spell_enabled
            && self.spell.available()
            && self.docs[self.active].text.len() < 512 * 1024;
        let word_wrap = self.word_wrap;
        let ligatures = self.ligatures;
        let tab_size = self.tab_cols();
        let editor_id = self.editor_id();
        let doc_id = self.docs[self.active].id;
        let language = self.docs[self.active].language.clone();
        {
            let state = self.editor_states.entry(doc_id).or_default();
            match (&language, &state.hl) {
                (None, Some(_)) => state.hl = None,
                (Some(language), hl)
                    if hl
                        .as_ref()
                        .map(|cache| &cache.syntax != language)
                        .unwrap_or(true) =>
                {
                    state.hl = Some(crate::syntax::HlCache::new(language.clone()));
                }
                _ => {}
            }
        }
        // egui's TextEdit treats any pointer press as the start of a text
        // selection, including the secondary button. Keep the existing range
        // so opening our context menu does not collapse it before Cut/Copy.
        let selection_before_context_menu = ctx
            .input(|i| i.pointer.secondary_pressed())
            .then(|| {
                egui::text_edit::TextEditState::load(ctx, editor_id)
                    .and_then(|state| state.cursor.char_range())
            })
            .flatten();

        let dark = self.dark;
        let imported_syntax_theme = self
            .custom_theme
            .as_ref()
            .map(|theme| Arc::clone(&theme.syntax));
        let spell = &self.spell;
        let mut highlight = self
            .editor_states
            .get_mut(&doc_id)
            .and_then(|state| state.hl.as_mut());
        let mut layouter = move |ui: &egui::Ui,
                                 buf: &dyn egui::TextBuffer,
                                 wrap_width: f32|
              -> Arc<egui::Galley> {
            let text = buf.as_str();
            let mut job = LayoutJob::default();
            job.wrap.max_width = if word_wrap { wrap_width } else { f32::INFINITY };
            let normal = TextFormat {
                font_id: font.clone(),
                color: text_color,
                line_height,
                ..Default::default()
            };
            if let Some(cache) = highlight.as_deref_mut() {
                cache.sync_source(text);
                let lines: Vec<&str> = text.split('\n').collect();
                cache.ensure(
                    lines.len(),
                    lines.len(),
                    dark,
                    imported_syntax_theme.as_deref(),
                    |i| {
                        let mut line = String::with_capacity(lines[i].len() + 1);
                        line.push_str(lines[i]);
                        line.push('\n');
                        line
                    },
                );
                let active_spell = spell_on.then_some(spell);
                for (i, line) in lines.iter().enumerate() {
                    let spans = cache
                        .spans
                        .get(i)
                        .and_then(|spans| spans.as_deref())
                        .unwrap_or(&[]);
                    editor::append_highlighted_line(
                        &mut job,
                        line,
                        spans,
                        active_spell,
                        &normal,
                        Color32::from_rgb(232, 82, 82),
                    );
                    if i + 1 < lines.len() {
                        job.append("\n", 0.0, normal.clone());
                    }
                }
                if !ligatures {
                    editor::break_ligatures(&mut job);
                }
                editor::apply_tab_stops(&mut job, tab_size);
                return ui.fonts_mut(|f| f.layout_job(job));
            }
            if !spell_on {
                job.append(text, 0.0, normal);
            } else {
                let bad = TextFormat {
                    underline: egui::Stroke::new(1.5, Color32::from_rgb(232, 82, 82)),
                    ..normal.clone()
                };
                append_spellchecked(&mut job, text, spell, &normal, &bad);
            }
            if !ligatures {
                editor::break_ligatures(&mut job);
            }
            editor::apply_tab_stops(&mut job, tab_size);
            ui.fonts_mut(|f| f.layout_job(job))
        };

        let gutter_font = FontId::new(self.font_size * self.zoom / 100.0, editor_family());
        let show_ln = self.show_line_numbers;
        let doc = &mut self.docs[self.active];
        let line_count = doc.text.split('\n').count().max(1);
        let output = ui
            .horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let gutter = if show_ln {
                    let digits = line_count.to_string().len().max(2);
                    let char_w = ui.fonts_mut(|f| f.glyph_width(&gutter_font, '0'));
                    let w = char_w * digits as f32 + 14.0;
                    let left = ui.cursor().min.x;
                    ui.add_space(w);
                    Some((left, w))
                } else {
                    None
                };
                let output = egui::ScrollArea::both()
                    .id_salt(("editor_scroll", doc.id))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let output = egui::TextEdit::multiline(&mut doc.text)
                            .id(editor_id)
                            .frame(egui::Frame::NONE)
                            .desired_width(f32::INFINITY)
                            .desired_rows(30)
                            .lock_focus(true) // Tab inserts a tab character
                            .layouter(&mut layouter)
                            .show(ui);
                        if output.response.dragged_by(egui::PointerButton::Primary) {
                            editor::scroll_while_selecting(ui, word_wrap);
                        }
                        output
                    })
                    .inner;
                if let Some((gutter_left, gutter_w)) = gutter {
                    let painter = ui.painter();
                    let clip = ui.clip_rect();
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(gutter_left, clip.top()),
                            egui::pos2(gutter_left + gutter_w, clip.bottom()),
                        ),
                        0.0,
                        gutter_bg,
                    );
                    let color = ui.visuals().weak_text_color();
                    let x_right = gutter_left + gutter_w - 8.0;
                    let mut line_no = 1usize;
                    let mut starts_line = true;
                    for row in &output.galley.rows {
                        if starts_line {
                            let y = output.galley_pos.y + row.pos.y;
                            let h = row.row.size.y;
                            if y <= clip.bottom() && y + h >= clip.top() {
                                painter.text(
                                    egui::pos2(x_right, y),
                                    egui::Align2::RIGHT_TOP,
                                    line_no.to_string(),
                                    gutter_font.clone(),
                                    color,
                                );
                            } else if y > clip.bottom() {
                                break;
                            }
                            line_no += 1;
                        } else if row.pos.y + output.galley_pos.y > clip.bottom() {
                            break;
                        }
                        starts_line = row.ends_with_newline;
                    }
                }
                output
            })
            .inner;

        if output.response.changed() {
            self.text_changed_pending = true;
        }
        if let Some(range) = selection_before_context_menu {
            let mut state =
                egui::text_edit::TextEditState::load(ctx, editor_id).unwrap_or_default();
            state.cursor.set_char_range(Some(range));
            state.store(ctx, editor_id);
        }

        if self.focus_editor {
            output.response.request_focus();
            self.focus_editor = false;
        }

        // capture the word under a right-click for the context menu
        if output.response.secondary_clicked() {
            self.ctx_word = None;
            if let Some(pos) = output.response.interact_pointer_pos() {
                let rel = pos - output.galley_pos;
                let ccursor = output.galley.cursor_from_pos(rel);
                let idx = usize::from(ccursor.index);
                let text = &self.docs[self.active].text;
                if let Some((a, b, word)) = word_at(text, idx) {
                    if spell::checkable(&word, prev_char(text, a)) && !self.spell.check(&word) {
                        self.ctx_suggestions = self.spell.suggest(&word);
                        self.ctx_word = Some((a, b, word));
                    }
                }
            }
        }

        let mut replace_with: Option<(usize, usize, String)> = None;
        let mut add_word: Option<String> = None;
        let ctx_word = self.ctx_word.clone();
        let suggestions = self.ctx_suggestions.clone();
        output.response.context_menu(|ui| {
            ui.set_min_width(180.0);
            if let Some((a, b, word)) = &ctx_word {
                ui.label(
                    egui::RichText::new(format!("\"{word}\" not in dictionary"))
                        .italics()
                        .weak(),
                );
                if suggestions.is_empty() {
                    ui.label("(no suggestions)");
                } else {
                    for s in &suggestions {
                        if ui.button(s).clicked() {
                            replace_with = Some((*a, *b, s.clone()));
                            ui.close();
                        }
                    }
                }
                if ui.button("Add to dictionary").clicked() {
                    add_word = Some(word.clone());
                    ui.close();
                }
                ui.separator();
            }
            if ui.button("Cut").clicked() {
                self.pending_events.push(egui::Event::Cut);
                ui.close();
            }
            if ui.button("Copy").clicked() {
                self.pending_events.push(egui::Event::Copy);
                ui.close();
            }
            if ui.button("Paste").clicked() {
                if let Some(s) = clipboard_text() {
                    self.pending_events.push(egui::Event::Paste(s));
                }
                ui.close();
            }
            if ui.button("Select All").clicked() {
                let n = self.docs[self.active].text.chars().count();
                self.set_cursor(ui.ctx(), 0, n);
                ui.close();
            }
        });

        if let Some((a, b, s)) = replace_with {
            self.edit_text(ctx, (a, b), &s);
            self.ctx_word = None;
        }
        if let Some(w) = add_word {
            self.spell.add_word(&w);
            self.ctx_word = None;
        }
    }

    fn markdown_view(&mut self, ui: &mut egui::Ui) {
        // the editor isn't drawn this frame; don't leave focus on a widget
        // that no longer exists (accessibility clients reject that)
        let editor_id = self.editor_id();
        ui.memory_mut(|m| m.surrender_focus(editor_id));
        let mut back_to_editor = false;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Rendered Markdown")
                    .italics()
                    .weak()
                    .size(12.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("Edit")
                    .on_hover_text("Back to the raw editor (Ctrl+Shift+M)")
                    .clicked()
                {
                    back_to_editor = true;
                }
            });
        });
        ui.separator();
        let doc = &self.docs[self.active];
        let doc_id = doc.id;
        let text = doc.text.clone();
        egui::ScrollArea::vertical()
            .id_salt(("md_render", doc_id))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui_commonmark::CommonMarkViewer::new().show(ui, &mut self.md_cache, &text);
            });
        if back_to_editor {
            self.toggle_markdown_preview();
        }
    }

    /// The tab size as a whole number of columns.
    fn tab_cols(&self) -> usize {
        (self.tab_size.round() as usize).clamp(1, editor::MAX_TAB_SIZE)
    }

    /// Give the keyboard focus to the editor when the window becomes active,
    /// and when no other widget has the focus. The caret is then visible and
    /// the user can type at once, without a click in the text.
    fn keep_editor_focus(&mut self, ctx: &egui::Context) {
        let window_focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let gained = window_focused && !self.window_focused;
        self.window_focused = window_focused;
        if !window_focused || self.close_confirm.is_some() || self.reload_confirm.is_some() {
            return;
        }
        let nothing_focused = ctx.memory(|m| m.focused().is_none());
        let popup_open = egui::Popup::is_any_open(ctx);
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        if gained || (nothing_focused && !popup_open && !pointer_down) {
            self.focus_editor = true;
        }
    }

    fn preview_editor_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let font = FontId::new(self.font_size * self.zoom / 100.0, editor_family());
        let base_row = ui.fonts_mut(|f| f.row_height(&font));
        let row_h = base_row * self.line_height.max(0.5);
        let p = self.palette();
        let mut th = editor::theme_from_palette(&p, font, row_h, self.dark);
        th.ligatures = self.ligatures;
        th.tab_size = self.tab_cols();
        let spell_on = self.spell_enabled
            && self.spell.available()
            && self.docs[self.active].text.len() < 4 * 1024 * 1024;
        let editor_id = egui::Id::new(("editor", self.docs[self.active].id));
        let request_focus = self.focus_editor;
        self.focus_editor = false;
        let imported_syntax_theme = self
            .custom_theme
            .as_ref()
            .map(|theme| Arc::clone(&theme.syntax));

        // keep the highlight cache in step with the document's language
        {
            let lang = self.docs[self.active].language.clone();
            let id = self.docs[self.active].id;
            let state = self.editor_states.entry(id).or_default();
            match (&lang, &state.hl) {
                (None, Some(_)) => state.hl = None,
                (Some(l), hl) if hl.as_ref().map(|h| &h.syntax != l).unwrap_or(true) => {
                    state.hl = Some(crate::syntax::HlCache::new(l.clone()));
                }
                _ => {}
            }
        }

        let marks_doc = self.marks_doc;
        let doc = &mut self.docs[self.active];
        let state = self.editor_states.entry(doc.id).or_default();
        let spell = if spell_on { Some(&self.spell) } else { None };
        let marks: &[(usize, usize)] = if marks_doc == doc.id {
            &self.search_marks
        } else {
            &[]
        };
        let result = editor::show(
            ui,
            editor_id,
            &mut doc.text,
            doc.revision,
            state,
            &th,
            self.show_line_numbers,
            self.word_wrap,
            spell,
            marks,
            request_focus,
            imported_syntax_theme.as_deref(),
        );
        if result.changed {
            // char offsets are stale after edits
            self.search_marks.clear();
            self.text_changed_pending = true;
        }
        if let Some(word) = result.add_word {
            self.spell.add_word(&word);
        }
        let _ = ctx;
    }

    /// Replace a char range in the active document, routing through the
    /// preview editor's undo stack when it is active.
    fn edit_text(&mut self, ctx: &egui::Context, range: (usize, usize), s: &str) {
        self.text_changed_pending = true;
        if self.preview_editor {
            let now = ctx.input(|i| i.time);
            let doc = &mut self.docs[self.active];
            let state = self.editor_states.entry(doc.id).or_default();
            state.apply_edit(&mut doc.text, range, s, now);
            self.focus_editor = true;
            return;
        }
        let text = &mut self.docs[self.active].text;
        let sb = char_to_byte(text, range.0.min(range.1));
        let eb = char_to_byte(text, range.0.max(range.1));
        text.replace_range(sb..eb, s);
        self.touch_active();
        let after = range.0.min(range.1) + s.chars().count();
        self.set_cursor(ctx, after, after);
    }

    /// Bump the active document's revision after a text mutation that
    /// bypassed the preview editor (find/replace, plugins, suggestions).
    fn touch_active(&mut self) {
        let doc = &mut self.docs[self.active];
        doc.touch();
        if let Some(state) = self.editor_states.get_mut(&doc.id) {
            state.note_external_change();
        }
    }

    /// Turn typing, text changes and selection changes into (throttled)
    /// plugin events. Runs at the start of the frame, after last frame's
    /// edits have landed.
    fn queue_change_events(&mut self, ctx: &egui::Context) {
        if !self.pending_input.is_empty() {
            let text = std::mem::take(&mut self.pending_input);
            self.queued_plugin_events.push(("input".to_string(), Some(text)));
        }
        let focused = ctx.memory(|m| m.has_focus(self.editor_id()));
        let typed: String = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect()
        });
        if focused && !typed.is_empty() {
            self.pending_input.push_str(&typed);
        }
        let now = ctx.input(|i| i.time);
        let doc_id = self.docs[self.active].id;
        let sel = self.cursor_range(ctx).unwrap_or((0, 0));
        let fire_throttled = now - self.last_text_event > 0.25;
        if self.text_changed_pending && fire_throttled {
            self.text_changed_pending = false;
            self.last_text_event = now;
            self.queued_plugin_events.push(("text_changed".to_string(), None));
        }
        if self.last_selection != Some((doc_id, sel)) {
            let first = self.last_selection.is_none();
            self.last_selection = Some((doc_id, sel));
            if !first {
                self.queued_plugin_events.push(("selection_changed".to_string(), None));
            }
        }
        if self.text_changed_pending {
            ctx.request_repaint_after(std::time::Duration::from_millis(260));
        }
    }

    /// Apply finished background plugin work: new dictionaries, a new
    /// grammar set, changed feature scripts.
    fn poll_plugins(&mut self, ctx: &egui::Context) {
        let poll = self.pm.poll();
        if let Some((dicts, _)) = poll.spell {
            self.spell = Spell::with_dictionaries(dicts, self.pm.user_dictionary_path());
        }
        if poll.syntaxes_installed {
            // files opened before their file type was available
            for doc in &mut self.docs {
                if doc.language.is_none() {
                    if let Some(path) = &doc.path {
                        doc.language = crate::syntax::detect(path);
                    }
                }
            }
        }
        if poll.features_changed {
            self.plugin_host = PluginHost::load(&self.pm.feature_scripts());
            self.surface_plugin_errors();
        }
        if let Some(msg) = poll.messages.last() {
            self.flash(msg.clone());
        }
        if self.pm.is_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let status_items = self.status_items.clone();
        let doc = &self.docs[self.active];
        let install_hint = match (&doc.path, &doc.language) {
            (Some(path), None) => self
                .pm
                .suggestion_for(path)
                .map(|m| (m.id.clone(), m.name.clone())),
            _ => None,
        };
        let (line, col) = self
            .cursor_range(ui.ctx())
            .map(|(_, end)| line_col(&doc.text, end))
            .unwrap_or((1, 1));
        let chars = doc.text.chars().count();
        let cur_enc = doc.encoding;
        let cur_le = doc.line_ending;
        let cur_lang = doc.language.clone();
        let zoom = self.zoom;
        ui.horizontal(|ui| {
            if let Some((msg, _)) = &self.status_msg {
                ui.label(msg.clone());
            } else {
                ui.label(format!("Ln {line}, Col {col}"));
                if let Some(hint) = install_hint.clone() {
                    let (id, name) = hint;
                    if ui
                        .small_button(format!("Install {name} highlighting"))
                        .on_hover_text("Install the file type plugin for this file")
                        .clicked()
                    {
                        self.pm.install(&id);
                    }
                }
            }
            let mut new_enc: Option<Encoding> = None;
            let mut new_le: Option<LineEnding> = None;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(cur_enc.label(), |ui| {
                    for enc in [
                        Encoding::Utf8,
                        Encoding::Utf8Bom,
                        Encoding::Utf16Le,
                        Encoding::Utf16Be,
                        Encoding::Ansi,
                    ] {
                        if ui.radio(cur_enc == enc, enc.label()).clicked() {
                            new_enc = Some(enc);
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Encoding used when saving");
                ui.separator();
                ui.menu_button(cur_le.label(), |ui| {
                    for le in [LineEnding::Crlf, LineEnding::Lf] {
                        if ui.radio(cur_le == le, le.label()).clicked() {
                            new_le = Some(le);
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Line endings used when saving");
                ui.separator();
                let lang_label = cur_lang.clone().unwrap_or_else(|| "Plain Text".into());
                ui.menu_button(lang_label, |ui| {
                    self.language_picker_contents(ui);
                })
                .response
                .on_hover_text("Syntax highlighting language");
                ui.separator();
                ui.label(format!("{zoom:.0}%"));
                ui.separator();
                ui.label(format!("{chars} characters"));
                for (_, text, tooltip) in status_items.iter().rev() {
                    ui.separator();
                    let r = ui.label(text.as_str());
                    if !tooltip.is_empty() {
                        r.on_hover_text(tooltip.as_str());
                    }
                }
            });
            if let Some(enc) = new_enc {
                self.docs[self.active].encoding = enc;
            }
            if let Some(le) = new_le {
                self.docs[self.active].line_ending = le;
            }
        });
    }

    fn language_picker_contents(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(220.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.lang_filter).hint_text("Filter languages…"),
        );
        ui.separator();
        let current = self.docs[self.active].language.clone();
        let filter = self.lang_filter.to_lowercase();
        let mut selected: Option<Option<String>> = None;
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                if (filter.is_empty() || "plain text".contains(&filter))
                    && ui
                        .selectable_label(current.is_none(), "Plain Text")
                        .clicked()
                {
                    selected = Some(None);
                    ui.close();
                }
                for name in &crate::syntax::global().names {
                    if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                        continue;
                    }
                    if ui
                        .selectable_label(current.as_deref() == Some(name), name)
                        .clicked()
                    {
                        selected = Some(Some(name.clone()));
                        ui.close();
                    }
                }
            });
        ui.separator();
        if ui.button("More file types…").clicked() {
            self.pm.open_at(crate::packages::Kind::Filetype);
            ui.close();
        }
        if let Some(language) = selected {
            self.docs[self.active].language = language;
            self.lang_filter.clear();
        }
    }

    fn find_window(&mut self, ctx: &egui::Context) {
        if !self.find_open {
            return;
        }
        let mut open = self.find_open;
        let title = if self.show_replace { "Replace" } else { "Find" };
        egui::Window::new(title)
            .id(egui::Id::new("find_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos(ctx.content_rect().right_top() + egui::vec2(-320.0, 80.0))
            .show(ctx, |ui| {
                let mut do_next = false;
                let mut do_prev = false;
                egui::Grid::new("find_grid").num_columns(2).show(ui, |ui| {
                    ui.label("Find what:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.find_text).desired_width(200.0),
                    );
                    if self.focus_find {
                        resp.request_focus();
                        self.focus_find = false;
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        do_next = true;
                        resp.request_focus();
                    }
                    ui.end_row();
                    if self.show_replace {
                        ui.label("Replace with:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.replace_text)
                                .desired_width(200.0),
                        );
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.match_case, "Match case");
                    ui.checkbox(&mut self.wrap_around, "Wrap around");
                    if ui
                        .checkbox(&mut self.use_regex, "Regex")
                        .on_hover_text("Regular expression (supports lookaround and backreferences; $1 in Replace)")
                        .changed()
                    {
                        self.regex_error = None;
                    }
                });
                if let Some(err) = &self.regex_error {
                    ui.colored_label(
                        Color32::from_rgb(232, 82, 82),
                        format!("Pattern error: {err}"),
                    );
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Find Next").clicked() {
                        do_next = true;
                    }
                    if ui.button("Find Previous").clicked() {
                        do_prev = true;
                    }
                    if self.show_replace {
                        if ui.button("Replace").clicked() {
                            self.replace_current(ctx);
                        }
                        if ui.button("Replace All").clicked() {
                            self.replace_all(ctx);
                        }
                    } else if ui.small_button("Replace…").clicked() {
                        self.show_replace = true;
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Mark All").clicked() {
                        self.mark_all(ctx);
                    }
                    if !self.search_marks.is_empty() {
                        if ui.button("Clear Marks").clicked() {
                            self.search_marks.clear();
                        }
                    }
                });
                if do_next {
                    self.find_next(ctx, false);
                }
                if do_prev {
                    self.find_next(ctx, true);
                }
            });
        self.find_open = open;
    }

    fn goto_window(&mut self, ctx: &egui::Context) {
        if !self.goto_open {
            return;
        }
        let mut open = self.goto_open;
        egui::Window::new("Go To Line")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Line number:");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.goto_line).desired_width(80.0),
                    );
                    resp.request_focus();
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        self.goto_line(ctx);
                    }
                });
                if ui.button("Go To").clicked() {
                    self.goto_line(ctx);
                }
            });
        self.goto_open = open && self.goto_open;
    }

    fn theme_url_window(&mut self, ctx: &egui::Context) {
        if !self.theme_url_open {
            return;
        }
        let mut open = self.theme_url_open;
        let downloading = self.theme_download_rx.is_some();
        let mut start_import = false;
        egui::Window::new("Get VS Code Theme")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label("Copy a theme URL from vscodethemes.com and paste it below.");
                ui.hyperlink_to("Browse vscodethemes.com", "https://vscodethemes.com/");
                ui.add_space(6.0);
                let response = ui.add_enabled(
                    !downloading,
                    egui::TextEdit::singleline(&mut self.theme_url)
                        .hint_text("https://vscodethemes.com/e/publisher.extension/theme")
                        .desired_width(f32::INFINITY),
                );
                if let Some(error) = &self.theme_url_error {
                    ui.colored_label(egui::Color32::from_rgb(0xF1, 0x70, 0x7B), error);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !downloading && !self.theme_url.trim().is_empty(),
                            egui::Button::new(if downloading { "Downloading…" } else { "Import" }),
                        )
                        .clicked()
                    {
                        start_import = true;
                    }
                    if downloading {
                        ui.spinner();
                        ui.label("Downloading the Marketplace extension…");
                    }
                });
                if response.lost_focus()
                    && ui.input(|input| input.key_pressed(Key::Enter))
                    && !downloading
                    && !self.theme_url.trim().is_empty()
                {
                    start_import = true;
                }
            });
        self.theme_url_open = open;
        if start_import {
            self.start_theme_url_import();
        }
    }

    fn prefs_window(&mut self, ctx: &egui::Context) {
        if !self.prefs_open {
            return;
        }
        let mut open = self.prefs_open;
        let fonts: Vec<(String, PathBuf)> = self.ensure_font_list().to_vec();
        let mut font_changed: Option<PathBuf> = None;
        let mut tab_size_changed = false;
        egui::Window::new("Preferences")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                egui::Grid::new("prefs_grid")
                    .num_columns(2)
                    .spacing([12.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Font:");
                        egui::ComboBox::from_id_salt("font_picker")
                            .selected_text(self.editor_font.clone())
                            .width(200.0)
                            .show_ui(ui, |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.font_filter)
                                        .hint_text("Filter…"),
                                );
                                ui.separator();
                                let filter = self.font_filter.to_lowercase();
                                for (name, path) in fonts.iter().filter(|(n, _)| {
                                    filter.is_empty() || n.to_lowercase().contains(&filter)
                                }) {
                                    if ui
                                        .selectable_label(self.editor_font == *name, name)
                                        .clicked()
                                    {
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

                        ui.label("Line numbers:");
                        ui.checkbox(&mut self.show_line_numbers, "Show line numbers");
                        ui.end_row();

                        ui.label("Word wrap:");
                        ui.checkbox(&mut self.word_wrap, "Wrap long lines");
                        ui.end_row();

                        ui.label("Ligatures:");
                        ui.checkbox(&mut self.ligatures, "Use font ligatures")
                            .on_hover_text(
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

                        ui.label("Menu bar:");
                        ui.checkbox(&mut self.show_menu, "Always show menu bar")
                            .on_hover_text(
                                "When off, the menus open from the Notey icon in the title bar",
                            );
                        ui.end_row();

                        ui.label("Editor:");
                        ui.checkbox(&mut self.preview_editor, "Virtualized editor")
                            .on_hover_text(
                                "Lays out only visible lines, so large files stay fast, and \
                                 adds multi-cursor and column editing. Turn off to use the \
                                 classic editor, which shows inline IME composition.",
                            );
                        ui.end_row();
                    });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Reset to defaults").clicked() {
                        self.font_size = 15.0;
                        self.line_height = 1.0;
                        if (self.tab_size - 4.0).abs() > f32::EPSILON {
                            self.tab_size = 4.0;
                            tab_size_changed = true;
                        }
                        self.show_line_numbers = false;
                        self.ligatures = true;
                        self.word_wrap = true;
                        self.show_menu = false;
                        self.autosave = false;
                        self.autosave_secs = 30.0;
                        if self.editor_font != "Consolas" {
                            self.editor_font = "Consolas".to_string();
                            font_changed = fonts
                                .iter()
                                .find(|(n, _)| n == "Consolas")
                                .map(|(_, p)| p.clone());
                        }
                    }
                });
            });
        if let Some(path) = font_changed {
            rebuild_fonts(ctx, Some(&path), self.tab_size);
        } else if tab_size_changed {
            let name = self.editor_font.clone();
            let path = self.font_path(&name);
            rebuild_fonts(ctx, path.as_deref(), self.tab_size);
        }
        self.prefs_open = open;
    }

    fn plugin_alert_windows(&mut self, ctx: &egui::Context) {
        let mut close: Option<usize> = None;
        for (i, (title, body)) in self.plugin_alerts.iter().enumerate() {
            let mut open = true;
            egui::Window::new(title.clone())
                .id(egui::Id::new(("plugin_alert", i)))
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(body.clone());
                    ui.add_space(6.0);
                    if ui.button("OK").clicked() {
                        close = Some(i);
                    }
                });
            if !open {
                close = Some(i);
            }
        }
        if let Some(i) = close {
            self.plugin_alerts.remove(i);
        }
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let mut open = self.about_open;
        egui::Window::new("About Notey")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Notey");
                ui.label("A native Rust notepad with dark mode, tabs, and spellcheck.");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.add_space(6.0);
                ui.label("Built with egui / eframe and spellbook (Hunspell).");
            });
        self.about_open = open;
    }

    fn reload_modal(&mut self, ctx: &egui::Context) {
        let Some(id) = self.reload_confirm else { return };
        let Some(idx) = self.docs.iter().position(|d| d.id == id) else {
            self.reload_confirm = None;
            return;
        };
        let title = self.docs[idx].title();
        let modified = self.docs[idx].modified();
        let mut action: Option<bool> = None; // Some(true) = reload
        egui::Modal::new(egui::Id::new("reload_confirm")).show(ctx, |ui| {
            ui.set_min_width(340.0);
            ui.heading("File changed on disk");
            ui.add_space(4.0);
            ui.label(format!(
                "\"{title}\" was modified by another program."
            ));
            if modified {
                ui.colored_label(
                    Color32::from_rgb(232, 82, 82),
                    "Reloading will discard your unsaved changes in this tab.",
                );
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Reload").clicked() {
                    action = Some(true);
                }
                if ui.button("Keep This Version").clicked() {
                    action = Some(false);
                }
            });
        });
        match action {
            Some(true) => {
                match self.docs[idx].reload() {
                    Ok(()) => {
                        let doc_id = self.docs[idx].id;
                        if let Some(state) = self.editor_states.get_mut(&doc_id) {
                            state.note_external_change();
                        }
                        self.flash(format!("Reloaded {title}"));
                    }
                    Err(e) => self.flash(format!("Reload failed: {e}")),
                }
                self.reload_confirm = None;
            }
            Some(false) => {
                // refresh the known mtime so we don't re-prompt for this write
                if let Some(path) = self.docs[idx].path.clone() {
                    self.docs[idx].disk_mtime =
                        std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                }
                self.reload_confirm = None;
            }
            None => {}
        }
    }

    fn close_confirm_modal(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.close_confirm else { return };
        let is_app = matches!(pending, PendingClose::App);
        let (title_text, body) = match pending {
            PendingClose::Tab(i) => {
                let t = self.docs.get(*i).map(|d| d.title()).unwrap_or_default();
                (
                    "Unsaved changes".to_string(),
                    format!("Do you want to save changes to {t}?"),
                )
            }
            PendingClose::App => {
                let n = self.docs.iter().filter(|d| d.modified()).count();
                (
                    "Unsaved changes".to_string(),
                    format!("You have unsaved changes in {n} tab(s). Save before exiting?"),
                )
            }
        };
        let mut action: Option<&str> = None;
        egui::Modal::new(egui::Id::new("close_confirm")).show(ctx, |ui| {
            ui.set_min_width(320.0);
            ui.heading(title_text);
            ui.add_space(4.0);
            ui.label(body);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let save_label = if is_app { "Save All" } else { "Save" };
                if ui.button(save_label).clicked() {
                    action = Some("save");
                }
                if ui.button("Don't Save").clicked() {
                    action = Some("discard");
                }
                if ui.button("Cancel").clicked() {
                    action = Some("cancel");
                }
            });
        });
        match action {
            Some("save") => match self.close_confirm.take() {
                Some(PendingClose::Tab(i)) => {
                    if self.save_doc(ctx, i) {
                        self.close_tab(i);
                    }
                }
                Some(PendingClose::App) => {
                    let mut ok = true;
                    for i in 0..self.docs.len() {
                        if self.docs[i].modified() && !self.save_doc(ctx, i) {
                            ok = false;
                            break;
                        }
                    }
                    if ok {
                        self.allow_quit = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                None => {}
            },
            Some("discard") => match self.close_confirm.take() {
                Some(PendingClose::Tab(i)) => self.close_tab(i),
                Some(PendingClose::App) => {
                    self.allow_quit = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                None => {}
            },
            Some("cancel") => {
                self.close_confirm = None;
            }
            _ => {}
        }
    }
}

impl eframe::App for NoteyApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        self.poll_theme_url_import(ctx);
        // inject queued events (menu-driven undo/cut/copy/paste/…) so the
        // focused TextEdit picks them up this frame
        if !self.pending_events.is_empty() {
            let events = std::mem::take(&mut self.pending_events);
            self.focus_drawn_editor(ctx);
            ctx.input_mut(|i| i.events.extend(events));
        }

        // window close (X button / Alt+F4 / File > Exit).
        // The primary window keeps unsaved tabs in the session (restored on
        // next launch), so it closes without prompting. Secondary windows
        // have no session, so they still ask.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_quit && !self.is_primary
        {
            if self.docs.iter().any(|d| d.modified()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.close_confirm = Some(PendingClose::App);
            }
        }

        // files dragged onto the window
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        for path in dropped {
            self.open_path(&path);
        }

        // files forwarded from other notey.exe launches ("open in existing window")
        let forwarded: Vec<PathBuf> = {
            let mut q = self.inbox.lock().unwrap();
            std::mem::take(&mut *q)
        };
        if !forwarded.is_empty() {
            for path in &forwarded {
                self.open_path(path);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        if self.close_confirm.is_none() {
            self.handle_plugin_keys(ctx);
            self.handle_plugin_shortcuts(ctx);
            self.handle_shortcuts(ctx);
        }

        // ---- plugin events & queued commands ----
        if !self.ready_fired {
            self.ready_fired = true;
            self.last_active_doc = self.docs[self.active].id;
            self.fire_event(ctx, "ready", None);
        }
        let cur_id = self.docs[self.active].id;
        if cur_id != self.last_active_doc {
            self.last_active_doc = cur_id;
            let path = self.docs[self.active]
                .path
                .as_ref()
                .map(|p| p.display().to_string());
            self.fire_event(ctx, "buffer_activated", path);
        }
        self.poll_plugins(ctx);
        self.queue_change_events(ctx);
        let queued = std::mem::take(&mut self.queued_plugin_events);
        for (kind, path) in queued {
            self.fire_event(ctx, &kind, path);
        }
        let cmds = std::mem::take(&mut self.pending_plugin_cmds);
        for (i, id) in cmds {
            self.run_plugin_command(ctx, i, &id);
        }

        // ---- disk change detection + autosave ----
        let now_t = ctx.input(|i| i.time);
        if now_t - self.last_disk_check > 3.0 {
            self.last_disk_check = now_t;
            if self.reload_confirm.is_none() {
                let mut deleted: Option<String> = None;
                for doc in &mut self.docs {
                    let Some(path) = doc.path.clone() else { continue };
                    let Some(known) = doc.disk_mtime else { continue };
                    match std::fs::metadata(&path).and_then(|m| m.modified()) {
                        Ok(m) if m > known => {
                            self.reload_confirm = Some(doc.id);
                            break;
                        }
                        Ok(_) => {}
                        Err(_) => {
                            doc.disk_mtime = None;
                            deleted = Some(doc.title());
                        }
                    }
                }
                if let Some(t) = deleted {
                    self.flash(format!("File on disk was deleted: {t}"));
                }
            }
        }
        if self.autosave {
            if now_t - self.last_autosave > self.autosave_secs.max(5.0) as f64 {
                self.last_autosave = now_t;
                let mut n = 0usize;
                for doc in &mut self.docs {
                    if doc.path.is_some() && doc.modified() && doc.save().is_ok() {
                        n += 1;
                    }
                }
                if n > 0 {
                    self.flash(format!("Autosaved {n} file(s)"));
                }
            }
        }
        if self.autosave || self.docs.iter().any(|d| d.disk_mtime.is_some()) {
            ctx.request_repaint_after(std::time::Duration::from_secs(3));
        }

        // drain incoming Find-in-Files results
        if let Some(rx) = &self.fif_rx {
            let mut disconnected = false;
            loop {
                match rx.try_recv() {
                    Ok(m) => self.fif_results.push(m),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
            if disconnected {
                self.fif_rx = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        let p = self.palette();
        egui::Panel::top(egui::Id::new("titlebar"))
            .resizable(false)
            .exact_size(40.0)
            .frame(egui::Frame::new().fill(p.chrome))
            .show(root, |ui| {
                self.title_bar(ctx, ui);
            });
        if self.show_menu {
            egui::Panel::top(egui::Id::new("menu"))
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(p.chrome)
                        .inner_margin(egui::Margin::symmetric(8, 2)),
                )
                .show(root, |ui| {
                    self.menu_bar(ctx, ui);
                });
        }
        if self.show_status {
            egui::Panel::bottom(egui::Id::new("status"))
                .resizable(false)
                .frame(
                    egui::Frame::new()
                        .fill(p.chrome)
                        .inner_margin(egui::Margin::symmetric(10, 4)),
                )
                .show(root, |ui| {
                    self.status_bar(ui);
                });
        }
        self.fif_results_panel(root, ctx);
        let central_frame = egui::Frame::new().fill(p.editor).inner_margin(egui::Margin {
            left: 8,
            right: 2,
            top: 6,
            bottom: 2,
        });
        egui::CentralPanel::default()
            .frame(central_frame)
            .show(root, |ui| {
                self.editor(ctx, ui);
            });

        self.handle_resize_edges(ctx);

        self.find_window(ctx);
        self.goto_window(ctx);
        self.theme_url_window(ctx);
        self.prefs_window(ctx);
        self.about_window(ctx);
        self.fif_window(ctx);
        self.pm.window(ctx);
        self.plugin_alert_windows(ctx);
        self.reload_modal(ctx);
        self.close_confirm_modal(ctx);

        // status flash timeout
        if let Some((_, shown_at)) = &mut self.status_msg {
            let now = ctx.input(|i| i.time);
            if *shown_at == 0.0 {
                *shown_at = now;
            } else if now - *shown_at > 3.0 {
                self.status_msg = None;
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(300));
        }

        // window title
        let doc = &self.docs[self.active];
        let title = format!(
            "{}{} - Notey",
            if doc.modified() { "*" } else { "" },
            doc.title()
        );
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let b = |v: bool| if v { "1" } else { "0" }.to_string();
        storage.set_string("dark", b(self.dark));
        storage.set_string(
            "theme_path",
            self.custom_theme
                .as_ref()
                .map(|theme| theme.path.display().to_string())
                .unwrap_or_default(),
        );
        storage.set_string("word_wrap", b(self.word_wrap));
        storage.set_string("show_status", b(self.show_status));
        storage.set_string("spell", b(self.spell_enabled));
        storage.set_string("line_numbers", b(self.show_line_numbers));
        storage.set_string("ligatures", b(self.ligatures));
        storage.set_string("show_menu", b(self.show_menu));
        storage.set_string("editor_virtualized", b(self.preview_editor));
        storage.set_string("font_size", self.font_size.to_string());
        storage.set_string("zoom", self.zoom.to_string());
        storage.set_string("line_height", self.line_height.to_string());
        storage.set_string("tab_size", self.tab_size.to_string());
        storage.set_string("editor_font", self.editor_font.clone());
        storage.set_string("autosave", b(self.autosave));
        storage.set_string("autosave_secs", self.autosave_secs.to_string());
        let recent: Vec<String> = self
            .recent_files
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        storage.set_string(
            "recent_files",
            serde_json::to_string(&recent).unwrap_or_default(),
        );
        // periodic + on-exit session snapshot (crash-safe unsaved tabs)
        if self.is_primary {
            session::save(&self.session_snapshot());
        }
    }
}

// ---------- helpers ----------

fn menu_item(ui: &mut egui::Ui, label: &str, shortcut: &str) -> bool {
    ui.add(egui::Button::new(label).shortcut_text(shortcut))
        .clicked()
}

fn key_event(key: Key, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// Build the full font setup: UI fonts (Consolas / Segoe UI) plus a dedicated
/// "editor" family pointing at the user's chosen font, with the monospace
/// stack as glyph fallback.
fn rebuild_fonts(ctx: &egui::Context, editor_font: Option<&std::path::Path>, tab_size: f32) {
    let mut fonts = egui::FontDefinitions::default();
    // prefer Segoe UI Variable (Win11) for UI text, fall back to Segoe UI
    let ui_font = if std::path::Path::new("C:/Windows/Fonts/SegUIVar.ttf").exists() {
        "SegUIVar.ttf"
    } else {
        "segoeui.ttf"
    };
    for (name, file, family) in [
        ("Consolas", "consola.ttf", FontFamily::Monospace),
        ("Segoe UI", ui_font, FontFamily::Proportional),
    ] {
        if let Some(bytes) = font_bytes(std::path::Path::new(&format!("C:/Windows/Fonts/{file}"))) {
            fonts
                .font_data
                .insert(name.to_string(), Arc::new(egui::FontData::from_static(bytes)));
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, name.to_string());
        }
    }

    let mut editor_stack: Vec<String> = Vec::new();
    if let Some(path) = editor_font {
        if let Some(bytes) = font_bytes(path) {
            fonts.font_data.insert(
                EDITOR_FONT_KEY.to_string(),
                Arc::new(egui::FontData::from_static(bytes)),
            );
            editor_stack.push(EDITOR_FONT_KEY.to_string());
        }
    }
    // fall back to the monospace stack for glyphs the chosen font lacks
    if let Some(mono) = fonts.families.get(&FontFamily::Monospace) {
        for name in mono {
            if !editor_stack.contains(name) {
                editor_stack.push(name.clone());
            }
        }
    }

    // apply the configured tab width to every font so it's consistent
    let tab_size = tab_size.round().clamp(1.0, editor::MAX_TAB_SIZE as f32);
    for fd in fonts.font_data.values_mut() {
        if (fd.tweak.tab_size - tab_size).abs() > f32::EPSILON {
            let mut f = (**fd).clone();
            f.tweak.tab_size = tab_size;
            *fd = Arc::new(f);
        }
    }

    // One copy of the editor stack for each tab width, so that a tab can go
    // to the next tab stop (see `editor::apply_tab_stops`). The font bytes
    // are static, thus each copy is small.
    for cols in 1..=editor::MAX_TAB_SIZE {
        let mut stack = Vec::with_capacity(editor_stack.len());
        for name in &editor_stack {
            let Some(fd) = fonts.font_data.get(name) else { continue };
            let mut f = (**fd).clone();
            f.tweak.tab_size = cols as f32;
            let key = format!("{name}#tab{cols}");
            fonts.font_data.insert(key.clone(), Arc::new(f));
            stack.push(key);
        }
        fonts.families.insert(editor::tab_family(cols), stack);
    }
    fonts.families.insert(editor_family(), editor_stack);
    ctx.set_fonts(fonts);
}

/// Font file bytes that live for the whole run. Each file is read one time,
/// so the tab-width copies of a font and later font changes share the bytes.
fn font_bytes(path: &std::path::Path) -> Option<&'static [u8]> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, &'static [u8]>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().ok()?;
    if let Some(bytes) = cache.get(path) {
        return Some(bytes);
    }
    let bytes: &'static [u8] = Box::leak(std::fs::read(path).ok()?.into_boxed_slice());
    cache.insert(path.to_path_buf(), bytes);
    Some(bytes)
}

/// Installed fonts from the Windows registry: display name -> font file path.
fn system_fonts() -> Vec<(String, PathBuf)> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let fonts_dir = PathBuf::from(
        std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into()),
    )
    .join("Fonts");

    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let Ok(key) = RegKey::predef(hive)
            .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts")
        else {
            continue;
        };
        for item in key.enum_values() {
            let Ok((name, _)) = item else { continue };
            let Ok(file) = key.get_value::<String, _>(&name) else { continue };
            let lower = file.to_lowercase();
            if !(lower.ends_with(".ttf") || lower.ends_with(".otf") || lower.ends_with(".ttc"))
            {
                continue;
            }
            let path = if file.contains('\\') || file.contains('/') {
                PathBuf::from(&file)
            } else {
                fonts_dir.join(&file)
            };
            // strip " (TrueType)" style suffixes
            let clean = name
                .rsplit_once(" (")
                .map(|(a, _)| a.to_string())
                .unwrap_or(name);
            out.push((clean, path));
        }
    }
    out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}


fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

fn line_col(text: &str, char_idx: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in text.chars().enumerate() {
        if i == char_idx {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn prev_char(text: &str, char_idx: usize) -> Option<char> {
    if char_idx == 0 {
        return None;
    }
    text.chars().nth(char_idx - 1)
}

/// Word (alphabetic run) containing char index `idx`; returns char range + word.
fn word_at(text: &str, idx: usize) -> Option<(usize, usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let i = idx.min(chars.len().saturating_sub(1));
    let probe = if spell::is_word_char(chars[i]) {
        i
    } else if i > 0 && spell::is_word_char(chars[i - 1]) {
        i - 1
    } else {
        return None;
    };
    let mut a = probe;
    while a > 0 && spell::is_word_char(chars[a - 1]) {
        a -= 1;
    }
    let mut b = probe + 1;
    while b < chars.len() && spell::is_word_char(chars[b]) {
        b += 1;
    }
    let word: String = chars[a..b].iter().collect();
    Some((a, b, word))
}

/// Append `text` to `job`, marking misspelled words with the `bad` format.
fn append_spellchecked(
    job: &mut LayoutJob,
    text: &str,
    spell: &Spell,
    normal: &TextFormat,
    bad: &TextFormat,
) {
    let mut chunk_start = 0usize; // byte index of pending normal text
    let mut prev: Option<char> = None;
    let mut iter = text.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        if spell::is_word_char(c) {
            // scan to end of word
            let start = i;
            let mut end = i + c.len_utf8();
            while let Some(&(j, cj)) = iter.peek() {
                if spell::is_word_char(cj) {
                    end = j + cj.len_utf8();
                    iter.next();
                } else {
                    break;
                }
            }
            let word = &text[start..end];
            let misspelled = spell::checkable(word, prev) && !spell.check(word);
            if misspelled {
                if chunk_start < start {
                    job.append(&text[chunk_start..start], 0.0, normal.clone());
                }
                job.append(word, 0.0, bad.clone());
                chunk_start = end;
            }
            prev = word.chars().last();
        } else {
            prev = Some(c);
        }
    }
    if chunk_start < text.len() {
        job.append(&text[chunk_start..], 0.0, normal.clone());
    }
}
