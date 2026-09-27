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

/// 2: command/key `when` conditions, key hooks, input/text_changed/
/// selection_changed events, status items, preview modes, notey.text,
/// notey.fs (capability-gated), notey.app.open/temp_path.
pub const API_VERSION: i64 = 2;

/// Permissions a script must declare (and, for installed plugins, the
/// package must grant) before it can use the matching API.
pub const CAP_FS_WRITE: &str = "fs.write";
pub const CAP_OPEN: &str = "open";

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
    /// "off", "rendered" or "split".
    SetPreview(String),
    /// Open a file or URL with the system's default app.
    OpenPath(PathBuf),
    /// Status bar item (key is unique per script): text, tooltip.
    SetStatusItem(String, String, String),
    ClearStatusItem(String),
}

/// Condition under which a command or key hook is active.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct When {
    /// Active document's language ("Plain Text" when none).
    pub language: Option<String>,
    /// Only while the editor has keyboard focus.
    pub editor_focus: bool,
}

impl When {
    fn parse(v: Option<&Dynamic>) -> Self {
        let Some(m) = v.and_then(|v| v.clone().try_cast::<Map>()) else {
            return Self::default();
        };
        Self {
            language: m.get("language").and_then(|v| v.clone().into_string().ok()),
            editor_focus: m
                .get("focus")
                .and_then(|v| v.clone().into_string().ok())
                .is_some_and(|f| f == "editor"),
        }
    }

    pub fn matches(&self, language: Option<&str>, editor_focused: bool) -> bool {
        let lang_ok = self
            .language
            .as_deref()
            .is_none_or(|want| want == language.unwrap_or("Plain Text"));
        lang_ok && (!self.editor_focus || editor_focused)
    }
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
    /// Active tab's preview mode ("off", "rendered", "split").
    pub preview: String,
    /// Set by the host for the running script.
    pub script_name: String,
    pub capabilities: Vec<String>,
}

impl ScriptCtx {
    fn has(&self, cap: &str) -> bool {
        self.capabilities.iter().any(|c| c == cap)
    }
}

type Ctx = Rc<RefCell<ScriptCtx>>;

// ---------- script + command metadata ----------

pub struct PluginCommand {
    pub id: String,
    pub title: String,
    pub shortcut: Option<KeyboardShortcut>,
    pub shortcut_text: String,
    pub when: When,
}

pub struct Script {
    pub name: String,
    pub path: PathBuf,
    pub ast: Rc<AST>,
    pub commands: Vec<PluginCommand>,
    pub events: HashSet<String>,
    /// Unmodified keys the script handles in `on_key` (e.g. Enter).
    pub keys: Vec<(Key, When)>,
    /// Permissions in effect: declared by the script and, for installed
    /// plugins, granted by the package manifest.
    pub capabilities: Vec<String>,
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

    /// Load the user's own scripts plus the entry scripts of enabled
    /// feature plugins (`(script path, capabilities the package grants)`).
    pub fn load(feature_scripts: &[(PathBuf, Vec<String>)]) -> Self {
        let mut host = Self::default();
        // (path, granted): None = the user's own script, trusted as written
        let mut paths: Vec<(PathBuf, Option<Vec<String>>)> = Vec::new();
        if let Some(dir) = Self::scripts_dir() {
            let _ = std::fs::create_dir_all(&dir);
            if let Ok(entries) = std::fs::read_dir(&dir) {
                paths.extend(
                    entries
                        .filter_map(Result::ok)
                        .map(|e| e.path())
                        .filter(|p| p.extension().map(|e| e == "rhai").unwrap_or(false))
                        .map(|p| (p, None)),
                );
            }
        }
        paths.sort_by(|a, b| a.0.cmp(&b.0));
        paths.extend(feature_scripts.iter().map(|(p, caps)| (p.clone(), Some(caps.clone()))));
        for (path, granted) in paths {
            match Self::load_script(&path, granted.as_deref()) {
                Ok(script) => host.scripts.push(script),
                Err(e) => host.errors.push(format!(
                    "{}: {e}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )),
            }
        }
        host
    }

    fn load_script(path: &Path, granted: Option<&[String]>) -> Result<Script, String> {
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
                    when: When::parse(m.get("when")),
                });
            }
        }

        let strings = |key: &str| -> Vec<String> {
            manifest
                .get(key)
                .and_then(|v| v.clone().try_cast::<Array>())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|v| v.into_string().ok())
                .collect()
        };
        let events: HashSet<String> = strings("events").into_iter().collect();

        let mut keys = Vec::new();
        for item in manifest
            .get("keys")
            .and_then(|v| v.clone().try_cast::<Array>())
            .unwrap_or_default()
        {
            let Some(m) = item.try_cast::<Map>() else { continue };
            let name = m.get("key").and_then(|v| v.clone().into_string().ok()).unwrap_or_default();
            if let Some(key) = key_from_name(&name.to_ascii_lowercase()) {
                keys.push((key, When::parse(m.get("when"))));
            }
        }

        let declared = strings("capabilities");
        let capabilities = match granted {
            None => declared,
            Some(granted) => declared.into_iter().filter(|c| granted.contains(c)).collect(),
        };

        Ok(Script {
            name,
            path: path.to_path_buf(),
            ast: Rc::new(ast),
            commands,
            events,
            keys,
            capabilities,
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

    /// Earlier versions seeded `markdown_preview.rhai` into the scripts
    /// folder; it is now the official Markdown Tools plugin. Remove the old
    /// copy so its shortcut isn't registered twice — but only if the user
    /// never edited it.
    pub fn remove_legacy_markdown_script() {
        let Some(dir) = Self::scripts_dir() else { return };
        let path = dir.join("markdown_preview.rhai");
        let unmodified = std::fs::read_to_string(&path)
            .map(|s| s.replace("\r\n", "\n") == LEGACY_MARKDOWN_SCRIPT)
            .unwrap_or(false);
        if unmodified {
            let _ = std::fs::remove_file(path);
        }
    }


    /// Run `on_command` in the given script. Returns the mutated context.
    pub fn run_command(&mut self, script_idx: usize, cmd_id: &str, ctx: ScriptCtx) -> ScriptCtx {
        self.run_fn(script_idx, "on_command", vec![Dynamic::from(ImmutableString::from(cmd_id))], ctx).0
    }

    /// Run `on_event` with an event map in every subscribed script.
    pub fn run_event(&mut self, script_idx: usize, event: Map, ctx: ScriptCtx) -> ScriptCtx {
        self.run_fn(script_idx, "on_event", vec![Dynamic::from(event)], ctx).0
    }

    /// Run `on_key(key)`; returns true if the script handled the key (the
    /// host then consumes it so the editor never sees it).
    pub fn run_key(&mut self, script_idx: usize, key: &str, ctx: ScriptCtx) -> (ScriptCtx, bool) {
        let (ctx, ret) =
            self.run_fn(script_idx, "on_key", vec![Dynamic::from(ImmutableString::from(key))], ctx);
        (ctx, ret.and_then(|v| v.as_bool().ok()).unwrap_or(false))
    }

    fn run_fn(
        &mut self,
        script_idx: usize,
        fn_name: &str,
        args: Vec<Dynamic>,
        mut ctx: ScriptCtx,
    ) -> (ScriptCtx, Option<Dynamic>) {
        let (ast, name) = {
            let s = &self.scripts[script_idx];
            (s.ast.clone(), s.name.clone())
        };
        ctx.script_name = name.clone();
        ctx.capabilities = self.scripts[script_idx].capabilities.clone();
        let shared: Ctx = Rc::new(RefCell::new(ctx));
        let engine = api_engine(shared.clone()); // resolves `notey` (see on_var)
        let mut scope = Scope::new();
        let result: Result<Dynamic, _> =
            engine.call_fn_with_options(
                rhai::CallFnOptions::new().eval_ast(false),
                &mut scope,
                &ast,
                fn_name,
                args,
            );
        let ret = match result {
            Ok(v) => Some(v),
            Err(e) => {
                // A script may simply not define this entry point (e.g. no
                // `on_event`); that's fine. Any other failure — including a
                // call to an API function that doesn't exist — is reported.
                let missing_entry = matches!(
                    &*e,
                    rhai::EvalAltResult::ErrorFunctionNotFound(sig, _)
                        if sig.split(['(', ' ']).next() == Some(fn_name)
                );
                if !missing_entry {
                    self.errors.push(format!("{name}: {fn_name}: {e}"));
                }
                None
            }
        };
        // Both the engine's registered closures AND the scope's `notey`
        // constant hold Rc clones of the context; drop them before
        // unwrapping, or the fallback path would discard queued actions.
        drop(engine);
        drop(scope);
        let ctx = match Rc::try_unwrap(shared) {
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
                    preview: c.preview.clone(),
                    script_name: c.script_name.clone(),
                    capabilities: c.capabilities.clone(),
                }
            }
        };
        (ctx, ret)
    }
}

/// The script earlier versions seeded as `markdown_preview.rhai`, kept only
/// to recognize (and remove) an unmodified legacy copy.
const LEGACY_MARKDOWN_SCRIPT: &str = r#"// Notey bundled plugin: Markdown raw/rendered toggle.
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
        "enter" => Enter, "space" => Space, "tab" => Tab, "escape" => Escape,
        "backspace" => Backspace, "delete" => Delete,
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
#[derive(Clone)]
struct TextApi;
#[derive(Clone)]
struct FsApi {
    ctx: Ctx,
}

type ApiResult<T> = Result<T, Box<rhai::EvalAltResult>>;

fn denied(cap: &str) -> Box<rhai::EvalAltResult> {
    format!("permission '{cap}' is required (declare it in register() capabilities)").into()
}

/// Count words the way word processors do: whitespace-separated tokens
/// containing at least one letter or digit, so list bullets, `#`, `>` and
/// stray punctuation don't count.
pub fn word_count(s: &str) -> usize {
    s.split_whitespace()
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .count()
}

/// Render Markdown to an HTML fragment (CommonMark + tables, strikethrough,
/// task lists).
pub fn markdown_to_html(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut out = String::new();
    html::push_html(&mut out, Parser::new_ext(md, opts));
    out
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
    set_expression_limits(&mut engine);
    engine
}

/// Rhai's defaults reject ordinary code such as a chain of `&&` conditions
/// or nested `if` expressions; allow real-world scripts while still bounding
/// parser recursion.
fn set_expression_limits(engine: &mut Engine) {
    engine.set_max_expr_depths(256, 128);
}

/// Engine with the full notey.* API bound to `ctx`.
fn api_engine(ctx: Ctx) -> Engine {
    let mut engine = Engine::new();
    engine.set_max_operations(50_000_000); // generous, but no infinite loops
    set_expression_limits(&mut engine);

    // Resolve `notey` everywhere, including inside a script's own helper
    // functions: Rhai functions can't see the caller's scope, so a scope
    // variable would only work in the entry point.
    #[allow(deprecated)] // "volatile" API, not deprecated; rhai is pinned in Cargo.lock
    engine.on_var(move |name, _, _| {
        Ok((name == "notey").then(|| Dynamic::from(NoteyApi { ctx: ctx.clone() })))
    });

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

    // ---- ui (API 2): preview modes, status items, save dialog ----
    engine.register_fn("preview", |u: &mut UiApi| -> ImmutableString {
        u.ctx.borrow().preview.clone().into()
    });
    engine.register_fn("set_preview", |u: &mut UiApi, mode: ImmutableString| -> ApiResult<()> {
        if !matches!(mode.as_str(), "off" | "rendered" | "split") {
            return Err(format!("unknown preview mode '{mode}' (use off, rendered or split)").into());
        }
        let mut c = u.ctx.borrow_mut();
        c.preview = mode.to_string();
        c.actions.push(HostAction::SetPreview(mode.to_string()));
        Ok(())
    });
    fn status_key(c: &ScriptCtx, id: &str) -> String {
        format!("{}::{id}", c.script_name)
    }
    engine.register_fn(
        "set_status_item",
        |u: &mut UiApi, id: ImmutableString, text: ImmutableString| {
            let mut c = u.ctx.borrow_mut();
            let key = status_key(&c, &id);
            c.actions.push(HostAction::SetStatusItem(key, text.to_string(), String::new()));
        },
    );
    engine.register_fn(
        "set_status_item",
        |u: &mut UiApi, id: ImmutableString, text: ImmutableString, tooltip: ImmutableString| {
            let mut c = u.ctx.borrow_mut();
            let key = status_key(&c, &id);
            c.actions.push(HostAction::SetStatusItem(key, text.to_string(), tooltip.to_string()));
        },
    );
    engine.register_fn("clear_status_item", |u: &mut UiApi, id: ImmutableString| {
        let mut c = u.ctx.borrow_mut();
        let key = status_key(&c, &id);
        c.actions.push(HostAction::ClearStatusItem(key));
    });
    engine.register_fn(
        "save_file_dialog",
        |_: &mut UiApi, default_name: ImmutableString| -> ImmutableString {
            rfd::FileDialog::new()
                .set_file_name(default_name.as_str())
                .save_file()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
                .into()
        },
    );

    // ---- editor (API 2) ----
    engine.register_fn("line_range_at", |e: &mut EditorApi, pos: i64| -> Map {
        let c = e.ctx.borrow();
        let chars: Vec<char> = c.text.chars().collect();
        let p = (pos.max(0) as usize).min(chars.len());
        let start = chars[..p].iter().rposition(|&ch| ch == '\n').map(|i| i + 1).unwrap_or(0);
        let end = chars[p..].iter().position(|&ch| ch == '\n').map(|i| p + i).unwrap_or(chars.len());
        range_map(start, end)
    });

    // ---- text helpers (API 2): fast host implementations ----
    engine.register_type::<TextApi>();
    engine.register_get("text", |_: &mut NoteyApi| TextApi);
    engine.register_fn("word_count", |_: &mut TextApi, s: ImmutableString| -> i64 {
        word_count(&s) as i64
    });
    engine.register_fn("char_count", |_: &mut TextApi, s: ImmutableString| -> i64 {
        s.chars().count() as i64
    });
    engine.register_fn("markdown_to_html", |_: &mut TextApi, s: ImmutableString| -> ImmutableString {
        markdown_to_html(&s).into()
    });

    // ---- file system (API 2, capability-gated) ----
    engine.register_type::<FsApi>();
    engine.register_get("fs", |n: &mut NoteyApi| FsApi { ctx: n.ctx.clone() });
    engine.register_fn(
        "write_text",
        |f: &mut FsApi, path: ImmutableString, text: ImmutableString| -> ApiResult<()> {
            if !f.ctx.borrow().has(CAP_FS_WRITE) {
                return Err(denied(CAP_FS_WRITE));
            }
            std::fs::write(path.as_str(), text.as_bytes())
                .map_err(|e| format!("could not write {path}: {e}").into())
        },
    );

    // ---- app (API 2) ----
    engine.register_fn("open", |a: &mut AppApi, path: ImmutableString| -> ApiResult<()> {
        let mut c = a.ctx.borrow_mut();
        if !c.has(CAP_OPEN) {
            return Err(denied(CAP_OPEN));
        }
        c.actions.push(HostAction::OpenPath(PathBuf::from(path.as_str())));
        Ok(())
    });
    engine.register_fn("temp_path", |_: &mut AppApi, name: ImmutableString| -> ImmutableString {
        // keep only a plain file name inside Notey's temp folder
        let safe: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
            .collect();
        let dir = std::env::temp_dir().join("Notey");
        let _ = std::fs::create_dir_all(&dir);
        dir.join(safe.trim_start_matches('.')).display().to_string().into()
    });

    engine
}

#[cfg(test)]
#[path = "plugins_tests.rs"]
mod tests;

