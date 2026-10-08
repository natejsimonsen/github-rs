//! GitHub-flavored Markdown, drawn like github.com's `markdown-body` CSS:
//! 14 px text on 21 px lines, 16 px between blocks, Primer colors.
//!
//! The source is parsed once into a small block tree (cached by its hash).
//! Each frame then builds one text layout per paragraph; egui caches those,
//! so a long comment is cheap after its first frame.

use crate::theme::{self, Palette};
use egui::epaint::text::{Galley, PlacedRow};
use egui::load::TexturePoll;
use egui::text::{LayoutJob, TextFormat};
use egui::text_selection::LabelSelectionState;
use egui::{Align, Color32, CursorIcon, FontFamily, FontId, Id, Pos2, Rect, Sense, Shape, Stroke, Ui, UiBuilder, Vec2, pos2, vec2};
use pulldown_cmark::{Alignment, BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

/// Draws GitHub-flavored Markdown. `source` has already been through
/// util::clean_markdown and gfm::to_markdown. Returns a URL if a link was clicked.
pub fn show(ui: &mut egui::Ui, id: egui::Id, source: &str) -> Option<String> {
    let doc = parsed(source);
    let p = theme::palette(ui.ctx());
    let mut r = Render { p, color: p.fg, size: 14.0, first: None, clicked: None, metrics: HashMap::new(), depth: 0 };
    ui.vertical(|ui| {
        // We place everything ourselves, and our text has explicit colors.
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        ui.visuals_mut().override_text_color = None;
        r.blocks(ui, id, &doc, false, false);
    });
    r.clicked
}

thread_local! {
    static CACHE: RefCell<HashMap<u64, Arc<Vec<Block>>>> = RefCell::new(HashMap::new());
}

/// The block tree for `source`, parsed once and then kept.
fn parsed(source: &str) -> Arc<Vec<Block>> {
    let key = Id::new(source).value();
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(doc) = c.get(&key) {
            return doc.clone();
        }
        // Edited previews add new entries; don't let them pile up forever.
        if c.len() > 2000 {
            c.clear();
        }
        let doc = Arc::new(parse(source));
        c.insert(key, doc.clone());
        doc
    })
}

// ---------------------------------------------------------------------------
// The block tree

#[derive(Clone, Copy, Default, PartialEq, Debug)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    /// A footnote reference: small and raised.
    note: bool,
    /// Index into `Inline::links`.
    link: Option<usize>,
}

#[derive(Clone, PartialEq, Debug)]
enum Span {
    Text(String, Style),
    /// A color emoji picture (an `emoji:` URI), drawn at text size.
    Emoji(String, Style),
    /// A normal picture. Paragraphs move these onto their own line.
    Image {
        url: String,
        alt: String,
        link: Option<String>,
    },
}

/// A run of styled text, like one paragraph.
#[derive(Clone, Default, PartialEq, Debug)]
struct Inline {
    spans: Vec<Span>,
    links: Vec<String>,
}

impl Inline {
    fn push_text(&mut self, text: &str, style: Style) {
        if let Some(Span::Text(t, s)) = self.spans.last_mut()
            && *s == style
        {
            t.push_str(text);
            return;
        }
        self.spans.push(Span::Text(text.to_string(), style));
    }

    fn plain(text: &str) -> Inline {
        let mut i = Inline::default();
        i.push_text(text, Style::default());
        i
    }

    fn is_blank(&self) -> bool {
        self.spans.iter().all(|s| matches!(s, Span::Text(t, _) if t.trim().is_empty()))
    }

    /// Where pictures can't get their own line (table cells, headings),
    /// show their alt text as a link instead.
    fn images_as_links(mut self) -> Inline {
        for i in 0..self.spans.len() {
            if let Span::Image { url, alt, link } = &self.spans[i] {
                let text = if alt.trim().is_empty() { "image".to_string() } else { alt.clone() };
                self.links.push(link.clone().unwrap_or_else(|| url.clone()));
                let style = Style { link: Some(self.links.len() - 1), ..Style::default() };
                self.spans[i] = Span::Text(text, style);
            }
        }
        self
    }

    #[cfg(test)]
    fn text(&self) -> String {
        self.spans
            .iter()
            .map(|s| match s {
                Span::Text(t, _) => t.as_str(),
                Span::Emoji(..) => "<emoji>",
                Span::Image { .. } => "<image>",
            })
            .collect()
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Alert {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

#[derive(Clone, PartialEq, Debug)]
enum Block {
    Para(Inline),
    Heading(u8, Inline),
    Code {
        lang: String,
        text: String,
    },
    Quote(Vec<Block>),
    Alert(Alert, Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    /// The first row is the header.
    Table {
        align: Vec<Alignment>,
        rows: Vec<Vec<Inline>>,
    },
    Image {
        url: String,
        alt: String,
        link: Option<String>,
    },
    Rule,
    Details {
        summary: Inline,
        open: bool,
        body: Vec<Block>,
    },
    Footnotes(Vec<Item>),
}

#[derive(Clone, Default, PartialEq, Debug)]
struct Item {
    task: Option<bool>,
    /// Loose items wrap their text in paragraphs, with 16 px between.
    loose: bool,
    blocks: Vec<Block>,
}

fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_FOOTNOTES | Options::ENABLE_GFM
}

fn parse(source: &str) -> Vec<Block> {
    let mut b = Builder::default();
    for event in Parser::new_ext(source, options()) {
        b.event(event);
    }
    b.finish()
}

/// One line of Markdown (like a `<summary>`), as styled text.
fn parse_inline(source: &str) -> Inline {
    let blocks = parse(source);
    match blocks.into_iter().next() {
        Some(Block::Para(i)) | Some(Block::Heading(_, i)) => i.images_as_links(),
        _ => Inline::plain(source),
    }
}

/// Drops spaces left at the ends of a paragraph by a picture moved out of it.
fn trim(mut inl: Inline) -> Inline {
    if let Some(Span::Text(t, _)) = inl.spans.first_mut() {
        *t = t.trim_start().to_string();
    }
    if let Some(Span::Text(t, _)) = inl.spans.last_mut() {
        t.truncate(t.trim_end().len());
    }
    inl
}

/// An open container while building the tree.
enum Frame {
    Root,
    Quote(Option<BlockQuoteKind>),
    List(Option<u64>, Vec<Item>),
    Item(Option<bool>, bool),
    /// `<details>`: its summary and whether it starts open.
    Details(Inline, bool),
    Footnote,
}

struct Level {
    frame: Frame,
    blocks: Vec<Block>,
}

/// Turns pulldown-cmark's events into the block tree.
#[derive(Default)]
struct Builder {
    stack: Vec<Level>,
    inline: Option<Inline>,
    /// The open inline text isn't in a paragraph (tight list items).
    implicit: bool,
    bold: u32,
    italic: u32,
    strike: u32,
    links: Vec<usize>,
    /// Picture being read: URL and alt text.
    image: Option<(String, String)>,
    heading: Option<u8>,
    code: Option<(String, String)>,
    html: Option<String>,
    table: Option<(Vec<Alignment>, Vec<Vec<Inline>>)>,
    footnotes: Vec<Item>,
}

impl Builder {
    fn style(&self) -> Style {
        Style { bold: self.bold > 0, italic: self.italic > 0, strike: self.strike > 0, code: false, note: false, link: self.links.last().copied() }
    }

    fn push(&mut self, block: Block) {
        if self.stack.is_empty() {
            self.stack.push(Level { frame: Frame::Root, blocks: Vec::new() });
        }
        self.stack.last_mut().unwrap().blocks.push(block);
    }

    fn inline(&mut self) -> &mut Inline {
        if self.inline.is_none() {
            self.implicit = true;
        }
        self.inline.get_or_insert_with(Inline::default)
    }

    /// Ends loose text (from a tight list item) before a new block starts.
    fn flush(&mut self) {
        if self.implicit
            && let Some(i) = self.inline.take()
        {
            self.push_para(i);
        }
        self.implicit = false;
    }

    /// Adds a paragraph, moving pictures onto lines of their own.
    fn push_para(&mut self, inl: Inline) {
        let fresh = || Inline { spans: Vec::new(), links: inl.links.clone() };
        let mut cur = fresh();
        for span in inl.spans.iter().cloned() {
            if let Span::Image { url, alt, link } = span {
                if !cur.is_blank() {
                    self.push(Block::Para(trim(std::mem::replace(&mut cur, fresh()))));
                }
                self.push(Block::Image { url, alt, link });
            } else {
                cur.spans.push(span);
            }
        }
        if !cur.is_blank() {
            self.push(Block::Para(trim(cur)));
        }
    }

    fn open(&mut self, frame: Frame) {
        self.flush();
        if self.stack.is_empty() {
            self.stack.push(Level { frame: Frame::Root, blocks: Vec::new() });
        }
        self.stack.push(Level { frame, blocks: Vec::new() });
    }

    /// Closes the innermost container and adds it to its parent.
    fn close_top(&mut self) {
        self.flush();
        let Some(level) = self.stack.pop() else { return };
        let blocks = level.blocks;
        match level.frame {
            Frame::Root => self.stack.push(Level { frame: Frame::Root, blocks }),
            Frame::Quote(kind) => self.push(match kind {
                None => Block::Quote(blocks),
                Some(k) => Block::Alert(
                    match k {
                        BlockQuoteKind::Note => Alert::Note,
                        BlockQuoteKind::Tip => Alert::Tip,
                        BlockQuoteKind::Important => Alert::Important,
                        BlockQuoteKind::Warning => Alert::Warning,
                        BlockQuoteKind::Caution => Alert::Caution,
                    },
                    blocks,
                ),
            }),
            Frame::List(start, items) => self.push(Block::List { start, items }),
            Frame::Item(task, loose) => {
                let item = Item { task, loose, blocks };
                match self.stack.last_mut() {
                    Some(Level { frame: Frame::List(_, items), .. }) => items.push(item),
                    _ => item.blocks.into_iter().for_each(|b| self.push(b)),
                }
            }
            Frame::Details(summary, open) => self.push(Block::Details { summary, open, body: blocks }),
            Frame::Footnote => self.footnotes.push(Item { blocks, ..Item::default() }),
        }
    }

    /// Closes a Markdown container, and any `<details>` left open inside it.
    fn close_container(&mut self) {
        self.flush();
        while matches!(self.stack.last(), Some(Level { frame: Frame::Details(..), .. })) {
            self.close_top();
        }
        self.close_top();
    }

    fn event(&mut self, event: Event) {
        if let Some((_, text)) = &mut self.code {
            match event {
                Event::Text(t) => text.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, mut text) = self.code.take().unwrap();
                    if text.ends_with('\n') {
                        text.pop();
                    }
                    self.push(Block::Code { lang, text });
                }
                _ => {}
            }
            return;
        }
        if let Some(html) = &mut self.html {
            match event {
                Event::Html(t) | Event::Text(t) => html.push_str(&t),
                Event::End(TagEnd::HtmlBlock) => {
                    let html = self.html.take().unwrap();
                    self.html_block(&html);
                }
                _ => {}
            }
            return;
        }
        if let Some((_, alt)) = &mut self.image {
            match event {
                Event::Text(t) | Event::Code(t) => alt.push_str(&t),
                Event::End(TagEnd::Image) => {
                    let (url, alt) = self.image.take().unwrap();
                    let style = self.style();
                    let link = style.link.and_then(|l| self.inline().links.get(l).cloned());
                    if url.starts_with(crate::gfm::EMOJI_SCHEME) {
                        self.inline().spans.push(Span::Emoji(url, style));
                    } else {
                        self.inline().spans.push(Span::Image { url, alt, link });
                    }
                }
                _ => {}
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                let style = self.style();
                self.inline().push_text(&t, style);
            }
            Event::Code(t) => {
                let style = Style { code: true, ..self.style() };
                self.inline().push_text(&t, style);
            }
            // Real tags were already removed by clean_markdown; what's left
            // is text that only looks like HTML, like `Vec<T>`.
            // A <br> left in a table cell (see util::html_table) breaks the line.
            Event::InlineHtml(t) if matches!(t.trim().to_ascii_lowercase().as_str(), "<br>" | "<br/>" | "<br />") => {
                let style = self.style();
                self.inline().push_text("\n", style);
            }
            Event::InlineHtml(t) | Event::Html(t) | Event::InlineMath(t) | Event::DisplayMath(t) => {
                let style = self.style();
                self.inline().push_text(&t, style);
            }
            // In GitHub comments every newline is a line break, unlike in
            // Markdown files.
            Event::SoftBreak => {
                let style = self.style();
                self.inline().push_text("\n", style);
            }
            Event::HardBreak => {
                let style = self.style();
                self.inline().push_text("\n", style);
            }
            Event::FootnoteReference(label) => {
                let style = Style { note: true, ..self.style() };
                self.inline().push_text(&format!("[{label}]"), style);
            }
            Event::Rule => {
                self.flush();
                self.push(Block::Rule);
            }
            Event::TaskListMarker(done) => {
                if let Some(Level { frame: Frame::Item(task, _), .. }) = self.stack.last_mut() {
                    *task = Some(done);
                }
            }
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.flush();
                if let Some(Level { frame: Frame::Item(_, loose), .. }) = self.stack.last_mut() {
                    *loose = true;
                }
                self.inline = Some(Inline::default());
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.heading = Some(level as u8);
                self.inline = Some(Inline::default());
            }
            Tag::BlockQuote(kind) => self.open(Frame::Quote(kind)),
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or("").to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::HtmlBlock => {
                self.flush();
                self.html = Some(String::new());
            }
            Tag::List(start) => self.open(Frame::List(start, Vec::new())),
            Tag::Item => self.open(Frame::Item(None, false)),
            Tag::FootnoteDefinition(_) => self.open(Frame::Footnote),
            Tag::Table(align) => {
                self.flush();
                self.table = Some((align, Vec::new()));
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some((_, rows)) = &mut self.table {
                    rows.push(Vec::new());
                }
            }
            Tag::TableCell => {
                self.inline = Some(Inline::default());
                self.implicit = false;
            }
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => {
                let inl = self.inline();
                inl.links.push(dest_url.to_string());
                let i = inl.links.len() - 1;
                self.links.push(i);
            }
            Tag::Image { dest_url, .. } => {
                self.inline();
                self.image = Some((dest_url.to_string(), String::new()));
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if let Some(i) = self.inline.take() {
                    self.push_para(i);
                }
                self.implicit = false;
            }
            TagEnd::Heading(_) => {
                let inl = self.inline.take().unwrap_or_default();
                let level = self.heading.take().unwrap_or(1);
                self.push(Block::Heading(level, inl.images_as_links()));
            }
            TagEnd::BlockQuote(_) | TagEnd::Item | TagEnd::FootnoteDefinition => self.close_container(),
            TagEnd::List(_) => {
                self.flush();
                self.close_top();
            }
            TagEnd::TableCell => {
                let inl = self.inline.take().unwrap_or_default().images_as_links();
                if let Some((_, rows)) = &mut self.table
                    && let Some(row) = rows.last_mut()
                {
                    row.push(inl);
                }
            }
            TagEnd::Table => {
                if let Some((align, rows)) = self.table.take() {
                    self.push(Block::Table { align, rows });
                }
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.links.pop();
            }
            _ => {}
        }
    }

    /// clean_markdown leaves `<details>`, `<summary>…</summary>` and
    /// `</details>` as HTML blocks; they open and close a foldable section.
    fn html_block(&mut self, html: &str) {
        let mut rest = html;
        loop {
            let lower = rest.to_ascii_lowercase();
            let next = ["<details", "</details", "<summary"].into_iter().filter_map(|t| lower.find(t).map(|i| (i, t))).min();
            let Some((i, tag)) = next else {
                self.html_text(rest);
                return;
            };
            self.html_text(&rest[..i]);
            let end = rest[i..].find('>').map_or(rest.len(), |j| i + j + 1);
            match tag {
                "<details" => {
                    let open = lower[i..end].contains("open");
                    self.open(Frame::Details(Inline::default(), open));
                    rest = &rest[end..];
                }
                "</details" => {
                    if matches!(self.stack.last(), Some(Level { frame: Frame::Details(..), .. })) {
                        self.close_top();
                    }
                    rest = &rest[end..];
                }
                _ => {
                    let body = &rest[end..];
                    let (inner, after) = match body.to_ascii_lowercase().find("</summary>") {
                        Some(k) => (&body[..k], &body[k + "</summary>".len()..]),
                        None => (body, ""),
                    };
                    let summary = parse_inline(inner.trim());
                    match self.stack.last_mut() {
                        Some(Level { frame: Frame::Details(s, _), .. }) => *s = summary,
                        _ => self.push(Block::Para(summary)),
                    }
                    rest = after;
                }
            }
        }
    }

    /// Other HTML that survived clean_markdown is text that looks like a tag.
    fn html_text(&mut self, text: &str) {
        let text = text.trim();
        if !text.is_empty() {
            self.flush();
            self.push(Block::Para(Inline::plain(text)));
        }
    }

    fn finish(mut self) -> Vec<Block> {
        self.flush();
        if let Some(i) = self.inline.take() {
            self.push_para(i);
        }
        while self.stack.len() > 1 {
            self.close_top();
        }
        let mut blocks = self.stack.pop().map(|l| l.blocks).unwrap_or_default();
        if !self.footnotes.is_empty() {
            blocks.push(Block::Footnotes(self.footnotes));
        }
        blocks
    }
}

// ---------------------------------------------------------------------------
// Drawing

/// The base text style of a block.
#[derive(Clone, Copy)]
struct Base {
    size: f32,
    bold: bool,
    mono: bool,
    color: Color32,
    /// Line height in points.
    line: f32,
}

/// Where things sit inside a laid-out paragraph, by char index.
#[derive(Default)]
struct Runs {
    links: Vec<(Range<usize>, usize)>,
    code: Vec<Range<usize>>,
    code_size: f32,
    emoji: Vec<(usize, String, f32)>,
}

struct Render {
    p: &'static Palette,
    /// Text color here (muted inside quotes).
    color: Color32,
    /// Base font size here (smaller in footnotes).
    size: f32,
    /// Baseline of the first line drawn since this was last cleared, so
    /// list markers can line up with it.
    first: Option<f32>,
    clicked: Option<String>,
    /// Baseline and line height of each font, measured once per frame.
    metrics: HashMap<(bool, bool, u32), (f32, f32)>,
    /// How many lists we're inside.
    depth: usize,
}

fn font(mono: bool, bold: bool, size: f32) -> FontId {
    match (mono, bold) {
        (true, true) => FontId::new(size, FontFamily::Name(theme::MONO_BOLD.into())),
        (true, false) => theme::mono(size),
        (false, true) => theme::bold(size),
        (false, false) => theme::body(size),
    }
}

fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

fn alert_style(kind: Alert, dark: bool) -> (Color32, &'static str) {
    let (light, dark_c, name) = match kind {
        Alert::Note => (0x0969da, 0x4493f8, "Note"),
        Alert::Tip => (0x1a7f37, 0x3fb950, "Tip"),
        Alert::Important => (0x8250df, 0xab7df8, "Important"),
        Alert::Warning => (0x9a6700, 0xd29922, "Warning"),
        Alert::Caution => (0xd1242f, 0xf85149, "Caution"),
    };
    (hex(if dark { dark_c } else { light }), name)
}

/// The char under `pos` (relative to the galley), if any.
fn char_at(galley: &Galley, pos: Vec2) -> Option<usize> {
    let mut start = 0;
    for row in &galley.rows {
        if (row.pos.y..=row.pos.y + row.size.y).contains(&pos.y) {
            let x = pos.x - row.pos.x;
            return row.glyphs.iter().position(|g| x >= g.pos.x && x < g.pos.x + g.advance_width).map(|i| start + i);
        }
        start += row.glyphs.len() + row.ends_with_newline as usize;
    }
    None
}

/// Calls `f(row, left, right, first glyph)` for each row's piece of `range`.
fn row_pieces(galley: &Galley, range: &Range<usize>, mut f: impl FnMut(&PlacedRow, f32, f32, &egui::epaint::text::Glyph)) {
    let mut start = 0;
    for row in &galley.rows {
        let n = row.glyphs.len();
        let (a, b) = (range.start.max(start), range.end.min(start + n));
        if a < b {
            let (g0, g1) = (&row.glyphs[a - start], &row.glyphs[b - 1 - start]);
            f(row, row.pos.x + g0.pos.x, row.pos.x + g1.pos.x + g1.advance_width, g0);
        }
        start += n + row.ends_with_newline as usize;
    }
}

impl Render {
    fn base(&self) -> Base {
        Base { size: self.size, bold: false, mono: false, color: self.color, line: (self.size * 1.5).round() }
    }

    /// Baseline (from the top of an unpadded line) and line height of a font.
    fn metric(&mut self, ui: &Ui, mono: bool, bold: bool, size: f32) -> (f32, f32) {
        *self.metrics.entry((mono, bold, size.to_bits())).or_insert_with(|| {
            let g = ui.painter().layout_no_wrap("x".into(), font(mono, bold, size), Color32::WHITE);
            let row = &g.rows[0];
            (row.glyphs.first().map_or(size, |g| g.pos.y), row.size.y)
        })
    }

    fn layout(&mut self, ui: &Ui, inl: &Inline, base: Base, wrap: f32, halign: Align) -> (Arc<Galley>, Runs) {
        let mut job = LayoutJob::default();
        job.wrap.max_width = wrap;
        job.halign = halign;
        let mut runs = Runs { code_size: base.size * 0.85, ..Runs::default() };
        let (base_asc, _) = self.metric(ui, base.mono, base.bold, base.size);
        let mut chars = 0;
        // Room to leave after inline code, for its padded background.
        let mut pad = 0.0;
        for span in &inl.spans {
            match span {
                Span::Text(text, st) => {
                    let mono = base.mono || st.code;
                    let bold = base.bold || st.bold;
                    let size = if st.code {
                        runs.code_size
                    } else if st.note {
                        base.size * 0.75
                    } else {
                        base.size
                    };
                    let (asc, _) = self.metric(ui, mono, bold, size);
                    // egui puts every font's baseline at its own ascent, so
                    // smaller fonts would ride high. Shorten their line and
                    // push them to the bottom to share the base baseline.
                    let drop = base_asc - asc;
                    let (line_height, valign) = if drop > 0.0 && !st.note { (base.line - drop, Align::BOTTOM) } else { (base.line, Align::TOP) };
                    let color = if st.link.is_some() { self.p.accent } else { base.color };
                    let format = TextFormat {
                        font_id: font(mono, bold, size),
                        color,
                        italics: st.italic,
                        strikethrough: if st.strike { Stroke::new(1.0, color) } else { Stroke::NONE },
                        line_height: Some(line_height),
                        valign,
                        ..Default::default()
                    };
                    let code_pad = if st.code { 0.3 * size } else { 0.0 };
                    job.append(text, pad + code_pad, format);
                    pad = code_pad;
                    let n = text.chars().count();
                    if st.code {
                        runs.code.push(chars..chars + n);
                    }
                    if let Some(l) = st.link {
                        runs.links.push((chars..chars + n, l));
                    }
                    chars += n;
                }
                Span::Emoji(uri, st) => {
                    // An em space holds the picture's place; we paint over it.
                    let size = (base.size * 18.0 / 14.0).round();
                    let format = TextFormat {
                        font_id: theme::body(size),
                        color: Color32::TRANSPARENT,
                        line_height: Some(base.line),
                        valign: Align::TOP,
                        ..Default::default()
                    };
                    job.append("\u{2003}", pad, format);
                    pad = 0.0;
                    runs.emoji.push((chars, uri.clone(), size));
                    if let Some(l) = st.link {
                        runs.links.push((chars..chars + 1, l));
                    }
                    chars += 1;
                }
                Span::Image { alt, .. } => {
                    let format = TextFormat { font_id: theme::body(base.size), color: base.color, ..Default::default() };
                    job.append(alt, pad, format);
                    pad = 0.0;
                    chars += alt.chars().count();
                }
            }
        }
        (ui.painter().layout_job(job), runs)
    }

    /// Draws laid-out text whose top-left line box starts at `pos`. When
    /// `interactive`, the text is selectable and its links work.
    #[allow(clippy::too_many_arguments)]
    fn paint_text(&mut self, ui: &Ui, id: Id, pos: Pos2, galley: &Arc<Galley>, runs: &Runs, links: &[String], base: Base, interactive: bool) {
        let (asc, height) = self.metric(ui, base.mono, base.bold, base.size);
        // CSS centers text in its line box; egui puts it at the top.
        let ppp = ui.pixels_per_point();
        let shift = (((base.line - height) / 2.0).max(0.0) * ppp).round() / ppp;
        let gpos = pos + vec2(0.0, shift);
        if self.first.is_none()
            && let Some(row) = galley.rows.first()
        {
            self.first = Some(gpos.y + row.pos.y + asc);
        }
        let rect = galley.rect.translate(pos.to_vec2());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let painter = ui.painter();
        let bg = painter.add(Shape::Noop);
        let mut hovered = None;
        if interactive {
            let resp = ui.interact(rect, id, Sense::click_and_drag() - Sense::FOCUSABLE);
            hovered = resp.hover_pos().and_then(|p| char_at(galley, p - gpos)).and_then(|c| runs.links.iter().find(|(r, _)| r.contains(&c)).map(|(_, l)| *l));
            LabelSelectionState::label_text_selection(ui, &resp, gpos, galley.clone(), self.color, Stroke::NONE);
            if let Some(url) = hovered.and_then(|l| links.get(l)) {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                if resp.clicked() {
                    self.clicked = Some(url.clone());
                }
                resp.on_hover_text_at_pointer(url.as_str());
            }
        } else {
            painter.galley(gpos, galley.clone(), self.color);
        }

        // Inline code: a rounded, padded background under the text.
        let code_bg = if self.p.dark { Color32::from_rgba_unmultiplied(101, 108, 118, 51) } else { Color32::from_rgba_unmultiplied(129, 139, 152, 31) };
        let pad = 0.4 * runs.code_size;
        let mut shapes = Vec::new();
        for range in &runs.code {
            row_pieces(galley, range, |row, x0, x1, g| {
                let top = gpos.y + row.pos.y + g.pos.y - g.font_ascent - 0.2 * runs.code_size;
                let r = Rect::from_min_max(pos2(gpos.x + x0 - pad, top), pos2(gpos.x + x1 + pad, top + g.font_height + 0.4 * runs.code_size));
                shapes.push(Shape::rect_filled(r, 6.0, code_bg));
            });
        }
        painter.set(bg, Shape::Vec(shapes));

        // Underline only the link under the pointer, like github.com.
        if let Some(l) = hovered {
            for (range, _) in runs.links.iter().filter(|(_, i)| *i == l) {
                row_pieces(galley, range, |row, x0, x1, g| {
                    let y = (gpos.y + row.pos.y + g.pos.y + 1.5).round() + 0.5 / ppp;
                    painter.hline((gpos.x + x0)..=(gpos.x + x1), y, Stroke::new(1.0, self.p.accent));
                });
            }
        }

        for (c, uri, size) in &runs.emoji {
            row_pieces(galley, &(*c..c + 1), |row, x0, x1, _| {
                let center = pos2(gpos.x + (x0 + x1) / 2.0, pos.y + row.pos.y + base.line / 2.0);
                egui::Image::new(uri.as_str()).paint_at(ui, Rect::from_center_size(center, Vec2::splat(*size)));
            });
        }
    }

    /// A wrapped run of text on its own line(s).
    fn text(&mut self, ui: &mut Ui, id: Id, inl: &Inline, base: Base) {
        let (galley, runs) = self.layout(ui, inl, base, ui.available_width(), Align::LEFT);
        // Trailing spaces (and emoji placeholders) may hang past the wrap
        // width; don't let them widen the page.
        let (_, rect) = ui.allocate_space(vec2(galley.size().x.min(ui.available_width()), galley.size().y));
        self.paint_text(ui, id, rect.min, &galley, &runs, &inl.links, base, true);
    }

    /// Draws blocks with CSS-style margins between them (the larger of the
    /// two wins), and none before the first or after the last.
    fn blocks(&mut self, ui: &mut Ui, id: Id, blocks: &[Block], tight: bool, nested: bool) {
        let mut gap: Option<f32> = None;
        for (i, b) in blocks.iter().enumerate() {
            let (top, bottom) = match b {
                Block::Heading(..) => (24.0, 16.0),
                Block::Rule => (24.0, 24.0),
                Block::Para(_) if tight => (0.0, 0.0),
                Block::List { .. } if nested => (0.0, 0.0),
                _ => (0.0, 16.0),
            };
            if let Some(g) = gap {
                ui.add_space(f32::max(g, top));
            }
            self.block(ui, id.with(i), b);
            gap = Some(bottom);
        }
    }

    fn block(&mut self, ui: &mut Ui, id: Id, block: &Block) {
        match block {
            Block::Para(inl) => self.text(ui, id, inl, self.base()),
            Block::Heading(level, inl) => self.heading(ui, id, *level, inl),
            Block::Code { lang, text } => self.code(ui, id, lang, text),
            Block::Quote(blocks) => {
                let color = self.p.fg_muted;
                self.quote(ui, id, blocks, self.p.border, color, None);
            }
            Block::Alert(kind, blocks) => {
                let (color, _) = alert_style(*kind, self.p.dark);
                self.quote(ui, id, blocks, color, self.color, Some(*kind));
            }
            Block::List { start, items } => self.list(ui, id, *start, items),
            Block::Table { align, rows } => self.table(ui, id, align, rows),
            Block::Image { url, alt, link } => self.image(ui, id, url, alt, link.as_deref()),
            Block::Rule => {
                let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 4.0));
                ui.painter().rect_filled(rect, 0.0, self.p.border_muted);
            }
            Block::Details { summary, open, body } => self.details(ui, id, summary, *open, body),
            Block::Footnotes(items) => {
                let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 1.0));
                ui.painter().rect_filled(rect, 0.0, self.p.border);
                ui.add_space(16.0);
                let (size, color) = (self.size, self.color);
                self.size = 12.0;
                self.color = self.p.fg_muted;
                self.list(ui, id, Some(1), items);
                (self.size, self.color) = (size, color);
            }
        }
    }

    fn heading(&mut self, ui: &mut Ui, id: Id, level: u8, inl: &Inline) {
        let scale = [2.0, 1.5, 1.25, 1.0, 0.875, 0.85][(level.clamp(1, 6) - 1) as usize];
        let size = self.size * scale;
        let color = if level == 6 { self.p.fg_muted } else { self.color };
        let base = Base { size, bold: true, mono: false, color, line: size * 1.25 };
        self.text(ui, id, inl, base);
        if level <= 2 {
            ui.add_space(0.3 * size);
            let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 1.0));
            ui.painter().rect_filled(rect, 0.0, self.p.border_muted);
        }
    }

    fn code(&mut self, ui: &mut Ui, id: Id, lang: &str, text: &str) {
        let size = self.size * 0.85;
        let base = Base { size, bold: false, mono: true, color: self.p.fg, line: (size * 1.45 * 2.0).round() / 2.0 };
        let mut job = LayoutJob::default();
        let format = |color: Color32, background: Color32| TextFormat {
            font_id: theme::mono(size),
            color,
            background,
            expand_bg: 0.0,
            line_height: Some(base.line),
            ..Default::default()
        };
        if lang == "diff" {
            // Like github.com's diff highlighting: tinted added/removed lines.
            let p = self.p;
            for line in text.split_inclusive('\n') {
                let f = match line.as_bytes().first() {
                    Some(b'+') => format(p.open, p.diff_add),
                    Some(b'-') => format(p.closed, p.diff_del),
                    Some(b'@') => format(p.merged, Color32::TRANSPARENT),
                    _ => format(p.fg, Color32::TRANSPARENT),
                };
                job.append(line, 0.0, f);
            }
        } else if let Some(lines) = crate::syntax::lang_for_fence(lang).and_then(|l| crate::syntax::highlight(&l, text)) {
            // Tree-sitter colors, line by line.
            let p = self.p;
            for (n, line) in text.split('\n').enumerate() {
                if n > 0 {
                    job.append("\n", 0.0, format(p.fg, Color32::TRANSPARENT));
                }
                let mut at = 0;
                for (r, tok) in lines.get(n).map(Vec::as_slice).unwrap_or(&[]) {
                    let (a, b) = (r.start.min(line.len()), r.end.min(line.len()));
                    if a > at && line.is_char_boundary(at) && line.is_char_boundary(a) {
                        job.append(&line[at..a], 0.0, format(p.fg, Color32::TRANSPARENT));
                    }
                    if b > a && line.is_char_boundary(a) && line.is_char_boundary(b) {
                        job.append(&line[a..b], 0.0, format(crate::syntax::color(*tok, p), Color32::TRANSPARENT));
                        at = b;
                    }
                }
                if at < line.len() {
                    job.append(&line[at..], 0.0, format(p.fg, Color32::TRANSPARENT));
                }
            }
        } else {
            job.append(text, 0.0, format(self.p.fg, Color32::TRANSPARENT));
        }
        let galley = ui.painter().layout_job(job);
        // The frame's padding comes out of the width we were given.
        let inner_w = (ui.available_width() - 32.0).max(40.0);
        egui::Frame::new().fill(self.p.canvas_subtle).corner_radius(6.0).inner_margin(16.0).show(ui, |ui| {
            ui.set_width(inner_w);
            solid_bar(ui);
            egui::ScrollArea::horizontal().id_salt(id).auto_shrink([false, true]).max_width(inner_w).show(ui, |ui| {
                let (_, rect) = ui.allocate_space(galley.size());
                self.paint_text(ui, id.with("text"), rect.min, &galley, &Runs::default(), &[], base, true);
            });
        });
    }

    /// Blockquotes and alerts: a colored bar on the left, padded content.
    fn quote(&mut self, ui: &mut Ui, id: Id, blocks: &[Block], bar: Color32, color: Color32, alert: Option<Alert>) {
        let top = ui.cursor().top();
        let mut rect = ui.available_rect_before_wrap();
        let left = rect.left();
        rect.min.x += 4.0 + 16.0;
        rect.max.x -= 16.0;
        let saved = self.color;
        self.color = color;
        let inner = ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            if let Some(kind) = alert {
                ui.add_space(8.0);
                self.alert_title(ui, kind);
                ui.add_space(16.0);
            }
            self.blocks(ui, id, blocks, false, false);
            if alert.is_some() {
                ui.add_space(8.0);
            }
        });
        self.color = saved;
        let bottom = inner.response.rect.bottom();
        ui.painter().rect_filled(Rect::from_min_max(pos2(left, top), pos2(left + 4.0, bottom)), 0.0, bar);
    }

    fn alert_title(&mut self, ui: &mut Ui, kind: Alert) {
        let (color, name) = alert_style(kind, self.p.dark);
        let (_, rect) = ui.allocate_space(vec2(ui.available_width(), 16.0));
        let icon = Rect::from_min_size(rect.min, Vec2::splat(16.0));
        let painter = ui.painter();
        let stroke = Stroke::new(1.5, color);
        let c = icon.center();
        let at = |x: f32, y: f32| pos2(icon.left() + x, icon.top() + y);
        // Small line icons in the spirit of Octicons.
        match kind {
            Alert::Note => {
                painter.circle_stroke(c, 6.75, stroke);
                painter.circle_filled(at(8.0, 5.0), 1.0, color);
                painter.line_segment([at(8.0, 7.5), at(8.0, 11.5)], stroke);
            }
            Alert::Tip => {
                painter.circle_stroke(at(8.0, 6.0), 4.75, stroke);
                painter.line_segment([at(6.0, 12.0), at(10.0, 12.0)], stroke);
                painter.line_segment([at(6.5, 14.5), at(9.5, 14.5)], stroke);
            }
            Alert::Important => {
                let r = Rect::from_min_max(at(1.25, 1.75), at(14.75, 11.75));
                painter.rect_stroke(r, 2.0, stroke, egui::StrokeKind::Middle);
                painter.line_segment([at(4.5, 11.75), at(4.5, 14.75)], stroke);
                painter.line_segment([at(4.5, 14.75), at(7.5, 11.75)], stroke);
                painter.line_segment([at(8.0, 4.0), at(8.0, 7.25)], stroke);
                painter.circle_filled(at(8.0, 9.5), 1.0, color);
            }
            Alert::Warning => {
                painter.add(Shape::closed_line(vec![at(8.0, 1.5), at(15.0, 14.25), at(1.0, 14.25)], stroke));
                painter.line_segment([at(8.0, 6.0), at(8.0, 9.5)], stroke);
                painter.circle_filled(at(8.0, 11.75), 1.0, color);
            }
            Alert::Caution => {
                let pts = (0..8)
                    .map(|i| {
                        let a = std::f32::consts::PI / 8.0 + i as f32 * std::f32::consts::PI / 4.0;
                        c + vec2(a.cos(), a.sin()) * 7.0
                    })
                    .collect();
                painter.add(Shape::closed_line(pts, stroke));
                painter.line_segment([at(8.0, 4.25), at(8.0, 8.5)], stroke);
                painter.circle_filled(at(8.0, 11.0), 1.0, color);
            }
        }
        let text = painter.layout_no_wrap(name.into(), theme::bold(self.size), color);
        painter.galley(pos2(icon.right() + 8.0, c.y - text.size().y / 2.0), text, color);
    }

    fn list(&mut self, ui: &mut Ui, id: Id, start: Option<u64>, items: &[Item]) {
        let depth = self.depth;
        self.depth += 1;
        let loose = items.iter().any(|i| i.loose);
        let indent = 2.0 * self.size;
        let base = self.base();
        let (asc, height) = self.metric(ui, false, false, base.size);
        for (n, item) in items.iter().enumerate() {
            if n > 0 {
                ui.add_space(if loose { 16.0 } else { 4.0 });
            }
            let top = ui.cursor().top();
            let mut rect = ui.available_rect_before_wrap();
            let content = rect.left() + indent;
            rect.min.x = content;
            let outer = self.first.take();
            ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                if item.blocks.is_empty() {
                    ui.add_space(base.line);
                }
                self.blocks(ui, id.with(n), &item.blocks, !loose, true);
            });
            // Line markers up with the item's first line of text.
            let baseline = self.first.take().unwrap_or(top + (base.line - height) / 2.0 + asc);
            self.first = outer.or(Some(baseline));
            // Middle of the lowercase letters.
            let mid = baseline - 0.26 * base.size;
            let s = base.size / 14.0;
            let painter = ui.painter();
            if let Some(done) = item.task {
                let r = Rect::from_center_size(pos2(content - 1.4 * base.size + 7.0, mid - 1.0), Vec2::splat(14.0));
                if done {
                    painter.rect_filled(r, 3.0, self.p.accent);
                    let at = |x: f32, y: f32| pos2(r.left() + x, r.top() + y);
                    painter.add(Shape::line(vec![at(3.5, 7.25), at(6.0, 9.75), at(10.75, 4.75)], Stroke::new(1.75, Color32::WHITE)));
                } else {
                    painter.rect_filled(r, 3.0, self.p.canvas);
                    painter.rect_stroke(r, 3.0, Stroke::new(1.0, self.p.fg_muted), egui::StrokeKind::Inside);
                }
            } else if let Some(first) = start {
                let label = list_number(first + n as u64, depth);
                let g = painter.layout_no_wrap(label, theme::body(base.size), base.color);
                painter.galley(pos2(content - 0.3 * base.size - g.size().x, baseline - asc), g, base.color);
            } else {
                let c = pos2(content - 0.75 * base.size, mid);
                match depth {
                    0 => painter.circle_filled(c, 2.75 * s, base.color),
                    1 => painter.circle_stroke(c, 2.5 * s, Stroke::new(1.0, base.color)),
                    _ => painter.rect_filled(Rect::from_center_size(c, Vec2::splat(5.0 * s)), 0.0, base.color),
                };
            }
        }
        self.depth -= 1;
    }

    fn table(&mut self, ui: &mut Ui, id: Id, align: &[Alignment], rows: &[Vec<Inline>]) {
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let (pad_x, pad_y) = (13.0, 6.0);
        let plain = self.base();
        let base = move |header: bool| Base { bold: header, ..plain };
        let empty = Inline::default();
        let cell = |r: usize, c: usize| rows[r].get(c).unwrap_or(&empty);

        // Each column's natural width, and its widest word (it can't get
        // narrower than that), like a browser's automatic table layout.
        let (mut max_w, mut min_w) = (vec![0.0f32; cols], vec![0.0f32; cols]);
        for r in 0..rows.len() {
            for c in 0..cols {
                let (g, _) = self.layout(ui, cell(r, c), base(r == 0), f32::INFINITY, Align::LEFT);
                max_w[c] = max_w[c].max(g.size().x);
                for row in &g.rows {
                    let mut word_start = None;
                    for gl in &row.glyphs {
                        if gl.chr.is_whitespace() {
                            word_start = None;
                        } else {
                            let s = *word_start.get_or_insert(gl.pos.x);
                            min_w[c] = min_w[c].max(gl.pos.x + gl.advance_width - s);
                        }
                    }
                }
            }
        }
        let avail = ui.available_width() - cols as f32 * 2.0 * pad_x - 1.0;
        // Short columns ("✔️ Pass") keep their whole width when squeezed;
        // only the long ones wrap, like in a browser.
        let short: Vec<bool> = if max_w.iter().sum::<f32>() <= avail { vec![false; cols] } else { max_w.iter().map(|&w| w <= avail / cols as f32).collect() };
        let fixed: f32 = (0..cols).filter(|&c| short[c]).map(|c| max_w[c]).sum();
        let (sum_min, sum_max) = (0..cols).filter(|&c| !short[c]).fold((0.0, 0.0), |(a, b), c| (a + min_w[c], b + max_w[c]));
        let room = avail - fixed;
        let t = if sum_max <= room {
            1.0
        } else if sum_max > sum_min {
            ((room - sum_min) / (sum_max - sum_min)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        // Rounded down when squeezed, so the table never ends a few px too wide.
        let round = |w: f32| if t < 1.0 { w.floor() } else { w.ceil() };
        let widths: Vec<f32> = (0..cols).map(|c| if short[c] { max_w[c].ceil() } else { round(min_w[c] + (max_w[c] - min_w[c]) * t) } + 2.0 * pad_x).collect();

        // Lay out every cell at its column width.
        let mut cells = Vec::with_capacity(rows.len());
        let mut heights = Vec::with_capacity(rows.len());
        for r in 0..rows.len() {
            let mut row = Vec::with_capacity(cols);
            let mut h: f32 = base(r == 0).line;
            for (c, w) in widths.iter().enumerate() {
                let halign = match align.get(c) {
                    Some(Alignment::Center) => Align::Center,
                    Some(Alignment::Right) => Align::RIGHT,
                    _ => Align::LEFT,
                };
                let laid = self.layout(ui, cell(r, c), base(r == 0), w - 2.0 * pad_x, halign);
                h = h.max(laid.0.size().y);
                row.push((laid, halign));
            }
            heights.push(h + 2.0 * pad_y);
            cells.push(row);
        }

        let total = vec2(widths.iter().sum::<f32>() + 1.0, heights.iter().sum::<f32>() + 1.0);
        let draw = |ui: &mut Ui| {
            let (_, rect) = ui.allocate_space(total);
            let painter = ui.painter().clone();
            let stroke = Stroke::new(1.0, self.p.border);
            let mut y = rect.top();
            for (r, row) in cells.into_iter().enumerate() {
                let mut x = rect.left();
                let h = heights[r];
                if r > 0 && r % 2 == 0 {
                    let band = Rect::from_min_size(pos2(x, y), vec2(total.x - 1.0, h));
                    painter.rect_filled(band, 0.0, self.p.canvas_subtle);
                }
                for (c, ((galley, runs), halign)) in row.into_iter().enumerate() {
                    let w = widths[c];
                    let cell_rect = Rect::from_min_size(pos2(x, y), vec2(w, h));
                    painter.rect_stroke(cell_rect.translate(vec2(0.5, 0.5)), 0.0, stroke, egui::StrokeKind::Middle);
                    let ax = match halign {
                        Align::Center => cell_rect.center().x,
                        Align::RIGHT => cell_rect.right() - pad_x,
                        _ => cell_rect.left() + pad_x,
                    };
                    // Middle, like a browser's table cell.
                    let pos = pos2(ax, y + ((h - galley.size().y) / 2.0).max(pad_y));
                    self.paint_text(ui, id.with((r, c)), pos, &galley, &runs, &cell(r, c).links, base(r == 0), true);
                    x += w;
                }
                y += h;
            }
        };
        if total.x > ui.available_width() {
            let w = ui.available_width();
            ui.scope(|ui| {
                solid_bar(ui);
                egui::ScrollArea::horizontal().id_salt(id).auto_shrink([false, true]).max_width(w).show(ui, draw);
            });
        } else {
            draw(ui);
        }
    }

    fn image(&mut self, ui: &mut Ui, id: Id, url: &str, alt: &str, link: Option<&str>) {
        let target = link.unwrap_or(url);
        let avail = ui.available_width();
        let img = egui::Image::new(url);
        match img.load_for_size(ui.ctx(), vec2(avail, f32::INFINITY)) {
            Ok(TexturePoll::Ready { texture }) => {
                let mut size = texture.size;
                if size.x > avail {
                    size *= avail / size.x;
                }
                let resp = ui.add(egui::Image::from_texture(texture).fit_to_exact_size(size).sense(Sense::click()));
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    self.clicked = Some(target.to_string());
                }
            }
            Ok(TexturePoll::Pending { .. }) => {
                ui.add(egui::Spinner::new().size(16.0).color(self.p.fg_muted));
            }
            Err(_) => {
                // Private attachments need a browser session: link to them.
                let name = if alt.trim().is_empty() { url.rsplit('/').next().unwrap_or(url) } else { alt };
                let inl = Inline { spans: vec![Span::Text(name.to_string(), Style { link: Some(0), ..Style::default() })], links: vec![target.to_string()] };
                self.text(ui, id, &inl, self.base());
            }
        }
    }

    fn details(&mut self, ui: &mut Ui, id: Id, summary: &Inline, open_default: bool, body: &[Block]) {
        let key = id.with("open");
        let mut open = ui.ctx().data(|d| d.get_temp::<bool>(key)).unwrap_or(open_default);
        let base = self.base();
        let marker = base.size + 2.0;
        let (galley, runs) = self.layout(ui, summary, base, ui.available_width() - marker, Align::LEFT);
        let (_, rect) = ui.allocate_space(vec2(ui.available_width(), galley.size().y.max(base.line)));
        let resp = ui.interact(rect, id.with("summary"), Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if resp.clicked() {
            open = !open;
            ui.ctx().data_mut(|d| d.insert_temp(key, open));
        }
        let had_first = self.first.is_some();
        self.paint_text(ui, id, rect.min + vec2(marker, 0.0), &galley, &runs, &summary.links, base, false);
        let (asc, height) = self.metric(ui, false, false, base.size);
        let mid = rect.top() + (base.line - height) / 2.0 + asc - 0.3 * base.size;
        let x = rect.left() + 1.0;
        let s = base.size / 14.0;
        let pts = if open {
            vec![pos2(x, mid - 2.5 * s), pos2(x + 8.0 * s, mid - 2.5 * s), pos2(x + 4.0 * s, mid + 2.5 * s)]
        } else {
            vec![pos2(x + 1.5 * s, mid - 4.0 * s), pos2(x + 6.5 * s, mid), pos2(x + 1.5 * s, mid + 4.0 * s)]
        };
        ui.painter().add(Shape::convex_polygon(pts, self.color, Stroke::NONE));
        if !had_first && self.first.is_none() {
            self.first = Some(rect.top() + asc);
        }
        if open && !body.is_empty() {
            ui.add_space(4.0);
            self.blocks(ui, id.with("body"), body, false, false);
        }
    }
}

/// "3." at the top level, "iii." one list down, "c." deeper, like github.com.
fn list_number(n: u64, depth: usize) -> String {
    match depth {
        0 => format!("{n}."),
        1 => format!("{}.", roman(n)),
        _ => {
            let mut s = String::new();
            let mut n = n;
            while n > 0 {
                n -= 1;
                s.insert(0, (b'a' + (n % 26) as u8) as char);
                n /= 26;
            }
            format!("{s}.")
        }
    }
}

fn roman(mut n: u64) -> String {
    const TABLE: [(u64, &str); 13] =
        [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
    let mut s = String::new();
    for (v, r) in TABLE {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// A scrollbar that stays put under content that overflows sideways, so
/// it's clear there's more (egui's default fades out when idle).
pub fn solid_bar(ui: &mut Ui) {
    let mut scroll = egui::style::ScrollStyle::solid();
    scroll.bar_width = 6.0;
    scroll.bar_inner_margin = 4.0;
    scroll.fade.strength = 0.0;
    ui.spacing_mut().scroll = scroll;
    // The handle needs to stand out from its track in light mode too.
    let handle = if ui.visuals().dark_mode { egui::Color32::from_rgb(0x3d, 0x44, 0x4d) } else { egui::Color32::from_rgb(0xd1, 0xd9, 0xe0) };
    ui.visuals_mut().widgets.inactive.bg_fill = handle;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para(b: &Block) -> &Inline {
        match b {
            Block::Para(i) => i,
            other => panic!("not a paragraph: {other:?}"),
        }
    }

    #[test]
    fn inline_styles() {
        let doc = parse("a **b** *c* ~~d~~ `e` [f](https://x)\nline  \nnext");
        assert_eq!(doc.len(), 1);
        let p = para(&doc[0]);
        assert_eq!(p.text(), "a b c d e f\nline\nnext");
        let style = |want: &str| {
            p.spans.iter().find_map(|s| match s {
                Span::Text(t, st) if t == want => Some(*st),
                _ => None,
            })
        };
        assert!(style("b").unwrap().bold);
        assert!(style("c").unwrap().italic);
        assert!(style("d").unwrap().strike);
        assert!(style("e").unwrap().code);
        assert_eq!(style("f").unwrap().link, Some(0));
        assert_eq!(p.links, vec!["https://x".to_string()]);
    }

    #[test]
    fn blocks_and_margins() {
        let doc = parse("# Title\n\ntext\n\n---\n\n```rust\nfn x() {}\n```\n\n> quote\n\n> [!WARNING]\n> careful");
        assert!(matches!(&doc[0], Block::Heading(1, i) if i.text() == "Title"));
        assert!(matches!(&doc[2], Block::Rule));
        assert_eq!(doc[3], Block::Code { lang: "rust".into(), text: "fn x() {}".into() });
        assert!(matches!(&doc[4], Block::Quote(b) if para(&b[0]).text() == "quote"));
        assert!(matches!(&doc[5], Block::Alert(Alert::Warning, b) if para(&b[0]).text() == "careful"));
    }

    #[test]
    fn lists_and_tasks() {
        let doc = parse("- [x] done\n- [ ] todo\n  - nested\n\n3. three\n4. four");
        let Block::List { start: None, items } = &doc[0] else { panic!("{doc:?}") };
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].task, Some(true));
        assert_eq!(items[1].task, Some(false));
        assert!(!items[0].loose);
        assert_eq!(para(&items[1].blocks[0]).text(), "todo");
        assert!(matches!(&items[1].blocks[1], Block::List { items, .. } if items.len() == 1));
        assert!(matches!(&doc[1], Block::List { start: Some(3), items } if items.len() == 2));

        let loose = parse("- a\n\n- b");
        assert!(matches!(&loose[0], Block::List { items, .. } if items[0].loose));
        assert_eq!(list_number(3, 1), "iii.");
        assert_eq!(list_number(28, 2), "ab.");
    }

    #[test]
    fn tables() {
        let doc = parse("| a | b |\n|:--|--:|\n| `1` | [2](u) |\n| 3 | ![x](https://i/p.png) |");
        let Block::Table { align, rows } = &doc[0] else { panic!("{doc:?}") };
        assert_eq!(align, &vec![Alignment::Left, Alignment::Right]);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0][1].text(), "b");
        assert_eq!(rows[1][1].links, vec!["u".to_string()]);
        // Pictures in cells become links.
        assert_eq!(rows[2][1].text(), "x");
        assert_eq!(rows[2][1].links, vec!["https://i/p.png".to_string()]);
    }

    #[test]
    fn images_get_their_own_line() {
        let doc = parse("before ![pic](https://a/b.png) after :) ![e](emoji:https://e/1.png)");
        assert_eq!(para(&doc[0]).text(), "before");
        assert!(matches!(&doc[1], Block::Image { url, alt, link: None } if url == "https://a/b.png" && alt == "pic"));
        assert_eq!(para(&doc[2]).text(), "after :) <emoji>");
        let linked = parse("[![b](https://a/b.png)](https://site)");
        assert!(matches!(&linked[0], Block::Image { link: Some(l), .. } if l == "https://site"));
        assert_eq!(linked.len(), 1);
    }

    #[test]
    fn details_fold() {
        let md = crate::util::clean_markdown("<details><summary>More **info**</summary>\nhidden text\n</details>\nafter");
        let doc = parse(&md);
        let Block::Details { summary, open, body } = &doc[0] else { panic!("{md:?} -> {doc:?}") };
        assert_eq!(summary.text(), "More info");
        assert!(!open);
        assert_eq!(para(&body[0]).text(), "hidden text");
        assert_eq!(para(&doc[1]).text(), "after");

        let open = parse("<details open>\n\n<summary>S</summary>\n\nbody\n\n</details>");
        assert!(matches!(&open[0], Block::Details { open: true, body, .. } if body.len() == 1));
        // Unclosed: still folds what follows.
        let unclosed = parse("<details>\n<summary>S</summary>\n\nbody");
        assert!(matches!(&unclosed[0], Block::Details { body, .. } if body.len() == 1));
    }

    #[test]
    fn footnotes_and_leftover_html() {
        let doc = parse("See[^1].\n\n[^1]: The note.\n\n<Foo>");
        assert_eq!(para(&doc[0]).text(), "See[1].");
        assert!(matches!(doc.last(), Some(Block::Footnotes(items)) if items.len() == 1));
        assert!(doc.iter().any(|b| matches!(b, Block::Para(i) if i.text() == "<Foo>")));
    }

    #[test]
    fn parse_is_cached() {
        let a = parsed("cached *text*");
        let b = parsed("cached *text*");
        assert!(Arc::ptr_eq(&a, &b));
    }
}
