//! Text styles that plugins put on ranges of a buffer (API 3): a text color
//! and a background color. Notey keeps them in memory only. They are not
//! written to the file.
//!
//! Runs use char offsets. When the text changes, `sync` compares it with the
//! last text it saw and moves the runs to agree with the edit.

use eframe::egui::text::{ByteIndex, LayoutJob, LayoutSection};
use eframe::egui::Color32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Run {
    pub start: usize,
    pub end: usize,
    pub fg: Option<Color32>,
    pub bg: Option<Color32>,
}

/// What to do with one property of the styled range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    Keep,
    Set(Color32),
    Clear,
}

impl Change {
    fn over(self, old: Option<Color32>) -> Option<Color32> {
        match self {
            Change::Keep => old,
            Change::Set(c) => Some(c),
            Change::Clear => None,
        }
    }
}

#[derive(Default)]
pub struct Styles {
    /// Sorted, with no overlaps, and each run has a color.
    runs: Vec<Run>,
    /// The text that `runs` agree with.
    text: String,
}

impl Styles {
    #[cfg(test)]
    pub fn runs(&self) -> &[Run] {
        &self.runs
    }

    /// Set or clear the colors of the char range `a..b` of `text`.
    pub fn apply(&mut self, text: &str, a: usize, b: usize, fg: Change, bg: Change) {
        self.sync(text);
        let (a, b) = (a.min(b), a.max(b));
        if a == b {
            return;
        }
        let mut out = Vec::with_capacity(self.runs.len() + 3);
        let mut pos = a; // the first position in a..b that no run covers
        for r in self.runs.drain(..) {
            if r.end <= a || r.start >= b {
                out.push(r);
                continue;
            }
            if r.start < a {
                out.push(Run { end: a, ..r });
            }
            let s = r.start.max(a);
            if s > pos {
                out.push(Run { start: pos, end: s, fg: fg.over(None), bg: bg.over(None) });
            }
            let e = r.end.min(b);
            out.push(Run { start: s, end: e, fg: fg.over(r.fg), bg: bg.over(r.bg) });
            pos = e;
            if r.end > b {
                out.push(Run { start: b, ..r });
            }
        }
        if pos < b {
            out.push(Run { start: pos, end: b, fg: fg.over(None), bg: bg.over(None) });
        }
        out.sort_by_key(|r| r.start);
        self.runs = normalize(out);
        if self.runs.is_empty() {
            self.text.clear();
        } else if self.text != text {
            self.text = text.to_string();
        }
    }

    /// Move the runs to agree with `text`. The edit is the part between the
    /// common prefix and the common suffix of the old and the new text. Text
    /// typed inside a run gets its colors. Text typed at the edge of a run
    /// does not.
    pub fn sync(&mut self, text: &str) {
        if self.runs.is_empty() {
            self.text.clear();
            return;
        }
        if self.text == text {
            return;
        }
        let old = std::mem::take(&mut self.text);
        let mut prefix = old
            .bytes()
            .zip(text.bytes())
            .take_while(|(x, y)| x == y)
            .count();
        while !old.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let max_suffix = old.len().min(text.len()) - prefix;
        let mut suffix = old
            .bytes()
            .rev()
            .zip(text.bytes().rev())
            .take(max_suffix)
            .take_while(|(x, y)| x == y)
            .count();
        while !old.is_char_boundary(old.len() - suffix) || !text.is_char_boundary(text.len() - suffix) {
            suffix -= 1;
        }
        let p = old[..prefix].chars().count();
        let removed = old[prefix..old.len() - suffix].chars().count();
        let inserted = text[prefix..text.len() - suffix].chars().count();
        let old_end = p + removed;
        let shift = |x: usize| x + inserted - removed;
        for r in &mut self.runs {
            r.start = if r.start < p {
                r.start
            } else if r.start >= old_end {
                shift(r.start)
            } else {
                p + inserted
            };
            r.end = if r.end <= p {
                r.end
            } else if r.end >= old_end {
                shift(r.end)
            } else {
                p
            };
        }
        let runs = std::mem::take(&mut self.runs);
        self.runs = normalize(runs);
        if !self.runs.is_empty() {
            self.text = text.to_string();
        }
    }

    /// The runs that touch the char range `start..end`, as byte ranges
    /// relative to `slice` (the text of that range).
    pub fn byte_runs(&self, slice: &str, start: usize, end: usize) -> Vec<(usize, usize, Run)> {
        let first = self.runs.partition_point(|r| r.end <= start);
        let mut out = Vec::new();
        for r in &self.runs[first..] {
            if r.start >= end {
                break;
            }
            let a = r.start.max(start) - start;
            let b = r.end.min(end) - start;
            out.push((a, b, *r));
        }
        if out.is_empty() {
            return out;
        }
        // one pass over the slice converts the char offsets to bytes
        let mut bytes: Vec<usize> = Vec::with_capacity(out.len() * 2);
        let wanted: Vec<usize> = out.iter().flat_map(|(a, b, _)| [*a, *b]).collect();
        let mut chars = slice.char_indices().map(|(i, _)| i).chain(std::iter::once(slice.len()));
        let mut ci = 0usize;
        let mut cur = chars.next().unwrap_or(slice.len());
        for w in wanted {
            while ci < w {
                cur = chars.next().unwrap_or(slice.len());
                ci += 1;
            }
            bytes.push(cur);
        }
        out.iter()
            .enumerate()
            .map(|(i, (_, _, r))| (bytes[2 * i], bytes[2 * i + 1], *r))
            .collect()
    }
}

/// Remove empty and colorless runs, and join neighbors with the same colors.
fn normalize(runs: Vec<Run>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::with_capacity(runs.len());
    for r in runs {
        if r.start >= r.end || (r.fg.is_none() && r.bg.is_none()) {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end == r.start && last.fg == r.fg && last.bg == r.bg => last.end = r.end,
            _ => out.push(r),
        }
    }
    out
}

/// Put the colors of `runs` (byte ranges of `job.text`) on the job's
/// sections. A text color replaces the syntax color. The spell underline
/// stays.
pub fn apply_to_job(job: &mut LayoutJob, runs: &[(usize, usize, Run)]) {
    if runs.is_empty() {
        return;
    }
    let mut out: Vec<LayoutSection> = Vec::with_capacity(job.sections.len() + runs.len() * 2);
    let mut ri = 0usize;
    for section in &job.sections {
        let (start, end) = (section.byte_range.start.0, section.byte_range.end.0);
        let mut at = start;
        while ri < runs.len() && runs[ri].1 <= at {
            ri += 1;
        }
        let mut k = ri;
        while at < end {
            let (piece_end, run) = match runs.get(k) {
                Some(&(a, _, _)) if a > at => (a.min(end), None),
                Some(&(_, b, r)) => (b.min(end), Some(r)),
                None => (end, None),
            };
            let mut format = section.format.clone();
            if let Some(r) = run {
                if let Some(c) = r.fg {
                    format.color = c;
                }
                if let Some(c) = r.bg {
                    format.background = c;
                }
            }
            out.push(LayoutSection {
                leading_space: if at == start { section.leading_space } else { 0.0 },
                byte_range: ByteIndex(at)..ByteIndex(piece_end),
                format,
            });
            at = piece_end;
            if runs.get(k).is_some_and(|&(_, b, _)| b <= at) {
                k += 1;
            }
        }
        if start == end {
            out.push(section.clone());
        }
    }
    job.sections = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color32 = Color32::from_rgb(205, 49, 49);
    const BLUE: Color32 = Color32::from_rgb(36, 114, 200);

    fn spans(s: &Styles) -> Vec<(usize, usize)> {
        s.runs().iter().map(|r| (r.start, r.end)).collect()
    }

    #[test]
    fn apply_splits_and_merges_runs() {
        let text = "hello world";
        let mut s = Styles::default();
        s.apply(text, 0, 5, Change::Set(RED), Change::Keep);
        s.apply(text, 3, 8, Change::Keep, Change::Set(BLUE));
        let r = s.runs();
        assert_eq!(spans(&s), vec![(0, 3), (3, 5), (5, 8)]);
        assert_eq!((r[1].fg, r[1].bg), (Some(RED), Some(BLUE)));
        assert_eq!((r[2].fg, r[2].bg), (None, Some(BLUE)));
        s.apply(text, 0, 11, Change::Clear, Change::Clear);
        assert!(s.runs().is_empty());
    }

    #[test]
    fn edits_move_the_runs() {
        let mut s = Styles::default();
        s.apply("abc def", 4, 7, Change::Set(RED), Change::Keep);
        s.sync("xx abc def"); // typed before the run
        assert_eq!(spans(&s), vec![(7, 10)]);
        s.sync("xx abc dXef"); // typed inside the run: it grows
        assert_eq!(spans(&s), vec![(7, 11)]);
        s.sync("xx abc dXef!"); // typed at the end: it does not grow
        assert_eq!(spans(&s), vec![(7, 11)]);
        s.sync("xx abc d!"); // deleted the end of the run
        assert_eq!(spans(&s), vec![(7, 8)]);
        s.sync("xx abc "); // deleted all of it
        assert!(s.runs().is_empty());
    }

    #[test]
    fn multibyte_text_uses_char_offsets() {
        let mut s = Styles::default();
        s.apply("é b", 2, 3, Change::Set(RED), Change::Keep);
        s.sync("éé b");
        assert_eq!(spans(&s), vec![(3, 4)]);
        assert_eq!(s.byte_runs("éé b", 0, 4).iter().map(|(a, b, _)| (*a, *b)).collect::<Vec<_>>(), vec![(5, 6)]);
    }

    #[test]
    fn job_sections_take_the_run_colors() {
        let mut job = LayoutJob::default();
        job.append("abcdef", 0.0, Default::default());
        let mut s = Styles::default();
        s.apply("abcdef", 2, 4, Change::Set(RED), Change::Set(BLUE));
        apply_to_job(&mut job, &s.byte_runs("abcdef", 0, 6));
        let parts: Vec<_> = job
            .sections
            .iter()
            .map(|x| (x.byte_range.start.0, x.byte_range.end.0, x.format.color, x.format.background))
            .collect();
        let d = eframe::egui::TextFormat::default();
        assert_eq!(
            parts,
            vec![(0, 2, d.color, d.background), (2, 4, RED, BLUE), (4, 6, d.color, d.background)]
        );
    }
}
