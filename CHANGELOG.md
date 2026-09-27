# Changelog

All notable changes to Notey are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed
- License changed from MIT to Apache License 2.0. Releases up to and
  including 0.3.0 remain available under MIT.

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

[Unreleased]: https://github.com/deepfold9118/notey/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/deepfold9118/notey/releases/tag/v0.3.0
