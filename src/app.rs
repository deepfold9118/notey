//! The Notey window: a title bar with the tabs, an optional menu bar, the
//! editor (or the Settings tab), and the status bar. Each tab owns a
//! CodeMirror [`EditorView`]; the editor state holds the text and the
//! [`Document`] holds the file it belongs to.

mod chrome;
mod find;
mod settings_tab;
mod spelling;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use codemirror_commands::{
    default_keymap, history, history_keymap, indent_less, insert_tab, redo, select_all, undo, HistoryConfig,
};
use codemirror_state::{
    Compartment, EditorState, EditorStateConfig, Extension, TransactionSpec, ALLOW_MULTIPLE_SELECTIONS, TAB_SIZE,
};
use codemirror_search::{search, search_keymap, search_panel_open, SearchConfig};
use codemirror_view::{keymap, line_numbers, line_wrapping, EditorView, KeyBinding, THEME, UPDATE_LISTENER};
use futures::channel::mpsc::UnboundedReceiver;
use futures::StreamExt;
use gpui::{
    actions, div, prelude::*, px, App, Context, Entity, ExternalPaths, FocusHandle, KeyBinding as GpuiKeyBinding,
    PathPromptOptions, PromptLevel, SharedString, Subscription, Task, Window,
};

use crate::doc::Document;
use crate::ipc::clean_path;
use crate::session::{self, Session, SessionTab};
use crate::settings::Settings;
use crate::spellcheck::{spellcheck, SpellHandle};
use crate::theme::{editor_theme, palette, Palette};

use chrome::Menu;

actions!(
    notey,
    [
        NewTab,
        NewWindow,
        Open,
        Save,
        SaveAs,
        CloseTab,
        NextTab,
        PrevTab,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        InsertTimeDate,
        OpenSettings,
        Exit,
        Undo,
        Redo,
        Cut,
        Copy,
        Paste,
        Delete,
        SelectAll,
        About,
    ]
);

/// The font for menus, tabs and the status bar.
const UI_FONT: &str = "Segoe UI";
/// How often the primary window writes the session, so that a crash loses
/// at most this much unsaved text.
const SESSION_INTERVAL: Duration = Duration::from_secs(30);
/// How long a status bar message stays.
const FLASH_TIME: Duration = Duration::from_secs(4);

/// Register the application's key bindings. The editor handles its own
/// keys; these are the window's.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        GpuiKeyBinding::new("ctrl-n", NewTab, None),
        GpuiKeyBinding::new("ctrl-shift-n", NewWindow, None),
        GpuiKeyBinding::new("ctrl-o", Open, None),
        GpuiKeyBinding::new("ctrl-s", Save, None),
        GpuiKeyBinding::new("ctrl-shift-s", SaveAs, None),
        GpuiKeyBinding::new("ctrl-w", CloseTab, None),
        GpuiKeyBinding::new("ctrl-tab", NextTab, None),
        GpuiKeyBinding::new("ctrl-shift-tab", PrevTab, None),
        GpuiKeyBinding::new("ctrl-=", ZoomIn, None),
        GpuiKeyBinding::new("ctrl-+", ZoomIn, None),
        GpuiKeyBinding::new("ctrl--", ZoomOut, None),
        GpuiKeyBinding::new("ctrl-0", ZoomReset, None),
        GpuiKeyBinding::new("f5", InsertTimeDate, None),
        GpuiKeyBinding::new("ctrl-,", OpenSettings, None),
    ]);
    cx.bind_keys(find::bindings());
}

struct Tab {
    doc: Document,
    view: Entity<EditorView>,
    /// The editor state at the last check, to notice changes.
    seen: EditorState,
    /// Set by the editor's update listener when the text changes.
    doc_changed: Arc<AtomicBool>,
    /// Counts text changes.
    version: u64,
    modified: bool,
    chars: usize,
    line: usize,
    col: usize,
    _observe: Subscription,
}

impl Tab {
    fn state<'a>(&self, cx: &'a App) -> &'a EditorState {
        self.view.read(cx).state()
    }

    /// Recompute what the tab strip and status bar show.
    fn refresh(&mut self, state: EditorState, doc_changed: bool) {
        if doc_changed {
            self.modified = self.doc.modified(state.doc());
            self.chars = state.doc().chunks().map(|c| c.chars().count()).sum();
        }
        let head = state.selection().main().head();
        let line = state.doc().line_at(head);
        self.line = line.number;
        self.col = line.text[..head - line.from].chars().count() + 1;
        self.seen = state;
    }
}

pub struct NoteyApp {
    tabs: Vec<Tab>,
    active: usize,
    next_doc_id: u64,
    next_untitled: usize,
    settings: Settings,
    palette: Palette,
    /// The editor configuration that follows the settings (theme, wrapping,
    /// line numbers, tab size), the same compartment in every editor.
    config: Compartment,
    /// The Settings tab is in the tab strip.
    settings_open: bool,
    /// The Settings tab is the visible tab. `active` still points to a
    /// document.
    settings_active: bool,
    open_menu: Option<Menu>,
    find: find::FindBar,
    status_msg: Option<SharedString>,
    flash_task: Option<Task<()>>,
    /// Owns the single-instance socket and the saved session. Secondary
    /// windows neither restore nor overwrite the session.
    is_primary: bool,
    session_dirty: bool,
    /// A secondary window with unsaved tabs may close (the user agreed).
    allow_close: bool,
    /// The spell checker, once the dictionaries have loaded.
    spell: Option<SpellHandle>,
    context_menu: Option<spelling::ContextMenu>,
    icon_font: SharedString,
    focus_handle: FocusHandle,
    last_title: String,
    _tasks: Vec<Task<()>>,
}

impl NoteyApp {
    pub fn new(
        paths: Vec<PathBuf>,
        is_primary: bool,
        mut inbox: UnboundedReceiver<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = Settings::load();
        let fonts = cx.text_system().all_font_names();
        let icon_font = if fonts.iter().any(|f| f == "Segoe Fluent Icons") {
            "Segoe Fluent Icons"
        } else {
            "Segoe MDL2 Assets"
        };
        let find = find::FindBar::new(&palette(settings.dark), cx);
        let mut app = NoteyApp {
            tabs: Vec::new(),
            active: 0,
            next_doc_id: 1,
            next_untitled: 1,
            palette: palette(settings.dark),
            settings,
            config: Compartment::new(),
            settings_open: false,
            settings_active: false,
            open_menu: None,
            find,
            status_msg: None,
            flash_task: None,
            is_primary,
            session_dirty: false,
            allow_close: false,
            spell: None,
            context_menu: None,
            icon_font: icon_font.into(),
            focus_handle: cx.focus_handle(),
            last_title: String::new(),
            _tasks: Vec::new(),
        };
        if is_primary {
            app.restore_session(cx);
        }
        for path in &paths {
            app.open_path(path, window, cx);
        }
        if app.tabs.is_empty() {
            app.new_tab(window, cx);
        }

        // Files that a second `notey.exe` forwards.
        app._tasks.push(cx.spawn_in(window, async move |this, cx| {
            while let Some(path) = inbox.next().await {
                if this.update_in(cx, |this, window, cx| this.open_path(&path, window, cx)).is_err() {
                    break;
                }
            }
        }));
        if is_primary {
            app._tasks.push(cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(SESSION_INTERVAL).await;
                let alive = this.update(cx, |this, cx| {
                    if this.session_dirty {
                        this.save_session(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }));
        }
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| this.should_close(window, cx)).unwrap_or(true)
        });
        app.start_spell_load(cx);
        app.focus_active(window, cx);
        app
    }

    // ---------- tabs ----------

    fn editor_config(&self) -> Extension {
        let mut exts = vec![
            THEME.of(Arc::new(editor_theme(&self.palette, &self.settings))),
            TAB_SIZE.of(self.settings.tab_size),
            search(Some(SearchConfig {
                match_background: Some(self.palette.find_match),
                selected_match_background: Some(self.palette.find_match_selected),
                ..Default::default()
            })),
        ];
        if self.settings.word_wrap {
            exts.push(line_wrapping());
        }
        if self.settings.line_numbers {
            exts.push(line_numbers());
        }
        if let (true, Some(spell)) = (self.settings.spellcheck, &self.spell) {
            exts.push(spellcheck(spell.clone()));
        }
        Extension::list(exts)
    }

    fn add_tab(&mut self, doc: Document, text: String, cx: &mut Context<Self>) -> usize {
        let mut bindings = default_keymap();
        bindings.extend(history_keymap());
        // Tab inserts a tab, or indents a selection; Shift-Tab dedents.
        bindings.push(KeyBinding::new("Tab").run_state(insert_tab).shift_state(indent_less));
        bindings.extend(search_keymap());
        let doc_changed = Arc::new(AtomicBool::new(false));
        let flag = doc_changed.clone();
        let extensions = Extension::list([
            history(HistoryConfig::default()),
            keymap(bindings),
            ALLOW_MULTIPLE_SELECTIONS.of(true),
            UPDATE_LISTENER.of(Arc::new(move |update: &codemirror_view::ViewUpdate| {
                if update.doc_changed() {
                    flag.store(true, Ordering::Relaxed);
                }
            })),
            self.config.of(self.editor_config()),
        ]);
        let state = EditorState::create(EditorStateConfig::new().doc(text).extensions(extensions));
        let view = cx.new(|cx| EditorView::new(state.clone(), cx));
        let observe = cx.observe(&view, |this, view, cx| this.editor_changed(&view, cx));
        let mut tab = Tab {
            doc,
            view,
            seen: state.clone(),
            doc_changed,
            version: 0,
            modified: false,
            chars: 0,
            line: 1,
            col: 1,
            _observe: observe,
        };
        tab.refresh(state, true);
        self.tabs.push(tab);
        self.session_dirty = true;
        self.tabs.len() - 1
    }

    /// The editor notified: if its state changed, update the tab's info.
    fn editor_changed(&mut self, view: &Entity<EditorView>, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.view == *view) else { return };
        let state = view.read(cx).state().clone();
        if state.ptr_eq(&tab.seen) {
            return;
        }
        let doc_changed = tab.doc_changed.swap(false, Ordering::Relaxed);
        let was_open = search_panel_open(&tab.seen);
        tab.refresh(state, doc_changed);
        if doc_changed {
            tab.version += 1;
            self.session_dirty = true;
        }
        if self.tabs.get(self.active).is_some_and(|t| t.view == *view) {
            self.find_editor_changed(was_open, cx);
        }
        cx.notify();
    }

    fn tab_index(&self, doc_id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.doc.id == doc_id)
    }

    fn new_doc_id(&mut self) -> u64 {
        self.next_doc_id += 1;
        self.next_doc_id - 1
    }

    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let doc = Document::new(self.new_doc_id(), self.next_untitled);
        self.next_untitled += 1;
        let idx = self.add_tab(doc, String::new(), cx);
        self.activate(idx, window, cx);
    }

    fn activate(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        // An open find bar moves to the new tab with its query.
        let query = self
            .find_open(cx)
            .then(|| codemirror_search::get_search_query(self.tabs[self.active].state(cx)));
        self.active = idx.min(self.tabs.len() - 1);
        self.settings_active = false;
        self.carry_find(query, cx);
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Give the keyboard focus to the visible editor, or to the window
    /// when the Settings tab is visible.
    fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_active {
            window.focus(&self.focus_handle);
        } else if let Some(tab) = self.tabs.get(self.active) {
            tab.view.read(cx).focus(window);
        }
    }

    fn request_close_tab(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(idx) else { return };
        if !tab.modified {
            self.close_tab(idx, window, cx);
            return;
        }
        let id = tab.doc.id;
        let message = format!("Do you want to save changes to {}?", tab.doc.title());
        let answer = window.prompt(PromptLevel::Warning, &message, None, &["Save", "Don't Save", "Cancel"], cx);
        cx.spawn_in(window, async move |this, cx| {
            let close = match answer.await {
                Ok(0) => match this.update(cx, |this, cx| this.save_by_id(id, cx)) {
                    Ok(saving) => saving.await,
                    Err(_) => false,
                },
                Ok(1) => true,
                _ => false,
            };
            if close {
                let _ = this.update_in(cx, |this, window, cx| {
                    if let Some(idx) = this.tab_index(id) {
                        this.close_tab(idx, window, cx);
                    }
                });
            }
        })
        .detach();
    }

    fn close_tab(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.remove(idx);
        if self.tabs.is_empty() {
            self.new_tab(window, cx);
        }
        if self.active > idx || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
        }
        self.session_dirty = true;
        self.focus_active(window, cx);
        cx.notify();
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = true;
        self.settings_active = true;
        self.focus_active(window, cx);
        cx.notify();
    }

    fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.settings_active = false;
        self.focus_active(window, cx);
        cx.notify();
    }

    // ---------- files ----------

    fn note_recent(&mut self, path: &Path) {
        let p = clean_path(path.to_path_buf());
        let recent = &mut self.settings.recent_files;
        recent.retain(|r| r != &p);
        recent.insert(0, p);
        recent.truncate(10);
        self.settings.save();
    }

    /// Open `path` as a tab, or show its tab if it is already open.
    fn open_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let path = clean_path(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()));
        if let Some(i) = self.tabs.iter().position(|t| t.doc.path.as_deref() == Some(&*path)) {
            self.activate(i, window, cx);
            self.note_recent(&path);
            return;
        }
        match Document::open(self.new_doc_id(), &path) {
            Ok((doc, text)) => {
                // Replace a pristine untitled tab.
                if self.tabs.len() == 1 && self.tabs[0].doc.path.is_none() && self.tabs[0].state(cx).doc().is_empty()
                {
                    self.tabs.clear();
                }
                let idx = self.add_tab(doc, text, cx);
                self.activate(idx, window, cx);
                self.note_recent(&path);
            }
            Err(e) => self.flash(format!("Could not open {}: {e}", path.display()), cx),
        }
    }

    fn open_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked =
            cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    for path in paths {
                        this.open_path(&path, window, cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Write the tab to its file. Returns whether it saved.
    fn write_tab(&mut self, idx: usize, cx: &mut Context<Self>) -> bool {
        let text = self.tabs[idx].state(cx).doc().clone();
        let tab = &mut self.tabs[idx];
        match tab.doc.save(&text) {
            Ok(()) => {
                tab.modified = false;
                let title = tab.doc.title();
                if let Some(path) = tab.doc.path.clone() {
                    self.note_recent(&path);
                }
                self.session_dirty = true;
                self.flash(format!("Saved {title}"), cx);
                true
            }
            Err(e) => {
                self.flash(format!("Save failed: {e}"), cx);
                false
            }
        }
    }

    /// Save the document with id `id`, asking for a file name if it has
    /// none. The task resolves to whether it saved.
    fn save_by_id(&mut self, id: u64, cx: &mut Context<Self>) -> Task<bool> {
        let Some(idx) = self.tab_index(id) else { return Task::ready(false) };
        if self.tabs[idx].doc.path.is_some() {
            Task::ready(self.write_tab(idx, cx))
        } else {
            self.save_as_by_id(id, cx)
        }
    }

    fn save_as_by_id(&mut self, id: u64, cx: &mut Context<Self>) -> Task<bool> {
        let Some(idx) = self.tab_index(id) else { return Task::ready(false) };
        let doc = &self.tabs[idx].doc;
        let (dir, name) = match &doc.path {
            Some(path) => (
                path.parent().map(Path::to_path_buf).unwrap_or_default(),
                path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            ),
            None => (documents_dir(), format!("{}.txt", doc.title())),
        };
        let picked = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = picked.await else { return false };
            this.update(cx, |this, cx| {
                let Some(idx) = this.tab_index(id) else { return false };
                this.tabs[idx].doc.path = Some(path);
                cx.notify();
                this.write_tab(idx, cx)
            })
            .unwrap_or(false)
        })
    }

    // ---------- session ----------

    fn restore_session(&mut self, cx: &mut Context<Self>) {
        let Some(session) = session::load() else { return };
        for tab in session.tabs {
            let id = self.new_doc_id();
            let (mut doc, text) = match (&tab.path, tab.dirty_text) {
                (Some(path), Some(text)) => (Document::restore_file(id, path), text),
                (Some(path), None) => match Document::open(id, path) {
                    Ok(opened) => opened,
                    Err(_) => continue, // the file is gone
                },
                (None, Some(text)) => {
                    self.next_untitled = self.next_untitled.max(tab.untitled_n + 1);
                    (Document::new(id, tab.untitled_n), text)
                }
                (None, None) => continue,
            };
            doc.language = tab.language;
            self.add_tab(doc, text, cx);
        }
        if !self.tabs.is_empty() {
            self.active = session.active.min(self.tabs.len() - 1);
        }
    }

    fn session_snapshot(&self, cx: &App) -> Session {
        let mut tabs = Vec::new();
        let mut active = 0;
        for (i, t) in self.tabs.iter().enumerate() {
            let text = t.state(cx).doc();
            // Skip pristine empty untitled tabs.
            if t.doc.path.is_none() && text.is_empty() {
                continue;
            }
            if i == self.active {
                active = tabs.len();
            }
            tabs.push(SessionTab {
                path: t.doc.path.clone(),
                untitled_n: t.doc.untitled_n,
                dirty_text: t.modified.then(|| text.to_string()),
                language: t.doc.language.clone(),
            });
        }
        Session { active, tabs }
    }

    fn save_session(&mut self, cx: &App) {
        session::save(&self.session_snapshot(cx));
        self.session_dirty = false;
    }

    /// The window's close button. The primary window keeps unsaved tabs in
    /// the session; a secondary window asks first.
    fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.settings.save();
        if self.is_primary {
            self.save_session(cx);
            return true;
        }
        let unsaved = self.tabs.iter().filter(|t| t.modified).count();
        if unsaved == 0 || self.allow_close {
            return true;
        }
        let message = if unsaved == 1 {
            "A tab has unsaved changes.".to_string()
        } else {
            format!("{unsaved} tabs have unsaved changes.")
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &message,
            Some("This window has no saved session, so closing it loses the changes."),
            &["Close Anyway", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update_in(cx, |this, window, _| {
                    this.allow_close = true;
                    window.remove_window();
                });
            }
        })
        .detach();
        false
    }

    // ---------- settings ----------

    /// Change the settings, then apply and save them.
    fn update_settings(&mut self, change: impl FnOnce(&mut Settings), cx: &mut Context<Self>) {
        change(&mut self.settings);
        self.palette = palette(self.settings.dark);
        self.find.set_palette(&self.palette, cx);
        let effect = self.config.reconfigure(self.editor_config());
        for tab in &self.tabs {
            let effect = effect.clone();
            tab.view.update(cx, |view, cx| view.dispatch_spec(TransactionSpec::new().effect(effect), cx));
        }
        self.settings.save();
        cx.notify();
    }

    fn flash(&mut self, msg: String, cx: &mut Context<Self>) {
        self.status_msg = Some(msg.into());
        self.flash_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FLASH_TIME).await;
            let _ = this.update(cx, |this, cx| {
                this.status_msg = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    // ---------- actions ----------

    /// Run `f` on the visible editor, if a document tab is visible.
    fn with_editor(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut EditorView, &mut Context<EditorView>)) {
        if self.settings_active {
            return;
        }
        if let Some(tab) = self.tabs.get(self.active) {
            tab.view.update(cx, f);
        }
    }

    fn register_actions(&self, root: gpui::Stateful<gpui::Div>, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        root.on_action(cx.listener(|this, _: &NewTab, window, cx| this.new_tab(window, cx)))
            .on_action(cx.listener(|_, _: &NewWindow, _, _| {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).arg("--new-window").spawn();
                }
            }))
            .on_action(cx.listener(|this, _: &Open, window, cx| this.open_dialog(window, cx)))
            .on_action(cx.listener(|this, _: &Save, _, cx| {
                if let Some(tab) = this.tabs.get(this.active) {
                    this.save_by_id(tab.doc.id, cx).detach();
                }
            }))
            .on_action(cx.listener(|this, _: &SaveAs, _, cx| {
                if let Some(tab) = this.tabs.get(this.active) {
                    this.save_as_by_id(tab.doc.id, cx).detach();
                }
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if this.settings_active {
                    this.close_settings(window, cx);
                } else {
                    this.request_close_tab(this.active, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| {
                let n = this.tabs.len();
                this.activate((this.active + 1) % n, window, cx);
            }))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| {
                let n = this.tabs.len();
                this.activate((this.active + n - 1) % n, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.update_settings(|s| s.zoom = (s.zoom + 10.0).min(500.0), cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.update_settings(|s| s.zoom = (s.zoom - 10.0).max(10.0), cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.update_settings(|s| s.zoom = 100.0, cx)))
            .on_action(cx.listener(|this, _: &InsertTimeDate, _, cx| {
                let stamp = chrono::Local::now().format("%-I:%M %p %-m/%-d/%Y").to_string();
                this.with_editor(cx, |view, cx| view.insert_text(&stamp, "input", cx));
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)))
            .on_action(cx.listener(|_, _: &Exit, window, _| window.remove_window()))
            .on_action(cx.listener(|this, _: &Undo, _, cx| this.with_editor(cx, |v, cx| _ = v.run_state_command(undo, cx))))
            .on_action(cx.listener(|this, _: &Redo, _, cx| this.with_editor(cx, |v, cx| _ = v.run_state_command(redo, cx))))
            .on_action(cx.listener(|this, _: &Cut, _, cx| this.with_editor(cx, |v, cx| _ = v.copy(true, cx))))
            .on_action(cx.listener(|this, _: &Copy, _, cx| this.with_editor(cx, |v, cx| _ = v.copy(false, cx))))
            .on_action(cx.listener(|this, _: &Paste, _, cx| this.with_editor(cx, |v, cx| _ = v.paste_from_clipboard(cx))))
            .on_action(cx.listener(|this, _: &Delete, _, cx| {
                this.with_editor(cx, |v, cx| _ = v.run_state_command(codemirror_commands::delete_char_forward, cx))
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.with_editor(cx, |v, cx| _ = v.run_state_command(select_all, cx))))
            .on_action(cx.listener(|_, _: &About, window, cx| {
                let detail = format!("Version {}\nA native notepad, rebuilt on GPUI and a Rust port of CodeMirror.", env!("CARGO_PKG_VERSION"));
                drop(window.prompt(PromptLevel::Info, "Notey", Some(&detail), &["OK"], cx));
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                for path in paths.paths() {
                    if path.is_file() {
                        this.open_path(path, window, cx);
                    }
                }
            }))
    }
}

impl Render for NoteyApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = if self.settings_active {
            "Settings - Notey".to_string()
        } else {
            match self.tabs.get(self.active) {
                Some(tab) => format!("{}{} - Notey", if tab.modified { "*" } else { "" }, tab.doc.title()),
                None => "Notey".to_string(),
            }
        };
        if title != self.last_title {
            window.set_window_title(&title);
            self.last_title = title;
        }

        let p = self.palette;
        let body = if self.settings_active {
            self.settings_page(cx).into_any_element()
        } else {
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(|this, event, window, cx| this.open_context_menu(event, window, cx)),
                )
                .children(self.tabs.get(self.active).map(|t| t.view.clone()))
                .children(self.find_bar(window, cx))
                .children(self.context_popup(cx))
                .into_any_element()
        };
        let root = div()
            .id("notey")
            .key_context("Notey")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(p.chrome)
            .text_color(p.text)
            .font_family(UI_FONT)
            .text_size(px(13.))
            .child(self.title_bar(window, cx))
            .when(self.settings.show_menu, |d| d.child(self.menu_bar(cx)))
            .child(div().h(px(1.)).flex_none().bg(p.stroke))
            .child(body)
            .when(self.settings.show_status, |d| d.child(self.status_bar(cx)));
        let root = self.register_actions(root, cx);
        self.register_find_actions(root, cx)
    }
}

fn documents_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(|home| PathBuf::from(home).join("Documents"))
        .unwrap_or_default()
}
