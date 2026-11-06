//! Running Rhai plugin scripts (see `docs/plugin-api.md`): their commands
//! (menus, the right-click menu, shortcuts), key hooks and events. A script
//! works on a snapshot of the active tab; when it returns, its edits,
//! selection, text styles and host actions are applied.

use std::time::{Duration, Instant};

use codemirror_state::{ChangeSpec, EditorSelection, ADD_TO_HISTORY, TIME};
use gpui::{ClipboardItem, KeyDownEvent};

use super::*;
use crate::plugins::{BufInfo, History, HostAction, PluginHost, ScriptCtx};

/// The user event of plugin edits that join the user's typing in the
/// undo history (see the editor's history config).
pub(super) const PLUGIN_TYPING: &str = "input.type.plugin";

/// The editor's undo history: plugin results written after typing
/// (`set_history("join")`) undo together with the typing.
pub(super) fn editor_history() -> codemirror_state::Extension {
    codemirror_commands::history(codemirror_commands::HistoryConfig {
        join_to_event: Some(std::sync::Arc::new(|tr, adjacent| adjacent || tr.is_user_event(PLUGIN_TYPING))),
        ..codemirror_commands::HistoryConfig::default()
    })
}

#[cfg(test)]
mod tests {
    use codemirror_state::{ChangeSpec, EditorState, EditorStateConfig, TransactionSpec, TIME};

    use super::{editor_history, PLUGIN_TYPING};

    #[test]
    fn plugin_results_undo_with_the_typing() {
        let mut state = EditorState::create(EditorStateConfig::default().doc("2+2 ").extensions(editor_history()));
        let typed = state.update(TransactionSpec::new().changes(ChangeSpec::insert(4, "=>")).user_event("input.type"));
        state = typed.state().clone();
        // the result arrives later, as Math writes it
        let result = state.update(
            TransactionSpec::new().changes(ChangeSpec::insert(6, " 4")).user_event(PLUGIN_TYPING).annotation(TIME.of(0.0)),
        );
        state = result.state().clone();
        assert_eq!(state.doc().to_string(), "2+2 => 4");
        let mut undone = None;
        codemirror_commands::undo(&state, &mut |tr| undone = Some(tr.state().clone()));
        assert_eq!(undone.unwrap().doc().to_string(), "2+2 ");
    }
}
use crate::problems::{visible_range, Problem, SetProblems, Severity, SET_PROBLEMS};
use crate::styles::{StyleChange, STYLE_RANGE};

/// `text_changed` events come at most this often.
const TEXT_EVENT_INTERVAL: Duration = Duration::from_millis(250);
/// `text_idle` comes when the text has not changed for this long.
const TEXT_IDLE: Duration = Duration::from_millis(700);

/// Plugin state of the app.
#[derive(Default)]
pub(super) struct Scripting {
    pub(super) host: PluginHost,
    /// Plugin status bar items: (key, text, tooltip), in insertion order.
    pub(super) status_items: Vec<(String, String, String)>,
    text_changed_pending: bool,
    text_timer: Option<Task<()>>,
    idle_timer: Option<Task<()>>,
    last_text_event: Option<Instant>,
    last_selection: Option<(u64, (usize, usize))>,
    ready_fired: bool,
    last_active_doc: u64,
    /// Typed text and a text change of the active editor, not yet delivered.
    pub(super) pending_typed: String,
    pub(super) pending_doc_change: bool,
    pub(super) pending_check: bool,
    /// Events to deliver after this render: (kind, path).
    pub(super) queued: Vec<(String, Option<String>)>,
    /// Plugin downloads under way.
    fetching: std::collections::HashSet<String>,
}

fn char_to_byte(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}

fn byte_to_char(s: &str, b: usize) -> usize {
    s[..b.min(s.len())].chars().count()
}

impl NoteyApp {
    /// Load the user's scripts and the enabled Feature plugins' scripts.
    /// `report` flashes how many loaded.
    pub(super) fn reload_scripts(&mut self, report: bool, cx: &mut Context<Self>) {
        let feature_scripts: Vec<_> = self
            .plugins
            .store
            .enabled(crate::packages::Kind::Feature)
            .filter_map(|p| Some((p.file(p.manifest.script.as_ref()?), p.manifest.capabilities.clone())))
            .collect();
        crate::wasm_host::reset();
        self.scripting.host = PluginHost::load(&feature_scripts);
        // plugin settings sections are by script index
        self.settings_ui.forget_plugins();
        if matches!(self.settings_ui.section, super::settings_tab::Section::Plugin(_)) {
            self.settings_ui.section = super::settings_tab::Section::Editor;
        }
        if report {
            let n = self.scripting.host.scripts.len();
            self.flash(format!("Loaded {n} script{}", if n == 1 { "" } else { "s" }), cx);
        }
        self.surface_plugin_errors(cx);
        cx.notify();
    }

    /// Start the plugin host: example script, scripts, the `ready` event.
    pub(super) fn start_scripting(&mut self, cx: &mut Context<Self>) {
        PluginHost::seed_examples();
        PluginHost::remove_legacy_markdown_script();
        self.reload_scripts(false, cx);
    }

    pub(super) fn surface_plugin_errors(&mut self, cx: &mut Context<Self>) {
        if let Some(err) = self.scripting.host.errors.pop() {
            self.scripting.host.errors.clear();
            self.flash(format!("Plugin error: {err}"), cx);
        }
    }

    pub(super) fn script_snapshot(&self, cx: &App) -> Option<ScriptCtx> {
        self.script_snapshot_of(self.active, cx)
    }

    /// A snapshot of the tab at `idx`, as the tab scripts work on.
    fn script_snapshot_of(&self, idx: usize, cx: &App) -> Option<ScriptCtx> {
        let tab = self.tabs.get(idx)?;
        let state = tab.state(cx);
        let text = state.doc().to_string();
        let main = state.selection().main();
        let sel = (byte_to_char(&text, main.anchor()), byte_to_char(&text, main.head()));
        Some(ScriptCtx {
            text,
            sel,
            text_dirty: false,
            sel_dirty: false,
            buffers: self
                .tabs
                .iter()
                .enumerate()
                .map(|(i, t)| BufInfo {
                    id: t.doc.id,
                    title: t.doc.title(),
                    path: t.doc.path.as_ref().map(|p| p.display().to_string()),
                    modified: t.modified,
                    active: i == idx,
                })
                .collect(),
            active_id: tab.doc.id,
            language: tab.doc.language.clone(),
            config_dir: PluginHost::scripts_dir()
                .and_then(|d| d.parent().map(|p| p.display().to_string()))
                .unwrap_or_default(),
            actions: Vec::new(),
            status: None,
            preview: self.preview_mode(tab.doc.id).to_string(),
            script_name: String::new(),
            capabilities: Vec::new(),
            styles: Vec::new(),
            tab_color: tab.color.clone().unwrap_or_default(),
            ..ScriptCtx::default()
        })
    }

    /// Apply what a script did to the tab it ran on.
    fn apply_script_result(&mut self, doc_id: u64, out: ScriptCtx, window: &mut Window, cx: &mut Context<Self>) {
        let (error_color, hint_color) = (self.palette.error, self.palette.warning);
        if let Some(idx) = self.tab_index(doc_id) {
            let view = self.tabs[idx].view.clone();
            view.update(cx, |view, cx| {
                let mut spec = TransactionSpec::new();
                let mut any = false;
                let exact = out.exact.as_ref().filter(|(_, after)| *after == out.text).map(|(e, _)| e);
                if let (true, Some(edits)) = (out.text_dirty, exact) {
                    // replace_ranges: each edit on its own, so the cursor and
                    // styles elsewhere stay put
                    let old = view.state().doc().to_string();
                    // the edits are in order, so one pass finds every offset
                    let (mut at_char, mut at_byte) = (0usize, 0usize);
                    let mut byte_of = |c: usize| {
                        while at_char < c && at_byte < old.len() {
                            at_byte += old[at_byte..].chars().next().map_or(1, char::len_utf8);
                            at_char += 1;
                        }
                        at_byte
                    };
                    let mut specs = Vec::with_capacity(edits.len());
                    for (a, b, t) in edits {
                        let from = byte_of(*a);
                        let to = byte_of(*b);
                        specs.push(ChangeSpec::replace(from, to, t.clone()));
                    }
                    if !specs.is_empty() {
                        spec = spec.changes(ChangeSpec::List(specs));
                        any = true;
                    }
                } else if out.text_dirty {
                    // The smallest single replacement, so undo, scrolling and
                    // other cursors are disturbed as little as possible.
                    let old = view.state().doc().to_string();
                    let new = &out.text;
                    let mut prefix = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
                    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
                        prefix -= 1;
                    }
                    let max_suffix = old.len().min(new.len()) - prefix;
                    let mut suffix =
                        old.bytes().rev().zip(new.bytes().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();
                    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
                        suffix -= 1;
                    }
                    if prefix + suffix != old.len() || prefix + suffix != new.len() {
                        let insert = new[prefix..new.len() - suffix].to_string();
                        spec = spec.changes(ChangeSpec::replace(prefix, old.len() - suffix, insert));
                        any = true;
                    }
                }
                if out.text_dirty {
                    spec = match out.history {
                        History::Normal => spec.user_event("input.plugin"),
                        // joins the last typing step (the time is unknown, so
                        // no group delay applies)
                        History::Join => spec.user_event(PLUGIN_TYPING).annotation(TIME.of(0.0)),
                        History::Skip => spec.user_event("input.plugin").annotation(ADD_TO_HISTORY.of(false)),
                    };
                }
                if out.sel_dirty {
                    let (a, h) = (char_to_byte(&out.text, out.sel.0), char_to_byte(&out.text, out.sel.1));
                    spec = spec.selection(EditorSelection::single(a, h));
                    if !out.sel_quiet {
                        spec = spec.scroll_into_view();
                    }
                    any = true;
                }
                // The style ops use offsets into the script's text, which is
                // now the document's text.
                for op in &out.styles {
                    let (from, to) = (char_to_byte(&out.text, op.start), char_to_byte(&out.text, op.end));
                    spec = spec.effect(STYLE_RANGE.of(StyleChange { from, to, fg: op.fg, bg: op.bg }));
                    any = true;
                }
                // Problems, in offsets of the script's text like the styles.
                if let Some(ops) = &out.problems {
                    let text = &out.text;
                    let problems = ops
                        .iter()
                        .map(|op| {
                            let (from, to) =
                                visible_range(text, char_to_byte(text, op.start), char_to_byte(text, op.end));
                            Problem {
                                from,
                                to,
                                severity: if op.error { Severity::Error } else { Severity::Hint },
                                message: op.message.clone(),
                                bar: op.bar,
                                color: if op.error { error_color } else { hint_color },
                            }
                        })
                        .collect();
                    spec = spec.effect(SET_PROBLEMS.of(SetProblems { source: out.script_name.clone(), problems }));
                    any = true;
                }
                if any {
                    view.dispatch_spec(spec, cx);
                }
            });
        }
        if let Some(msg) = out.status {
            self.flash(msg, cx);
        }
        for action in out.actions {
            match action {
                HostAction::SetTabColor(color) => self.set_tab_color(doc_id, color, cx),
                // the script's tab, which a tab menu command need not have active
                HostAction::SaveActive => self.save_by_id(doc_id, cx).detach(),
                action => self.host_action(action, window, cx),
            }
        }
        cx.notify();
    }

    fn host_action(&mut self, action: HostAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            HostAction::OpenFile(p) => self.open_path(&p, window, cx),
            HostAction::NewTab => self.new_tab(window, cx),
            HostAction::SaveActive => {
                if let Some(tab) = self.tabs.get(self.active) {
                    self.save_by_id(tab.doc.id, cx).detach();
                }
            }
            HostAction::ActivateBuffer(id) => {
                if let Some(i) = self.tab_index(id) {
                    self.activate(i, window, cx);
                }
            }
            HostAction::CloseBuffer(id) => {
                if let Some(i) = self.tab_index(id) {
                    self.request_close_tab(i, window, cx);
                }
            }
            HostAction::Alert(title, body) => {
                drop(window.prompt(PromptLevel::Info, &title, Some(&body), &["OK"], cx));
            }
            HostAction::SetClipboard(s) => cx.write_to_clipboard(ClipboardItem::new_string(s)),
            HostAction::SetLanguage(lang) => {
                let valid = match &lang {
                    None => Some(None),
                    Some(name) => crate::syntax::global().set.find_syntax_by_name(name).map(|sr| Some(sr.name.clone())),
                };
                match valid {
                    Some(l) => self.set_language(l, cx),
                    None => self.flash(format!("Unknown language: {}", lang.unwrap_or_default()), cx),
                }
            }
            HostAction::ToggleMarkdownPreview => self.toggle_markdown_preview(window, cx),
            HostAction::SetPreview(mode) => self.set_preview_mode(&mode, window, cx),
            HostAction::OpenPath(path) => {
                // The default app for the file (a browser for .html, say).
                let _ = std::process::Command::new("explorer").arg(&path).spawn();
            }
            HostAction::SetStatusItem(key, text, tooltip) => {
                let items = &mut self.scripting.status_items;
                match items.iter_mut().find(|(k, _, _)| *k == key) {
                    Some(item) => {
                        item.1 = text;
                        item.2 = tooltip;
                    }
                    None => items.push((key, text, tooltip)),
                }
            }
            HostAction::ClearStatusItem(key) => self.scripting.status_items.retain(|(k, _, _)| *k != key),
            HostAction::Fetch(url) => self.plugin_fetch(url, window, cx),
            HostAction::SetTabColor(_) => {}
            HostAction::Start(program, args, dir) => self.start_program(&program, &args, dir.as_deref(), cx),
        }
    }

    /// Start a program for a plugin. A console program gets a console
    /// window of its own.
    fn start_program(&mut self, program: &str, args: &[String], dir: Option<&Path>, cx: &mut Context<Self>) {
        let mut command = std::process::Command::new(program);
        command.args(args);
        if let Some(dir) = dir.filter(|d| d.is_dir()) {
            command.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_CONSOLE: u32 = 0x10;
            command.creation_flags(CREATE_NEW_CONSOLE);
        }
        if let Err(e) = command.spawn() {
            self.flash(format!("Could not start {program}: {e}"), cx);
        }
    }

    /// Download `url` for a plugin in the background, into the cache that
    /// `notey.net.cached` reads, then deliver a `net` event.
    fn plugin_fetch(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.scripting.fetching.insert(url.clone()) {
            return;
        }
        let Some(path) = crate::plugins::net_cache_path(&url) else { return };
        let source = url.clone();
        let job = cx.background_executor().spawn(async move {
            let body = crate::update::fetch(&source, 4 * 1024 * 1024)?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            let tmp = path.with_extension("part");
            std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
            std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.scripting.fetching.remove(&url);
                if result.is_ok() {
                    this.fire_event("net", Some(url), window, cx);
                }
            });
        })
        .detach();
    }

    /// Run a plugin command on the active tab, or on the tab `target` (a
    /// command of the tab right-click menu).
    pub(super) fn run_plugin_command(
        &mut self,
        script_idx: usize,
        cmd_id: &str,
        target: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if script_idx >= self.scripting.host.scripts.len() || (self.settings_active && target.is_none()) {
            return;
        }
        let idx = match target {
            Some(id) => match self.tab_index(id) {
                Some(i) => i,
                None => return,
            },
            None => self.active,
        };
        let Some(snapshot) = self.script_snapshot_of(idx, cx) else { return };
        let doc_id = snapshot.active_id;
        let out = self.scripting.host.run_command(script_idx, cmd_id, snapshot);
        self.apply_script_result(doc_id, out, window, cx);
        self.surface_plugin_errors(cx);
    }

    /// Deliver an event to the scripts that subscribed to it. `payload` is
    /// the typed text for `input`, else a path.
    pub(super) fn fire_event(&mut self, kind: &str, payload: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let subscribed: Vec<usize> = self
            .scripting
            .host
            .scripts
            .iter()
            .enumerate()
            .filter(|(_, s)| s.events.contains(kind))
            .map(|(i, _)| i)
            .collect();
        for idx in subscribed {
            let Some(snapshot) = self.script_snapshot(cx) else { return };
            let doc_id = snapshot.active_id;
            let mut event = rhai::Map::new();
            event.insert("kind".into(), rhai::Dynamic::from(kind.to_string()));
            event.insert("buffer_id".into(), rhai::Dynamic::from(doc_id as i64));
            if let Some(p) = &payload {
                let key = match kind {
                    "input" => "text",
                    "net" => "url",
                    _ => "path",
                };
                event.insert(key.into(), rhai::Dynamic::from(p.clone()));
            }
            let out = self.scripting.host.run_event(idx, event, snapshot);
            self.apply_script_result(doc_id, out, window, cx);
        }
        self.surface_plugin_errors(cx);
    }

    /// Whether events wait to be delivered.
    pub(super) fn script_events_due(&self) -> bool {
        let s = &self.scripting;
        let active = self.tabs.get(self.active).map(|t| t.doc.id);
        !s.queued.is_empty() || s.pending_check || !s.ready_fired || active.is_some_and(|id| id != s.last_active_doc)
    }

    /// Deliver the waiting events (after a render, outside it).
    pub(super) fn flush_script_events(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.check_active_events(window, cx);
        for (kind, path) in std::mem::take(&mut self.scripting.queued) {
            self.fire_event(&kind, path, window, cx);
        }
        if std::mem::take(&mut self.scripting.pending_check) {
            let typed = std::mem::take(&mut self.scripting.pending_typed);
            let changed = std::mem::take(&mut self.scripting.pending_doc_change);
            self.editor_events(typed, changed, window, cx);
        }
    }

    /// Called after every render: `ready` once, and `buffer_activated` when
    /// the active document changes.
    fn check_active_events(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.tabs.get(self.active).map(|t| t.doc.id) else { return };
        if !self.scripting.ready_fired {
            self.scripting.ready_fired = true;
            self.scripting.last_active_doc = id;
            self.fire_event("ready", None, window, cx);
        } else if id != self.scripting.last_active_doc {
            self.scripting.last_active_doc = id;
            let path = self.tabs[self.active].doc.path.as_ref().map(|p| p.display().to_string());
            self.fire_event("buffer_activated", path, window, cx);
        }
    }

    /// The active editor changed: `input`, `text_changed` (throttled) and
    /// `selection_changed` events.
    pub(super) fn editor_events(&mut self, typed: String, doc_changed: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !typed.is_empty() {
            self.fire_event("input", Some(typed), window, cx);
        }
        if doc_changed {
            self.scripting.text_changed_pending = true;
            self.schedule_text_event(window, cx);
            // `text_idle` once the typing pauses (each change starts it over)
            self.scripting.idle_timer = Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(TEXT_IDLE).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.scripting.idle_timer = None;
                    this.fire_event("text_idle", None, window, cx);
                });
            }));
        }
        let Some(tab) = self.tabs.get(self.active) else { return };
        let main = tab.state(cx).selection().main();
        let sel = (tab.doc.id, (main.anchor(), main.head()));
        if self.scripting.last_selection != Some(sel) {
            let first = self.scripting.last_selection.is_none();
            self.scripting.last_selection = Some(sel);
            if !first {
                self.fire_event("selection_changed", None, window, cx);
            }
        }
    }

    fn schedule_text_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.scripting.text_timer.is_some() {
            return;
        }
        let wait = self
            .scripting
            .last_text_event
            .map_or(Duration::ZERO, |t| TEXT_EVENT_INTERVAL.saturating_sub(t.elapsed()));
        self.scripting.text_timer = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.scripting.text_timer = None;
                if std::mem::take(&mut this.scripting.text_changed_pending) {
                    this.scripting.last_text_event = Some(Instant::now());
                    this.fire_event("text_changed", None, window, cx);
                }
            });
        }));
    }

    /// Plugin shortcuts and key hooks, before the editor sees the key.
    pub(super) fn plugin_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_active || self.scripting.host.scripts.is_empty() {
            return;
        }
        let Some(tab) = self.tabs.get(self.active) else { return };
        let language = tab.doc.language.clone();
        let focused = tab.view.read(cx).has_focus();
        let k = &event.keystroke;
        let command = self.scripting.host.scripts.iter().enumerate().find_map(|(i, s)| {
            s.commands.iter().find_map(|c| {
                (c.when.matches(language.as_deref(), focused) && c.shortcut.as_ref().is_some_and(|sc| sc.matches(k)))
                    .then(|| (i, c.id.clone()))
            })
        });
        if let Some((i, id)) = command {
            cx.stop_propagation();
            self.run_plugin_command(i, &id, None, window, cx);
            return;
        }
        let m = &k.modifiers;
        if !focused || m.control || m.alt || m.shift || m.platform {
            return;
        }
        let hooks: Vec<usize> = self
            .scripting
            .host
            .scripts
            .iter()
            .enumerate()
            .filter(|(_, s)| s.keys.iter().any(|(key, w)| *key == k.key && w.matches(language.as_deref(), true)))
            .map(|(i, _)| i)
            .collect();
        for i in hooks {
            let Some(snapshot) = self.script_snapshot(cx) else { return };
            let doc_id = snapshot.active_id;
            let (out, handled) = self.scripting.host.run_key(i, &crate::plugins::script_key_name(&k.key), snapshot);
            if handled {
                cx.stop_propagation();
            }
            self.apply_script_result(doc_id, out, window, cx);
            self.surface_plugin_errors(cx);
            if handled {
                break;
            }
        }
    }
}
