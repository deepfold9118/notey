//! Preview editor: a virtualized text-editing widget.
//!
//! Unlike egui's `TextEdit` (which lays out the whole buffer as one galley),
//! this widget lays out **only the visible lines**, so multi-megabyte files
//! stay responsive. It owns its cursor/selection model and undo stack, keyed
//! by character offsets so the rest of the app (find, go-to, plugins) speaks
//! the same coordinates as the classic editor.
//!
//! Features: word wrap (per-line wrap-row cache), multi-cursor editing
//! (Ctrl+Click to add cursors, Alt+drag for column selection, Escape to
//! collapse), per-line spellcheck, compound undo, and basic IME (candidate
//! window anchoring + commit; no inline composition preview yet).

use eframe::egui::{
    self, Color32, Event, FontId, Key, Modifiers, PointerButton, Sense, TextFormat,
};
use eframe::egui::text::{CCursor, LayoutJob};

use crate::spell::{self, Spell};
use crate::theme::Palette;

// ---------- per-document editor state ----------

#[derive(Clone, Copy)]
struct LineMeta {
    byte: usize,
    chr: usize,
}

struct SingleEdit {
    /// Char offset in the text as it was BEFORE the whole group applied
    /// (the group is applied in descending offset order, so every `at`
    /// stays valid in original coordinates).
    at: usize,
    removed: String,
    inserted: String,
}

#[derive(Clone)]
struct SelSnapshot {
    anchor: usize,
    cursor: usize,
    extras: Vec<(usize, usize)>,
}

/// One undo step: one or more simultaneous range replacements (one per
/// cursor), stored ascending by `at`.
struct EditOp {
    edits: Vec<SingleEdit>,
    sel_before: SelSnapshot,
    sel_after: SelSnapshot,
    time: f64,
}

#[derive(Default)]
pub struct EditorState {
    /// Selection head (where the caret is), in chars.
    pub cursor: usize,
    /// Selection anchor, in chars. anchor == cursor means no selection.
    pub anchor: usize,
    pub scroll_to_cursor: bool,

    lines: Vec<LineMeta>, // line starts; always at least one entry
    max_line_chars: usize,
    total_chars: usize,
    revision_seen: u64,
    indexed_once: bool,

    undo: Vec<EditOp>,
    redo: Vec<EditOp>,

    /// Misspelled word under the last right-click: (start, end, word).
    ctx_word: Option<(usize, usize, String)>,
    ctx_suggestions: Vec<String>,

    /// Wrapped-row count per line (u32::MAX = needs layout).
    wrap_rows: Vec<u32>,
    /// wrap_prefix[i] = visual rows before line i; len = lines + 1.
    wrap_prefix: Vec<u32>,
    wrap_valid: bool,
    /// (wrap width bits, font size bits) the cache was built for.
    wrap_key: (u32, u32),
    /// Preserved x position for repeated Up/Down.
    desired_x: Option<f32>,
    /// Secondary cursors as (anchor, head); the primary lives in
    /// `anchor`/`cursor`. Cleared by plain clicks, Escape, and external
    /// selection changes.
    pub extra_sels: Vec<(usize, usize)>,
    /// Origin of an in-progress Alt+drag column selection, in content
    /// coordinates relative to the text origin.
    column_drag_origin: Option<(f32, f32)>,
    /// Syntax highlight cache; None = plain text.
    pub hl: Option<crate::syntax::HlCache>,
}

impl EditorState {
    pub fn selection(&self) -> (usize, usize) {
        (self.cursor.min(self.anchor), self.cursor.max(self.anchor))
    }

    pub fn set_selection(&mut self, anchor: usize, cursor: usize) {
        self.anchor = anchor;
        self.cursor = cursor;
        self.extra_sels.clear();
        self.scroll_to_cursor = true;
    }

    pub fn collapse_extras(&mut self) {
        self.extra_sels.clear();
    }

    /// All selections ([primary, extras...]), unnormalized (anchor, head).
    fn all_sels(&self) -> Vec<(usize, usize)> {
        let mut v = Vec::with_capacity(1 + self.extra_sels.len());
        v.push((self.anchor, self.cursor));
        v.extend(self.extra_sels.iter().copied());
        v
    }

    /// Drop duplicate/overlapping cursors (primary wins).
    fn dedup_sels(&mut self) {
        let primary = self.selection();
        let mut seen: Vec<(usize, usize)> = vec![primary];
        self.extra_sels.retain(|&(a, h)| {
            let r = (a.min(h), a.max(h));
            let clashes = seen
                .iter()
                .any(|s| *s == r || (r.0 < s.1 && r.1 > s.0) || (r.0 == r.1 && s.0 == s.1 && r.0 == s.0));
            if clashes {
                false
            } else {
                seen.push(r);
                true
            }
        });
    }

    fn snapshot(&self) -> SelSnapshot {
        SelSnapshot {
            anchor: self.anchor,
            cursor: self.cursor,
            extras: self.extra_sels.clone(),
        }
    }

    fn restore_snapshot(&mut self, s: &SelSnapshot) {
        self.anchor = s.anchor;
        self.cursor = s.cursor;
        self.extra_sels = s.extras.clone();
        self.clamp();
    }

    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.total_chars);
        self.anchor = self.anchor.min(self.total_chars);
        let n = self.total_chars;
        for s in &mut self.extra_sels {
            s.0 = s.0.min(n);
            s.1 = s.1.min(n);
        }
    }

    /// Rebuild the line index if the document changed outside the editor.
    fn sync(&mut self, text: &str, revision: u64) {
        if self.indexed_once && self.revision_seen == revision {
            return;
        }
        self.rebuild_index(text);
        if let Some(hl) = &mut self.hl {
            hl.invalidate_all();
        }
        self.revision_seen = revision;
        self.indexed_once = true;
        self.clamp();
    }

    fn rebuild_index(&mut self, text: &str) {
        self.lines.clear();
        self.lines.push(LineMeta { byte: 0, chr: 0 });
        self.max_line_chars = 0;
        let mut chr = 0usize;
        let mut line_start_chr = 0usize;
        for (b, c) in text.char_indices() {
            chr += 1;
            if c == '\n' {
                self.max_line_chars = self.max_line_chars.max(chr - 1 - line_start_chr);
                self.lines.push(LineMeta { byte: b + 1, chr });
                line_start_chr = chr;
            }
        }
        self.total_chars = chr;
        self.max_line_chars = self.max_line_chars.max(chr - line_start_chr);
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn total_visual_rows(&self) -> u32 {
        *self.wrap_prefix.last().unwrap_or(&1).max(&1)
    }

    fn line_at_visual_row(&self, v: u32) -> usize {
        let last = self.line_count().saturating_sub(1);
        match self.wrap_prefix.binary_search(&v) {
            Ok(i) => i.min(last),
            Err(i) => i.saturating_sub(1).min(last),
        }
    }

    /// Line containing the given char offset.
    fn line_of_char(&self, chr: usize) -> usize {
        match self.lines.binary_search_by(|m| m.chr.cmp(&chr)) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        }
    }

    /// The text of line `i` (no trailing newline) plus its char start.
    fn line_slice<'t>(&self, text: &'t str, i: usize) -> (&'t str, usize) {
        let start = self.lines[i];
        let end_byte = self
            .lines
            .get(i + 1)
            .map(|m| m.byte - 1) // strip '\n'
            .unwrap_or(text.len());
        (&text[start.byte..end_byte.max(start.byte)], start.chr)
    }

    fn char_to_byte(&self, text: &str, chr: usize) -> usize {
        let line = self.line_of_char(chr);
        let meta = self.lines[line];
        let rest = chr - meta.chr;
        text[meta.byte..]
            .char_indices()
            .nth(rest)
            .map(|(b, _)| meta.byte + b)
            .unwrap_or(text.len())
    }

    // ---------- edits ----------

    fn ensure_index(&mut self, text: &str) {
        if !self.indexed_once {
            self.rebuild_index(text);
            self.indexed_once = true;
            self.clamp();
        }
    }

    /// Replace `range` (chars) with `insert` at the primary cursor only,
    /// recording undo. Collapses secondary cursors.
    pub fn apply_edit(
        &mut self,
        text: &mut String,
        range: (usize, usize),
        insert: &str,
        now: f64,
    ) {
        self.extra_sels.clear();
        self.apply_multi(text, vec![range], insert, now);
    }

    /// Replace every range in `sels` (input order: [primary, extras...])
    /// with `insert`, as ONE undo step. Each cursor ends up collapsed after
    /// its own insertion.
    pub fn apply_multi(
        &mut self,
        text: &mut String,
        sels: Vec<(usize, usize)>,
        insert: &str,
        now: f64,
    ) {
        self.ensure_index(text);
        // normalize: clamp, sort ascending, drop overlaps (keep first seen)
        let mut norm: Vec<(usize, usize, usize)> = sels
            .iter()
            .enumerate()
            .map(|(i, &(a, b))| {
                let (a, b) = (a.min(b), a.max(b));
                (a.min(self.total_chars), b.min(self.total_chars), i)
            })
            .collect();
        norm.sort_by_key(|&(a, _, _)| a);
        let mut filtered: Vec<(usize, usize, usize)> = Vec::with_capacity(norm.len());
        for r in norm {
            if let Some(l) = filtered.last() {
                // overlapping range, or duplicate caret at the same spot
                if r.0 < l.1 || (r.0 == l.0 && r.1 == l.1) {
                    continue;
                }
            }
            filtered.push(r);
        }
        if filtered.is_empty() {
            return;
        }

        let sel_before = self.snapshot();
        let ins_chars = insert.chars().count();
        let group_first_line = self.line_of_char(filtered[0].0);

        // apply descending so original-space offsets stay valid
        let mut edits: Vec<SingleEdit> = Vec::with_capacity(filtered.len());
        for &(a, b, _) in filtered.iter().rev() {
            let first_line = self.line_of_char(a);
            let last_line_old = self.line_of_char(b);
            let ab = self.char_to_byte(text, a);
            let bb = self.char_to_byte(text, b);
            let removed = text[ab..bb].to_string();
            text.replace_range(ab..bb, insert);

            // wrap cache: mark the edited lines dirty (original line space —
            // valid because we go top-down from the highest edit)
            if self.wrap_valid {
                let span_old = last_line_old - first_line + 1;
                let removed_newlines = removed.matches('\n').count();
                let inserted_newlines = insert.matches('\n').count();
                let span_new = (span_old as i64 + inserted_newlines as i64
                    - removed_newlines as i64)
                    .max(1) as usize;
                if first_line + span_old <= self.wrap_rows.len() {
                    self.wrap_rows.splice(
                        first_line..first_line + span_old,
                        std::iter::repeat(u32::MAX).take(span_new),
                    );
                } else {
                    self.wrap_valid = false;
                }
            }
            edits.push(SingleEdit {
                at: a,
                removed,
                inserted: insert.to_string(),
            });
        }
        edits.reverse(); // store ascending
        self.rebuild_index(text);
        let n_lines = self.line_count();
        if let Some(hl) = &mut self.hl {
            hl.invalidate_from(group_first_line, n_lines);
        }

        // final caret positions, ascending with cumulative shift
        let mut shift: i64 = 0;
        let mut new_pos: Vec<(usize, usize)> = Vec::with_capacity(filtered.len()); // (orig_idx, pos)
        for (k, &(a, b, orig)) in filtered.iter().enumerate() {
            let pos = (a as i64 + shift) as usize + ins_chars;
            shift += ins_chars as i64 - (b - a) as i64;
            new_pos.push((orig, pos));
            let _ = k;
        }
        new_pos.sort_by_key(|&(orig, _)| orig);
        if let Some(&(_, p)) = new_pos.first() {
            self.anchor = p;
            self.cursor = p;
        }
        self.extra_sels = new_pos.iter().skip(1).map(|&(_, p)| (p, p)).collect();
        self.clamp();
        self.scroll_to_cursor = true;
        self.desired_x = None;

        // group rapid single-cursor typing into one undo step
        let single = edits.len() == 1 && self.extra_sels.is_empty();
        let mergeable = single
            && edits[0].removed.is_empty()
            && ins_chars == 1
            && insert != "\n"
            && self
                .undo
                .last()
                .map(|op| {
                    op.edits.len() == 1
                        && op.edits[0].removed.is_empty()
                        && now - op.time < 1.0
                        && op.edits[0].at + op.edits[0].inserted.chars().count()
                            == edits[0].at
                })
                .unwrap_or(false);
        if mergeable {
            let snap = self.snapshot();
            let op = self.undo.last_mut().unwrap();
            op.edits[0].inserted.push_str(insert);
            op.sel_after = snap;
            op.time = now;
        } else {
            self.undo.push(EditOp {
                edits,
                sel_before,
                sel_after: self.snapshot(),
                time: now,
            });
            if self.undo.len() > 10_000 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
    }

    pub fn undo(&mut self, text: &mut String) {
        self.ensure_index(text);
        let Some(op) = self.undo.pop() else { return };
        // Revert ascending: when edit k is undone, all edits below it are
        // already restored, so the text below k matches the original and the
        // stored original-space offset is directly valid. (Edits above k are
        // still applied, but they cannot shift position k.)
        for e in &op.edits {
            let ins_chars = e.inserted.chars().count();
            let ab = char_to_byte_scan(text, e.at);
            let eb = char_to_byte_scan(text, e.at + ins_chars);
            text.replace_range(ab..eb, &e.removed);
        }
        self.wrap_valid = false;
        self.rebuild_index(text);
        if let Some(hl) = &mut self.hl {
            hl.invalidate_all();
        }
        self.restore_snapshot(&op.sel_before);
        self.scroll_to_cursor = true;
        self.redo.push(op);
    }

    pub fn redo(&mut self, text: &mut String) {
        self.ensure_index(text);
        let Some(op) = self.redo.pop() else { return };
        // reapply descending: original-space offsets stay valid
        for e in op.edits.iter().rev() {
            let ab = char_to_byte_scan(text, e.at);
            let eb = char_to_byte_scan(text, e.at + e.removed.chars().count());
            text.replace_range(ab..eb, &e.inserted);
        }
        self.wrap_valid = false;
        self.rebuild_index(text);
        if let Some(hl) = &mut self.hl {
            hl.invalidate_all();
        }
        self.restore_snapshot(&op.sel_after);
        self.scroll_to_cursor = true;
        self.undo.push(op);
    }

    /// External code changed the text directly (find/replace, plugins):
    /// undo history for those changes is not tracked in the preview editor.
    pub fn note_external_change(&mut self) {
        self.indexed_once = false;
        self.wrap_valid = false;
        if let Some(hl) = &mut self.hl {
            hl.invalidate_all();
        }
        self.undo.clear();
        self.redo.clear();
    }
}

/// Index-free char→byte conversion for texts whose line index is stale
/// (used during undo/redo replay).
fn char_to_byte_scan(text: &str, chr: usize) -> usize {
    text.char_indices()
        .nth(chr)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

// ---------- word helpers ----------

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn word_bounds(chars: &[char], i: usize) -> (usize, usize) {
    let n = chars.len();
    if n == 0 {
        return (0, 0);
    }
    let i = i.min(n - 1);
    let class = |c: char| {
        if is_word(c) {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    let k = class(chars[i]);
    let mut a = i;
    while a > 0 && class(chars[a - 1]) == k {
        a -= 1;
    }
    let mut b = i + 1;
    while b < n && class(chars[b]) == k {
        b += 1;
    }
    (a, b)
}

// ---------- the widget ----------

pub struct EditorTheme {
    pub font: FontId,
    pub row_height: f32,
    pub text: Color32,
    pub weak: Color32,
    pub selection: Color32,
    pub caret: Color32,
    pub current_line: Color32,
    pub misspell: Color32,
    pub gutter_bg: Color32,
    pub mark: Color32,
    pub dark: bool,
}

pub fn theme_from_palette(p: &Palette, font: FontId, row_height: f32, dark: bool) -> EditorTheme {
    EditorTheme {
        font,
        row_height,
        text: p.text,
        weak: p.text_weak,
        selection: p.selection,
        caret: p.accent,
        current_line: if dark {
            Color32::from_rgba_unmultiplied(255, 255, 255, 5)
        } else {
            Color32::from_rgba_unmultiplied(0, 0, 0, 6)
        },
        misspell: Color32::from_rgb(232, 82, 82),
        gutter_bg: p.chrome,
        mark: Color32::from_rgba_unmultiplied(
            p.accent.r(),
            p.accent.g(),
            p.accent.b(),
            48,
        ),
        dark,
    }
}

pub struct ShowResult {
    pub changed: bool,
    /// "Add to dictionary" was clicked for this word.
    pub add_word: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    editor_id: egui::Id,
    text: &mut String,
    revision: u64,
    state: &mut EditorState,
    th: &EditorTheme,
    show_line_numbers: bool,
    wrap: bool,
    spell: Option<&Spell>,
    marks: &[(usize, usize)],
    request_focus: bool,
    imported_syntax_theme: Option<&syntect::highlighting::Theme>,
) -> ShowResult {
    state.sync(text, revision);
    let mut changed = false;
    let mut add_word: Option<String> = None;
    let now = ui.input(|i| i.time);

    let char_w = ui.fonts_mut(|f| f.glyph_width(&th.font, '0')).max(1.0);
    let row_h = th.row_height;
    let n_lines = state.line_count();

    let gutter_w = if show_line_numbers {
        let digits = n_lines.to_string().len().max(2);
        digits as f32 * char_w + 16.0
    } else {
        0.0
    };

    // wrap width: fit the viewport minus gutter, paddings, and scrollbar
    let avail_w = ui.available_width();
    let wrap_w: Option<f32> = if wrap {
        Some((avail_w - gutter_w - 6.0 - 18.0).max(char_w * 4.0))
    } else {
        None
    };

    // ---- wrap cache (row counts per line + prefix sums) ----
    let wrap_key = (
        wrap_w.map(|w| w.to_bits()).unwrap_or(u32::MAX),
        th.font.size.to_bits(),
    );
    if !state.wrap_valid || state.wrap_key != wrap_key || state.wrap_rows.len() != n_lines {
        state.wrap_rows = vec![u32::MAX; n_lines];
        state.wrap_key = wrap_key;
        state.wrap_valid = true;
    }
    fill_wrap_cache(state, text, ui, th, spell, wrap_w);

    let est_width = gutter_w + state.max_line_chars as f32 * char_w * 1.05 + 120.0;
    let content_w = if wrap {
        avail_w - 4.0
    } else {
        est_width.max(avail_w)
    };
    let total = egui::vec2(
        content_w,
        state.total_visual_rows() as f32 * row_h + row_h * 2.0,
    );

    let scroll = if wrap {
        egui::ScrollArea::vertical()
    } else {
        egui::ScrollArea::both()
    };
    scroll
        .id_salt(editor_id.with("scroll"))
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            // The interactive widget must carry `editor_id`: focus is requested
            // under that id, and accessibility clients (AccessKit) panic if the
            // focused id isn't a widget in the tree.
            let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
            let resp = ui.interact(rect, editor_id, Sense::click_and_drag());
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Editor")
            });
            let text_x = rect.left() + gutter_w + 6.0;
            let painter = ui.painter_at(ui.clip_rect());

            if show_line_numbers {
                let clip = ui.clip_rect();
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        clip.left_top(),
                        egui::pos2(clip.left() + gutter_w, clip.bottom()),
                    ),
                    0.0,
                    th.gutter_bg,
                );
            }

            if request_focus
                || resp.clicked()
                || resp.drag_started_by(PointerButton::Primary)
            {
                ui.memory_mut(|m| m.request_focus(editor_id));
            }
            let focused = ui.memory(|m| m.has_focus(editor_id));

            // capture the misspelled word under a right-click for the menu
            if resp.secondary_clicked() {
                state.ctx_word = None;
                state.ctx_suggestions.clear();
                if let (Some(sp), Some(pos)) = (spell, resp.interact_pointer_pos()) {
                    let c = pos_to_char(
                        state, text, pos, ui, th, spell, wrap_w, rect.top(), text_x, row_h,
                    );
                    let line = state.line_of_char(c);
                    let (slice, start_chr) = state.line_slice(text, line);
                    let chars: Vec<char> = slice.chars().collect();
                    let col = c.saturating_sub(start_chr).min(chars.len().saturating_sub(1));
                    if !chars.is_empty() && spell::is_word_char(chars[col]) {
                        let mut a = col;
                        while a > 0 && spell::is_word_char(chars[a - 1]) {
                            a -= 1;
                        }
                        let mut b = col + 1;
                        while b < chars.len() && spell::is_word_char(chars[b]) {
                            b += 1;
                        }
                        let word: String = chars[a..b].iter().collect();
                        let prev = if a > 0 { Some(chars[a - 1]) } else { None };
                        if spell::checkable(&word, prev) && !sp.check(&word) {
                            state.ctx_suggestions = sp.suggest(&word);
                            state.ctx_word = Some((start_chr + a, start_chr + b, word));
                        }
                    }
                }
            }

            // ---- mouse → cursor ----
            if let Some(pos) = resp.interact_pointer_pos() {
                let c = pos_to_char(
                    state, text, pos, ui, th, spell, wrap_w, rect.top(), text_x, row_h,
                );
                let mods = ui.input(|i| i.modifiers);
                if resp.double_clicked() {
                    let line = state.line_of_char(c);
                    let (slice, start_chr) = state.line_slice(text, line);
                    let chars: Vec<char> = slice.chars().collect();
                    let (a, b) = word_bounds(&chars, c.saturating_sub(start_chr));
                    state.anchor = start_chr + a;
                    state.cursor = start_chr + b;
                    state.collapse_extras();
                } else if resp.triple_clicked() {
                    let line = state.line_of_char(c);
                    let (slice, start_chr) = state.line_slice(text, line);
                    let end = start_chr + slice.chars().count();
                    state.anchor = start_chr;
                    state.cursor = end;
                    state.collapse_extras();
                } else if resp.drag_started_by(PointerButton::Primary) {
                    if mods.alt {
                        // column (box) selection
                        state.column_drag_origin =
                            Some((pos.x - text_x, pos.y - rect.top()));
                        state.anchor = c;
                        state.cursor = c;
                        state.collapse_extras();
                    } else {
                        state.column_drag_origin = None;
                        state.cursor = c;
                        if !mods.shift {
                            state.anchor = c;
                        }
                        state.collapse_extras();
                    }
                    state.desired_x = None;
                } else if resp.dragged_by(PointerButton::Primary) {
                    if let Some(origin) = state.column_drag_origin {
                        column_select(
                            state,
                            text,
                            ui,
                            th,
                            spell,
                            wrap_w,
                            origin,
                            (pos.x - text_x, pos.y - rect.top()),
                            row_h,
                        );
                    } else {
                        state.cursor = c;
                    }
                } else if resp.clicked() {
                    if mods.ctrl && !mods.shift && !mods.alt {
                        // Ctrl+Click adds a cursor
                        let old = (state.anchor, state.cursor);
                        state.extra_sels.push(old);
                        state.anchor = c;
                        state.cursor = c;
                        state.dedup_sels();
                    } else {
                        state.cursor = c;
                        if !mods.shift {
                            state.anchor = c;
                        }
                        state.collapse_extras();
                    }
                    state.desired_x = None;
                }
                if resp.drag_stopped_by(PointerButton::Primary) {
                    state.column_drag_origin = None;
                }
            }

            // ---- keyboard ----
            if focused {
                let events = ui.input(|i| i.events.clone());
                let page = (viewport.height() / row_h).max(1.0) as i64;
                for ev in &events {
                    match ev {
                        Event::Text(s) => {
                            if s.chars().all(|c| c.is_control()) {
                                continue;
                            }
                            let sels = state.all_sels();
                            state.apply_multi(text, sels, s, now);
                            changed = true;
                        }
                        Event::Paste(s) => {
                            let s = s.replace("\r\n", "\n").replace('\r', "\n");
                            let sels = state.all_sels();
                            state.apply_multi(text, sels, &s, now);
                            changed = true;
                        }
                        Event::Copy | Event::Cut => {
                            // multi-selection copy joins the pieces with newlines
                            let mut ranges: Vec<(usize, usize)> = state
                                .all_sels()
                                .iter()
                                .map(|&(a, h)| (a.min(h), a.max(h)))
                                .filter(|(a, b)| a != b)
                                .collect();
                            ranges.sort_by_key(|&(a, _)| a);
                            if !ranges.is_empty() {
                                let pieces: Vec<&str> = ranges
                                    .iter()
                                    .map(|&(a, b)| {
                                        let ab = state.char_to_byte(text, a);
                                        let bb = state.char_to_byte(text, b);
                                        &text[ab..bb]
                                    })
                                    .collect();
                                ui.ctx().copy_text(pieces.join("\n"));
                                if matches!(ev, Event::Cut) {
                                    let sels = state.all_sels();
                                    state.apply_multi(text, sels, "", now);
                                    changed = true;
                                }
                            }
                        }
                        Event::Ime(egui::ImeEvent::Commit(s)) => {
                            let sels = state.all_sels();
                            state.apply_multi(text, sels, s, now);
                            changed = true;
                        }
                        Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } => {
                            let vertical = match key {
                                Key::ArrowUp => Some(-1i64),
                                Key::ArrowDown => Some(1),
                                Key::PageUp => Some(-page),
                                Key::PageDown => Some(page),
                                _ => None,
                            };
                            if let Some(delta) = vertical {
                                // refresh geometry in case earlier events edited
                                state.collapse_extras();
                                fill_wrap_cache(state, text, ui, th, spell, wrap_w);
                                vertical_move(
                                    state,
                                    text,
                                    ui,
                                    th,
                                    spell,
                                    wrap_w,
                                    delta,
                                    modifiers.shift,
                                );
                                state.scroll_to_cursor = true;
                            } else if handle_key(state, text, *key, *modifiers, now, &mut changed)
                            {
                                state.scroll_to_cursor = true;
                                state.desired_x = None;
                            }
                        }
                        _ => {}
                    }
                }
            }

            // edits above may have changed geometry
            fill_wrap_cache(state, text, ui, th, spell, wrap_w);
            let n_lines = state.line_count();
            let total_rows = state.total_visual_rows();
            let sels_all = state.all_sels();
            let sel_ranges: Vec<(usize, usize)> = sels_all
                .iter()
                .map(|&(a, h)| (a.min(h), a.max(h)))
                .filter(|(a, b)| a != b)
                .collect();
            let heads: Vec<usize> = sels_all.iter().map(|&(_, h)| h).collect();
            let cursor_line = state.line_of_char(state.cursor);

            // ---- visible range ----
            let v_first = ((viewport.top() / row_h).floor().max(0.0) as u32)
                .min(total_rows.saturating_sub(1));
            let v_last = ((viewport.bottom() / row_h).ceil().max(0.0) as u32 + 1).min(total_rows);
            let first = state.line_at_visual_row(v_first);
            let last = (state.line_at_visual_row(v_last.saturating_sub(1)) + 1).min(n_lines);

            // syntax highlight: parse forward (cached) up to the visible bottom
            if let Some(mut hl) = state.hl.take() {
                hl.ensure(last, n_lines, th.dark, imported_syntax_theme, |i| {
                    let (s, _) = state.line_slice(text, i);
                    let mut l = String::with_capacity(s.len() + 1);
                    l.push_str(s);
                    l.push('\n');
                    l
                });
                state.hl = Some(hl);
            }

            for li in first..last {
                let y = rect.top() + state.wrap_prefix[li] as f32 * row_h;
                let line_rows = state.wrap_rows[li].max(1);
                let line_h = line_rows as f32 * row_h;
                let (slice, start_chr) = state.line_slice(text, li);
                let line_chars = slice.chars().count();
                let line_end = start_chr + line_chars;

                // current-line wash
                if li == cursor_line && sel_ranges.is_empty() {
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(rect.left(), y),
                            egui::vec2(rect.width(), line_h),
                        ),
                        0.0,
                        th.current_line,
                    );
                }

                let hl_spans: Option<Vec<(Color32, u32)>> = state
                    .hl
                    .as_ref()
                    .and_then(|h| h.spans.get(li).cloned().flatten());
                let galley =
                    line_galley_colored(ui, slice, th, spell, wrap_w, hl_spans.as_deref());

                // search marks (Mark All)
                for &(ma, mb) in marks {
                    if mb <= start_chr {
                        continue;
                    }
                    if ma > line_end {
                        break;
                    }
                    let a = ma.max(start_chr) - start_chr;
                    let b = mb.min(line_end) - start_chr;
                    paint_char_range(
                        &painter,
                        &galley,
                        a,
                        b,
                        y,
                        text_x,
                        wrap_w.unwrap_or(f32::INFINITY).min(rect.width()),
                        row_h,
                        th.mark,
                    );
                }

                // selection highlights (may span wrapped rows; one per cursor)
                for &(sel_a, sel_b) in &sel_ranges {
                    if !(sel_a < line_end + 1 && sel_b > start_chr) {
                        continue;
                    }
                    let a = sel_a.max(start_chr) - start_chr;
                    let b = sel_b.min(line_end) - start_chr;
                    let ra = galley.pos_from_cursor(CCursor::new(a));
                    let rb = galley.pos_from_cursor(CCursor::new(b));
                    let right_edge = wrap_w.unwrap_or(f32::INFINITY).min(rect.width());
                    let mut xb = rb.left();
                    if sel_b > line_end {
                        xb += char_w * 0.6; // show the selected newline
                    }
                    if (ra.top() - rb.top()).abs() < 0.5 {
                        painter.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(text_x + ra.left(), y + ra.top()),
                                egui::pos2(
                                    text_x + xb.max(ra.left() + 1.0),
                                    y + ra.top() + row_h,
                                ),
                            ),
                            0.0,
                            th.selection,
                        );
                    } else {
                        // first row: from a to the wrap edge
                        painter.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(text_x + ra.left(), y + ra.top()),
                                egui::pos2(text_x + right_edge, y + ra.top() + row_h),
                            ),
                            0.0,
                            th.selection,
                        );
                        // middle rows: full width
                        if rb.top() - ra.top() > row_h + 0.5 {
                            painter.rect_filled(
                                egui::Rect::from_min_max(
                                    egui::pos2(text_x, y + ra.top() + row_h),
                                    egui::pos2(text_x + right_edge, y + rb.top()),
                                ),
                                0.0,
                                th.selection,
                            );
                        }
                        // last row: from the left to b
                        painter.rect_filled(
                            egui::Rect::from_min_max(
                                egui::pos2(text_x, y + rb.top()),
                                egui::pos2(text_x + xb.max(1.0), y + rb.top() + row_h),
                            ),
                            0.0,
                            th.selection,
                        );
                    }
                }

                painter.galley(egui::pos2(text_x, y), galley.clone(), th.text);

                // carets (one per cursor on this line)
                if focused {
                    let blink_on = ((now * 2.0) as i64) % 2 == 0;
                    for (hi, &head) in heads.iter().enumerate() {
                        if head < start_chr || head > line_end {
                            continue;
                        }
                        let col = head - start_chr;
                        let r = galley.pos_from_cursor(CCursor::new(col));
                        if blink_on {
                            painter.rect_filled(
                                egui::Rect::from_min_size(
                                    egui::pos2(text_x + r.left(), y + r.top() + 2.0),
                                    egui::vec2(1.5, row_h - 4.0),
                                ),
                                0.0,
                                th.caret,
                            );
                        }
                        if hi == 0 {
                            // let the OS position the IME window at the primary caret
                            let caret_rect = egui::Rect::from_min_size(
                                egui::pos2(text_x + r.left(), y + r.top()),
                                egui::vec2(2.0, row_h),
                            );
                            ui.ctx().output_mut(|o| {
                                o.ime = Some(egui::output::IMEOutput {
                                    rect: caret_rect,
                                    cursor_rect: caret_rect,
                                    should_interrupt_composition: false,
                                });
                            });
                            ui.ctx().request_repaint_after(
                                std::time::Duration::from_millis(500),
                            );
                        }
                    }
                }

                // gutter (pinned left, numbers on the first visual row)
                if show_line_numbers {
                    let gx = ui.clip_rect().left();
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(gx, y),
                            egui::vec2(gutter_w, line_h),
                        ),
                        0.0,
                        th.gutter_bg,
                    );
                    painter.text(
                        egui::pos2(gx + gutter_w - 8.0, y + 2.0),
                        egui::Align2::RIGHT_TOP,
                        (li + 1).to_string(),
                        th.font.clone(),
                        th.weak,
                    );
                }
            }

            // ---- scroll caret into view ----
            if state.scroll_to_cursor {
                state.scroll_to_cursor = false;
                let line = state.line_of_char(state.cursor);
                let (slice, start_chr) = state.line_slice(text, line);
                let galley = line_galley(ui, slice, th, spell, wrap_w);
                let r = galley.pos_from_cursor(CCursor::new(state.cursor - start_chr));
                let y = rect.top() + state.wrap_prefix[line] as f32 * row_h + r.top();
                let target = egui::Rect::from_min_size(
                    egui::pos2(text_x + r.left(), y),
                    egui::vec2(2.0, row_h),
                );
                ui.scroll_to_rect(target.expand2(egui::vec2(40.0, row_h)), None);
            }

            // context menu (spell suggestions + clipboard basics)
            resp.context_menu(|ui| {
                ui.set_min_width(160.0);
                if let Some((wa, wb, word)) = state.ctx_word.clone() {
                    ui.label(
                        egui::RichText::new(format!("\"{word}\" not in dictionary"))
                            .italics()
                            .weak(),
                    );
                    if state.ctx_suggestions.is_empty() {
                        ui.label("(no suggestions)");
                    } else {
                        let suggestions = state.ctx_suggestions.clone();
                        for s in &suggestions {
                            if ui.button(s).clicked() {
                                state.apply_edit(text, (wa, wb), s, now);
                                changed = true;
                                state.ctx_word = None;
                                ui.close();
                            }
                        }
                    }
                    if ui.button("Add to dictionary").clicked() {
                        add_word = Some(word);
                        state.ctx_word = None;
                        ui.close();
                    }
                    ui.separator();
                }
                let (a, b) = state.selection();
                if ui.add_enabled(a != b, egui::Button::new("Cut")).clicked() {
                    let ab = state.char_to_byte(text, a);
                    let bb = state.char_to_byte(text, b);
                    ui.ctx().copy_text(text[ab..bb].to_string());
                    state.apply_edit(text, (a, b), "", now);
                    changed = true;
                    ui.close();
                }
                if ui.add_enabled(a != b, egui::Button::new("Copy")).clicked() {
                    let ab = state.char_to_byte(text, a);
                    let bb = state.char_to_byte(text, b);
                    ui.ctx().copy_text(text[ab..bb].to_string());
                    ui.close();
                }
                if ui.button("Paste").clicked() {
                    let clip = arboard::Clipboard::new()
                        .ok()
                        .and_then(|mut c| c.get_text().ok());
                    if let Some(s) = clip {
                        let s = s.replace("\r\n", "\n").replace('\r', "\n");
                        let sel = state.selection();
                        state.apply_edit(text, sel, &s, now);
                        changed = true;
                    }
                    ui.close();
                }
                ui.separator();
                if ui.button("Select All").clicked() {
                    state.anchor = 0;
                    state.cursor = state.total_chars;
                    ui.close();
                }
            });
        });

    ShowResult { changed, add_word }
}

/// Lay out any lines whose wrapped-row count is unknown, then rebuild the
/// prefix sums. Cheap when there is nothing to do.
fn fill_wrap_cache(
    state: &mut EditorState,
    text: &str,
    ui: &egui::Ui,
    th: &EditorTheme,
    spell: Option<&Spell>,
    wrap_w: Option<f32>,
) {
    let n = state.lines.len();
    if state.wrap_rows.len() != n {
        state.wrap_rows.resize(n, u32::MAX);
    }
    let mut any = state.wrap_prefix.len() != n + 1;
    for i in 0..n {
        if state.wrap_rows[i] != u32::MAX {
            continue;
        }
        any = true;
        state.wrap_rows[i] = match wrap_w {
            None => 1,
            Some(_) => {
                let byte0 = state.lines[i].byte;
                let byte1 = state
                    .lines
                    .get(i + 1)
                    .map(|m| m.byte - 1)
                    .unwrap_or(text.len())
                    .max(byte0);
                let slice = &text[byte0..byte1];
                if slice.is_empty() {
                    1
                } else {
                    line_galley(ui, slice, th, spell, wrap_w)
                        .rows
                        .len()
                        .max(1) as u32
                }
            }
        };
    }
    if any {
        state.wrap_prefix.clear();
        state.wrap_prefix.reserve(n + 1);
        let mut acc = 0u32;
        state.wrap_prefix.push(0);
        for r in &state.wrap_rows {
            acc = acc.saturating_add(*r);
            state.wrap_prefix.push(acc);
        }
    }
}

/// Hit-test a pointer position to a char offset (wrap-aware).
#[allow(clippy::too_many_arguments)]
fn pos_to_char(
    state: &EditorState,
    text: &str,
    pos: egui::Pos2,
    ui: &egui::Ui,
    th: &EditorTheme,
    spell: Option<&Spell>,
    wrap_w: Option<f32>,
    rect_top: f32,
    text_x: f32,
    row_h: f32,
) -> usize {
    let total = state.total_visual_rows();
    let v = (((pos.y - rect_top) / row_h).floor().max(0.0) as u32)
        .min(total.saturating_sub(1));
    let line = state.line_at_visual_row(v);
    let (slice, start_chr) = state.line_slice(text, line);
    let galley = line_galley(ui, slice, th, spell, wrap_w);
    let rel_y = pos.y - rect_top - state.wrap_prefix[line] as f32 * row_h;
    let col = usize::from(
        galley
            .cursor_from_pos(egui::vec2(pos.x - text_x, rel_y))
            .index,
    );
    start_chr + col.min(slice.chars().count())
}

/// Fill the given char-column range of a (possibly wrapped) line galley.
#[allow(clippy::too_many_arguments)]
fn paint_char_range(
    painter: &egui::Painter,
    galley: &egui::Galley,
    a: usize,
    b: usize,
    y: f32,
    text_x: f32,
    right_edge: f32,
    row_h: f32,
    color: Color32,
) {
    let ra = galley.pos_from_cursor(CCursor::new(a));
    let rb = galley.pos_from_cursor(CCursor::new(b));
    if (ra.top() - rb.top()).abs() < 0.5 {
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(text_x + ra.left(), y + ra.top()),
                egui::pos2(text_x + rb.left().max(ra.left() + 1.0), y + ra.top() + row_h),
            ),
            0.0,
            color,
        );
    } else {
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(text_x + ra.left(), y + ra.top()),
                egui::pos2(text_x + right_edge, y + ra.top() + row_h),
            ),
            0.0,
            color,
        );
        if rb.top() - ra.top() > row_h + 0.5 {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(text_x, y + ra.top() + row_h),
                    egui::pos2(text_x + right_edge, y + rb.top()),
                ),
                0.0,
                color,
            );
        }
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(text_x, y + rb.top()),
                egui::pos2(text_x + rb.left().max(1.0), y + rb.top() + row_h),
            ),
            0.0,
            color,
        );
    }
}

/// Build a rectangular (column) selection between two content-space points:
/// one cursor per visual row, each spanning the horizontal band.
#[allow(clippy::too_many_arguments)]
fn column_select(
    state: &mut EditorState,
    text: &str,
    ui: &egui::Ui,
    th: &EditorTheme,
    spell: Option<&Spell>,
    wrap_w: Option<f32>,
    origin: (f32, f32),
    current: (f32, f32),
    row_h: f32,
) {
    let total = state.total_visual_rows();
    let v0 = ((origin.1 / row_h).floor().max(0.0) as u32).min(total.saturating_sub(1));
    let v1 = ((current.1 / row_h).floor().max(0.0) as u32).min(total.saturating_sub(1));
    let (top, bot) = (v0.min(v1), v0.max(v1));
    let mut sels: Vec<(usize, usize)> = Vec::new();
    for v in top..=bot {
        let line = state.line_at_visual_row(v);
        let (slice, start) = state.line_slice(text, line);
        let g = line_galley(ui, slice, th, spell, wrap_w);
        let rel_y = (v - state.wrap_prefix[line]) as f32 * row_h + row_h * 0.5;
        let n = slice.chars().count();
        let a = usize::from(g.cursor_from_pos(egui::vec2(origin.0, rel_y)).index).min(n);
        let h = usize::from(g.cursor_from_pos(egui::vec2(current.0, rel_y)).index).min(n);
        sels.push((start + a, start + h));
    }
    if sels.is_empty() {
        return;
    }
    // the primary cursor follows the pointer
    let primary_idx = if v1 >= v0 { sels.len() - 1 } else { 0 };
    let (pa, ph) = sels.remove(primary_idx);
    state.anchor = pa;
    state.cursor = ph;
    state.extra_sels = sels;
    state.dedup_sels();
}

/// Move the caret by visual rows (handles wrapped lines and preserves the
/// desired x position across repeated moves).
#[allow(clippy::too_many_arguments)]
fn vertical_move(
    state: &mut EditorState,
    text: &str,
    ui: &egui::Ui,
    th: &EditorTheme,
    spell: Option<&Spell>,
    wrap_w: Option<f32>,
    delta_rows: i64,
    extend: bool,
) {
    let row_h = th.row_height;
    let line = state.line_of_char(state.cursor);
    let (slice, start) = state.line_slice(text, line);
    let galley = line_galley(ui, slice, th, spell, wrap_w);
    let r = galley.pos_from_cursor(CCursor::new(state.cursor - start));
    let row_in = ((r.top() / row_h).round().max(0.0)) as i64;
    let x = state.desired_x.unwrap_or_else(|| r.left());
    state.desired_x = Some(x);

    let total = state.total_visual_rows() as i64;
    let cur_v = state.wrap_prefix[line] as i64 + row_in;
    let target_v = (cur_v + delta_rows).clamp(0, total - 1);
    if target_v == cur_v {
        // already at the document edge: jump to start/end
        let pos = if delta_rows < 0 { 0 } else { state.total_chars };
        state.cursor = pos;
        if !extend {
            state.anchor = pos;
        }
        return;
    }
    let tline = state.line_at_visual_row(target_v as u32);
    let (tslice, tstart) = state.line_slice(text, tline);
    let tgalley = line_galley(ui, tslice, th, spell, wrap_w);
    let ty = (target_v - state.wrap_prefix[tline] as i64) as f32 * row_h + row_h * 0.5;
    let col = usize::from(tgalley.cursor_from_pos(egui::vec2(x, ty)).index);
    state.cursor = tstart + col.min(tslice.chars().count());
    if !extend {
        state.anchor = state.cursor;
    }
}

fn handle_key(
    state: &mut EditorState,
    text: &mut String,
    key: Key,
    m: Modifiers,
    now: f64,
    changed: &mut bool,
) -> bool {
    let (sel_a, sel_b) = state.selection();
    let has_sel = sel_a != sel_b;
    let extend = m.shift;

    // v1: plain navigation collapses secondary cursors
    if !state.extra_sels.is_empty()
        && matches!(key, Key::ArrowLeft | Key::ArrowRight | Key::Home | Key::End)
    {
        state.collapse_extras();
    }

    let move_to = |state: &mut EditorState, pos: usize, extend: bool| {
        state.cursor = pos.min(state.total_chars);
        if !extend {
            state.anchor = state.cursor;
        }
    };

    match key {
        Key::ArrowLeft => {
            let pos = if m.ctrl {
                prev_word(state, text, state.cursor)
            } else if has_sel && !extend {
                sel_a
            } else {
                state.cursor.saturating_sub(1)
            };
            move_to(state, pos, extend);
            true
        }
        Key::ArrowRight => {
            let pos = if m.ctrl {
                next_word(state, text, state.cursor)
            } else if has_sel && !extend {
                sel_b
            } else {
                state.cursor + 1
            };
            move_to(state, pos, extend);
            true
        }
        Key::Home => {
            let pos = if m.ctrl {
                0
            } else {
                let line = state.line_of_char(state.cursor);
                state.lines[line].chr
            };
            move_to(state, pos, extend);
            true
        }
        Key::End => {
            let pos = if m.ctrl {
                state.total_chars
            } else {
                let line = state.line_of_char(state.cursor);
                let (slice, start) = state.line_slice(text, line);
                start + slice.chars().count()
            };
            move_to(state, pos, extend);
            true
        }
        Key::Backspace => {
            let ranges: Vec<(usize, usize)> = state
                .all_sels()
                .iter()
                .map(|&(a, h)| {
                    let (lo, hi) = (a.min(h), a.max(h));
                    if lo != hi {
                        (lo, hi)
                    } else if hi > 0 {
                        let from = if m.ctrl {
                            prev_word(state, text, hi)
                        } else {
                            hi - 1
                        };
                        (from, hi)
                    } else {
                        (0, 0)
                    }
                })
                .collect();
            state.apply_multi(text, ranges, "", now);
            *changed = true;
            true
        }
        Key::Delete => {
            let total = state.total_chars;
            let ranges: Vec<(usize, usize)> = state
                .all_sels()
                .iter()
                .map(|&(a, h)| {
                    let (lo, hi) = (a.min(h), a.max(h));
                    if lo != hi {
                        (lo, hi)
                    } else if hi < total {
                        let to = if m.ctrl {
                            next_word(state, text, hi)
                        } else {
                            hi + 1
                        };
                        (hi, to)
                    } else {
                        (total, total)
                    }
                })
                .collect();
            state.apply_multi(text, ranges, "", now);
            *changed = true;
            true
        }
        Key::Enter => {
            let sels = state.all_sels();
            state.apply_multi(text, sels, "\n", now);
            *changed = true;
            true
        }
        Key::Tab => {
            let sels = state.all_sels();
            state.apply_multi(text, sels, "\t", now);
            *changed = true;
            true
        }
        Key::Escape => {
            let had = !state.extra_sels.is_empty();
            state.collapse_extras();
            had
        }
        Key::A if m.ctrl => {
            state.anchor = 0;
            state.cursor = state.total_chars;
            state.collapse_extras();
            false
        }
        Key::Z if m.ctrl && !m.shift => {
            state.undo(text);
            *changed = true;
            true
        }
        Key::Y if m.ctrl => {
            state.redo(text);
            *changed = true;
            true
        }
        Key::Z if m.ctrl && m.shift => {
            state.redo(text);
            *changed = true;
            true
        }
        _ => false,
    }
}

fn prev_word(state: &EditorState, text: &str, from: usize) -> usize {
    if from == 0 {
        return 0;
    }
    let line = state.line_of_char(from.saturating_sub(1));
    let (slice, start) = state.line_slice(text, line);
    if from <= start {
        return start.saturating_sub(1); // jump over newline
    }
    let chars: Vec<char> = slice.chars().collect();
    let mut i = (from - start).min(chars.len());
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    if i == 0 {
        return start.saturating_sub(1);
    }
    let (a, _) = word_bounds(&chars, i - 1);
    start + a
}

fn next_word(state: &EditorState, text: &str, from: usize) -> usize {
    if from >= state.total_chars {
        return state.total_chars;
    }
    let line = state.line_of_char(from);
    let (slice, start) = state.line_slice(text, line);
    let chars: Vec<char> = slice.chars().collect();
    let mut i = from - start;
    if i >= chars.len() {
        return (start + chars.len() + 1).min(state.total_chars); // next line
    }
    let (_, b) = word_bounds(&chars, i);
    i = b;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    start + i
}

/// Layout one line with syntax colors, spell underlines, and wrapping.
/// `line_height` is pinned to the theme row height so every visual row has
/// identical geometry (the wrap cache and painters rely on this). Syntax
/// colors never change glyph metrics, so hit-testing may pass `None` spans.
fn line_galley_colored(
    ui: &egui::Ui,
    slice: &str,
    th: &EditorTheme,
    spell: Option<&Spell>,
    wrap_w: Option<f32>,
    hl_spans: Option<&[(Color32, u32)]>,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_w.unwrap_or(f32::INFINITY);
    let base = TextFormat {
        font_id: th.font.clone(),
        color: th.text,
        line_height: Some(th.row_height),
        ..Default::default()
    };
    if let Some(spans) = hl_spans {
        let bad_ranges = match spell {
            Some(sp) => misspelled_byte_ranges(slice, sp),
            None => Vec::new(),
        };
        append_colored(
            &mut job,
            slice,
            spans,
            &bad_ranges,
            &base,
            th.misspell,
        );
        return ui.ctx().fonts_mut(|f| f.layout_job(job));
    }
    let normal = base;
    match spell {
        Some(sp) => {
            let bad = TextFormat {
                underline: egui::Stroke::new(1.5, th.misspell),
                ..normal.clone()
            };
            append_spellchecked_line(&mut job, slice, sp, &normal, &bad);
        }
        None => job.append(slice, 0.0, normal),
    }
    ui.ctx().fonts_mut(|f| f.layout_job(job))
}

/// Plain layout used for hit-testing and painting when no syntax is active.
fn line_galley(
    ui: &egui::Ui,
    slice: &str,
    th: &EditorTheme,
    spell: Option<&Spell>,
    wrap_w: Option<f32>,
) -> std::sync::Arc<egui::Galley> {
    line_galley_colored(ui, slice, th, spell, wrap_w, None)
}

/// Byte ranges of misspelled words in the line.
fn misspelled_byte_ranges(text: &str, sp: &Spell) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut prev: Option<char> = None;
    let mut iter = text.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        if spell::is_word_char(c) {
            let start = i;
            let mut end = i + c.len_utf8();
            while let Some(&(j, cj)) = iter.peek() {
                if spell::is_word_char(cj) {
                    end = j + cj.len_utf8();
                    iter.next();
                } else {
                    break;
                }
            }
            let word = &text[start..end];
            if spell::checkable(word, prev) && !sp.check(word) {
                out.push((start, end));
            }
            prev = word.chars().last();
        } else {
            prev = Some(c);
        }
    }
    out
}

/// Append `text` split on both syntax-span and misspelling boundaries.
fn append_colored(
    job: &mut LayoutJob,
    text: &str,
    spans: &[(Color32, u32)],
    bad_ranges: &[(usize, usize)],
    base: &TextFormat,
    misspell: Color32,
) {
    let len = text.len();
    let mut pos = 0usize;
    let mut si = 0usize;
    let mut span_end = spans.first().map(|s| s.1 as usize).unwrap_or(len);
    let mut span_color = spans.first().map(|s| s.0).unwrap_or(base.color);
    let mut bi = 0usize;
    while pos < len {
        // advance to the span containing `pos`
        while pos >= span_end && si + 1 < spans.len() {
            si += 1;
            span_end += spans[si].1 as usize;
            span_color = spans[si].0;
        }
        if pos >= span_end {
            span_end = len; // spans ran short; paint the tail in base color
            span_color = base.color;
        }
        // advance past finished misspell ranges
        while bi < bad_ranges.len() && bad_ranges[bi].1 <= pos {
            bi += 1;
        }
        let (in_bad, bad_edge) = if bi < bad_ranges.len() {
            let (ba, bb) = bad_ranges[bi];
            if pos >= ba {
                (true, bb)
            } else {
                (false, ba)
            }
        } else {
            (false, len)
        };
        let next = span_end.min(bad_edge).min(len);
        if next <= pos {
            break; // defensive: no progress possible
        }
        let mut fmt = TextFormat {
            color: span_color,
            ..base.clone()
        };
        if in_bad {
            fmt.underline = egui::Stroke::new(1.5, misspell);
        }
        job.append(&text[pos..next], 0.0, fmt);
        pos = next;
    }
}

/// Add one syntax-colored line to a larger text layout while retaining
/// spellcheck underlines. Used by the standard editor's full-buffer galley.
pub(crate) fn append_highlighted_line(
    job: &mut LayoutJob,
    text: &str,
    spans: &[(Color32, u32)],
    spell: Option<&Spell>,
    base: &TextFormat,
    misspell: Color32,
) {
    let bad_ranges = spell
        .map(|sp| misspelled_byte_ranges(text, sp))
        .unwrap_or_default();
    append_colored(job, text, spans, &bad_ranges, base, misspell);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_colors_keep_spell_underlines() {
        let sp = crate::spell::tests::english();
        let line = "// this recieve is wrong";
        let red = Color32::from_rgb(232, 82, 82);
        let spans = [(Color32::GRAY, line.len() as u32)]; // one comment span
        let mut job = LayoutJob::default();
        append_highlighted_line(&mut job, line, &spans, Some(&sp), &TextFormat::default(), red);
        let underlined: Vec<&str> = job
            .sections
            .iter()
            .filter(|s| s.format.underline.width > 0.0)
            .map(|s| &line[s.byte_range.start.0..s.byte_range.end.0])
            .collect();
        assert_eq!(underlined, ["recieve"]);
        assert!(job.sections.iter().all(|s| s.format.color == Color32::GRAY));
    }

    #[test]
    fn gutter_uses_window_chrome_color() {
        for dark in [false, true] {
            let palette = crate::theme::palette(dark);
            let theme = theme_from_palette(
                &palette,
                FontId::monospace(14.0),
                18.0,
                dark,
            );
            assert_eq!(theme.gutter_bg, palette.chrome);
            assert_ne!(theme.gutter_bg, palette.editor);
        }
    }

    fn state_for(text: &str) -> EditorState {
        let mut s = EditorState::default();
        s.ensure_index(text);
        s
    }

    #[test]
    fn index_empty_and_lines() {
        let s = state_for("");
        assert_eq!(s.line_count(), 1);
        assert_eq!(s.total_chars, 0);

        let s = state_for("a\nbb\nccc");
        assert_eq!(s.line_count(), 3);
        assert_eq!(s.total_chars, 8);
        assert_eq!(s.max_line_chars, 3);

        // trailing newline creates an empty last line
        let s = state_for("a\n");
        assert_eq!(s.line_count(), 2);
    }

    #[test]
    fn line_slice_and_lookup() {
        let text = "alpha\nbëta\ngamma";
        let s = state_for(text);
        assert_eq!(s.line_slice(text, 0).0, "alpha");
        assert_eq!(s.line_slice(text, 1).0, "bëta");
        assert_eq!(s.line_slice(text, 2).0, "gamma");
        assert_eq!(s.line_of_char(0), 0);
        assert_eq!(s.line_of_char(5), 0); // the newline belongs to line 0's end
        assert_eq!(s.line_of_char(6), 1);
        assert_eq!(s.line_of_char(999), 2);
    }

    #[test]
    fn char_to_byte_multibyte() {
        let text = "héllo\nwörld";
        let s = state_for(text);
        // 'é' is 2 bytes: char 2 -> byte 3
        assert_eq!(s.char_to_byte(text, 2), 3);
        // char index past end clamps to len
        assert_eq!(s.char_to_byte(text, 999), text.len());
    }

    #[test]
    fn edit_insert_delete_roundtrip() {
        let mut text = String::from("hello world");
        let mut s = state_for(&text);
        s.apply_edit(&mut text, (5, 5), " brave", 0.0);
        assert_eq!(text, "hello brave world");
        assert_eq!(s.cursor, 11);
        s.apply_edit(&mut text, (5, 11), "", 10.0);
        assert_eq!(text, "hello world");
        s.undo(&mut text);
        assert_eq!(text, "hello brave world");
        s.undo(&mut text);
        assert_eq!(text, "hello world");
        s.redo(&mut text);
        assert_eq!(text, "hello brave world");
        s.redo(&mut text);
        assert_eq!(text, "hello world");
    }

    #[test]
    fn typing_groups_into_one_undo() {
        let mut text = String::new();
        let mut s = state_for(&text);
        for (i, c) in ["a", "b", "c"].iter().enumerate() {
            let sel = s.selection();
            s.apply_edit(&mut text, sel, c, i as f64 * 0.1);
        }
        assert_eq!(text, "abc");
        assert_eq!(s.undo.len(), 1);
        s.undo(&mut text);
        assert_eq!(text, "");
    }

    #[test]
    fn edit_at_bounds_and_unicode() {
        let mut text = String::from("日本語\nテスト");
        let mut s = state_for(&text);
        s.apply_edit(&mut text, (0, 0), "→", 0.0);
        assert_eq!(text, "→日本語\nテスト");
        let end = s.total_chars;
        s.apply_edit(&mut text, (end, end + 50), "!", 1.5);
        assert!(text.ends_with('!'));
        // out-of-range selection clamps instead of panicking
        s.apply_edit(&mut text, (500, 900), "x", 3.0);
        assert!(text.ends_with('x'));
    }

    #[test]
    fn external_change_resets_safely() {
        let mut text = String::from("one\ntwo");
        let mut s = state_for(&text);
        s.set_selection(0, 7);
        text = String::from("x");
        s.note_external_change();
        s.sync(&text, 1);
        assert_eq!(s.selection(), (0, 1)); // clamped
        s.apply_edit(&mut text, (0, 1), "yz", 0.0);
        assert_eq!(text, "yz");
    }

    #[test]
    fn word_navigation() {
        let text = "foo bar_baz  qux\nnext";
        let s = state_for(text);
        assert_eq!(next_word(&s, text, 0), 4); // after "foo "
        assert_eq!(next_word(&s, text, 4), 13); // after "bar_baz  "
        assert_eq!(prev_word(&s, text, 13), 4);
        assert_eq!(prev_word(&s, text, 4), 0);
        // across the newline
        assert_eq!(next_word(&s, text, 16), 17);
        // Ctrl+Left from the start of line 2 lands on "qux" (last word of line 1)
        assert_eq!(prev_word(&s, text, 17), 13);
    }

    #[test]
    fn key_navigation_at_edges() {
        let mut text = String::from("ab\ncd");
        let mut s = state_for(&text);
        let mut changed = false;
        // backspace at 0: no-op
        handle_key(&mut s, &mut text, Key::Backspace, Modifiers::NONE, 0.0, &mut changed);
        assert_eq!(text, "ab\ncd");
        // delete at end: no-op
        s.set_selection(5, 5);
        handle_key(&mut s, &mut text, Key::Delete, Modifiers::NONE, 0.0, &mut changed);
        assert_eq!(text, "ab\ncd");
        // left at 0 stays; right at end stays
        s.set_selection(0, 0);
        handle_key(&mut s, &mut text, Key::ArrowLeft, Modifiers::NONE, 0.0, &mut changed);
        assert_eq!(s.cursor, 0);
        s.set_selection(5, 5);
        handle_key(&mut s, &mut text, Key::ArrowRight, Modifiers::NONE, 0.0, &mut changed);
        assert_eq!(s.cursor, 5);
    }

    #[test]
    fn selection_delete_and_enter() {
        let mut text = String::from("hello\nworld");
        let mut s = state_for(&text);
        s.set_selection(2, 8);
        let mut changed = false;
        handle_key(&mut s, &mut text, Key::Enter, Modifiers::NONE, 0.0, &mut changed);
        assert_eq!(text, "he\nrld");
        assert!(changed);
        s.undo(&mut text);
        assert_eq!(text, "hello\nworld");
    }

    #[test]
    fn multi_cursor_insert_positions() {
        let mut text = String::from("aaa\nbbb\nccc");
        let mut s = state_for(&text);
        // carets at the start of each line
        s.anchor = 0;
        s.cursor = 0;
        s.extra_sels = vec![(4, 4), (8, 8)];
        s.apply_multi(&mut text, vec![(0, 0), (4, 4), (8, 8)], "> ", 0.0);
        assert_eq!(text, "> aaa\n> bbb\n> ccc");
        // every caret sits right after its insertion
        assert_eq!((s.anchor, s.cursor), (2, 2));
        assert_eq!(s.extra_sels, vec![(8, 8), (14, 14)]);
        assert_eq!(s.undo.len(), 1); // one compound step

        s.undo(&mut text);
        assert_eq!(text, "aaa\nbbb\nccc");
        assert_eq!((s.anchor, s.cursor), (0, 0));
        assert_eq!(s.extra_sels, vec![(4, 4), (8, 8)]);

        s.redo(&mut text);
        assert_eq!(text, "> aaa\n> bbb\n> ccc");
        assert_eq!(s.extra_sels, vec![(8, 8), (14, 14)]);
    }

    #[test]
    fn multi_cursor_delete_varying_widths() {
        let mut text = String::from("xx123yy45zz6");
        let mut s = state_for(&text);
        // selections over the digit runs: 2-5, 7-9, 11-12
        s.anchor = 2;
        s.cursor = 5;
        s.extra_sels = vec![(7, 9), (11, 12)];
        let sels = vec![(2, 5), (7, 9), (11, 12)];
        s.apply_multi(&mut text, sels, "", 0.0);
        assert_eq!(text, "xxyyzz");
        s.undo(&mut text);
        assert_eq!(text, "xx123yy45zz6");
        s.redo(&mut text);
        assert_eq!(text, "xxyyzz");
    }

    #[test]
    fn multi_cursor_overlaps_and_dupes_filtered() {
        let mut text = String::from("abcdef");
        let mut s = state_for(&text);
        // duplicate caret and an overlapping range collapse to sane edits
        s.apply_multi(&mut text, vec![(1, 3), (2, 4), (1, 3), (5, 5)], "_", 0.0);
        assert_eq!(text, "a_de_f");
    }

    #[test]
    fn wrap_cache_splice_stays_aligned() {
        let mut text = String::from("one\ntwo\nthree");
        let mut s = state_for(&text);
        // simulate a built wrap cache
        s.wrap_rows = vec![1, 1, 1];
        s.wrap_valid = true;
        // replace "two" with two lines -> line count 3 -> 4
        s.apply_edit(&mut text, (4, 7), "2a\n2b", 0.0);
        assert_eq!(text, "one\n2a\n2b\nthree");
        assert_eq!(s.line_count(), 4);
        assert!(s.wrap_valid);
        assert_eq!(s.wrap_rows.len(), 4);
        assert_eq!(s.wrap_rows[0], 1); // untouched line kept
        assert_eq!(s.wrap_rows[1], u32::MAX); // edited region marked
        assert_eq!(s.wrap_rows[2], u32::MAX);
        assert_eq!(s.wrap_rows[3], 1); // shifted line kept

        // deleting a newline shrinks the cache
        s.wrap_rows = vec![1, 1, 1, 1];
        s.apply_edit(&mut text, (3, 4), "", 5.0); // remove first '\n'
        assert_eq!(s.line_count(), 3);
        assert_eq!(s.wrap_rows.len(), 3);
        assert_eq!(s.wrap_rows[0], u32::MAX);
        assert_eq!(s.wrap_rows[1], 1);
    }
}

fn append_spellchecked_line(
    job: &mut LayoutJob,
    text: &str,
    sp: &Spell,
    normal: &TextFormat,
    bad: &TextFormat,
) {
    let mut chunk_start = 0usize;
    let mut prev: Option<char> = None;
    let mut iter = text.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        if spell::is_word_char(c) {
            let start = i;
            let mut end = i + c.len_utf8();
            while let Some(&(j, cj)) = iter.peek() {
                if spell::is_word_char(cj) {
                    end = j + cj.len_utf8();
                    iter.next();
                } else {
                    break;
                }
            }
            let word = &text[start..end];
            if spell::checkable(word, prev) && !sp.check(word) {
                if chunk_start < start {
                    job.append(&text[chunk_start..start], 0.0, normal.clone());
                }
                job.append(word, 0.0, bad.clone());
                chunk_start = end;
            }
            prev = word.chars().last();
        } else {
            prev = Some(c);
        }
    }
    if chunk_start < text.len() {
        job.append(&text[chunk_start..], 0.0, normal.clone());
    }
}
