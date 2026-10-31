# A CodeMirror-style editing core for Notey, in Rust

Status: **in progress.** Scope decided 2026-10-30 (§10). The Rust port lives
in its own repository, `D:\Projects\codemirror-rs`; `@codemirror/state` is
ported there as the `codemirror-state` crate.

## 1. Goal

Give Notey an editing core with CodeMirror 6's architecture: an immutable
editor state, changes described as data, an extension system built from
facets and state fields, mappable decorations, and incremental parsing. Today
editing, undo, layout, highlighting and painting are spread across one
2,400-line widget (`src/editor.rs`). The aim is to split those concerns the
way CodeMirror does, so features such as find, plugins, spellcheck, text
colors and syntax highlighting all speak one change model and get undo for
free.

CodeMirror 6 is © Marijn Haverbeke and others, MIT licence (§9). This doc was
written from the sources on Marijn's forge, `code.haverbeke.berlin`, at
state 6.7.6, view 6.43.15, language 6.13.1, commands 6.11.1, search 6.7.2,
autocomplete 6.20.4, @lezer/common 1.5.4, @lezer/lr 1.4.11 (October 2026).

## 2. Notey's editor today

| Concern | Where | How |
|---|---|---|
| Text storage | `doc.rs` `Document.text: String` | Whole buffer in one `String`, `\n` line endings (CRLF converted on load/save). `saved_text: String` clone for the modified check (full string compare). |
| Line index | `editor.rs` `EditorState.lines: Vec<LineMeta>` | Byte and char start of every line. **Rebuilt from scratch after every edit** (`rebuild_index`), O(document). |
| Positions | everywhere | **Char offsets** (Unicode scalar values): editor, find, go-to, plugin API v3, text styles. Converted to bytes by scanning. |
| Edits | `EditorState::apply_multi` | `String::replace_range` per cursor, descending order. One `EditOp` per undo step. |
| Undo | `EditorState.undo/redo: Vec<EditOp>` | Stores removed/inserted strings plus selection snapshots. Typing is grouped (single chars within 1 s). Capped at 10,000 steps. Edits made outside the widget (find/replace, plugins) call `note_external_change`, which **clears the undo history**. `Replace All` replaces the whole document as one edit. |
| Selection | `EditorState.anchor/cursor/extra_sels` | Primary plus secondary `(anchor, head)` pairs; dedup by hand. Alt+drag column selection. `desired_x` keeps the goal column. |
| Layout | `editor::show`, `fill_wrap_cache`, `line_galley_colored` | Virtualised: only visible lines become egui `LayoutJob`s/galleys. A per-line wrapped-row count plus prefix sums gives the scroll height. Ligature breaking and tab stops are applied to each `LayoutJob`. |
| Highlighting | `syntax.rs` `HlCache` | syntect with Sublime grammars from file-type plugins. Parser state snapshot per line; parses forward lazily up to the last visible line; invalidates from the first edited line. |
| Text colors (plugin API 3) | `styles.rs` `Styles` | Sorted runs in char offsets. `sync` finds the edit by diffing the old and new **whole text** (common prefix/suffix) every frame something changed, and keeps a full copy of the text to do it. |
| Search marks | `app.rs` `search_marks: Vec<(usize, usize)>` | Char ranges; **cleared on any edit** because they can't be mapped. |
| Spellcheck | `editor.rs` + `spell.rs` | Recomputed per visible line while building the galley; disabled above 4 MB. |
| Plugins | `plugins.rs` `ScriptCtx` | A Rhai script gets a **clone of the whole text**, edits it, and the host writes it back by diffing (`apply_text_diff`), which goes through `edit_text` as one replacement. |
| Input | `editor::show`, `handle_key` | egui `Event::Text/Paste/Copy/Cut/Ime/Key` handled inline; a hard-coded `match` on keys. Focus lock filter so arrows/Tab/Escape stay in the editor. |

There is also a second editor path: egui's own `TextEdit` (`preview_editor =
false`). It is still selectable in Settings.

### Where it couples to egui

- `editor::show` mixes input, state mutation, layout and painting in one
  immediate-mode function. The state model (`EditorState`) is mostly
  egui-free, but `desired_x`, `column_drag_origin` and the wrap cache are
  view state living beside document state.
- Layout and hit testing use egui galleys directly (`pos_from_cursor`,
  `cursor_from_pos`); colors are `egui::Color32` inside `HlCache` and
  `Styles`.
- Keys are `egui::Key`/`Modifiers`; IME is `egui::ImeEvent`.

## 3. CodeMirror 6 in brief

CodeMirror is a set of packages. The ones that matter here:

- **`@codemirror/state`**: `Text` (a balanced tree of line arrays), `ChangeSet`
  (changes as a flat run-length list of kept/replaced sections, with `map`,
  `compose`, `invert`, `mapPos`), `EditorSelection` (ranges with anchor/head,
  goal column, main index), `Transaction` (changes + selection + effects +
  annotations), `Facet` (a typed input slot whose many inputs are combined
  into one output), `StateField` (a value updated by every transaction),
  `Extension` (a nested list of facet inputs, fields and precedence
  wrappers), `Compartment` (a reconfigurable part of the extension tree),
  `RangeSet` (immutable, chunked, mappable ranges used for decorations and
  markers). The state is immutable: `state.update(spec)` returns a
  transaction whose `.state` is the next state.
- **`@codemirror/view`**: `EditorView` owns the DOM. `dispatch(tr)` →
  `update([tr])` → a `ViewUpdate` goes to every `ViewPlugin` → the view
  redraws its content DOM for the visible viewport and schedules a
  read/measure/write cycle. A `HeightMap` stores estimated heights for
  unrendered lines and measured heights for rendered ones. Decorations
  (mark, widget, replace, line) come from a facet as `RangeSet`s. Input
  arrives through DOM events and a `MutationObserver`.
- **Lezer** (`@lezer/common`, `@lezer/lr`, `@lezer/highlight`) and
  **`@codemirror/language`**: an incremental LR (GLR-capable) parser whose
  grammars are compiled ahead of time by `@lezer/generator` into compact
  tables. It produces a `Tree` and reuses `TreeFragment`s of the old tree
  after an edit. `@codemirror/language` runs the parse in time-boxed slices
  in the background, exposes `syntaxTree(state)`, and builds highlighting,
  folding and indentation on the tree (parse work: 20 ms synchronously per
  change, then 25–100 ms slices with pauses). `StreamLanguage` wraps
  line-by-line tokenisers, storing tokenizer state on tree nodes so parsing
  can resume, and is the closest equivalent to Notey's syntect set-up.
- **`@codemirror/commands`**: commands are `(view) => bool`; the history is a
  state field that stores *inverted change sets* plus selections, joins
  adjacent events by time and user-event type, and maps stored events
  through changes that it didn't record (remote or non-history changes).
- **`@codemirror/search`**: `SearchCursor`/`RegExpCursor` iterate over the
  `Text` without flattening it; a state field holds the query; a panel shows
  the UI; matches are drawn as decorations.
- **`@codemirror/autocomplete`**: completion sources return results for a
  `CompletionContext`; a state field runs a small state machine
  (pending/active/closed); `closeBrackets` is an input handler plus a
  state field that tracks auto-inserted brackets.

## 4. Mapping CodeMirror onto Rust and egui

### 4.1 Port nearly one-to-one

These are pure data structures and algorithms with no DOM in them. They are
where CodeMirror's quality comes from, and its test suites can be ported as
the correctness oracle.

| CodeMirror | Rust design |
|---|---|
| `ChangeSet`, `ChangeDesc`, `MapMode`, `mapPos`, `compose`, `map`, `invert`, `iterChanges` | `change.rs`. Same flat `sections: Vec<usize>` encoding (pairs of length and inserted-length, with a sentinel for "kept"), inserted text as `Vec<Text>`. Positions are `usize`. |
| `EditorSelection`, `SelectionRange` | `selection.rs`. `SelectionRange { anchor, head, goal_column: Option<u32>, assoc, bidi_level }` (CM packs flags into one number; Rust can use plain fields). `EditorSelection { ranges: SmallVec<[_; 1]>, main: usize }` with CM's normalisation (sort, merge overlaps). This replaces `anchor/cursor/extra_sels` and the hand-written dedup. |
| `Transaction`, `TransactionSpec`, `Annotation`, `StateEffect` | `transaction.rs`. Annotations and effects are typed keys over `Box<dyn Any>` values: `AnnotationType<T>`, `StateEffectType<T>` with an optional `map` fn. Predefined: `user_event` (`"input.type"`, `"delete.backward"`, `"undo"`, …), `add_to_history`, `remote`. |
| `Facet`, `StateField`, `Extension`, `Prec`, `Compartment`, `Configuration.resolve` | `facet.rs`. `Facet<I, O>` is a `'static` handle with an id; `combine: fn(&[I]) -> O`. An `Extension` is an enum (`List`, `FacetValue`, `FacetCompute`, `Field`, `Prec(level, Box<Extension>)`, `Compartment(id, Box<Extension>)`). Resolving flattens by precedence into a slot table; values are `Arc<dyn Any + Send + Sync>`, read back by downcast in a typed accessor `state.facet(&TAB_SIZE) -> &usize`. Computed facets and fields record their dependencies so unchanged slots are reused, as CM does. |
| `EditorState` | `state.rs`. `{ doc: Text, selection: EditorSelection, config: Arc<Configuration>, values: Vec<Arc<dyn Any>> }`. Cloning a state is cheap (rope and `Arc`s), so the immutable model costs little. Also `transactionFilter`, `changeFilter`, `transactionExtender`, `languageData`, `charCategorizer` (replaces `is_word`/`word_bounds`). |
| `RangeSet`, `RangeValue`, `RangeSetBuilder`, `RangeSet.map/iter/spans/compare` | `rangeset.rs`. Chunked sorted ranges, mapped through a `ChangeDesc`. Used for search marks, spell underlines, text colors, folds, selection-match highlights. Replaces `styles.rs`' whole-text diffing and the "clear marks on edit" behaviour. |
| History (`history`, `undo`, `redo`, joining, selection events, mapping) | `commands/history.rs`. `HistoryState { done, undone }` branches of `HistEvent { changes (inverted), effects, mapped, start_selection, selections_after }`. Joins `input.type`/`delete` events within `newGroupDelay` (CM: 500 ms; Notey today: 1 s) when adjacent; maps stored events through `add_to_history: false` changes instead of clearing the stack; `invertedEffects` facet; `isolateHistory`. Find/replace and plugin edits become undoable. |
| Commands (cursor moves, deletes, line ops, indent) | `commands/`. `fn(&mut EditorView) -> bool` (or a target holding state + dispatch). Port the subset Notey has, then the rest. |
| `SearchCursor`, `RegExpCursor`, search state | `search/`. A cursor over rope chunks. Literal search ports directly. For regex, see 4.3. |
| Autocomplete, `closeBrackets` | Later phase. State machine and sources port directly; the popup is an egui `Area`. |

### 4.2 Replace

| CodeMirror | Rust/egui replacement |
|---|---|
| `Text` (B-tree of line arrays) | A `Text` newtype over a rope, **`crop`** recommended (see 4.3). Keeps CM's `line(n)`, `lineAt(pos)`, `sliceString`, `iter` API on top. |
| DOM content view (`DocView` and its `Tile` tree, DOM reconciliation) | **Per-frame egui painting.** Each visible line becomes a `LayoutJob` built from the text, highlighting and mark decorations; egui caches identical galleys. No retained tree to reconcile. |
| Measure/write cycle (`requestMeasure`) | The galley *is* the measurement. After layout, the view writes measured line heights back into the height map in the same frame. |
| `HeightMap`, viewport | Port nearly as is: CM's height map is a mutable binary tree (`HeightMapText`, `HeightMapBlock`, `HeightMapGap` for unmeasured estimates, rebalanced at 2:1) fed by a `HeightOracle`; the viewport is the visible area plus a margin (CM: 1000 px) plus the main selection's lines, and over-long lines are cut into `LineGap`s. It replaces Notey's flat `wrap_rows` / `wrap_prefix` vectors, which are O(lines) to rebuild. |
| DOM input, `InputState`, `DOMObserver`, `contenteditable` | egui `Event`s → a **keymap facet** (`KeyBinding { key, run, shift }`) → commands → transactions. `Event::Text` → `inputHandler` facet, then insert. IME: egui `ImeEvent::Preedit/Commit`; preedit shown as a temporary mark decoration. |
| `coordsAtPos`, `posAtCoords`, `moveVertically`, `moveToLineBoundary` | Galley hit testing (`pos_from_cursor`, `cursor_from_pos`) plus the height map for lines that aren't laid out. Logic of CM's cursor motion ports; geometry comes from galleys. |
| `ViewPlugin` | `trait ViewPlugin { fn update(&mut self, u: &ViewUpdate); fn decorations(&self) -> Option<&DecorationSet>; fn paint(&self, ...) {} }`, created from a facet, one instance per view. |
| Decorations: mark / line | Mark → `TextFormat` overrides (color, background, underline, italics) on the `LayoutJob`. Line → a background rect or gutter marker. |
| Decorations: widget / replace (block) | Inline widgets → a reserved-width section in the job and a paint callback. Replace → skip the range when building the job (folding). Block widgets → height-map entries painted between lines. Keep CM's rule that only decoration *sets* (not per-view functions) may add block widgets or replace line breaks, since those change heights; and keep its `Side` ordering constants for widgets at the same position. |
| Panels, tooltips, gutters | egui `Panel`s, `Area`s / `Tooltip`s, and a painted gutter column (`gutter` facet with markers as `RangeSet`). |
| CSS themes (`EditorView.theme`, `HighlightStyle`) | `Theme` facet holding colors (from Notey's `Palette`); `HighlightStyle` maps highlight tags to `TextFormat`. |
| Bidi (`BidiSpan`, `computeOrder`) | **No equivalent in egui's text layout today**; see risks. |

### 4.3 Crates that could stand in

| Need | Option | Trade-off |
|---|---|---|
| Text | `crop` 0.4 (MIT) | **Recommended.** Byte-indexed B-tree rope with `Arc` copy-on-write (16-byte clones, so immutable states are cheap), line metric, optional `utf16-metric` and `graphemes` features. Active (pushes Aug 2026). No char metric: char offsets for the plugin API come from line lookup plus a scan of one line. |
| | `ropey` 1.6 (MIT) | Mature and widely used, cheap clones, tracks chars and lines, but **char-indexed**, and 1.x has not had a release since 2023. |
| | `ropey` 2.0 (beta, MIT/Apache) | Byte-indexed with optional char/UTF-16/line metrics: the best fit on paper, but still `2.0.0-beta.1` (Aug 2025). A candidate to switch to once stable; the `Text` newtype keeps that cheap. |
| | Port CM's `Text` | 32-way tree balanced by line count, one JS string per line. Exact semantics, but it counts UTF-16 units and gives no advantage over a rope. Not recommended. |
| | Zed's `rope` / `sum_tree` | Good prior art for a summarised B-tree (also a model for `RangeSet` and the height map). Zed's `rope` and `text` crates are **GPL-3.0**: study, don't copy. |
| Parsing | `syntect` (current) | Already powers ~45 file-type plugins as data (`.sublime-syntax`). Line-based, like CM's `StreamLanguage`. No syntax tree, so no tree-based folding, indentation or bracket structure. |
| | `tree-sitter` 0.27 (MIT) | Mature, incremental (edit the old tree with `InputEdit`, reparse with it), time-sliceable through `ParseOptions::progress_callback`, grammars for nearly every language, query-based highlighting (`.scm` files are data). But each grammar is compiled C code: bundling grammars grows the binary; loading them at runtime as plugins means DLLs or the WASM loader (which pulls in `wasmtime`). |
| | Port Lezer | Matches CodeMirror exactly. The runtime (`@lezer/common` + `@lezer/lr`: trees, `TreeBuffer`, fragment reuse, GLR with at most 12 stacks, error recovery) is a contained port, and it can read the serialised tables that `@lezer/generator` emits (format documented in its `encode.ts`/`decode.ts`), so grammars could be compiled with the JS tool and shipped as data in file-type plugins. Costs: a Node step in the grammar build (or a port of the generator: full LR(1) collapsed to LALR-like), and several grammars rely on hand-written JS *external tokenizers* that each need a Rust port. **No mature Rust port exists**: `rezel` says it is not production-ready and has no incremental reparse; `texera-core` (a claimed `@codemirror/state` port) is GPL-3.0 and days old. |
| Regex over a rope | `regex-cursor` (on `regex-automata`) | Searches rope chunks without flattening. No look-around/back-references, which `fancy-regex` (current) supports. A pragmatic option: materialise a `String` only for patterns that need `fancy-regex`. |
| Graphemes, words | `unicode-segmentation` | Cursor motion by grapheme cluster (CM uses `findClusterBreak`). Notey currently moves by `char`, which splits emoji and combining marks. |
| Bidi | `unicode-bidi` | Computes visual order, but egui's layout would still need to render it. |
| Shaping | `cosmic-text` 0.19, `parley` 0.12 | Real shaping and bidi; replacing egui's text layout inside the editor is a large job. Out of scope for parity. egui 0.35 already shapes with `harfrust` (kerning, ligatures) but does not reorder bidi text. |

### 4.4 Positions

CodeMirror counts UTF-16 code units everywhere, because its positions are JS
string offsets; `ChangeSet`, `RangeSet`, the height map, the history and
Lezer's trees all carry them, so the choice of unit reaches every module and
has to be made first. Rust strings, ropes, regex engines and
tree-sitter work in **bytes**. Notey's plugin API, find and styles work in
**chars**. Recommendation: the core uses **byte offsets** (always on a char
boundary), and the host converts at the edges that promise chars (plugin API
v3, go-to, status bar): find the line in O(log n), then count chars within
that line. The alternative, keeping char offsets everywhere, avoids touching
the plugin API but makes every regex and parser call convert.

### 4.5 Language layer

`@codemirror/language`'s *architecture* ports regardless of the parser:
a `Language` facet chosen per document, a parse state field that advances in
time-boxed slices from the frame loop (CM's work budget idea) and keeps
the work since the last edit, a `HighlightStyle` mapping from abstract tags
to colors, and `indentService` / `foldService` facets. Behind it, a
`Parser` trait with backends:

1. **`StreamParser` over syntect** first. It keeps every existing file-type
   plugin working unchanged and is how CM itself runs legacy modes. Line
   states are stored at intervals and mapped through changes instead of
   being truncated.
2. **A tree backend** (tree-sitter or a Lezer port) later, for languages
   where structure matters. This is a scope decision (question 3).

## 5. Module layout

One new workspace crate, independent of `notey`'s app code, egui only in
`view/`:

```
crates/notey-editor/              (name to be decided)
  LICENSE-CodeMirror              MIT notice, kept verbatim
  src/
    lib.rs
    text.rs                       Text newtype over the rope (crop); lines, slices, iterators, offset conversion
    state/
      change.rs                   ChangeSet, ChangeDesc, MapMode
      selection.rs                EditorSelection, SelectionRange
      transaction.rs              Transaction, TransactionSpec, Annotation, StateEffect
      facet.rs                    Facet, StateField, Extension, Prec, Compartment, Configuration
      state.rs                    EditorState, filters, extenders, language data
      rangeset.rs                 RangeSet, RangeValue, RangeSetBuilder
      charcat.rs                  CharCategory, word boundaries
    commands/
      history.rs                  undo/redo
      cursor.rs, delete.rs, lines.rs, indent.rs
    search/
      cursor.rs                   SearchCursor, regex cursor
      query.rs                    search state field + effects
    language/
      language.rs                 Language facet, parse field, time budget
      stream.rs                   StreamParser trait + syntect adapter
      highlight.rs                tags, HighlightStyle
      indent.rs, fold.rs
    view/                         the only egui-dependent module (feature "egui")
      view.rs                     EditorView: dispatch, ViewUpdate, frame entry point
      plugin.rs                   ViewPlugin
      decoration.rs               Decoration kinds, DecorationSet
      heightmap.rs                line heights, viewport
      layout.rs                   LayoutJob building, ligatures, tab stops (moved from editor.rs)
      input.rs                    egui events, keymap, IME, clipboard
      motion.rs                   coords <-> positions, vertical motion
      paint.rs                    selections, carets, current line
      gutter.rs                   line numbers, markers
    autocomplete/                 later
  tests/                          ports of CodeMirror's state/commands tests
```

Notey's app keeps `Document` for file I/O (encoding, line endings, path) but
its `text: String` becomes the `EditorState` (or a handle to it). Spellcheck
and text colors become `ViewPlugin`s / state fields that produce decoration
sets. The plugin host turns a script's edits into a transaction with
`user_event = "plugin"`.

## 6. Milestones

Each milestone ends with tests passing and, from M3, a build the user can
run. The new editor runs **beside** the current one behind a setting until
M6.

| # | Milestone | Done when |
|---|---|---|
| M0 | Crate skeleton, licence notice, `Text` over the chosen rope | Builds in the workspace; `Text` line/offset tests pass. |
| M1 | `ChangeSet`, `EditorSelection`, `Transaction`, facets, fields, `Prec`, compartments, annotations, effects, filters, `RangeSet` | Headless; CodeMirror's `test-change`, `test-selection`, `test-facet`/`test-state`, `test-rangeset` cases ported and passing. |
| M2 | History and the commands Notey has today: char/word/line motion, Home/End, PageUp/Down, select all, backspace/delete (char and word), Enter, Tab, Escape (collapse cursors), add cursor, keymap facet | Headless command tests, including the cases in `editor.rs`' current tests. |
| M3 | egui view: virtualised lines, height map, word wrap, carets, multi-selection painting, mouse (click, double/triple, drag, Ctrl+Click, Alt+drag column), autoscroll while selecting, IME commit and candidate window, gutter, current line, focus lock, AccessKit info | Selectable as "experimental editor" in Settings. The focus, selection and IME fixes from 0.5.3–0.5.6 still hold (the existing tests move over). |
| M4 | Decorations: search marks, spell underlines and the suggestion menu, plugin text colors, all as mapped `RangeSet`s | Marks and colors survive edits; no whole-text diffing per frame. |
| M5 | Language layer with the syntect stream adapter, time-budgeted, imported themes | All current file-type plugins highlight the same as today; a 50 MB file stays responsive while highlighting catches up. |
| M6 | App integration: find/replace, go to line, plugins (`ScriptCtx` edits → transactions), session save/restore, modified flag from history or a saved-version marker instead of a `saved_text` copy | **Parity.** New editor becomes the default; the old `editor.rs` (and possibly egui `TextEdit` mode) is removed one release later. |
| M7+ | Beyond parity, in an order the user picks: search panel over the rope, bracket matching, `closeBrackets`, auto-indent, code folding, autocomplete, a tree parser backend, grapheme-cluster motion, bidi | Per feature. |

## 7. Risks

1. **Size.** CodeMirror 6 is tens of thousands of lines of TypeScript across
   the packages above, plus Lezer. A full port is a multi-month project.
   Parity with Notey's current editor needs roughly `state`, a subset of
   `commands`, the history, and a new egui view (M0–M6); that is the
   realistic first target.
2. **egui text layout.** egui 0.35 shapes with `harfrust` but does not
   reorder bidirectional text, so right-to-left text can't match CodeMirror. Fixing it means a
   different text stack inside the editor (`cosmic-text`/`parley`), which is
   a project of its own.
3. **Immediate mode vs retained DOM.** CodeMirror's view relies on
   retaining DOM between updates. Notey re-lays out visible lines every
   frame; this is fine for a viewport of lines but needs galley caching
   keyed by line text + decorations + width, and care with very long lines
   (a single 1 MB line).
4. **Dynamic typing in the extension system.** Facets and effects are
   loosely typed in JS. In Rust they need `Any`/downcasting behind typed
   handles; getting an ergonomic API that does not leak `Box<dyn Any>` to
   callers takes iteration.
5. **Parser choice** (see 4.3 and question 3). Lezer gives fidelity but a
   lot of porting and per-grammar JS; tree-sitter gives breadth but fights
   the "file types are downloadable data plugins" rule; syntect keeps today's
   plugins but gives no tree.
6. **Regressions in hard-won behaviour.** Focus handling, IME anchoring,
   drag selection and the shutdown path were fixed release by release. Running
   the new editor beside the old one and moving the existing tests over is
   the mitigation.
7. **Position units.** Moving the core to byte offsets while the plugin API
   stays in chars adds conversions at the boundary; a missed conversion is a
   silent off-by-N bug with non-ASCII text. Property tests with random
   Unicode edits cover this.
8. **Plugin API compatibility.** Plugins receive the whole text as a string
   today. That can stay (materialise on demand), but large files pay for it;
   a range-based API would be a v4 change.

## 8. What changes for users at parity

Nothing visible should regress. Gains: undo covers find/replace, Replace
All (as precise changes rather than a whole-document swap) and plugin edits;
search marks and colors stay put while typing; edits in large files no longer
rebuild the whole line index; grapheme-correct cursor motion (if taken into
scope).

## 9. Licence

CodeMirror 6 and Lezer are MIT-licensed. The notices, from the LICENSE files
on the CodeMirror forge:

- `@codemirror/state`, `view`, `language`, `commands`, `search`,
  `autocomplete`, `lint`, `streamparser`: "Copyright (C) 2018-2021 by Marijn
  Haverbeke <marijn@haverbeke.berlin> and others"
- `@lezer/common`, `lr`, `highlight`, `generator`: "Copyright (C) 2018 by
  Marijn Haverbeke <marijn@haverbeke.berlin> and others"

A port is a derivative work, so the crate must carry that
notice and the MIT permission text (`LICENSE-CodeMirror`), and ported files
should say which CodeMirror file they derive from. MIT is compatible with
Notey's Apache-2.0 licence; the README's licence section should mention the
included MIT-licensed code.

## 10. Decisions (2026-10-30)

1. **Order:** build the Rust version of CodeMirror first, then rebuild Notey
   around it. This replaces the "run beside, then swap" plan in §6: the
   milestones there now describe the order of the port, and Notey switches
   once the port reaches parity with today's editor.
2. **Where:** its own repository, `D:\Projects\codemirror-rs` (local only
   until the user decides to publish it). Notey will depend on it by path.
   One crate per CodeMirror package (`codemirror-state`,
   `codemirror-commands`, `codemirror-view`, ...) rather than the single
   crate sketched in §5; the modules inside each crate follow §5.
3. **Fidelity:** package by package, mirroring CodeMirror's packages and
   APIs, starting with `state`. The view renders with egui.
4. **Parser:** decided later, when the port reaches
   `@codemirror/language`. `state`, `commands` and `view` don't depend on it.
   (`commands` uses the parser-free part of `language`: indent units,
   indent services and plain-text bracket matching. That part is ported as
   a first slice of `codemirror-language` and behaves like CodeMirror with
   no language loaded.)
5. **Positions:** byte offsets (the recommendation in §4.4); the user did not
   object. Notey's plugin API keeps char offsets and converts at the
   boundary.
6. **Text:** `crop` rope, as recommended in §4.3.
7. **UI framework (changed 2026-10-30): GPUI, not egui.** A GPUI view of
   the port (Zed's framework, `gpui` 0.2.2) felt much faster to the user
   than the egui view, and renders CJK and emoji through DirectWrite.
   Both run under 2 ms per frame in release builds. The port's view moves
   to GPUI, the egui view is removed (it stays in the codemirror-rs
   history), and the Notey rebuild uses GPUI for the whole app. The egui
   view and §5's egui modules (`LayoutJob`, `ScrollArea`, egui events) are
   superseded.

Done so far:

- `@codemirror/state` (M0 and M1 in §6), with CodeMirror's test suites for
  text, changes, selections, facets, state and range sets ported and
  passing.
- The state-level half of `@codemirror/commands` (M2): undo history,
  comment toggling, and the commands that need no layout: delete by
  character, word or indent unit, Enter with indentation, Tab, indent and
  dedent, move and copy lines, select all and select line, collapse the
  selection, matching bracket, transpose, trailing whitespace. The history,
  comment and state-level command tests are ported and pass. The tests that
  need a parser (`indentSelection`, JavaScript indentation) wait for
  `language`.

- The rest of M2 and the first slice of M3, `codemirror-view`, now drawn
  with GPUI (decision 7): an `EditorView` entity (virtualised lines, word
  wrap, tab stops, multiple selections and carets, active line, line
  numbers, scrollbar, scroll anchoring, IME through GPUI's input handler,
  clipboard, mouse selection including Ctrl-click and Alt-drag rectangles),
  view plugins and updates, mark and line decorations, the height map
  (ported with its tests), key bindings, cursor motion, and in `commands`
  the layout-dependent commands plus the standard, default, Emacs and
  history keymaps. The motion tests run in GPUI's test context. A
  `codemirror-demo` window shows it working. The first egui version of the
  view is in the codemirror-rs history (commit `1aa5e46`).

Still to do in M3: drawing widgets and replaced (folded) ranges, bidi text,
a general gutter API, and accessibility. Notey itself is then rebuilt on
GPUI around this view (instead of the "experimental editor" switch that
§6 planned for the egui app).

The Notey rebuild is done: since 0.8.0, `main` is a GPUI app with Notey's
title bar, tabs, menus, status bar, Settings tab, files, session and
single-instance forwarding, one `EditorView` per tab. Its modules are
listed in [gpui-rebuild.md](gpui-rebuild.md).

`@codemirror/search` is ported as `codemirror-search` (cursors, queries,
find and replace commands, match highlighting, key bindings; the host
draws the panel), and Notey's Find and Replace bar is built on it.
