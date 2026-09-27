//! Tier-1 plugin host: Rhai scripts implementing the `notey.*` API
//! (see docs/plugin-api.md). Scripts live in `%APPDATA%\Notey\scripts`,
//! register commands into the Plugins menu, and receive host events.
//!
//! Scripts run against a snapshot context: the active buffer's text and
//! selection are copied in, mutated freely (read-your-writes), and applied
//! back plus queued host actions when the script returns. A failing script
//! can therefore never leave the app in a half-mutated state.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use eframe::egui::{Key, KeyboardShortcut, Modifiers};
use rhai::{Array, Dynamic, Engine, ImmutableString, Map, Scope, AST};

pub const API_VERSION: i64 = 1;

// ---------- host-visible results of a script run ----------

pub enum HostAction {
    OpenFile(PathBuf),
    NewTab,
    SaveActive,
    ActivateBuffer(u64),
    CloseBuffer(u64),
    Alert(String, String),
    SetClipboard(String),
    SetLanguage(Option<String>),
    ToggleMarkdownPreview,
}

#[derive(Clone)]
pub struct BufInfo {
    pub id: u64,
    pub title: String,
    pub path: Option<String>,
    pub modified: bool,
    pub active: bool,
}

/// Snapshot the script reads and mutates.
pub struct ScriptCtx {
    pub text: String,
    pub sel: (usize, usize), // char offsets
    pub text_dirty: bool,
    pub sel_dirty: bool,
    pub buffers: Vec<BufInfo>,
    pub active_id: u64,
    pub language: Option<String>,
    pub config_dir: String,
    pub actions: Vec<HostAction>,
    pub status: Option<String>,
}

type Ctx = Rc<RefCell<ScriptCtx>>;

// ---------- script + command metadata ----------

pub struct PluginCommand {
    pub id: String,
    pub title: String,
    pub shortcut: Option<KeyboardShortcut>,
    pub shortcut_text: String,
}

pub struct Script {
    pub name: String,
    pub path: PathBuf,
    pub ast: Rc<AST>,
    pub commands: Vec<PluginCommand>,
    pub events: HashSet<String>,
}

#[derive(Default)]
pub struct PluginHost {
    pub scripts: Vec<Script>,
    pub errors: Vec<String>,
}

impl PluginHost {
    pub fn scripts_dir() -> Option<PathBuf> {
        eframe::storage_dir("Notey").map(|d| {
            d.parent()
                .map(|p| p.join("scripts"))
                .unwrap_or_else(|| d.join("scripts"))
        })
    }

    pub fn load() -> Self {
        let mut host = Self::default();
        let Some(dir) = Self::scripts_dir() else {
            return host;
        };
        let _ = std::fs::create_dir_all(&dir);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return host;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "rhai").unwrap_or(false))
            .collect();
        paths.sort();
        for path in paths {
            match Self::load_script(&path) {
                Ok(script) => host.scripts.push(script),
                Err(e) => host.errors.push(format!(
                    "{}: {e}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )),
            }
        }
        host
    }

    fn load_script(path: &Path) -> Result<Script, String> {
        let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let engine = bare_engine();
        let ast = engine.compile(&src).map_err(|e| e.to_string())?;
        // run register() to get the manifest
        let mut scope = Scope::new();
        let manifest: Map = engine
            .call_fn(&mut scope, &ast, "register", ())
            .map_err(|e| format!("register() failed: {e}"))?;

        let name = manifest
            .get("name")
            .and_then(|v| v.clone().into_string().ok())
            .unwrap_or_else(|| {
                path.file_stem().unwrap_or_default().to_string_lossy().into_owned()
            });

        if let Some(min_api) = manifest.get("min_api").and_then(|v| v.as_int().ok()) {
            if min_api > API_VERSION {
                return Err(format!(
                    "requires API {min_api}, host provides {API_VERSION}"
                ));
            }
        }

        let mut commands = Vec::new();
        if let Some(arr) = manifest.get("commands") {
            let arr: Array = arr.clone().try_cast().unwrap_or_default();
            for item in arr {
                let m: Map = match item.try_cast() {
                    Some(m) => m,
                    None => continue,
                };
                let id = m
                    .get("id")
                    .and_then(|v| v.clone().into_string().ok())
                    .unwrap_or_default();
                if id.is_empty() {
                    continue;
                }
                let title = m
                    .get("title")
                    .and_then(|v| v.clone().into_string().ok())
                    .unwrap_or_else(|| id.clone());
                let shortcut_text = m
                    .get("shortcut")
                    .and_then(|v| v.clone().into_string().ok())
                    .unwrap_or_default();
                let shortcut = parse_shortcut(&shortcut_text);
                commands.push(PluginCommand {
                    id,
                    title,
                    shortcut,
                    shortcut_text,
                });
            }
        }

        let mut events = HashSet::new();
        if let Some(arr) = manifest.get("events") {
            let arr: Array = arr.clone().try_cast().unwrap_or_default();
            for item in arr {
                if let Ok(s) = item.into_string() {
                    events.insert(s);
                }
            }
        }

        Ok(Script {
            name,
            path: path.to_path_buf(),
            ast: Rc::new(ast),
            commands,
            events,
        })
    }

    /// Seed an example script on first run so the Plugins menu isn't empty.
    pub fn seed_examples() {
        let Some(dir) = Self::scripts_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let example = dir.join("sort_lines.rhai");
        if example.exists() {
            return;
        }
        let has_any = std::fs::read_dir(&dir)
            .map(|mut e| e.any(|f| f.is_ok()))
            .unwrap_or(false);
        if has_any {
            return;
        }
        let _ = std::fs::write(
            &example,
            r#"// Example Notey plugin script (Rhai).
// Docs: see docs/plugin-api.md in the Notey repository.

fn register() {
    #{
        name: "Sort Lines",
        version: "1.0.0",
        min_api: 1,
        commands: [
            #{ id: "sort_asc", title: "Sort Lines Ascending", shortcut: "Ctrl+Alt+S" },
            #{ id: "sort_desc", title: "Sort Lines Descending" },
        ],
        events: [],
    }
}

fn on_command(id) {
    let sel = notey.editor.selection_or_all();
    let text = notey.editor.get_range(sel);
    let lines = text.split("\n");
    lines.sort();
    if id == "sort_desc" { lines.reverse(); }
    notey.editor.replace_range(sel, join(lines, "\n"));
    notey.ui.status(`Sorted ${lines.len()} lines`);
}

fn join(parts, sep) {
    let out = "";
    let first = true;
    for p in parts {
        if !first { out += sep; }
        out += p;
        first = false;
    }
    out
}
"#,
        );
    }

    /// Install the bundled Markdown preview plugin if the user hasn't
    /// already got (or edited) one. Unlike `seed_examples`, this runs even
    /// when other scripts exist, so existing installs pick it up — but it
    /// never overwrites a present file.
    pub fn seed_markdown_plugin() {
        let Some(dir) = Self::scripts_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("markdown_preview.rhai");
        if path.exists() {
            return;
        }
        let _ = std::fs::write(&path, MARKDOWN_PREVIEW_SCRIPT);
    }

    /// Run `on_command` in the given script. Returns the mutated context.
    pub fn run_command(&mut self, script_idx: usize, cmd_id: &str, ctx: ScriptCtx) -> ScriptCtx {
        self.run_fn(script_idx, "on_command", vec![Dynamic::from(ImmutableString::from(cmd_id))], ctx)
    }

    /// Run `on_event` with an event map in every subscribed script.
    pub fn run_event(&mut self, script_idx: usize, event: Map, ctx: ScriptCtx) -> ScriptCtx {
        self.run_fn(script_idx, "on_event", vec![Dynamic::from(event)], ctx)
    }

    fn run_fn(
        &mut self,
        script_idx: usize,
        fn_name: &str,
        args: Vec<Dynamic>,
        ctx: ScriptCtx,
    ) -> ScriptCtx {
        let (ast, name) = {
            let s = &self.scripts[script_idx];
            (s.ast.clone(), s.name.clone())
        };
        let shared: Ctx = Rc::new(RefCell::new(ctx));
        let engine = api_engine(shared.clone());
        let mut scope = Scope::new();
        scope.push_constant("notey", NoteyApi { ctx: shared.clone() });
        let result: Result<Dynamic, _> =
            engine.call_fn_with_options(
                rhai::CallFnOptions::new().eval_ast(false),
                &mut scope,
                &ast,
                fn_name,
                args,
            );
        if let Err(e) = result {
            let msg = e.to_string();
            if !msg.contains("not found") {
                self.errors.push(format!("{name}: {fn_name}: {msg}"));
            }
        }
        // Both the engine's registered closures AND the scope's `notey`
        // constant hold Rc clones of the context; drop them before
        // unwrapping, or the fallback path would discard queued actions.
        drop(engine);
        drop(scope);
        match Rc::try_unwrap(shared) {
            Ok(cell) => cell.into_inner(),
            Err(rc) => {
                // a script kept a reference alive; still recover everything,
                // moving the (non-cloneable) actions out
                let mut c = rc.borrow_mut();
                ScriptCtx {
                    text: c.text.clone(),
                    sel: c.sel,
                    text_dirty: c.text_dirty,
                    sel_dirty: c.sel_dirty,
                    buffers: c.buffers.clone(),
                    active_id: c.active_id,
                    language: c.language.clone(),
                    config_dir: c.config_dir.clone(),
                    actions: std::mem::take(&mut c.actions),
                    status: c.status.clone(),
                }
            }
        }
    }
}

/// Bundled plugin: raw/rendered toggle for Markdown tabs. The rendering
/// itself lives in the host (a script cannot paint); this command flips it.
const MARKDOWN_PREVIEW_SCRIPT: &str = r#"// Notey bundled plugin: Markdown raw/rendered toggle.
// Delete this file to remove the command; it will not be recreated once a
// file with this name has existed. Docs: docs/plugin-api.md in the repo.

fn register() {
    #{
        name: "Markdown Preview",
        version: "1.0.0",
        min_api: 1,
        commands: [
            #{ id: "toggle_md", title: "Toggle Raw/Rendered Markdown", shortcut: "Ctrl+Shift+M" },
        ],
        events: [],
    }
}

fn on_command(id) {
    if id == "toggle_md" {
        if notey.editor.language() == "Markdown" {
            notey.ui.toggle_markdown_preview();
        } else {
            notey.ui.status("Not a Markdown tab - set the language to Markdown in the status bar first");
        }
    }
}
"#;

// ---------- shortcut parsing ("Ctrl+Alt+S") ----------

pub fn parse_shortcut(s: &str) -> Option<KeyboardShortcut> {
    if s.is_empty() {
        return None;
    }
    let mut mods = Modifiers::NONE;
    let mut key: Option<Key> = None;
    for part in s.split('+') {
        match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods = mods.plus(Modifiers::CTRL),
            "alt" => mods = mods.plus(Modifiers::ALT),
            "shift" => mods = mods.plus(Modifiers::SHIFT),
            other => key = key_from_name(other),
        }
    }
    key.map(|k| KeyboardShortcut::new(mods, k))
}

fn key_from_name(name: &str) -> Option<Key> {
    use Key::*;
    Some(match name {
        "a" => A, "b" => B, "c" => C, "d" => D, "e" => E, "f" => F, "g" => G,
        "h" => H, "i" => I, "j" => J, "k" => K, "l" => L, "m" => M, "n" => N,
        "o" => O, "p" => P, "q" => Q, "r" => R, "s" => S, "t" => T, "u" => U,
        "v" => V, "w" => W, "x" => X, "y" => Y, "z" => Z,
        "0" => Num0, "1" => Num1, "2" => Num2, "3" => Num3, "4" => Num4,
        "5" => Num5, "6" => Num6, "7" => Num7, "8" => Num8, "9" => Num9,
        "f1" => F1, "f2" => F2, "f3" => F3, "f4" => F4, "f5" => F5, "f6" => F6,
        "f7" => F7, "f8" => F8, "f9" => F9, "f10" => F10, "f11" => F11,
        "f12" => F12,
        _ => return None,
    })
}

// ---------- Rhai API objects (dot syntax: notey.editor.get_text()) ----------

#[derive(Clone)]
struct NoteyApi {
    ctx: Ctx,
}
#[derive(Clone)]
struct EditorApi {
    ctx: Ctx,
}
#[derive(Clone)]
struct BuffersApi {
    ctx: Ctx,
}
#[derive(Clone)]
struct AppApi {
    ctx: Ctx,
}
#[derive(Clone)]
struct UiApi {
    ctx: Ctx,
}

fn parse_search_opts(m: &Map) -> (crate::search::SearchOpts, bool, usize) {
    let get_bool = |k: &str, d: bool| {
        m.get(k).and_then(|v| v.as_bool().ok()).unwrap_or(d)
    };
    let from = m
        .get("from")
        .and_then(|v| v.as_int().ok())
        .map(|v| v.max(0) as usize)
        .unwrap_or(0);
    (
        crate::search::SearchOpts {
            regex: get_bool("regex", false),
            match_case: get_bool("match_case", false),
        },
        get_bool("wrap", false),
        from,
    )
}

fn range_map(start: usize, end: usize) -> Map {
    let mut m = Map::new();
    m.insert("start".into(), Dynamic::from(start as i64));
    m.insert("end".into(), Dynamic::from(end as i64));
    m
}

fn map_range(m: &Map, max: usize) -> (usize, usize) {
    let get = |k: &str| {
        m.get(k)
            .and_then(|v| v.as_int().ok())
            .map(|v| v.max(0) as usize)
            .unwrap_or(0)
    };
    let (a, b) = (get("start"), get("end"));
    (a.min(max), b.min(max))
}

fn char_to_byte(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}

/// Engine with no API — used only to compile and run register().
fn bare_engine() -> Engine {
    let mut engine = Engine::new();
    engine.set_max_operations(1_000_000);
    engine
}

/// Engine with the full notey.* API bound to `ctx`.
fn api_engine(_ctx: Ctx) -> Engine {
    let mut engine = Engine::new();
    engine.set_max_operations(50_000_000); // generous, but no infinite loops

    engine
        .register_type::<NoteyApi>()
        .register_type::<EditorApi>()
        .register_type::<BuffersApi>()
        .register_type::<AppApi>()
        .register_type::<UiApi>();

    engine.register_get("editor", |n: &mut NoteyApi| EditorApi { ctx: n.ctx.clone() });
    engine.register_get("buffers", |n: &mut NoteyApi| BuffersApi { ctx: n.ctx.clone() });
    engine.register_get("app", |n: &mut NoteyApi| AppApi { ctx: n.ctx.clone() });
    engine.register_get("ui", |n: &mut NoteyApi| UiApi { ctx: n.ctx.clone() });

    // ---- editor ----
    engine.register_fn("get_text", |e: &mut EditorApi| -> ImmutableString {
        e.ctx.borrow().text.clone().into()
    });
    engine.register_fn("set_text", |e: &mut EditorApi, s: ImmutableString| {
        let mut c = e.ctx.borrow_mut();
        c.text = s.to_string();
        c.text_dirty = true;
        let n = c.text.chars().count();
        c.sel = (n.min(c.sel.0), n.min(c.sel.1));
    });
    engine.register_fn("length", |e: &mut EditorApi| -> i64 {
        e.ctx.borrow().text.chars().count() as i64
    });
    engine.register_fn("line_count", |e: &mut EditorApi| -> i64 {
        e.ctx.borrow().text.split('\n').count() as i64
    });
    engine.register_fn("line_text", |e: &mut EditorApi, n: i64| -> ImmutableString {
        e.ctx
            .borrow()
            .text
            .split('\n')
            .nth(n.max(0) as usize)
            .unwrap_or("")
            .into()
    });
    engine.register_fn("get_range", |e: &mut EditorApi, r: Map| -> ImmutableString {
        let c = e.ctx.borrow();
        let max = c.text.chars().count();
        let (a, b) = map_range(&r, max);
        let (a, b) = (a.min(b), a.max(b));
        let sb = char_to_byte(&c.text, a);
        let eb = char_to_byte(&c.text, b);
        c.text[sb..eb].into()
    });
    engine.register_fn(
        "replace_range",
        |e: &mut EditorApi, r: Map, s: ImmutableString| {
            let mut c = e.ctx.borrow_mut();
            let max = c.text.chars().count();
            let (a, b) = map_range(&r, max);
            let (a, b) = (a.min(b), a.max(b));
            let sb = char_to_byte(&c.text, a);
            let eb = char_to_byte(&c.text, b);
            c.text.replace_range(sb..eb, &s);
            c.text_dirty = true;
            let after = a + s.chars().count();
            c.sel = (after, after);
            c.sel_dirty = true;
        },
    );
    engine.register_fn("insert", |e: &mut EditorApi, pos: i64, s: ImmutableString| {
        let mut c = e.ctx.borrow_mut();
        let max = c.text.chars().count();
        let p = (pos.max(0) as usize).min(max);
        let bp = char_to_byte(&c.text, p);
        c.text.insert_str(bp, &s);
        c.text_dirty = true;
        let after = p + s.chars().count();
        c.sel = (after, after);
        c.sel_dirty = true;
    });
    engine.register_fn("append", |e: &mut EditorApi, s: ImmutableString| {
        let mut c = e.ctx.borrow_mut();
        c.text.push_str(&s);
        c.text_dirty = true;
    });
    engine.register_fn("cursor", |e: &mut EditorApi| -> i64 {
        e.ctx.borrow().sel.1 as i64
    });
    engine.register_fn("set_cursor", |e: &mut EditorApi, pos: i64| {
        let mut c = e.ctx.borrow_mut();
        let max = c.text.chars().count();
        let p = (pos.max(0) as usize).min(max);
        c.sel = (p, p);
        c.sel_dirty = true;
    });
    engine.register_fn("selection", |e: &mut EditorApi| -> Map {
        let c = e.ctx.borrow();
        range_map(c.sel.0.min(c.sel.1), c.sel.0.max(c.sel.1))
    });
    engine.register_fn("set_selection", |e: &mut EditorApi, r: Map| {
        let mut c = e.ctx.borrow_mut();
        let max = c.text.chars().count();
        let (a, b) = map_range(&r, max);
        c.sel = (a, b);
        c.sel_dirty = true;
    });
    engine.register_fn("selection_or_all", |e: &mut EditorApi| -> Map {
        let c = e.ctx.borrow();
        let (a, b) = (c.sel.0.min(c.sel.1), c.sel.0.max(c.sel.1));
        if a == b {
            range_map(0, c.text.chars().count())
        } else {
            range_map(a, b)
        }
    });
    engine.register_fn("select_all", |e: &mut EditorApi| {
        let mut c = e.ctx.borrow_mut();
        let n = c.text.chars().count();
        c.sel = (0, n);
        c.sel_dirty = true;
    });
    engine.register_fn(
        "find",
        |e: &mut EditorApi, pattern: ImmutableString, o: Map| -> Dynamic {
            let c = e.ctx.borrow();
            let (opts, wrap, from) = parse_search_opts(&o);
            match crate::search::compile(&pattern, opts)
                .ok()
                .and_then(|re| crate::search::find_next(&c.text, &re, from, false, wrap))
            {
                Some((a, b)) => Dynamic::from(range_map(a, b)),
                None => Dynamic::UNIT,
            }
        },
    );
    engine.register_fn(
        "find_all",
        |e: &mut EditorApi, pattern: ImmutableString, o: Map| -> Array {
            let c = e.ctx.borrow();
            let (opts, _, _) = parse_search_opts(&o);
            match crate::search::compile(&pattern, opts) {
                Ok(re) => crate::search::find_all(&c.text, &re, 10_000)
                    .into_iter()
                    .map(|(a, b)| Dynamic::from(range_map(a, b)))
                    .collect(),
                Err(_) => Array::new(),
            }
        },
    );
    engine.register_fn(
        "replace_all",
        |e: &mut EditorApi,
         pattern: ImmutableString,
         repl: ImmutableString,
         o: Map|
         -> i64 {
            let mut c = e.ctx.borrow_mut();
            let (opts, _, _) = parse_search_opts(&o);
            let Ok(re) = crate::search::compile(&pattern, opts) else {
                return 0;
            };
            let (new, count) =
                crate::search::replace_all(&c.text, &re, &repl, opts.regex);
            if count > 0 {
                c.text = new;
                c.text_dirty = true;
                let n = c.text.chars().count();
                c.sel = (c.sel.0.min(n), c.sel.1.min(n));
            }
            count as i64
        },
    );
    engine.register_fn("language", |e: &mut EditorApi| -> ImmutableString {
        e.ctx
            .borrow()
            .language
            .clone()
            .unwrap_or_else(|| "Plain Text".to_string())
            .into()
    });
    engine.register_fn("set_language", |e: &mut EditorApi, name: ImmutableString| {
        let lang = if name.is_empty() || name.eq_ignore_ascii_case("plain text") {
            None
        } else {
            Some(name.to_string())
        };
        e.ctx.borrow_mut().actions.push(HostAction::SetLanguage(lang));
    });

    // ---- buffers ----
    engine.register_fn("list", |b: &mut BuffersApi| -> Array {
        b.ctx
            .borrow()
            .buffers
            .iter()
            .map(|i| {
                let mut m = Map::new();
                m.insert("id".into(), Dynamic::from(i.id as i64));
                m.insert("title".into(), Dynamic::from(ImmutableString::from(i.title.as_str())));
                m.insert(
                    "path".into(),
                    match &i.path {
                        Some(p) => Dynamic::from(ImmutableString::from(p.as_str())),
                        None => Dynamic::UNIT,
                    },
                );
                m.insert("modified".into(), Dynamic::from(i.modified));
                m.insert("active".into(), Dynamic::from(i.active));
                Dynamic::from(m)
            })
            .collect()
    });
    engine.register_fn("active", |b: &mut BuffersApi| -> i64 {
        b.ctx.borrow().active_id as i64
    });
    engine.register_fn("activate", |b: &mut BuffersApi, id: i64| {
        b.ctx
            .borrow_mut()
            .actions
            .push(HostAction::ActivateBuffer(id.max(0) as u64));
    });
    engine.register_fn("open", |b: &mut BuffersApi, path: ImmutableString| {
        b.ctx
            .borrow_mut()
            .actions
            .push(HostAction::OpenFile(PathBuf::from(path.to_string())));
    });
    engine.register_fn("new", |b: &mut BuffersApi| {
        b.ctx.borrow_mut().actions.push(HostAction::NewTab);
    });
    engine.register_fn("save", |b: &mut BuffersApi| {
        b.ctx.borrow_mut().actions.push(HostAction::SaveActive);
    });
    engine.register_fn("close", |b: &mut BuffersApi, id: i64| {
        b.ctx
            .borrow_mut()
            .actions
            .push(HostAction::CloseBuffer(id.max(0) as u64));
    });
    engine.register_fn("path", |b: &mut BuffersApi, id: i64| -> Dynamic {
        let c = b.ctx.borrow();
        match c
            .buffers
            .iter()
            .find(|i| i.id == id.max(0) as u64)
            .and_then(|i| i.path.clone())
        {
            Some(p) => Dynamic::from(ImmutableString::from(p)),
            None => Dynamic::UNIT,
        }
    });

    // ---- app ----
    engine.register_fn("version", |_: &mut AppApi| -> ImmutableString {
        env!("CARGO_PKG_VERSION").into()
    });
    engine.register_fn("api_version", |_: &mut AppApi| -> i64 { API_VERSION });
    engine.register_fn("config_dir", |a: &mut AppApi| -> ImmutableString {
        a.ctx.borrow().config_dir.clone().into()
    });
    engine.register_fn("clipboard", |_: &mut AppApi| -> ImmutableString {
        arboard::Clipboard::new()
            .ok()
            .and_then(|mut c| c.get_text().ok())
            .unwrap_or_default()
            .into()
    });
    engine.register_fn("set_clipboard", |a: &mut AppApi, s: ImmutableString| {
        a.ctx
            .borrow_mut()
            .actions
            .push(HostAction::SetClipboard(s.to_string()));
    });

    // ---- ui ----
    engine.register_fn("status", |u: &mut UiApi, s: ImmutableString| {
        u.ctx.borrow_mut().status = Some(s.to_string());
    });
    engine.register_fn(
        "alert",
        |u: &mut UiApi, title: ImmutableString, body: ImmutableString| {
            u.ctx
                .borrow_mut()
                .actions
                .push(HostAction::Alert(title.to_string(), body.to_string()));
        },
    );
    engine.register_fn("toggle_markdown_preview", |u: &mut UiApi| {
        u.ctx
            .borrow_mut()
            .actions
            .push(HostAction::ToggleMarkdownPreview);
    });

    engine
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_markdown_plugin_parses_and_registers() {
        let dir = std::env::temp_dir().join(format!(
            "notey-md-plugin-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("markdown_preview.rhai");
        std::fs::write(&path, MARKDOWN_PREVIEW_SCRIPT).unwrap();

        let script = PluginHost::load_script(&path).expect("bundled script loads");
        assert_eq!(script.name, "Markdown Preview");
        assert_eq!(script.commands.len(), 1);
        assert_eq!(script.commands[0].id, "toggle_md");
        assert_eq!(
            script.commands[0].shortcut,
            parse_shortcut("Ctrl+Shift+M")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn markdown_toggle_action_reaches_host_via_real_dispatch() {
        // exercises the exact runtime path: bare-engine-compiled AST from
        // disk, run_command with call options, error swallowing and all
        let dir = std::env::temp_dir().join(format!(
            "notey-md-dispatch-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("markdown_preview.rhai");
        std::fs::write(&path, MARKDOWN_PREVIEW_SCRIPT).unwrap();
        let script = PluginHost::load_script(&path).expect("loads");
        let mut host = PluginHost {
            scripts: vec![script],
            errors: Vec::new(),
        };
        let ctx = ScriptCtx {
            text: String::new(),
            sel: (0, 0),
            text_dirty: false,
            sel_dirty: false,
            buffers: Vec::new(),
            active_id: 1,
            language: Some("Markdown".to_string()),
            config_dir: String::new(),
            actions: Vec::new(),
            status: None,
        };
        let out = host.run_command(0, "toggle_md", ctx);
        assert!(host.errors.is_empty(), "plugin errors: {:?}", host.errors);
        assert!(
            matches!(out.actions.as_slice(), [HostAction::ToggleMarkdownPreview]),
            "expected toggle action; status was {:?}",
            out.status
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
