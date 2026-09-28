# Changelog

All notable changes to Notey are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/deepfold9118/notey/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/deepfold9118/notey/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/deepfold9118/notey/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/deepfold9118/notey/releases/tag/v0.3.0
