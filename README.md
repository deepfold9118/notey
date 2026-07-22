# Notey

A native Rust notepad for Windows with dark mode, tabs, and spellcheck.
Built with [egui/eframe](https://github.com/emilk/egui) and
[spellbook](https://github.com/helix-editor/spellbook) (Hunspell-compatible).

The UI is styled after Fluent / WinUI (Windows 11 Notepad): a frameless
window with a custom title bar containing the tab strip and caption
buttons, Fluent dark and light palettes (`src/theme.rs`), Segoe UI
Variable, rounded menus and dialogs, and an accent-colored selection.
Window dragging, double-click maximize, and edge resizing are all
implemented on top of the frameless window.

By default there is no menu bar — the File/Edit/View/Help menus open from
the Notey icon in the top-left of the title bar. Turn on "Always show menu
bar" in Edit > Preferences (Ctrl+,) to get the classic menu row back.

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
starts. Closing an individual tab still prompts to save. Secondary
`--new-window` windows have no session, so they prompt on close instead.

## Features

- **Dark mode** by default (toggle in View menu; remembered between runs)
- **Tabs** — Ctrl+N new tab, Ctrl+W close, Ctrl+Tab / Ctrl+Shift+Tab to switch,
  middle-click a tab to close it, unsaved-changes prompt on close
- **Spellchecker** — red underline on misspelled words; right-click a word for
  suggestions or "Add to dictionary" (en_US Hunspell dictionary, embedded)
- File: New Tab / New Window / Open / Save / Save As / Print / Exit
- Edit: Undo, Redo, Cut, Copy, Paste, Delete, Find, Find Next (F3),
  Find Previous (Shift+F3), Replace, Go To line (Ctrl+G), Select All,
  Time/Date (F5), Preferences (Ctrl+,)
- **Preferences** (Edit > Preferences): font picker (all installed system
  fonts, with filter), font size, line height, line numbers, word wrap —
  all persisted between runs
- View: Zoom (Ctrl+Plus / Ctrl+Minus / Ctrl+0), status bar toggle,
  spellcheck toggle, dark mode toggle
- Status bar: line/column, character count, zoom, and **clickable** line
  endings (CRLF / LF) and encoding (UTF-8, UTF-8 BOM, UTF-16 LE/BE, ANSI)
  selectors for the current file
- Encoding and line endings are detected on open and preserved on save

## Preview editor

Preferences > Editor > "Preview editor (virtualized)" swaps egui's TextEdit
for Notey's own editor widget (`src/editor.rs`): it lays out only the
visible lines, so large files stay responsive, and it owns its
cursor/selection/undo model (the foundation for multi-cursor, folding, and
markers on the Notepad++ roadmap). It supports word wrap (per-line wrap-row
cache with placeholder invalidation), **multi-cursor editing** (Ctrl+Click
to add cursors, Alt+drag for column/box selection, Escape to collapse; all
cursors type/delete/paste simultaneously as one undo step), spellcheck with
right-click suggestions, visual-row cursor navigation, undo integration for
find/replace and plugin edits, and basic IME (caret-anchored candidate
window, committed text insertion — no inline composition preview yet).
Default off; the classic editor remains the default until parity. Core
logic is unit-tested (`cargo test`).

## Plugins (Rhai scripts)

Notey has a Tier-1 plugin system: Rhai scripts in `%APPDATA%\Notey\scripts`
register commands into the **Plugins** menu (with optional keyboard
shortcuts) and can subscribe to host events (`ready`, `buffer_opened`,
`buffer_activated`, `before_save`, `after_save`). Scripts drive the editor
through the `notey.*` API — see [docs/plugin-api.md](docs/plugin-api.md).
An example script (`sort_lines.rhai`, Ctrl+Alt+S) is created on first run.
Plugins > Reload Scripts picks up changes without restarting.

Scripts run sandboxed (no filesystem or network access) with an operation
limit, and a failing script can never corrupt the buffer: scripts mutate a
snapshot that is only applied back when they finish.

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
- The spellchecker skips words with digits, URLs/emails (heuristic), and is
  automatically disabled for files larger than 512 KB.
