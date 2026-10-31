# Notey

A native Rust notepad for Windows with dark mode, tabs, and spellcheck.
Built with [GPUI](https://crates.io/crates/gpui) (the UI framework of the
Zed editor), a Rust port of [CodeMirror 6](https://codemirror.net/) for the
editor, and [spellbook](https://github.com/helix-editor/spellbook)
(Hunspell-compatible) for spellcheck.

![Notey with three tabs open, showing syntax-highlighted Rust code in the dark Fluent theme](docs/screenshot.png)

The UI is styled after Fluent / WinUI (Windows 11 Notepad): a frameless
window with a custom title bar containing the tab strip and caption
buttons, Fluent dark and light palettes (`src/theme.rs`), Segoe UI
Variable, rounded menus and dialogs, and an accent-colored selection.
Dialogs (save changes, reload, errors) are drawn inside the window in the
same style.

By default, Notey shows a menu bar with the File, Edit, and Help menus.
All settings are in the Settings tab. To open it, click the gear icon at the
bottom left, or use Edit > Settings (Ctrl+,). For a more compact window,
turn off "Show menu bar" in Settings > Appearance. Then the menus open from
the Notey icon in the top-left of the title bar.

Up to 0.7.x, Notey was built with egui. That version is kept on the
[`archive/egui-0.7`](https://github.com/deepfold9118/notey/tree/archive/egui-0.7)
branch.

## Run

```
cargo run --release
```

or launch `target\release\notey.exe` directly. You can pass file paths as
arguments (`notey.exe notes.txt`) or drag files onto the window.

Opening a file while Notey is already running adds it as a tab in the
existing window (single-instance forwarding over a named pipe). Pass
`--new-window` to force a separate window instead.

Closing the main window never loses work: open tabs — including unsaved
text — are stored in a session (`%APPDATA%\Notey\data\session.json`, also
snapshotted every ~30 s for crash safety) and restored the next time Notey
starts. The session is also saved when Windows shuts down. Closing an
individual tab still prompts to save. Secondary `--new-window` windows have
no session, so they prompt on close instead.

## Features

- **Dark mode** by default (change it in Settings > Appearance; remembered between runs)
- **Tabs** — Ctrl+N new tab, Ctrl+W close, Ctrl+Tab / Ctrl+Shift+Tab to switch,
  middle-click a tab to close it, unsaved-changes prompt on close. The tabs
  can be in the title bar or in a vertical list on the left (Settings >
  Appearance > Tabs).
- **Spellchecker** (plugin) — red underline on misspelled words; right-click
  a word for suggestions or "Add to dictionary"; dictionaries come from
  Language plugins
- **Syntax highlighting** (plugins) — one File Type plugin per language (45
  available), auto-detected by file extension with a filterable language
  picker in the status bar
- **Themes** — built-in Notey Dark and Light themes plus imported VS Code
  color-theme JSON/JSONC and TextMate `.tmTheme` files; workbench colors apply
  across the full Notey interface and `tokenColors` drive syntax highlighting.
  Notey can also import a copied `vscodethemes.com` URL by downloading and
  extracting the complete theme from its Visual Studio Marketplace extension;
  downloaded packages and all their theme variants remain available under
  Settings > Appearance
- File: New Tab / New Window / Open / Save / Save As / Print / Exit
- Edit: Undo, Redo, Cut, Copy, Paste, Delete, Select All, Find, Replace,
  Find in Files, Go To line (Ctrl+G), Insert Time/Date (F5), the commands of
  plugins, and Settings (Ctrl+,). Find Next is F3 and Find Previous is Shift+F3.
- **Search**: regex find/replace via fancy-regex (lookaround +
  backreferences, `$1` capture expansion in Replace), match case and whole
  word, highlighting of all matches, and **Find in Files** (Ctrl+Shift+F) —
  background directory grep respecting .gitignore, with a results panel
  and click-to-open at the matching line
- **Settings tab** (gear icon, Edit > Settings, or Ctrl+,): a list of
  sections on the left and their settings on the right.
  - Editor: font (all installed system fonts, with filter), font size, line
    height, tab size, word wrap, line numbers, ligatures, and autosave
  - Appearance: theme, theme import from a file or a `vscodethemes.com` URL,
    tab position, menu bar, and status bar
  - Spellcheck: on or off, the languages, and the personal word list
  - Plugins: install, update, turn on or off, and remove plugins
- Zoom: Ctrl+Plus / Ctrl+Minus / Ctrl+0. The status bar shows the zoom level.
- Status bar: line/column, character count, zoom, and **clickable** line
  endings (CRLF / LF) and encoding (UTF-8, UTF-8 BOM, UTF-16 LE/BE, ANSI)
  selectors for the current file
- Encoding and line endings are detected on open and preserved on save
- **File watching**: open files are polled for on-disk changes — a prompt
  offers Reload / Keep when another program modified the file (deletions
  are flagged in the status bar)
- **Recent Files** submenu in the File menu (last 10, persisted)
- **Autosave** (Settings > Editor): periodically saves modified file-backed tabs
- **Screen readers**: the window, menus, tabs, editor and status bar are
  exposed through UI Automation (via [AccessKit](https://accesskit.dev/)),
  so Narrator and other screen readers can read and operate them

## Editor

The editor is [codemirror-rs](https://github.com/deepfold9118/codemirror-rs),
a Rust port of CodeMirror 6's state, view, commands and search packages,
drawn with GPUI (design notes: [docs/codemirror-rust-design.md](docs/codemirror-rust-design.md)). It lays out only the visible lines, so large files stay responsive,
and syntax highlighting and spellcheck work on the visible lines only.

- **Multiple cursors**: Ctrl+Click adds a cursor, Alt+drag makes a
  column (box) selection, Ctrl+Alt+Up/Down adds a cursor above or below,
  Ctrl+D selects the next occurrence of the selection, and Ctrl+Shift+L
  selects all of them. Escape goes back to one cursor. All cursors type,
  delete and paste together as one undo step.
- Alt+Up/Down moves lines, Tab and Shift+Tab indent and outdent the
  selected lines, and Ctrl+Backspace / Ctrl+Delete delete by word.
- Undo groups typing, and find/replace and plugin edits can be undone too.
- Word wrap, line numbers, and IME input with inline composition.

## Plugins

Notey keeps its core minimal; everything else is a plugin, managed from
**Settings > Plugins** in three tabs:

- **Features** — Spellcheck, Markdown Tools (Ctrl+B/I/E/K formatting, list
  continuation, rendered and side-by-side preview), Word Count, Autocorrect,
  Text Color, and Export & Print (HTML export, formatted printing via your
  browser).
- **Languages** — one spellcheck dictionary each (16 available: English
  variants, Spanish, French, German, Italian, Portuguese, Dutch, Polish,
  Swedish, Danish, Russian, Ukrainian). A word is accepted if any enabled
  language knows it; "Add to dictionary" words are saved permanently.
- **File Types** — syntax highlighting, one plugin per file type (45
  available). Open a file Notey doesn't recognize and the status bar offers
  the matching plugin.

Official plugins download from this repo's [`plugins/`](plugins/) folder
and are checksum-verified before install. The installer includes a default
set (Spellcheck, English (US), Markdown Tools, Word Count, and common file
types), so everything works offline out of the box.

You can also write your own Rhai scripts in `%APPDATA%\Notey\scripts`;
their commands appear in the Edit menu (Settings > Plugins > Reload Scripts
loads the changes). See [docs/plugin-api.md](docs/plugin-api.md). Scripts can't
touch files or the network unless they declare a permission, and a
failing script can never corrupt the buffer: scripts edit a snapshot that
is only applied when they finish.

## Installer

`installer/notey.iss` is an Inno Setup script. Build it with:

```
ISCC.exe installer\notey.iss
```

which produces `installer_out\NoteySetup.exe`. The installer is per-user (no
admin rights needed) and:

- installs to `%LOCALAPPDATA%\Programs\Notey` with a Start menu entry and
  uninstaller
- registers Notey in the **Open with** menu for common text extensions
  (.txt, .log, .md, .json, .xml, .csv, .yaml, .toml, .ini, …)
- optionally adds "Open with Notey" (tab in the existing window) and
  "Open in new Notey window" right-click entries for all files
  (shown under "Show more options" on Windows 11)
- optional desktop shortcut (unchecked by default)

Rebuild the app first (`cargo build --release`) so the installer picks up the
latest exe.

## Notes

- Print works on saved files: it hands the file to the default `.txt` print
  handler via the shell Print verb.
- The spellchecker skips words with digits, URLs/emails (heuristic), and
  ordinals such as "2nd".

## Download

Grab `NoteySetup.exe` from the
[latest release](https://github.com/deepfold9118/notey/releases/latest).
The installer is per-user and needs no administrator rights. See
[CHANGELOG.md](CHANGELOG.md) for what changed in each version.

## License

Notey is licensed under the [Apache License, Version 2.0](LICENSE).

Official plugins under [`plugins/`](plugins/) keep their upstream licenses,
which ship inside each plugin: spellcheck dictionaries come from the
[LibreOffice dictionaries](https://github.com/LibreOffice/dictionaries)
project, and file type grammars from
[Sublime Text's Packages](https://github.com/sublimehq/Packages) with
[bat](https://github.com/sharkdp/bat)'s syntect compatibility patches.
