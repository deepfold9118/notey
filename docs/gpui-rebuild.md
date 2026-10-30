# Rebuilding Notey on GPUI and codemirror-rs

The `rebuild/gpui` branch rebuilds Notey on [GPUI](https://crates.io/crates/gpui)
(Zed's UI framework) and the Rust port of CodeMirror 6 in
`D:\Projects\codemirror-rs` (see [codemirror-rust-design.md](codemirror-rust-design.md)
§10). The egui version stays on `main` until the rebuild reaches parity.

Build: `cargo build` needs `codemirror-rs` next to this repository
(`../codemirror-rs`), because Notey depends on its crates by path.

## Layout

| File | What it does |
|---|---|
| `src/main.rs` | Crash log, arguments, single-instance forwarding, the GPUI window |
| `src/app.rs` | `NoteyApp`: tabs (one `EditorView` each), files, session, actions and keys |
| `src/app/chrome.rs` | Title bar with tabs and caption buttons, menu bar and menus, status bar |
| `src/app/find.rs` | The find and replace bar and the Go To bar |
| `src/app/settings_tab.rs` | The Settings tab |
| `src/settings.rs` | Settings as `settings.json`; imports the egui version's `app.ron` once |
| `src/theme.rs` | Fluent palettes and the editor theme made from them |
| `src/doc.rs`, `src/session.rs`, `src/ipc.rs` | Carried over from `main` (doc.rs now keeps only the saved text; the editor state holds the live text) |

Data stays in `%APPDATA%\Notey\data`, so the session and settings carry over
from the egui version. `NOTEY_DATA_DIR` points a run at another folder and
gives it its own single-instance socket, for testing beside an everyday
Notey.

## Done

- Frameless window with Notey's title bar: icon, tabs (modified dot, close
  button, middle-click close), new-tab button, Windows caption buttons
  (snap layouts work, as they are real caption hit-test areas), drag and
  double-click maximize.
- Menu bar (File, Edit, Help) with drop-down menus, Open Recent, and the
  icon menu when the menu bar is hidden.
- Editor: the CodeMirror view per tab, with history, the default keymap,
  Tab and Shift+Tab, word wrap, line numbers, tab size, font size, line
  height, and zoom, reconfigured live through one compartment.
- Files: Open (multiple), Save, Save As, encodings and line endings detected
  and kept, close prompts for unsaved tabs, drag and drop onto the window.
- Session restore and a save every 30 s in the primary window; secondary
  windows ask before discarding unsaved tabs.
- Single-instance forwarding over the named pipe.
- Status bar: line and column, characters, zoom, line endings and encoding
  pickers, flash messages.
- Settings tab: theme, menu bar, status bar, word wrap, line numbers, font
  size, line height, tab size.
- Find and Replace on codemirror-search: a bar over the top right of the
  editor (Ctrl+F, Ctrl+H, F3, Shift+F3, Escape) with match case, whole word
  and regular expressions (look-around, backreferences, `$1` in the
  replacement), Replace and Replace all, a match count, and highlighted
  matches. Its fields are small CodeMirror editors. The query is kept per
  tab and follows you to the next tab while the bar is open. Go To line
  (Ctrl+G) also takes `+n`, `-n`, `n%` and `line:column`.
- Keys: Ctrl+N, Ctrl+Shift+N, Ctrl+O, Ctrl+S, Ctrl+Shift+S, Ctrl+W,
  Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+= / Ctrl+- / Ctrl+0, F5, Ctrl+,, Ctrl+F,
  Ctrl+H, F3, Shift+F3 and Ctrl+G. In the editor, Ctrl+D selects the next
  occurrence of the selection and Ctrl+Shift+L all of them.

## To port from `main`

Get an old module with `git show main:src/<file>`.

- Find in Files (`search.rs` carries over as is).
- Spellcheck (`spell.rs` carries over; underlines as mark decorations,
  suggestions in a context menu).
- Syntax highlighting (`syntax.rs`, syntect) until the port's language
  package decides the parser; the language picker in the status bar.
- Plugins (`plugins.rs`, `plugin_manager.rs`, `packages.rs`, Rhai scripts,
  the plugin API in `docs/plugin-api.md`), Markdown preview, Text Color.
- Themes: imported VS Code and TextMate themes and the theme marketplace
  (`theme.rs`, `theme_marketplace.rs`).
- Settings still missing: editor font picker, ligatures, autosave, tabs on
  the left, spellcheck and plugin sections.
- File watching and the reload prompt, Print, the right-click menu.
- Session end: `shutdown.rs` handled `WM_ENDSESSION` for winit. Under GPUI
  the session is saved every 30 s and on close; saving on Windows shutdown
  still needs a hook.
- Window size and position memory.
