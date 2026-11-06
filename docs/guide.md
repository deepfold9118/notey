# User guide

[Home](./)

## Getting around

By default, Notey shows a menu bar with the File, Edit, and Help menus.
All settings are in the Settings tab: click the gear icon at the bottom
left, or use Edit > Settings (Ctrl+,). For a more compact window, turn off
"Show menu bar" in Settings > Appearance. Then the menus open from the
Notey icon in the top-left of the title bar.

Open files by dragging them onto the window, from File > Open, or with
"Open with" in Explorer. Opening a file while Notey is running adds it as
a tab in the existing window.

Closing the window never loses work. Your open tabs, including unsaved
text, are kept and come back the next time Notey starts. Closing a single
tab asks whether to save it.

## Tabs

- Ctrl+N opens a new tab, Ctrl+W closes one, and Ctrl+Tab /
  Ctrl+Shift+Tab switch between them. Middle-click a tab to close it.
- File > New Window opens a separate window. Extra windows are for quick,
  temporary work: they don't keep a session, so they ask to save when
  they close.
- The tabs can be in the title bar or in a list on the left
  (Settings > Appearance > Tabs).
- Right-click a tab for its menu: pin it, give it a color, open it in
  split view, close it or the tabs around it, copy its path, or show it in
  File Explorer.

### Pinned tabs

Pin a tab (right-click > Pin Tab) to keep it open: pinned tabs move to the
front, show a pin in place of the close button, and come back every time
Notey starts. To close one, unpin it first by clicking its pin. If a
pinned file is missing when Notey starts, for example on a network share
that is offline, its tab stays, shown in italics, and the file loads when
it is back. Pinning is for the main window.

### Tab colors

With the Tab Colors plugin (installed by default), right-click a tab and
pick a color from Color. The color shows as a stripe on the tab. The
colors come from the theme, so they always match it, and a file keeps its
color the next time it opens.

## Split view

View > Split View (`Ctrl+\`) shows two tabs side by side; right-click a tab
and choose Open in Split View to pick which one goes beside the current
tab. Click in a pane to work in it. Clicking a tab shows it in the pane
you are working in, and the tab shown in the other pane is highlighted in
the tab strip. Drag the line between the panes to resize them. View >
Close Split View goes back to one pane.

## Search

- Ctrl+F finds and Ctrl+H replaces. F3 and Shift+F3 go to the next and
  previous match, and all matches are highlighted.
- Search supports regular expressions (with lookaround and
  backreferences, and `$1` in the replacement), match case, and whole word.
- **Find in Files** (Ctrl+Shift+F) searches a folder in the background,
  skips files ignored by `.gitignore`, and opens a result at its line when
  you click it.
- Ctrl+G goes to a line.

## The editor

- **Multiple cursors**: Ctrl+Click adds a cursor, Alt+drag selects a
  column, Ctrl+Alt+Up/Down adds a cursor above or below, Ctrl+D selects the
  next occurrence of the selection, and Ctrl+Shift+L selects all of them.
  Escape goes back to one cursor.
- Alt+Up/Down moves lines, Tab and Shift+Tab indent and outdent the
  selected lines, and Ctrl+Backspace / Ctrl+Delete delete by word.
- F5 inserts the time and date.
- Zoom with Ctrl+Plus, Ctrl+Minus and Ctrl+0 (also in the View menu).
- Large files stay fast: Notey lays out only the lines you can see.

## Files

- The status bar shows the line and column, character count and zoom.
  Click the line endings (CRLF / LF) or the encoding (UTF-8, UTF-8 BOM,
  UTF-16 LE/BE, ANSI) to change them for the current file. Both are
  detected when a file opens and kept when it is saved.
- When another program changes an open file, Notey offers to reload it.
- File > Recent Files lists the last 10 files.
- **Autosave** (Settings > Editor) saves modified files periodically.
- Files on network shares open like any other.

## Settings

- **Editor**: font, font size, line height, tab size, word wrap, line
  numbers, ligatures and autosave.
- **Appearance**: theme, tab position, menu bar and status bar. Besides
  the built-in Notey Dark and Light themes, you can import VS Code color
  themes (JSON), TextMate `.tmTheme` files, or a copied `vscodethemes.com`
  link.
- **Spellcheck**: on or off, the languages, and your personal word list.
- **Plugins**: install, update, turn on or off, and remove plugins.
- **Updates**: Notey looks for a new version once a day and offers to
  install it, keeping your tabs. Help > Check for Updates looks right away.

## Plugins

Notey keeps its core small, and most features are plugins, managed in
Settings > Plugins:

- **Features**: Spellcheck, Markdown Tools (formatting shortcuts, list
  continuation and a rendered preview), Word Count, Autocorrect, Text
  Color, Tab Colors, Terminal, Export & Print, and
  [Math](https://github.com/deepfold9118/notey/blob/main/plugins/features/feature-math/README.md).
- **Languages**: spellcheck dictionaries for 16 languages. A word is
  accepted if any enabled language knows it. Right-click a misspelled word
  for suggestions or "Add to dictionary".
- **File Types**: syntax highlighting for 45 file types. Notey picks the
  type from the file extension or, for new tabs, from the text. When a file
  needs a plugin you don't have, the status bar offers it.

The installer includes Spellcheck, English (US), Markdown Tools, Word Count,
Tab Colors and common file types, so Notey works offline out of the box. Plugins are
checksum-verified before they install.

### Math

With the Math plugin, write `=>` after a calculation and Notey writes the
result after it, updating it as you type:

```
price = 20
price * 3 => 60
100 USD in EUR => 89.09 EUR
```

It handles definitions and functions, symbolic algebra, solving for an
unknown, derivatives, matrices, and currency conversion with the European
Central Bank's daily rates. Edit > Math turns it off for a tab. See the
[Math reference](https://github.com/deepfold9118/notey/blob/main/plugins/features/feature-math/README.md)
for everything it can do.

### Terminal

With the Terminal plugin, a tab's right-click menu has **Open in
Terminal**, which opens a shell in the file's folder, and **Run in
Terminal** (Ctrl+F5 for the active tab), which runs the file: Python with
`py`, JavaScript with `node`, PowerShell scripts, batch files, shell
scripts, Ruby, Lua, Perl and Go. Running saves the file first; an
untitled tab runs from a temporary copy, so you can jot some code and run
it. The window stays open afterwards, so you can read the output.

Settings > Terminal (shown while the plugin is enabled) picks the terminal
app and the shell from what is installed, or Other with a path, and lets
you change the command for each kind of file, for example to use a
particular Python. The plugin asks for permission to start programs when
you install it.

### Your own scripts

Put Rhai scripts in `%APPDATA%\Notey\scripts`; their commands appear in
the Edit menu. See the [Plugin API](plugin-api.md).

## Screen readers

The window, menus, tabs, editor and status bar work with Narrator and
other screen readers.
