//! Preview editor: a virtualized text-editing widget.
//!
//! Unlike egui's `TextEdit` (which lays out the whole buffer as one galley),
//! this widget lays out **only the visible lines**, so multi-megabyte files
//! stay responsive. It owns its cursor/selection model and undo stack, keyed
//! by character offsets so the rest of the app (find, go-to, plugins) speaks
//! the same coordinates as the classic editor.
//!
//! Preview limitations (documented in Preferences): no word wrap, no IME
//! composition, single cursor (multi-cursor-ready data model comes with the
//! full swap).

use eframe::egui::{
    self, Color32, Event, FontId, Key, Modifiers, Sense, TextFormat,
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

struct EditOp {
    at: usize, // char offset
    removed: String,
    inserted: String,
    sel_before: (usize, usize),
    sel_after: (usize, usize),
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
}

impl EditorState {
    pub fn selection(&self) -> (usize, usize) {
        (self.cursor.min(self.anchor), self.cursor.max(self.anchor))
    }

    pub fn set_selection(&mut self, anchor: usize, cursor: usize) {
        self.anchor = anchor;
        self.cursor = cursor;
        self.scroll_to_cursor = true;
    }

    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.total_chars);
        self.anchor = self.anchor.min(self.total_chars);
    }

    /// Rebuild the line index if the document changed outside the editor.
    fn sync(&mut self, text: &str, revision: u64) {
        if self.indexed_once && self.revision_seen == revision {
            return;
        }
        self.rebuild_index(text);
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

    /// Replace `range` (chars) with `insert`, recording undo.
    pub fn apply_edit(
        &mut self,
        text: &mut String,
        range: (usize, usize),
        insert: &str,
        now: f64,
    ) {
        self.ensure_index(text);
        let (a, b) = (range.0.min(range.1), range.0.max(range.1));
        let (a, b) = (a.min(self.total_chars), b.min(self.total_chars));
        let ab = self.char_to_byte(text, a);
        let bb = self.char_to_byte(text, b);
        let removed = text[ab..bb].to_string();
        let sel_before = (self.anchor, self.cursor);
        text.replace_range(ab..bb, insert);
        let after = a + insert.chars().count();
        self.anchor = after;
        self.cursor = after;
        self.scroll_to_cursor = true;
        self.rebuild_index(text);

        // group rapid single-character typing into one undo step
        let mergeable = removed.is_empty()
            && insert.chars().count() == 1
            && insert != "\n"
            && self
                .undo
                .last()
                .map(|op| {
                    op.removed.is_empty()
                        && now - op.time < 1.0
                        && op.at + op.inserted.chars().count() == a
                })
                .unwrap_or(false);
        if mergeable {
            let op = self.undo.last_mut().unwrap();
            op.inserted.push_str(insert);
            op.sel_after = (after, after);
            op.time = now;
        } else {
            self.undo.push(EditOp {
                at: a,
                removed,
                inserted: insert.to_string(),
                sel_before,
                sel_after: (after, after),
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
        let a = op.at;
        let end = a + op.inserted.chars().count();
        let ab = self.char_to_byte(text, a);
        let eb = self.char_to_byte(text, end);
        text.replace_range(ab..eb, &op.removed);
        self.rebuild_index(text);
        self.anchor = op.sel_before.0.min(self.total_chars);
        self.cursor = op.sel_before.1.min(self.total_chars);
        self.scroll_to_cursor = true;
        self.redo.push(op);
    }

    pub fn redo(&mut self, text: &mut String) {
        self.ensure_index(text);
        let Some(op) = self.redo.pop() else { return };
        let a = op.at;
        let end = a + op.removed.chars().count();
        let ab = self.char_to_byte(text, a);
        let eb = self.char_to_byte(text, end);
        text.replace_range(ab..eb, &op.inserted);
        self.rebuild_index(text);
        self.anchor = op.sel_after.0.min(self.total_chars);
        self.cursor = op.sel_after.1.min(self.total_chars);
        self.scroll_to_cursor = true;
        self.undo.push(op);
    }

    /// External code changed the text directly (find/replace, plugins):
    /// undo history for those changes is not tracked in the preview editor.
    pub fn note_external_change(&mut self) {
        self.indexed_once = false;
        self.undo.clear();
        self.redo.clear();
    }
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
        gutter_bg: p.editor,
    }
}

pub struct ShowResult {
    pub changed: bool,
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
    spell: Option<&Spell>,
    request_focus: bool,
) -> ShowResult {
    state.sync(text, revision);
    let mut changed = false;
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

    let est_width =
        gutter_w + state.max_line_chars as f32 * char_w * 1.05 + 120.0;
    let total = egui::vec2(
        est_width.max(ui.available_width()),
        n_lines as f32 * row_h + row_h * 2.0,
    );

    egui::ScrollArea::both()
        .id_salt(editor_id.with("scroll"))
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            let (rect, resp) =
                ui.allocate_exact_size(total, Sense::click_and_drag());
            let text_x = rect.left() + gutter_w + 6.0;
            let painter = ui.painter_at(ui.clip_rect());

            if request_focus || resp.clicked() || resp.drag_started() {
                ui.memory_mut(|m| m.request_focus(editor_id));
            }
            let focused = ui.memory(|m| m.has_focus(editor_id));

            // ---- mouse → cursor ----
            fn pos_to_char(
                state: &EditorState,
                text: &str,
                pos: egui::Pos2,
                ui: &egui::Ui,
                th: &EditorTheme,
                rect_top: f32,
                text_x: f32,
                row_h: f32,
            ) -> usize {
                let n = state.line_count();
                let line = (((pos.y - rect_top) / row_h).floor().max(0.0) as usize)
                    .min(n.saturating_sub(1));
                let (slice, start_chr) = state.line_slice(text, line);
                let galley = line_galley(ui, slice, th, None);
                let col = usize::from(
                    galley
                        .cursor_from_pos(egui::vec2(pos.x - text_x, 0.0))
                        .index,
                );
                start_chr + col.min(slice.chars().count())
            }

            if let Some(pos) = resp.interact_pointer_pos() {
                let c = pos_to_char(state, text, pos, ui, th, rect.top(), text_x, row_h);
                let shift = ui.input(|i| i.modifiers.shift);
                if resp.double_clicked() {
                    let line = state.line_of_char(c);
                    let (slice, start_chr) = state.line_slice(text, line);
                    let chars: Vec<char> = slice.chars().collect();
                    let (a, b) = word_bounds(&chars, c.saturating_sub(start_chr));
                    let (a, b) = (start_chr + a, start_chr + b);
                    state.anchor = a;
                    state.cursor = b;
                } else if resp.triple_clicked() {
                    let line = state.line_of_char(c);
                    let (slice, start_chr) = state.line_slice(text, line);
                    let end = start_chr + slice.chars().count();
                    state.anchor = start_chr;
                    state.cursor = end;
                } else if resp.drag_started() {
                    state.cursor = c;
                    if !shift {
                        state.anchor = c;
                    }
                } else if resp.dragged() {
                    state.cursor = c;
                } else if resp.clicked() {
                    state.cursor = c;
                    if !shift {
                        state.anchor = c;
                    }
                }
            }

            // ---- keyboard ----
            if focused {
                let events = ui.input(|i| i.events.clone());
                let page = (viewport.height() / row_h).max(1.0) as usize;
                for ev in &events {
                    match ev {
                        Event::Text(s) => {
                            // Tab/Enter arrive as Key events; skip control-only
                            // text to avoid double insertion.
                            if s.chars().all(|c| c.is_control()) {
                                continue;
                            }
                            let sel = state.selection();
                            state.apply_edit(text, sel, s, now);
                            changed = true;
                        }
                        Event::Paste(s) => {
                            // normalize Windows clipboard line endings
                            let s = s.replace("\r\n", "\n").replace('\r', "\n");
                            let sel = state.selection();
                            state.apply_edit(text, sel, &s, now);
                            changed = true;
                        }
                        Event::Copy | Event::Cut => {
                            let (a, b) = state.selection();
                            if a != b {
                                let ab = state.char_to_byte(text, a);
                                let bb = state.char_to_byte(text, b);
                                ui.ctx().copy_text(text[ab..bb].to_string());
                                if matches!(ev, Event::Cut) {
                                    state.apply_edit(text, (a, b), "", now);
                                    changed = true;
                                }
                            }
                        }
                        Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } => {
                            if handle_key(
                                state, text, *key, *modifiers, page, now, &mut changed,
                            ) {
                                state.scroll_to_cursor = true;
                            }
                        }
                        _ => {}
                    }
                }
            }

            // ---- visible range (line count may have changed from edits) ----
            let n_lines = state.line_count();
            let first =
                ((viewport.top() / row_h).floor().max(0.0) as usize).min(n_lines - 1);
            let last = (((viewport.bottom() / row_h).ceil()) as usize + 1).min(n_lines);
            let (sel_a, sel_b) = state.selection();
            let cursor_line = state.line_of_char(state.cursor);

            for li in first..last {
                let y = rect.top() + li as f32 * row_h;
                let (slice, start_chr) = state.line_slice(text, li);
                let line_chars = slice.chars().count();
                let line_end = start_chr + line_chars;

                // current-line wash
                if li == cursor_line && sel_a == sel_b {
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(rect.left(), y),
                            egui::vec2(rect.width(), row_h),
                        ),
                        0.0,
                        th.current_line,
                    );
                }

                let galley = line_galley(ui, slice, th, spell);

                // selection highlight
                if sel_a != sel_b && sel_a < line_end + 1 && sel_b > start_chr {
                    let a = sel_a.max(start_chr) - start_chr;
                    let b = sel_b.min(line_end) - start_chr;
                    let x0 = galley.pos_from_cursor(CCursor::new(a)).left();
                    let mut x1 = galley.pos_from_cursor(CCursor::new(b)).left();
                    if sel_b > line_end {
                        x1 += char_w * 0.6; // show the selected newline
                    }
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(text_x + x0, y),
                            egui::pos2(text_x + x1.max(x0 + 1.0), y + row_h),
                        ),
                        0.0,
                        th.selection,
                    );
                }

                let galley_y = y + (row_h - galley.size().y) / 2.0;
                painter.galley(egui::pos2(text_x, galley_y), galley.clone(), th.text);

                // caret
                if li == cursor_line && focused {
                    let blink_on = ((now * 2.0) as i64) % 2 == 0;
                    if blink_on {
                        let col = state.cursor - start_chr;
                        let x = galley.pos_from_cursor(CCursor::new(col)).left();
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(text_x + x, y + 2.0),
                                egui::vec2(1.5, row_h - 4.0),
                            ),
                            0.0,
                            th.caret,
                        );
                    }
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(500));
                }

                // gutter (pinned: drawn at viewport-left, after text)
                if show_line_numbers {
                    let gx = ui.clip_rect().left();
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            egui::pos2(gx, y),
                            egui::vec2(gutter_w, row_h),
                        ),
                        0.0,
                        th.gutter_bg,
                    );
                    painter.text(
                        egui::pos2(gx + gutter_w - 8.0, y + (row_h - galley.size().y) / 2.0),
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
                let y = rect.top() + line as f32 * row_h;
                let (slice, start_chr) = state.line_slice(text, line);
                let galley = line_galley(ui, slice, th, None);
                let x = galley
                    .pos_from_cursor(CCursor::new(state.cursor - start_chr))
                    .left();
                let target = egui::Rect::from_min_size(
                    egui::pos2(text_x + x, y),
                    egui::vec2(2.0, row_h),
                );
                ui.scroll_to_rect(target.expand2(egui::vec2(40.0, row_h)), None);
            }

            // basic context menu
            resp.context_menu(|ui| {
                ui.set_min_width(140.0);
                let (a, b) = state.selection();
                if ui
                    .add_enabled(a != b, egui::Button::new("Cut"))
                    .clicked()
                {
                    let ab = state.char_to_byte(text, a);
                    let bb = state.char_to_byte(text, b);
                    ui.ctx().copy_text(text[ab..bb].to_string());
                    state.apply_edit(text, (a, b), "", now);
                    changed = true;
                    ui.close();
                }
                if ui
                    .add_enabled(a != b, egui::Button::new("Copy"))
                    .clicked()
                {
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

    ShowResult { changed }
}

fn handle_key(
    state: &mut EditorState,
    text: &mut String,
    key: Key,
    m: Modifiers,
    page: usize,
    now: f64,
    changed: &mut bool,
) -> bool {
    let (sel_a, sel_b) = state.selection();
    let has_sel = sel_a != sel_b;
    let extend = m.shift;

    let mut move_to = |state: &mut EditorState, pos: usize, extend: bool| {
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
        Key::ArrowUp => {
            let line = state.line_of_char(state.cursor);
            if line > 0 {
                let col = state.cursor - state.lines[line].chr;
                let (prev, prev_chr) = state.line_slice(text, line - 1);
                move_to(state, prev_chr + col.min(prev.chars().count()), extend);
            } else {
                move_to(state, 0, extend);
            }
            true
        }
        Key::ArrowDown => {
            let line = state.line_of_char(state.cursor);
            if line + 1 < state.line_count() {
                let col = state.cursor - state.lines[line].chr;
                let (next, next_chr) = state.line_slice(text, line + 1);
                move_to(state, next_chr + col.min(next.chars().count()), extend);
            } else {
                move_to(state, state.total_chars, extend);
            }
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
        Key::PageUp | Key::PageDown => {
            let line = state.line_of_char(state.cursor);
            let col = state.cursor - state.lines[line].chr;
            let target = if key == Key::PageUp {
                line.saturating_sub(page)
            } else {
                (line + page).min(state.line_count() - 1)
            };
            let (slice, start) = state.line_slice(text, target);
            move_to(state, start + col.min(slice.chars().count()), extend);
            true
        }
        Key::Backspace => {
            if has_sel {
                state.apply_edit(text, (sel_a, sel_b), "", now);
            } else if state.cursor > 0 {
                let from = if m.ctrl {
                    prev_word(state, text, state.cursor)
                } else {
                    state.cursor - 1
                };
                state.apply_edit(text, (from, state.cursor), "", now);
            }
            *changed = true;
            true
        }
        Key::Delete => {
            if has_sel {
                state.apply_edit(text, (sel_a, sel_b), "", now);
            } else if state.cursor < state.total_chars {
                let to = if m.ctrl {
                    next_word(state, text, state.cursor)
                } else {
                    state.cursor + 1
                };
                state.apply_edit(text, (state.cursor, to), "", now);
            }
            *changed = true;
            true
        }
        Key::Enter => {
            state.apply_edit(text, (sel_a, sel_b), "\n", now);
            *changed = true;
            true
        }
        Key::Tab => {
            state.apply_edit(text, (sel_a, sel_b), "\t", now);
            *changed = true;
            true
        }
        Key::A if m.ctrl => {
            state.anchor = 0;
            state.cursor = state.total_chars;
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

/// Layout one line, with optional spellcheck underlines.
fn line_galley(
    ui: &egui::Ui,
    slice: &str,
    th: &EditorTheme,
    spell: Option<&Spell>,
) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    let normal = TextFormat {
        font_id: th.font.clone(),
        color: th.text,
        ..Default::default()
    };
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

#[cfg(test)]
mod tests {
    use super::*;

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
        handle_key(&mut s, &mut text, Key::Backspace, Modifiers::NONE, 10, 0.0, &mut changed);
        assert_eq!(text, "ab\ncd");
        // arrows past ends clamp
        s.set_selection(0, 0);
        handle_key(&mut s, &mut text, Key::ArrowUp, Modifiers::NONE, 10, 0.0, &mut changed);
        assert_eq!(s.cursor, 0);
        s.set_selection(5, 5);
        handle_key(&mut s, &mut text, Key::ArrowDown, Modifiers::NONE, 10, 0.0, &mut changed);
        assert_eq!(s.cursor, 5);
        // delete at end: no-op
        handle_key(&mut s, &mut text, Key::Delete, Modifiers::NONE, 10, 0.0, &mut changed);
        assert_eq!(text, "ab\ncd");
        // page up/down beyond bounds clamp
        handle_key(&mut s, &mut text, Key::PageUp, Modifiers::NONE, 100, 0.0, &mut changed);
        assert_eq!(s.line_of_char(s.cursor), 0);
        handle_key(&mut s, &mut text, Key::PageDown, Modifiers::NONE, 100, 0.0, &mut changed);
        assert_eq!(s.line_of_char(s.cursor), 1);
    }

    #[test]
    fn selection_delete_and_enter() {
        let mut text = String::from("hello\nworld");
        let mut s = state_for(&text);
        s.set_selection(2, 8);
        let mut changed = false;
        handle_key(&mut s, &mut text, Key::Enter, Modifiers::NONE, 10, 0.0, &mut changed);
        assert_eq!(text, "he\nrld");
        assert!(changed);
        s.undo(&mut text);
        assert_eq!(text, "hello\nworld");
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
