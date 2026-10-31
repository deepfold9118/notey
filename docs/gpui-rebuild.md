# Rebuilding Notey on GPUI and codemirror-rs

Since 0.8.0, Notey is built on [GPUI](https://crates.io/crates/gpui)
(Zed's UI framework) and the Rust port of CodeMirror 6 in `codemirror-rs`
(see [codemirror-rust-design.md](codemirror-rust-design.md) §10). The egui
version (0.7.x) is kept on the `archive/egui-0.7` branch.

Build: `cargo build` needs `codemirror-rs` next to this repository
(`../codemirror-rs`), because Notey depends on its crates by path.

## Layout

| File | What it does |
|---|---|
| `src/main.rs` | Crash log, arguments, single-instance forwarding, the GPUI window |
| `src/app.rs` | `NoteyApp`: tabs (one `EditorView` each), files, session, actions and keys |
| `src/app/chrome.rs` | Title bar with tabs and caption buttons, menu bar and menus, status bar |
| `src/app/find.rs` | The find and replace bar and the Go To bar |
| `src/app/find_in_files.rs` | The Find in Files panel |
| `src/app/watch.rs` | File watching and reload, autosave, window geometry |
| `src/a11y.rs`, `src/app/accessibility.rs` | The screen reader (UI Automation) tree and its actions |
| `src/prompt.rs` | In-window Fluent dialogs for `window.prompt` |
| `src/search.rs`, `src/shutdown.rs` | Carried over: the folder grep, and saving at Windows shutdown |
| `src/app/settings_tab.rs` | The Settings tab |
| `src/app/spelling.rs` | Loading the dictionaries, and the editor's right-click menu with suggestions |
| `src/app/languages.rs` | Building the grammar set from the File Type plugins, and the language picker |
| `src/highlight.rs` | The view plugin that colors the visible lines from a syntect parse cache |
| `src/syntax.rs` | Carried over: the syntect grammar set, language detection and the per-line parse cache |
| `src/spellcheck.rs` | The view plugin that underlines misspelled words in the visible lines |
| `src/spell.rs`, `src/packages.rs` | Carried over: the Hunspell checker (spellbook), and the plugin store that finds the installed Spellcheck and Language plugins |
| `src/app/themes.rs` | Notey Dark and Light, importing a theme file, and vscodethemes.com downloads |
| `src/app/plugins_ui.rs` | Settings > Plugins: install, update, turn on or off and remove plugins |
| `src/app/scripting.rs` | Running Rhai scripts: commands, shortcuts, key hooks, events, results |
| `src/app/preview.rs`, `src/markdown.rs` | The Markdown preview (rendered or side by side), drawn from pulldown-cmark |
| `src/styles.rs` | Plugin text colors (Text Color), a state field drawn as marks |
| `src/plugins.rs`, `src/theme_import.rs`, `src/theme_marketplace.rs` | Carried over: the Rhai plugin host and API 3, the VS Code/TextMate theme loader, and the Marketplace download |
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
- Settings tab with its sections (Editor, Appearance, Spellcheck, Plugins)
  and Reset to defaults: a font picker with a filter, font size, line
  height, tab size, word wrap, line numbers, ligatures, autosave; the theme,
  tabs at the top or on the left, menu bar, status bar; spellcheck, its
  languages and the personal word list.
- Themes: Notey Dark and Light, VS Code (JSON/JSONC) and TextMate theme
  files, and themes downloaded from a vscodethemes.com URL, across the whole
  window and the syntax colors.
- Tabs on the left: a vertical list with the new-tab button and the gear.
- Plugins: Settings > Plugins installs, updates, turns on or off and removes
  plugins from the registry (Features, Languages, File Types), and reloads
  what a change affects. The status bar offers the File Type plugin for a
  file it can't highlight.
- Rhai scripts (plugin API 3, `docs/plugin-api.md`): commands in the Edit
  menu (groups as submenus, color swatches) and in the right-click menu,
  shortcuts, key hooks (`on_key`, with the egui version's key names), the
  events (`ready`, `buffer_opened`, `buffer_activated`, `before_save`,
  `after_save`, `input`, `text_changed`, `selection_changed`), status bar
  items with tooltips, and the host actions. The official Features work:
  Word Count, Autocorrect, Markdown Tools (shortcuts, list continuation,
  preview), Text Color, Spellcheck.
- Markdown preview: rendered in place of the editor, or side by side.
- Text Color: plugin colors that move with edits and win over syntax colors.
- Tooltips on the status bar items, the gear, the new-tab buttons and plugin
  commands (the script behind them).
- The language picker has a filter field and a link to More file types;
  the tab list on the left can be resized by its right edge.
- Find and Replace on codemirror-search: a bar over the top right of the
  editor (Ctrl+F, Ctrl+H, F3, Shift+F3, Escape) with match case, whole word
  and regular expressions (look-around, backreferences, `$1` in the
  replacement), Replace and Replace all, a match count, and highlighted
  matches. Its fields are small CodeMirror editors. The query is kept per
  tab and follows you to the next tab while the bar is open. Go To line
  (Ctrl+G) also takes `+n`, `-n`, `n%` and `line:column`.
- Spellcheck: wavy red underlines, from the enabled Language plugins when
  the Spellcheck plugin is installed (the plugins already in
  `%APPDATA%\Notey\plugins` work as they are). Right-click a word for
  suggestions and Add to dictionary (`user_dictionary.txt`, shared with
  the egui version); Cut, Copy, Paste and Select All are below them. On or
  off in Settings > Editor, which lists the loaded languages. Words with
  digits (`2nd`) are no longer split and flagged as `nd`.
- Syntax highlighting with syntect, from the enabled File Type plugins (the
  grammar cache in `plugins\.cache` is shared with the egui version). The
  language is detected from the file extension and can be changed in the
  status bar's language picker; it is saved in the session. Only the lines
  down to the bottom of the screen are parsed, and an edit re-parses from
  the edited line.
- Find in Files (Ctrl+Shift+F): a panel below the editor with the pattern,
  folder (with Browse), file filters, match case and regex; results stream
  in (up to 2000, .gitignore respected) and open at their line.
- Print (Ctrl+P) hands the saved file to the Windows print verb, as before.
- Files changed by another program ask to reload (every 3 s); deleted files
  are reported. Autosave, with its interval, in Settings > Editor.
- The window reopens at its last position and size (or maximized).
- Windows shutdown and installer upgrades save the session (`shutdown.rs`
  sends `WM_CLOSE` to GPUI on `WM_ENDSESSION`).
- Dialogs are drawn in the window in the Fluent style.
- Screen readers: GPUI 0.2 has no accessibility support, so Notey builds
  its own AccessKit tree (`a11y.rs`, `app/accessibility.rs`) and serves it
  through UI Automation by subclassing the window (the window opens hidden
  until the adapter is attached). It has the tabs, caption buttons, menus
  and their items, the editor as a multiline text input (text, a run per
  line, the selection), the find bar, the status bar, the Settings
  sections with checkboxes and spin buttons, and dialogs; screen readers
  can click, focus, select text and step values. The egui version's
  `NOTEY_ACCESSKIT` switch is not needed: the tree starts when a screen
  reader first asks.
- Keys: Ctrl+N, Ctrl+Shift+N, Ctrl+O, Ctrl+S, Ctrl+Shift+S, Ctrl+P, Ctrl+W,
  Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+= / Ctrl+- / Ctrl+0, F5, Ctrl+,, Ctrl+F,
  Ctrl+H, F3, Shift+F3 and Ctrl+G. In the editor, Ctrl+D selects the next
  occurrence of the selection and Ctrl+Shift+L all of them.
