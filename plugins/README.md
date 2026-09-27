# Notey official plugins

This folder is Notey's plugin registry. The app downloads plugins from here
(through `index.json`) in **Plugins > Manage Plugins**.

| Folder | Kind | What it provides |
|---|---|---|
| `features/` | `feature` | A capability: a Rhai script, or a built-in engine the plugin switches on (e.g. Spellcheck) |
| `languages/` | `language` | A Hunspell dictionary used by the Spellcheck feature |
| `filetypes/` | `filetype` | Sublime `.sublime-syntax` grammars that give a file type syntax highlighting |

Each plugin folder holds a `plugin.json` manifest and the files it lists.
`index.json` repeats every manifest with a SHA-256 and size for each file;
the app refuses any download that doesn't match.

## Rebuilding

Languages and file types are generated from pinned upstream sources, never
edited by hand:

```
cargo xtask plugins   # regenerate languages/ and filetypes/, validate, rewrite index.json
cargo xtask index     # only rewrite index.json (after editing features/)
```

The generator validates every dictionary and grammar with the same engines
the app uses, converts dictionaries to UTF-8, applies bat's syntect
compatibility patches to Sublime's grammars, and copies each upstream
license into the plugin that uses it. Upstream commits are pinned at the top
of `xtask/src/main.rs`.

Files in this folder are byte-exact (`.gitattributes` disables line-ending
conversion), because their checksums are published in `index.json`.
