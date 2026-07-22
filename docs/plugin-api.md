# Notey Plugin API — Design Sketch (v0)

> **Implementation status (July 2026):** Tier 1 (Rhai scripts) is live in
> `src/plugins.rs`. Implemented: registration manifest (name/min_api/
> commands/shortcuts/events), `notey.editor` text/selection subset,
> `notey.buffers`, `notey.app` (version/config_dir/clipboard),
> `notey.ui.status` / `alert`, and events `ready`, `buffer_opened`,
> `buffer_activated`, `before_save`, `after_save`. Not yet implemented:
> `line_range`, pos/line conversions, undo grouping, `find*`/`replace_all`,
> encoding/line-ending setters, `confirm`/`prompt`/`form`/panels,
> `selection_changed`/`text_changed`/`theme_changed` events, capabilities
> beyond the default grant, and the WASM tier.

One API, two bindings: **Rhai scripts** (`scripts/`, hot-reloaded, sandboxed by
construction) and **WASM plugins** (`plugins/`, wasmtime, capability-gated).
Both speak the same vocabulary; a WASM plugin imports these as host functions,
a script calls them as `notey.*`.

Design lineage: Notepad++'s three surfaces — command registration
(`getFuncsArray`), notifications (`beNotified`), editor control
(`NPPM_*`/`SCI_*`) — carried over; Win32 transport, implicit ABI, and
unsandboxed trust replaced.

## Conventions

- **Positions** are character offsets (Unicode scalars), not bytes; `Range` is
  `{ start, end }` with `start <= end`. Line/column helpers convert.
- **Buffers** are identified by `buffer_id` (stable for the tab's lifetime).
  Editor calls operate on the **active buffer** unless a `buffer_id` is given.
- **Errors**: every call returns a result; a failing plugin can never crash or
  wedge the host. Plugin errors land in a "Plugin output" panel + status bar.
- **api_version**: single integer, bumped on breaking change. Manifests
  declare `min_api`. Current: `1`.

## Registration (manifest)

Rhai: a `register()` function returning a map. WASM: an exported `register()`
returning the same shape as JSON. Commands become Plugins-menu entries — the
`FuncItem[]` analog.

```rhai
fn register() {
    #{
        name: "Sort Lines",
        version: "1.0.0",
        min_api: 1,
        commands: [
            #{ id: "sort_asc",  title: "Sort Lines Ascending", shortcut: "Ctrl+Alt+S" },
            #{ id: "sort_desc", title: "Sort Lines Descending" },
        ],
        events: ["after_save"],          // opt-in subscriptions only
        capabilities: [],                 // e.g. ["fs.read", "fs.write"]
    }
}

fn on_command(id) {
    if id == "sort_asc" {
        let sel = notey.editor.selection_or_all();
        let lines = notey.editor.get_range(sel).split('\n');
        lines.sort();
        notey.editor.replace_range(sel, lines.join("\n"));
        notey.ui.status(`Sorted ${lines.len()} lines`);
    }
}

fn on_event(ev) { /* ev.kind, ev.buffer_id, ... */ }
```

## `notey.editor` — the SCI_* analog

Text:

| Call | Notes |
|---|---|
| `get_text() -> String` / `set_text(s)` | whole buffer |
| `length() -> int`, `line_count() -> int` | |
| `get_range(range) -> String` | |
| `replace_range(range, s)` | single undo step |
| `insert(pos, s)`, `append(s)` | |
| `line_text(n) -> String`, `line_range(n) -> Range` | |
| `pos_to_line_col(pos) -> (line, col)`, `line_col_to_pos(line, col) -> pos` | |

Selection & cursor (multi-cursor-ready: plural forms exist from day one, even
while the editor core only supports one):

| Call | Notes |
|---|---|
| `cursor() -> pos`, `set_cursor(pos)` | |
| `selection() -> Range`, `set_selection(range)` | primary selection |
| `selections() -> [Range]`, `set_selections([ranges])` | multi-cursor |
| `selection_or_all() -> Range` | common plugin idiom |
| `select_all()` | |

Editing & view:

| Call | Notes |
|---|---|
| `undo()`, `redo()`, `cut()`, `copy()`, `paste()` | |
| `begin_undo_group()` / `end_undo_group()` | compound actions = one undo |
| `goto_line(n)`, `scroll_to(pos)` | |
| `zoom() -> f32`, `set_zoom(f32)` | |

Search (host-side `fancy-regex`; plugins never ship their own regex engine):

| Call | Notes |
|---|---|
| `find(pattern, opts) -> Range?` | `opts: { regex, match_case, wrap, from }` |
| `find_all(pattern, opts) -> [Range]` | |
| `replace_all(pattern, repl, opts) -> int` | returns replacement count |

Document metadata:

| Call | Notes |
|---|---|
| `is_modified() -> bool` | |
| `encoding() -> String`, `set_encoding(s)` | "UTF-8", "UTF-16 LE", … |
| `line_ending() -> String`, `set_line_ending(s)` | "CRLF" / "LF" |
| `language() -> String`, `set_language(s)` | reserved until syntax lands |

Reserved for the editor core's later phases (names stable now, calls error
with "unsupported" until then): `fold(line)`, `unfold(line)`,
`add_marker(line, kind)`, `clear_markers(kind)`.

## `notey.buffers` / `notey.app` — the NPPM_* analog

| Call | N++ ancestor |
|---|---|
| `buffers.list() -> [{id, title, path, modified, active}]` | NPPM_GETNBOPENFILES et al |
| `buffers.active() -> id` | NPPM_GETCURRENTBUFFERID |
| `buffers.activate(id)` | NPPM_ACTIVATEDOC |
| `buffers.open(path) -> id` | NPPM_DOOPEN |
| `buffers.new() -> id` | NPPM_MENUCOMMAND(IDM_FILE_NEW) |
| `buffers.save(id?, path?) -> bool` | NPPM_SAVECURRENTFILE |
| `buffers.close(id, force?) -> bool` | NPPM_MENUCOMMAND(IDM_FILE_CLOSE) |
| `buffers.path(id) -> String?` | NPPM_GETFULLPATHFROMBUFFERID |
| `app.version() -> String`, `app.api_version() -> int` | NPPM_GETNPPVERSION |
| `app.config_dir() -> String` | NPPM_GETPLUGINSCONFIGDIR |
| `app.clipboard() -> String`, `app.set_clipboard(s)` | |

## `notey.ui` — declarative only

| Call | Notes |
|---|---|
| `status(msg)` | status-bar flash |
| `alert(title, body)` | |
| `confirm(title, body) -> bool` | |
| `prompt(label, default?) -> String?` | |
| `form(schema) -> map?` | host-rendered dialog: `text`, `number`, `checkbox`, `select` fields — always in the app theme |
| `panel(id, title)` → handle: `set_text(md)`, `set_items([...])`, `on_item_click` | dockable panel (egui_dock), content as data |
| `set_command_enabled(id, bool)` | grey out own menu entries |

Deliberately absent: raw drawing, widget access, window handles. Plugin UI is
data; the host renders it in the current Fluent style. If a plugin genuinely
needs custom rendering someday, that becomes a separate, explicitly unstable
API.

## Events — the beNotified analog

Subscribed in the manifest; delivered as `{ kind, buffer_id?, ...payload }`.

| Event | Payload extras | N++ ancestor |
|---|---|---|
| `ready` | | NPPN_READY |
| `shutdown` | | NPPN_SHUTDOWN |
| `buffer_opened` / `buffer_closed` | `path?` | NPPN_FILEOPENED / FILECLOSED |
| `buffer_activated` | | NPPN_BUFFERACTIVATED |
| `before_save` | mutations applied before write | NPPN_FILEBEFORESAVE |
| `after_save` | `path` | NPPN_FILESAVED |
| `file_changed_on_disk` | `path` | NPPN_FILEDELETED etc. |
| `selection_changed` | `range` — throttled | SCN_UPDATEUI |
| `text_changed` | `revision` — batched per frame, never per keystroke | SCN_MODIFIED |
| `theme_changed` | `dark: bool` | NPPN_DARKMODECHANGED |

Reentrancy rule: event handlers may call any API; mutations from `before_save`
are applied before the file hits disk; everything else queues and applies at
the frame boundary.

## Capabilities & trust

Default grant: editor + buffers + ui + events — pure text manipulation needs
no permissions. Everything else is opt-in, declared in the manifest, shown to
the user at install time in Plugins Admin, revocable there:

- `fs.read` / `fs.write` — file access beyond open buffers (optionally scoped to paths)
- `net` — HTTP requests via a host-provided fetch (no raw sockets)
- `process` — spawn external programs (WASM tier only, never scripts)

## Distribution

`notey-plugin-list`: a signed JSON registry (name, version, min_api,
capabilities, artifact URL + hash) — the nppPluginList model — behind a
Plugins Admin dialog with in-app install/update/remove.

## Non-goals

- Binary or source compatibility with existing Notepad++ plugins.
- Plugin-drawn UI (declarative only, v1).
- Synchronous per-keystroke plugin hooks (batched `text_changed` only).
