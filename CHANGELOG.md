# Changelog

All notable changes to Notey are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.11.0] - 2026-11-05

### Added
- Terminal, a new Feature plugin: Open in Terminal and Run in Terminal in
  the tab menu, and Ctrl+F5 to run the active tab. Choose the terminal
  app (Windows Terminal, Alacritty, WezTerm, Warp or a console window)
  and the shell (PowerShell 7, Windows PowerShell, Command Prompt, Git
  Bash or WSL) in Settings > Terminal, which lists what is installed, or
  pick Other and give the program's path. Running a file saves it first;
  an untitled tab runs from a temporary copy. Python, JavaScript,
  PowerShell, batch files, shell scripts, Ruby, Lua, Perl and Go each have
  a command you can change. Warp can't be given a command, so with Warp
  a file runs in a console window.
- Plugin settings: a plugin can declare settings, and Notey shows them in
  a Settings section named after the plugin while it is enabled.
- Plugin API 6: settings (`notey.settings.get`), `notey.process.start`
  behind the new `run` permission, `notey.fs.exists`, `notey.app.which`
  and `notey.app.env`.

### Changed
- The plugin list describes a plugin's permissions in plain words, such
  as "download from the internet" or "start programs".

### Fixed
- `notey.buffers.save()` in a command of the tab menu saves the tab it
  runs on, not the active tab.

## [0.10.1] - 2026-11-05

### Changed
- The installer is named `notey_setup_windows_<version>.exe` (with a
  `.sha256` file beside it), and Notey's updater looks for that name.
  Releases also carry it as `NoteySetup.exe`, so older versions of Notey
  can still update.

### Fixed
- Python with type hints (`count: int = 10`, `def f(a: int) -> float:`) is
  recognized from its text; a short typed script stayed Plain Text.

## [0.10.0] - 2026-11-05

### Added
- Tabs have a right-click menu: Pin Tab, Color, Open in Split View, Close
  Tab, Close Other Tabs, Close Tabs to the Right, Copy Path and Show in
  File Explorer.
- Pinned tabs, in the main window: they move to the front with a pin in
  place of the close button, and can't be closed until they are unpinned
  (click the pin). They come back every time Notey starts; a pinned file
  that is missing, such as one on a network share that is offline, keeps
  its tab, shown in italics, and loads when the file is back.
- Split view (View > Split View, `Ctrl+\`, or Open in Split View on a tab):
  two tabs side by side. Click a pane to work in it; clicking a tab shows
  it in the pane you are working in. Drag the divider to resize the panes.
  The split view is kept in the session.
- A View menu, with Split View and the zoom commands.
- Tab Colors, a new Feature plugin, installed by default: right-click a
  tab and pick a color from Color. The colors come from the theme (a VS
  Code theme's terminal colors), so they match whichever theme is in use.
  A file keeps its color the next time it opens.
- Plugin API 5: commands in the tab right-click menu (`menu: "Tab"`), which
  run on the right-clicked tab; `notey.editor.tab_color` and
  `set_tab_color`; and color names (`red`, `blue`, ...) that follow the
  theme, for tab colors and menu swatches.

## [0.9.0] - 2026-11-05

### Added
- Math, a new Feature plugin: live math in any tab, in the style of
  Calca. Write `=>` after a calculation and the result appears after it,
  updated as you type. It has definitions and functions, names of several
  words, symbolic algebra (`y * (1 + 10%) => 1.1y`), solving for an
  undefined variable, derivatives, Taylor series and Jacobians, matrices,
  ranges, map/reduce/sum/prod, complex numbers, logic, hexadecimal and
  binary numbers, heading-scoped definitions and number cultures. Money is
  a unit (`$20`, `100 EUR`) and converts with `in` (`100 USD in EUR`),
  using the European Central Bank's daily rates. In plain text and
  Markdown it writes only results that say something new, and in code
  files only plain values after an empty `=>`, so ordinary `=>` arrows are
  left alone. Results written while typing undo with the typing.
- Plugin API 4: `notey.wasm.run` runs a sandboxed WebAssembly module that
  a plugin ships (no imports, a fuel budget, capped memory);
  `notey.net.fetch`, `cached` and `age` download into a cache, behind the
  new `net` permission, with a `net` event when a download finishes;
  `notey.editor.replace_ranges` applies several edits exactly, keeping the
  cursor and text styles elsewhere in place; and
  `notey.editor.set_history` makes a script's edits join the last undo
  step or stay out of the undo history.

### Fixed
- Release builds no longer contain file paths from the machine that built
  them.

## [0.8.8] - 2026-11-05

### Added
- Notey looks for a newer release on GitHub once a day and offers to
  install it: Update Now, Later, or Skip This Version. Updating downloads
  the installer, checks it against the release's SHA-256 checksum, closes
  Notey with its tabs kept in the session, upgrades it and starts it again.
  If other Notey windows are open, Notey first warns that the update
  closes them. Help > Check for Updates looks right away. Settings >
  Updates turns the daily check off.

## [0.8.7] - 2026-11-05

### Added
- Untitled tabs, and files without an extension, get their file type from
  their text. When typing pauses or text is pasted, Notey recognizes JSON,
  XML, HTML, Markdown, Python, Rust, JavaScript, C and C++, C#, Java, Go,
  SQL, YAML, CSS, shell scripts, batch files, PHP, Ruby, Lua, Perl,
  makefiles, diffs and LaTeX. If the File Type plugin for it is not
  installed, the status bar offers to install it, and the tab switches
  when it is. Choosing a file type in the status bar, or saving under a
  name with an extension, keeps the file type from then on.

### Fixed
- Save As did not change the file type of the tab. A new tab saved as a
  `.md` file stayed Plain Text, without highlighting and without the
  rendered and side-by-side Markdown views. Save As now picks the file type
  from the new name, as opening the file does.
- The Go and Lua file types showed as "Unnamed" in the status bar, because
  their grammars have no name. They are now named after their files.

## [0.8.6] - 2026-11-04

### Fixed
- Notey could crash while opening or scrolling a long document that
  contains characters of more than one byte in UTF-8, such as the
  box-drawing characters of text diagrams, when word wrap is on.

## [0.8.5] - 2026-11-01

### Fixed
- An editor font chosen in Notey 0.7 (such as "JetBrains Mono Regular")
  was not found, and the editor fell back to a proportional font, in which
  tabs and columns do not line up. Notey now changes such a name to its
  font family, or to Consolas if no family matches.

## [0.8.4] - 2026-11-01

### Fixed
- With line numbers on, the numbers of the lines just past the bottom of
  the editor were drawn over the status bar.

## [0.8.3] - 2026-11-01

### Fixed
- Files on a network share (a path such as `\\server\share\file.txt`)
  could not be opened.

## [0.8.2] - 2026-10-31

### Changed
- The menu of the Notey icon (when the menu bar is hidden) is no longer one
  long list. It has a submenu for File and for Edit, a Plugins and a Format
  submenu for the commands of plugins, and About Notey.
- Recent files are in an "Open Recent" submenu of the File menu.
- Submenus can contain submenus, and screen readers can read the items of
  open submenus.

## [0.8.1] - 2026-10-31

### Fixed
- The window could not be moved by dragging the title bar, and
  double-clicking the title bar and the minimize, maximize and close
  buttons did nothing.

## [0.8.0] - 2026-10-31

### Changed
- Notey is rebuilt on GPUI (the UI framework of the Zed editor) and a Rust
  port of CodeMirror 6. It looks and works the same as 0.7.0, and all
  plugins, themes, settings and sessions carry over. Drawing is done on the
  GPU, so scrolling and typing in large files are faster.
- The dialogs (save changes, reload, errors) are drawn in the Notey window
  in the Fluent style, not as Windows message boxes.
- Spellcheck and syntax highlighting work on the visible lines only, so
  spellcheck is no longer turned off for files over 512 KB.
- Tall menus scroll, and submenus open next to them.

### Added
- CodeMirror editing: Ctrl+D selects the next occurrence of the selection,
  Ctrl+Shift+L selects all of them, Ctrl+Alt+Up/Down adds a cursor above or
  below, and Alt+Up/Down moves lines. Ctrl+Click and Alt+drag still add
  cursors and box selections.
- IME input shows the text being composed in the line.

### Fixed
- Ordinals such as "2nd" are no longer marked as misspelled.

### Removed
- The "Virtualized editor" setting and the classic editor. There is one
  editor now.
- The egui version is kept on the `archive/egui-0.7` branch.

## [0.7.0] - 2026-10-24

### Added
- A Settings tab. It has a list of sections on the left (Editor,
  Appearance, Spellcheck, Plugins) and the settings of the selected section
  on the right. To open it, click the gear icon at the bottom left, use
  Edit > Settings, or push Ctrl+,. To close it, push Ctrl+W or click its X.
- A setting for the position of the tabs (Settings > Appearance > Tabs). The
  tabs can be in the title bar, or in a vertical list on the left, the same
  as the "Open Editors" list in VS Code.

### Changed
- The menu bar has only the File, Edit, and Help menus.
  - The settings of the View menu (word wrap, line numbers, status bar,
    spellcheck, and theme) are in the Settings tab. Zoom uses only the
    keyboard (Ctrl+Plus, Ctrl+Minus, Ctrl+0). The status bar shows the zoom
    level.
  - The Language menu is gone. Use the language picker in the status bar.
  - The commands of plugins, which were in the Format and Plugins menus,
    are in the Edit menu. Plugins do not need a change.
  - The plugin list, "Reload Scripts", and "Open Scripts Folder" are in
    Settings > Plugins.
  - "Find Next" and "Find Previous" are not in the Edit menu. Use F3 and
    Shift+F3.
  - "Time/Date" is now "Insert Time/Date".
- The Settings tab replaces the Preferences window, the Plugins window, and
  the "Get VS Code Theme" window.

### Fixed
- "Reset to defaults" turned off the menu bar, but the menu bar is on by
  default. Now it turns the menu bar and the status bar on, and puts the
  tabs back in the title bar.

## [0.6.0] - 2026-10-24

### Added
- The Text Color plugin. Right-click selected text, or open the Format menu,
  to give the text a color or a background color. You can select one of the
  16 terminal (ANSI) colors. The colors stay while the tab is open. Notey
  does not save them to the file. Install the plugin from Plugins > Manage
  Plugins > Features.
- A Format menu. It contains Word Wrap and the commands that plugins add
  to it.
- Plugin API 3. A command can go in the Format menu (`menu`), in a submenu
  (`group`), and in the editor's right-click menu (`context`). It can also
  show a color square (`swatch`). Scripts can put colors on text with
  `notey.editor.style_range` and remove them with
  `notey.editor.clear_styles`.

### Changed
- Notey now shows the menu bar by default, also if you turned it off
  before. To use the compact menu under the Notey icon again, turn off
  "Show menu bar" in Edit > Preferences.

### Fixed
- Clicks, mouse selection, and copy and paste respond faster. Notey now
  queues one frame for the display, not two. At 60 Hz, this removes
  approximately 16 ms of delay.

## [0.5.6] - 2026-10-16

### Fixed
- Windows shutdown and the Notey setup program (when it installs an update)
  can now close Notey. Before, Notey did not save the session at shutdown.
  Now it saves the session, as it does when you close the window.
- On Windows, Notey no longer starts the OpenGL graphics backend. That
  backend made a hidden window that did not respond, thus the setup program
  could not close Notey. Notey uses DirectX 12 or Vulkan. You can still
  select a backend with the `WGPU_BACKEND` environment variable.

## [0.5.5] - 2026-10-16

### Fixed
- When you open a file with Notey from File Explorer (for example, with
  "Open with") and Notey is already open, the Notey window now comes to the
  front, as in Notepad++. If the window is minimized, Notey restores it.
  Before, the file opened in a new tab, but the window stayed behind other
  windows or stayed minimized.

## [0.5.4] - 2026-10-09

### Fixed
- When you select text with the mouse, the editor now scrolls if you move
  the pointer above or below the text area (or to the left or right of it
  without word wrap). The farther the pointer is from the edge, the faster
  the scroll. The mouse wheel also scrolls while you select, so you can
  extend a selection past the visible lines.

## [0.5.3] - 2026-10-09

### Fixed
- The text cursor did not show, and typing did nothing, until you clicked in
  the text. The editor now gets the keyboard focus when the window becomes
  active, and when no other control has the focus.
- A mouse selection did not include the first character (or the last
  character, if you dragged backwards). The selection now starts where you
  pressed the mouse button, not where the drag started.
- A tab now goes to the next tab stop (a multiple of the tab size), as in
  Notepad++. Before, each tab had a fixed width, thus text after `1.` and a
  tab did not align with text after a tab alone.

## [0.5.2] - 2026-09-27

### Fixed
- The text cursor was nearly invisible in some imported themes (e.g.
  Tokyo Night, Gruvbox): it used the theme's focus-border color. It now
  uses the theme's cursor color (`editorCursor.foreground`, or the
  `.tmTheme` caret), falling back to the text color when that doesn't
  contrast with the editor background. The cursor is also slightly wider.

## [0.5.1] - 2026-09-27

### Fixed
- The text cursor disappeared after pressing an arrow key, Tab or Escape:
  those keys moved keyboard focus out of the editor (since 0.4.0).

## [0.5.0] - 2026-09-27

### Added
- Preferences > Ligatures: turn font ligatures (`->`, `!=`, `fi`, ...) on
  or off for fonts that have them, such as Cascadia Code or Fira Code. On by
  default, as before.

## [0.4.1] - 2026-09-27

### Fixed
- Plugins window: long descriptions ran underneath the Enabled and
  Install/Uninstall controls; they now wrap before the controls column.

## [0.4.0] - 2026-09-27

### Added
- **Plugins window** (Plugins > Manage Plugins) with Features, Languages
  and File Types tabs: download, enable, disable, update and remove
  official plugins from the repository's `plugins/` folder. Downloads are
  verified against published SHA-256 checksums.
- **16 spellcheck languages** as Language plugins: English (US, UK, Canada,
  Australia), Spanish, French, German, Italian, Portuguese (Brazil,
  Portugal), Dutch, Polish, Swedish, Danish, Russian, Ukrainian. Every
  enabled language is checked at once.
- **45 file types** as File Type plugins. The status bar offers the right
  plugin when you open a file Notey doesn't recognize.
- **Markdown Tools** plugin: Ctrl+B / Ctrl+I / Ctrl+E / Ctrl+K formatting,
  list continuation on Enter (bullets, numbers, task lists, quotes),
  rendered view (Ctrl+Shift+M) and side-by-side live preview (Ctrl+Alt+M).
- **Word Count** plugin: word count in the status bar (selection-aware),
  with reading time on hover.
- **Autocorrect** plugin: fixes common typos and a lone "i" in plain text
  and Markdown.
- **Export & Print** plugin: export as HTML (Markdown rendered) and print
  formatted through your browser, which can also save as PDF.
- Plugin API 2: command and key-hook conditions, `on_key`, `input` /
  `text_changed` / `selection_changed` events, status bar items, preview
  modes, `notey.text` helpers, and permission-gated `notey.fs.write_text`
  and `notey.app.open`.
- "Add to dictionary" words are now saved permanently.

### Changed
- The virtualized editor (large-file speed, multi-cursor, column editing)
  is now the default for everyone; Preferences > Editor > "Virtualized
  editor" switches back to the classic editor.
- Spellcheck, every dictionary and every syntax grammar moved out of the
  core into plugins. The installer includes a default set (Spellcheck,
  English (US), Markdown Tools, Word Count, and 12 common file types), so
  upgrades keep working offline.
- The Markdown rendered-view script previously seeded into the scripts
  folder is replaced by the official Markdown Tools plugin (an unmodified
  old copy is removed automatically).
- License changed from MIT to Apache License 2.0. Releases up to and
  including 0.3.0 remain available under MIT.

### Fixed
- Plugin scripts could not use the `notey` API inside their own helper
  functions.
- Errors from calling a missing plugin API function were silently hidden.
- Rhai rejected ordinary plugin code (e.g. long `&&` chains) as too complex.
- A crash inside the accessibility layer (AccessKit) when a screen reader or
  other UI Automation client was active while the editor requested focus.
- Two Notey windows installing the same plugin at once could clobber each
  other's download.
- Word counts no longer count list bullets, `#` and `>` as words.
- Crashes are now recorded to `%APPDATA%\Notey\crash.log`.

## [0.3.0] - 2026-09-27

First public release.

### Added
- **Preview editor** (Preferences > Editor): a virtualized editor that lays
  out only visible lines, with word wrap, multi-cursor editing (Ctrl+Click,
  Alt+drag column selection), compound undo, and basic IME support.
- **Syntax highlighting** for ~250 languages via syntect + two-face, with
  extension-based detection and a filterable language picker.
- **Regex search suite**: fancy-regex find/replace with capture expansion,
  Mark All, and Find in Files (Ctrl+Shift+F) with a live results panel.
- **VS Code theme marketplace**: install color themes from vscodethemes.com
  URLs; imported themes drive both UI colors and syntax highlighting.
- **Markdown preview plugin**: raw/rendered toggle for Markdown tabs
  (Ctrl+Shift+M), rendered with egui_commonmark.
- **Plugin system** (Rhai scripts): Plugins menu, shortcuts, host events,
  and the `notey.*` API documented in `docs/plugin-api.md`.
- File-change detection with reload prompt, Recent Files, and autosave.
- Edit > Preferences: font picker, line height, line numbers, tab size,
  word wrap, menu-bar visibility.

### Fixed
- Plugin host actions (open/new/save/close buffers, alerts, clipboard,
  language changes) were silently discarded after a script ran.

## [0.2.0] - 2026-07-04

### Added
- Fluent/WinUI-styled frameless window with tabs in the title bar.
- Session persistence: unsaved tabs survive closing the app.
- Single-instance forwarding (files open as tabs; `--new-window` opt-out).
- Inno Setup installer with Open With and context-menu integration.

## [0.1.0] - 2026-07-04

### Added
- Initial notepad: dark mode, tabs, Hunspell spellcheck, find/replace,
  encodings and line endings, zoom, printing.

[Unreleased]: https://github.com/deepfold9118/notey/compare/v0.11.0...HEAD
[0.11.0]: https://github.com/deepfold9118/notey/compare/v0.10.1...v0.11.0
[0.10.1]: https://github.com/deepfold9118/notey/compare/v0.10.0...v0.10.1
[0.10.0]: https://github.com/deepfold9118/notey/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/deepfold9118/notey/compare/v0.8.8...v0.9.0
[0.8.8]: https://github.com/deepfold9118/notey/compare/v0.8.7...v0.8.8
[0.8.7]: https://github.com/deepfold9118/notey/compare/v0.8.6...v0.8.7
[0.8.6]: https://github.com/deepfold9118/notey/compare/v0.8.5...v0.8.6
[0.8.5]: https://github.com/deepfold9118/notey/compare/v0.8.4...v0.8.5
[0.8.4]: https://github.com/deepfold9118/notey/compare/v0.8.3...v0.8.4
[0.8.3]: https://github.com/deepfold9118/notey/compare/v0.8.2...v0.8.3
[0.8.2]: https://github.com/deepfold9118/notey/compare/v0.8.1...v0.8.2
[0.8.1]: https://github.com/deepfold9118/notey/compare/v0.8.0...v0.8.1
[0.8.0]: https://github.com/deepfold9118/notey/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/deepfold9118/notey/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/deepfold9118/notey/compare/v0.5.6...v0.6.0
[0.5.6]: https://github.com/deepfold9118/notey/compare/v0.5.5...v0.5.6
[0.5.5]: https://github.com/deepfold9118/notey/compare/v0.5.4...v0.5.5
[0.5.4]: https://github.com/deepfold9118/notey/compare/v0.5.3...v0.5.4
[0.5.3]: https://github.com/deepfold9118/notey/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/deepfold9118/notey/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/deepfold9118/notey/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/deepfold9118/notey/compare/v0.4.1...v0.5.0
[0.4.1]: https://github.com/deepfold9118/notey/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/deepfold9118/notey/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/deepfold9118/notey/releases/tag/v0.3.0
