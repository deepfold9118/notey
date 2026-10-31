//! Markdown rendered with GPUI elements, for the Markdown preview: headings,
//! paragraphs with bold, italic, strikethrough, code and links, code
//! blocks, lists (with task boxes), block quotes, rules, tables and local
//! images. Links open in the default browser.

use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui::{
    div, img, prelude::*, px, AnyElement, FontStyle, FontWeight, HighlightStyle, Hsla, InteractiveText,
    SharedString, StrikethroughStyle, StyledText, UnderlineStyle,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// Colors and fonts for the rendered view.
#[derive(Clone)]
pub struct MdStyle {
    pub text: Hsla,
    pub weak: Hsla,
    pub accent: Hsla,
    pub code_bg: Hsla,
    pub stroke: Hsla,
    pub mono: SharedString,
    pub font_size: f32,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Span {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
}

/// Text with inline styles and links.
#[derive(Default)]
struct Inline {
    text: String,
    spans: Vec<(Range<usize>, Span)>,
    links: Vec<(Range<usize>, String)>,
}

enum Block {
    Heading(u8, Inline),
    Paragraph(Inline),
    Code(String),
    List { start: Option<u64>, items: Vec<(Option<bool>, Vec<Block>)> },
    Quote(Vec<Block>),
    Rule,
    Table { rows: Vec<Vec<Inline>> },
    Image { src: String, alt: String },
}

/// Builds blocks from pulldown-cmark events.
struct Builder<'a> {
    events: std::iter::Peekable<Parser<'a>>,
}

impl Builder<'_> {
    /// Blocks until the end of the enclosing container (or the input).
    fn blocks(&mut self, until: Option<TagEnd>) -> Vec<Block> {
        let mut out = Vec::new();
        while let Some(event) = self.events.next() {
            match event {
                Event::End(end) if Some(end) == until => break,
                Event::Start(Tag::Heading { level, .. }) => {
                    let n = match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    };
                    let inline = self.inline(TagEnd::Heading(level));
                    out.push(Block::Heading(n, inline));
                }
                Event::Start(Tag::Paragraph) => {
                    let mut inline = self.inline(TagEnd::Paragraph);
                    // A paragraph that is only an image shows the image.
                    if let Some(img) = inline.links.first().filter(|_| inline.text.starts_with('\u{1}')) {
                        out.push(Block::Image { src: img.1.clone(), alt: inline.text[1..].to_string() });
                    } else {
                        inline.text = inline.text.replace('\u{1}', "");
                        out.push(Block::Paragraph(inline));
                    }
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    let mut code = String::new();
                    for e in self.events.by_ref() {
                        match e {
                            Event::Text(t) => code.push_str(&t),
                            Event::End(TagEnd::CodeBlock) => break,
                            _ => {}
                        }
                    }
                    let _ = matches!(kind, CodeBlockKind::Fenced(_));
                    out.push(Block::Code(code.trim_end_matches('\n').to_string()));
                }
                Event::Start(Tag::List(start)) => {
                    let ordered = start.is_some();
                    let mut items = Vec::new();
                    while let Some(e) = self.events.next() {
                        match e {
                            Event::Start(Tag::Item) => {
                                let task = match self.events.peek() {
                                    Some(Event::TaskListMarker(done)) => {
                                        let done = *done;
                                        self.events.next();
                                        Some(done)
                                    }
                                    _ => None,
                                };
                                items.push((task, self.item_blocks()));
                            }
                            Event::End(TagEnd::List(_)) => break,
                            _ => {}
                        }
                    }
                    out.push(Block::List { start: if ordered { start } else { None }, items });
                }
                Event::Start(Tag::BlockQuote(_)) => {
                    let inner = self.blocks(Some(TagEnd::BlockQuote(None)));
                    out.push(Block::Quote(inner));
                }
                Event::Rule => out.push(Block::Rule),
                Event::Start(Tag::Table(_)) => {
                    let mut rows = Vec::new();
                    let mut row = Vec::new();
                    while let Some(e) = self.events.next() {
                        match e {
                            Event::Start(Tag::TableCell) => row.push(self.inline(TagEnd::TableCell)),
                            Event::End(TagEnd::TableHead) | Event::End(TagEnd::TableRow) => rows.push(std::mem::take(&mut row)),
                            Event::End(TagEnd::Table) => break,
                            _ => {}
                        }
                    }
                    out.push(Block::Table { rows });
                }
                Event::Html(h) | Event::InlineHtml(h) => {
                    let text = h.trim();
                    if !text.is_empty() {
                        out.push(Block::Code(text.to_string()));
                    }
                }
                // Loose text (in tight list items) is a paragraph.
                Event::Text(_) | Event::Code(_) | Event::Start(Tag::Emphasis | Tag::Strong | Tag::Link { .. }) => {
                    let mut inline = Inline::default();
                    self.push_inline(event, &mut inline, &mut Vec::new());
                    while let Some(next) = self.events.peek() {
                        if matches!(next, Event::End(_)) && until.is_some_and(|u| matches!(next, Event::End(e) if *e == u)) {
                            break;
                        }
                        if matches!(next, Event::Start(Tag::List(_) | Tag::Paragraph | Tag::CodeBlock(_))) {
                            break;
                        }
                        let e = self.events.next().unwrap();
                        self.push_inline(e, &mut inline, &mut Vec::new());
                    }
                    out.push(Block::Paragraph(inline));
                }
                _ => {}
            }
        }
        out
    }

    fn item_blocks(&mut self) -> Vec<Block> {
        self.blocks(Some(TagEnd::Item))
    }

    /// Inline content until `end`.
    fn inline(&mut self, end: TagEnd) -> Inline {
        let mut inline = Inline::default();
        let mut stack: Vec<(Tag, usize)> = Vec::new();
        while let Some(e) = self.events.next() {
            if matches!(&e, Event::End(e) if *e == end) {
                break;
            }
            self.push_inline_with(e, &mut inline, &mut stack);
        }
        inline
    }

    fn push_inline(&mut self, e: Event, inline: &mut Inline, stack: &mut Vec<(Tag<'static>, usize)>) {
        self.push_inline_with(e, inline, stack)
    }

    fn push_inline_with(&mut self, e: Event, inline: &mut Inline, stack: &mut Vec<(Tag<'static>, usize)>) {
        let span = |stack: &Vec<(Tag, usize)>| Span {
            bold: stack.iter().any(|(t, _)| matches!(t, Tag::Strong)),
            italic: stack.iter().any(|(t, _)| matches!(t, Tag::Emphasis)),
            strike: stack.iter().any(|(t, _)| matches!(t, Tag::Strikethrough)),
            code: false,
        };
        match e {
            Event::Text(t) => {
                let start = inline.text.len();
                inline.text.push_str(&t);
                inline.spans.push((start..inline.text.len(), span(stack)));
            }
            Event::Code(t) => {
                let start = inline.text.len();
                inline.text.push_str(&t);
                inline.spans.push((start..inline.text.len(), Span { code: true, ..span(stack) }));
            }
            Event::SoftBreak => inline.text.push(' '),
            Event::HardBreak => inline.text.push('\n'),
            Event::Start(Tag::Image { dest_url, .. }) => {
                // Marks an image; the alt text follows.
                if inline.text.is_empty() {
                    inline.text.push('\u{1}');
                }
                inline.links.push((inline.text.len()..inline.text.len(), dest_url.to_string()));
                stack.push((Tag::Image { link_type: pulldown_cmark::LinkType::Inline, dest_url: "".into(), title: "".into(), id: "".into() }, inline.text.len()));
            }
            Event::Start(tag) => {
                let tag = match tag {
                    Tag::Link { dest_url, .. } => Tag::Link {
                        link_type: pulldown_cmark::LinkType::Inline,
                        dest_url: dest_url.to_string().into(),
                        title: "".into(),
                        id: "".into(),
                    },
                    Tag::Emphasis => Tag::Emphasis,
                    Tag::Strong => Tag::Strong,
                    Tag::Strikethrough => Tag::Strikethrough,
                    _ => Tag::Paragraph,
                };
                stack.push((tag, inline.text.len()));
            }
            Event::End(_) => {
                if let Some((Tag::Link { dest_url, .. }, start)) = stack.pop() {
                    inline.links.push((start..inline.text.len(), dest_url.to_string()));
                }
            }
            Event::TaskListMarker(done) => inline.text.push_str(if done { "☑ " } else { "☐ " }),
            Event::FootnoteReference(r) => inline.text.push_str(&format!("[{r}]")),
            _ => {}
        }
    }
}

fn parse(md: &str) -> Vec<Block> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    Builder { events: Parser::new_ext(md, opts).peekable() }.blocks(None)
}

/// The rendered document. `base` is the folder relative image paths start
/// from; `id` keeps element ids apart between documents.
pub fn render(md: &str, base: Option<&Path>, style: &MdStyle) -> AnyElement {
    let blocks = parse(md);
    let mut counter = 0usize;
    div()
        .flex()
        .flex_col()
        .gap_3()
        .text_color(style.text)
        .text_size(px(style.font_size))
        .line_height(px(style.font_size * 1.6))
        .children(blocks.iter().map(|b| render_block(b, base, style, &mut counter)))
        .into_any_element()
}

fn render_inline(inline: &Inline, style: &MdStyle, counter: &mut usize) -> AnyElement {
    *counter += 1;
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for (range, span) in &inline.spans {
        if *span == Span::default() || range.is_empty() {
            continue;
        }
        highlights.push((
            range.clone(),
            HighlightStyle {
                font_weight: span.bold.then_some(FontWeight::BOLD),
                font_style: span.italic.then_some(FontStyle::Italic),
                strikethrough: span.strike.then(|| StrikethroughStyle { thickness: px(1.), color: None }),
                background_color: span.code.then_some(style.code_bg),
                ..Default::default()
            },
        ));
    }
    let links: Vec<(Range<usize>, String)> = inline.links.iter().filter(|(r, _)| !r.is_empty()).cloned().collect();
    for (range, _) in &links {
        highlights.push((
            range.clone(),
            HighlightStyle {
                color: Some(style.accent),
                underline: Some(UnderlineStyle { thickness: px(1.), color: Some(style.accent), wavy: false }),
                ..Default::default()
            },
        ));
    }
    // GPUI wants highlights in order and without overlaps: split at every
    // boundary and combine the highlights over each piece (links last).
    let mut cuts: Vec<usize> = highlights.iter().flat_map(|(r, _)| [r.start, r.end]).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let merged: Vec<(Range<usize>, HighlightStyle)> = cuts
        .windows(2)
        .filter_map(|w| {
            let piece = w[0]..w[1];
            let mut combined: Option<HighlightStyle> = None;
            for (r, h) in &highlights {
                if r.start <= piece.start && r.end >= piece.end {
                    combined = Some(combined.unwrap_or_default().highlight(*h));
                }
            }
            combined.map(|h| (piece, h))
        })
        .collect();
    let text = StyledText::new(SharedString::from(inline.text.clone())).with_highlights(merged);
    if links.is_empty() {
        return text.into_any_element();
    }
    let urls: Vec<String> = links.iter().map(|(_, u)| u.clone()).collect();
    InteractiveText::new(("md-inline", *counter), text)
        .on_click(links.into_iter().map(|(r, _)| r).collect(), move |i, _, _| {
            if let Some(url) = urls.get(i) {
                let _ = std::process::Command::new("explorer").arg(url).spawn();
            }
        })
        .into_any_element()
}

fn render_block(block: &Block, base: Option<&Path>, style: &MdStyle, counter: &mut usize) -> AnyElement {
    match block {
        Block::Heading(level, inline) => {
            let size = style.font_size * [2.0, 1.6, 1.3, 1.15, 1.0, 0.9][(*level as usize - 1).min(5)];
            div()
                .mt_2()
                .text_size(px(size))
                .line_height(px(size * 1.3))
                .font_weight(FontWeight::SEMIBOLD)
                .when(*level <= 2, |d| d.pb_1().border_b_1().border_color(style.stroke))
                .child(render_inline(inline, style, counter))
                .into_any_element()
        }
        Block::Paragraph(inline) => div().child(render_inline(inline, style, counter)).into_any_element(),
        Block::Code(code) => div()
            .p_3()
            .rounded(px(6.))
            .bg(style.code_bg)
            .font_family(style.mono.clone())
            .text_size(px(style.font_size * 0.9))
            .whitespace_nowrap()
            .overflow_hidden()
            .children(code.lines().map(|l| div().child(SharedString::from(l.to_string()))))
            .into_any_element(),
        Block::List { start, items } => div()
            .flex()
            .flex_col()
            .gap_1()
            .children(items.iter().enumerate().map(|(i, (task, blocks))| {
                let marker = match (start, task) {
                    (_, Some(true)) => "☑".to_string(),
                    (_, Some(false)) => "☐".to_string(),
                    (Some(n), None) => format!("{}.", n + i as u64),
                    (None, None) => "•".to_string(),
                };
                div()
                    .flex()
                    .gap_2()
                    .child(div().w(px(28.)).flex_none().flex().justify_end().text_color(style.weak).child(marker))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(blocks.iter().map(|b| render_block(b, base, style, counter))),
                    )
            }))
            .into_any_element(),
        Block::Quote(blocks) => div()
            .pl_3()
            .border_l_4()
            .border_color(style.stroke)
            .text_color(style.weak)
            .flex()
            .flex_col()
            .gap_2()
            .children(blocks.iter().map(|b| render_block(b, base, style, counter)))
            .into_any_element(),
        Block::Rule => div().h(px(1.)).bg(style.stroke).into_any_element(),
        Block::Table { rows } => div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(style.stroke)
            .rounded(px(4.))
            .children(rows.iter().enumerate().map(|(r, row)| {
                div()
                    .flex()
                    .when(r > 0, |d| d.border_t_1().border_color(style.stroke))
                    .when(r == 0, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .children(row.iter().map(|cell| div().flex_1().px_2().py_1().child(render_inline(cell, style, counter))))
            }))
            .into_any_element(),
        Block::Image { src, alt } => {
            let local = !src.contains("://");
            if local {
                let path = match base {
                    Some(base) => base.join(src),
                    None => PathBuf::from(src),
                };
                if path.is_file() {
                    return div().child(img(path).max_w_full()).into_any_element();
                }
            }
            div().text_color(style.weak).child(format!("[image: {alt}]")).into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_common_blocks() {
        let md = "# Title\n\nSome **bold** and *it* and `code` with a [link](https://x.y).\n\n- one\n- [x] done\n\n1. first\n2. second\n\n> quoted\n\n---\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let blocks = parse(md);
        let kinds: Vec<&str> = blocks
            .iter()
            .map(|b| match b {
                Block::Heading(..) => "h",
                Block::Paragraph(_) => "p",
                Block::Code(_) => "code",
                Block::List { start: None, .. } => "ul",
                Block::List { .. } => "ol",
                Block::Quote(_) => "quote",
                Block::Rule => "rule",
                Block::Table { .. } => "table",
                Block::Image { .. } => "img",
            })
            .collect();
        assert_eq!(kinds, ["h", "p", "ul", "ol", "quote", "rule", "code", "table"]);
        let Block::Paragraph(p) = &blocks[1] else { panic!() };
        assert_eq!(p.text, "Some bold and it and code with a link.");
        assert_eq!(&p.text[33..37], "link");
        assert_eq!(p.links, [(33..37, "https://x.y".to_string())]);
        assert!(p.spans.iter().any(|(r, s)| s.bold && &p.text[r.clone()] == "bold"));
        let Block::List { items, .. } = &blocks[2] else { panic!() };
        assert_eq!(items[1].0, Some(true));
        let Block::Table { rows } = &blocks[7] else { panic!() };
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1][1].text, "2");
    }

    #[test]
    fn an_image_paragraph_is_an_image() {
        let blocks = parse("![a cat](cat.png)\n");
        assert!(matches!(&blocks[0], Block::Image { src, alt } if src == "cat.png" && alt == "a cat"));
    }
}
