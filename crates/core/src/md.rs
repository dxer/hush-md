//! Markdown ↔ document tree.
//!
//! Reading records each block's byte range and the bytes it was read from;
//! writing emits those bytes back while the block's canonical form still
//! fingerprints the same. Only an edited block is serialized anew, so front
//! matter, HTML, unknown syntax and deliberate whitespace survive a load →
//! save round trip.
//!
//! Blocks: front matter, ATX and setext headings, paragraphs, bullet /
//! ordered / task lists (nested), block quotes, fenced and indented code,
//! thematic breaks, HTML blocks, pipe tables. Inline: emphasis, strong,
//! strikethrough, inline code, links, images, autolinks, escapes, hard
//! breaks, inline math (`$…$`) and footnotes-as-text.

use crate::model::*;
use crate::source::{FenceStyle, Fingerprint, Marker, MdKind, Origin, Source};
use std::hash::{Hash, Hasher};

/// Parse a Markdown file into blocks.
pub fn parse(src: &str) -> Vec<Block> {
    Parser { src, starts: line_starts(src) }.blocks(0, line_count(src), 0).0
}

/// Read a file: the blocks plus the leading bytes that belong to no block.
pub fn read(src: &str) -> Document {
    let blocks = parse(src);
    let front = match blocks.first().and_then(|b| b.origin.as_ref()).and_then(|o| o.source.as_ref()) {
        Some(s) => src[..s.range.start].to_string(),
        // Only blank lines, or an empty file: everything is front matter.
        None => src.to_string(),
    };
    Document { blocks, source: Some(crate::source::DocSource { front }), ..Default::default() }
}

/// Write a document back as Markdown.
pub fn write(doc: &Document) -> String {
    let mut out = String::new();
    if let Some(s) = &doc.source { out.push_str(&s.front); }
    let n = doc.blocks.len();
    for (i, b) in doc.blocks.iter().enumerate() {
        write_block(b, &mut out);
        match b.origin.as_ref().and_then(|o| o.source.as_ref()) {
            // Keep the blank space the file had after this block.
            Some(s) => out.push_str(&s.gap),
            // A block made in the editor: separate it from the next one.
            None if i + 1 < n => {
                if !out.ends_with('\n') { out.push('\n'); }
                out.push('\n');
            }
            None => {}
        }
    }
    out
}

/// Inline Markdown for a string: the editor's source view uses it to show a
/// paragraph's marks.
pub fn text(src: &str) -> RichText {
    let mut rt = RichText::default();
    inline(src, Marks::default(), &mut rt);
    rt
}

/// A document's title: its first top-level heading, or its first text, or
/// empty. This is what the sidebar, the breadcrumb and a rename start from.
pub fn title_of(doc: &Document) -> String {
    for b in &doc.blocks {
        if let BlockKind::Heading { level: 1, .. } = b.kind {
            let t = b.text.text.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    for b in &doc.blocks {
        if let BlockKind::Heading { .. } = b.kind {
            let t = b.text.text.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    doc.blocks.iter().find(|b| !b.text.text.trim().is_empty()).map(|b| b.text.text.trim().to_string()).unwrap_or_default()
}

/// The source a document needs before anything is typed into it.
pub fn empty() -> Document {
    Document { source: Some(crate::source::DocSource::default()), ..Default::default() }
}

fn is_pristine(b: &Block) -> bool {
    b.origin.as_ref().is_some_and(|o| !o.dirty && o.source.as_ref().is_some_and(|s| s.fingerprint == fingerprint(b)))
}

fn write_block(b: &Block, out: &mut String) {
    if is_pristine(b)
        && let Some(s) = &b.origin.as_ref().and_then(|o| o.source.as_ref())
    {
        out.push_str(&s.text);
        return;
    }
    serialize(b, out, 0);
}

/// Hash what the block *is*: its kind, text and children. Source bytes and
/// formatting choices (fence style, list marker) are deliberately excluded —
/// they describe how the block is written, not what it contains, and the
/// writer keeps them verbatim anyway.
pub fn fingerprint(b: &Block) -> Fingerprint {
    let mut s = String::new();
    crate::source::model_fingerprint(b, &mut s);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    b.children.len().hash(&mut h);
    for c in &b.children {
        let f = fingerprint(c);
        f.hash.hash(&mut h);
        f.len.hash(&mut h);
    }
    Fingerprint { hash: h.finish(), len: s.len() }
}

// ---- reading

fn line_starts(src: &str) -> Vec<usize> {
    let mut v = vec![0];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' && i + 1 < src.len() {
            v.push(i + 1);
        }
    }
    v
}

fn line_count(src: &str) -> usize {
    if src.is_empty() {
        0
    } else if src.ends_with('\n') {
        src.matches('\n').count()
    } else {
        src.matches('\n').count() + 1
    }
}

/// One line: its indentation in columns and its content.
#[derive(Clone, Copy)]
struct Line<'a> {
    indent: usize,
    text: &'a str,
}

struct Parser<'a> {
    src: &'a str,
    starts: Vec<usize>,
}

impl<'a> Parser<'a> {
    fn line(&self, i: usize) -> Line<'a> {
        let start = self.starts[i];
        let end = self.line_end(i);
        let raw = &self.src[start..end];
        let mut col = 0;
        for (k, c) in raw.char_indices() {
            match c {
                ' ' => col += 1,
                '\t' => col += 4 - col % 4,
                _ => return Line { indent: col, text: &raw[k..] },
            }
        }
        Line { indent: col, text: "" }
    }

    fn line_end(&self, i: usize) -> usize {
        let mut end = if i + 1 < self.starts.len() { self.starts[i + 1] - 1 } else { self.src.len() };
        if end > self.starts[i] && self.src.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        end
    }

    /// Bytes of lines `[a, b)`, newlines included.
    fn span(&self, a: usize, b: usize) -> std::ops::Range<usize> {
        if a >= self.starts.len() { return self.src.len()..self.src.len(); }
        let start = self.starts[a];
        let end = if b < self.starts.len() { self.starts[b] } else { self.src.len() };
        start..end
    }

    /// Bytes of lines `[a, b)`, empty when `a` is past the end.
    fn span_opt(&self, a: usize, b: usize) -> std::ops::Range<usize> {
        if a >= self.starts.len() { return self.src.len()..self.src.len(); }
        self.span(a, b)
    }

    fn blank(&self, i: usize) -> bool {
        self.line(i).text.trim().is_empty()
    }

    /// Parse `[from, to)` as a sequence of blocks at `base` columns of
    /// indentation. Returns the blocks and where parsing stopped.
    fn blocks(&mut self, from: usize, to: usize, base: usize) -> (Vec<Block>, usize) {
        let mut out = vec![];
        let mut i = from;
        while i < to {
            if self.blank(i) { i += 1; continue; }
            if self.line(i).indent < base { break; }
            let (mut blocks, next) = self.one(i, to, base);
            out.append(&mut blocks);
            if next <= i { break; }
            i = next;
        }
        (out, i)
    }

    /// One block at `i`; may expand to several blocks (a list is its items).
    fn one(&mut self, i: usize, to: usize, base: usize) -> (Vec<Block>, usize) {
        // Front matter only counts at the very top of the file.
        if i == 0 && let Some(end) = self.front_matter_end(to) {
            return (vec![self.verbatim(0, end, MdKind::Raw, "front-matter")], end);
        }
        let l = self.line(i);
        let t = l.text;
        if let Some((level, text)) = atx(t) {
            let mut rt = RichText::default();
            inline(text, Marks::default(), &mut rt);
            let b = Block::new(BlockKind::Heading { level, seq_auto: false }, rt);
            return (vec![self.finish(b, i, i + 1, MdKind::Heading)], i + 1);
        }
        if i + 1 < to && let Some(level) = setext_level(self.line(i + 1).text) {
            let mut rt = RichText::default();
            inline(t.trim_end(), Marks::default(), &mut rt);
            let b = Block::new(BlockKind::Heading { level, seq_auto: false }, rt);
            return (vec![self.finish(b, i, i + 2, MdKind::Heading)], i + 2);
        }
        if is_thematic(t) {
            let b = Block::new(BlockKind::Divider, RichText::default());
            return (vec![self.finish(b, i, i + 1, MdKind::Divider)], i + 1);
        }
        if let Some(f) = fence_open(t) {
            let end = self.fence_end(i, to, &f);
            let closed = end > i + 1 && end - 1 < to && fence_close(self.line(end - 1).text, &f);
            let body = self.fence_body(i, end, closed);
            let lang = f.info.split_whitespace().next().unwrap_or("").to_string();
            let mut rt = RichText::default();
            for (k, line) in body.split('\n').enumerate() {
                if k > 0 { rt.push("\n", Marks::default()); }
                if !line.is_empty() { rt.push(line, Marks::default()); }
            }
            let b = Block::new(BlockKind::Code { lang, caption: None, wrap: false }, rt);
            let style = FenceStyle { ch: f.ch, len: f.chars.chars().count() };
            return (vec![self.finish(b, i, end, MdKind::Fenced { fence: style })], end);
        }
        if let Some(end) = self.html_end(i, to) {
            return (vec![self.verbatim(i, end, MdKind::Html, "html")], end);
        }
        if l.indent >= base + 4 {
            return self.indented(i, to, base);
        }
        if let Some((b, end)) = self.table(i, to) {
            return (vec![self.finish(b, i, end, MdKind::Table)], end);
        }
        if t.starts_with('>') {
            return self.quote(i, to);
        }
        if list_marker(t).is_some() {
            return self.list(i, to);
        }
        if let Some((b, end)) = self.image(i, to) {
            return (vec![self.finish(b, i, end, MdKind::Paragraph)], end);
        }
        let (b, end) = self.paragraph(i, to);
        (vec![self.finish(b, i, end, MdKind::Paragraph)], end)
    }

    fn front_matter_end(&self, to: usize) -> Option<usize> {
        if self.line(0).text.trim_end() != "---" { return None; }
        (1..to).find(|&j| matches!(self.line(j).text.trim_end(), "---" | "...")).map(|j| j + 1)
    }

    /// A block kept byte for byte.
    fn verbatim(&self, a: usize, z: usize, md: MdKind, tag: &str) -> Block {
        let (sp, text, gap) = self.split(a, z);
        let b = Block::new(BlockKind::Raw { tag: tag.into(), xml: String::new() }, RichText::default());
        Block { origin: Some(Origin::clean(Source { range: sp, text, gap, fingerprint: Fingerprint::default() }, md)), ..b }
    }

    /// Attach the block's source: its own bytes plus the blank lines that
    /// follow, held separately so an edit keeps the spacing.
    fn finish(&self, mut b: Block, a: usize, z: usize, md: MdKind) -> Block {
        let (sp, text, gap) = self.split(a, z);
        let fp = fingerprint(&b);
        b.origin = Some(Origin::clean(Source { range: sp, text, gap, fingerprint: fp }, md));
        b
    }

    /// A block's own bytes `[a, z)` and the blank lines that follow it.
    fn split(&self, a: usize, z: usize) -> (std::ops::Range<usize>, String, String) {
        let mut end = z;
        while end < self.starts.len() && self.blank(end) { end += 1; }
        let sp = self.span(a, z);
        let text = self.src[sp.clone()].to_string();
        let gap = self.src[self.span_opt(z, end)].to_string();
        (sp, text, gap)
    }

    fn fence_end(&self, i: usize, to: usize, f: &Fence) -> usize {
        let mut j = i + 1;
        while j < to {
            if fence_close(self.line(j).text, f) { return j + 1; }
            j += 1;
        }
        j
    }

    /// Bytes between the opening fence and the closing fence (or the end).
    fn fence_body(&self, i: usize, end: usize, closed: bool) -> &'a str {
        let start = if i + 1 < self.starts.len() { self.starts[i + 1] } else { self.src.len() };
        // `end` is past the closing fence; the fence's own line is not body.
        let last = if closed && end > i + 1 && end - 1 < self.starts.len() { self.starts[end - 1] } else { self.src.len() };
        self.src[start.min(last)..last].trim_end_matches(['\n', '\r'])
    }

    fn indented(&mut self, i: usize, to: usize, base: usize) -> (Vec<Block>, usize) {
        let mut j = i;
        let mut body = String::new();
        while j < to {
            if self.blank(j) {
                let mut k = j + 1;
                while k < to && self.blank(k) { k += 1; }
                if k < to && self.line(k).indent >= base + 4 {
                    body.push('\n');
                    j += 1;
                    continue;
                }
                break;
            }
            if self.line(j).indent < base + 4 { break; }
            body.push_str(strip_cols(&self.src[self.starts[j]..self.line_end(j)], base + 4));
            body.push('\n');
            j += 1;
        }
        let rt = RichText::plain(body.trim_end_matches('\n'));
        let b = Block::new(BlockKind::Code { lang: String::new(), caption: None, wrap: false }, rt);
        (vec![self.finish(b, i, j, MdKind::Indented)], j)
    }

    fn html_end(&self, i: usize, to: usize) -> Option<usize> {
        let t = self.line(i).text.trim_start();
        let tag: String = t.strip_prefix('<')?.chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        const TAGS: &[&str] = &[
            "address", "article", "aside", "base", "blockquote", "body", "caption", "center", "col", "colgroup", "dd", "details",
            "dialog", "dir", "div", "dl", "dt", "fieldset", "figcaption", "figure", "footer", "form", "h1", "h2", "h3", "h4", "h5",
            "h6", "head", "header", "hr", "html", "iframe", "legend", "li", "link", "main", "menu", "nav", "noframes", "ol",
            "optgroup", "option", "p", "param", "section", "source", "summary", "table", "tbody", "td", "tfoot", "th", "thead",
            "title", "tr", "track", "ul", "pre", "script", "style", "video", "audio", "canvas", "svg",
        ];
        if !TAGS.contains(&tag.as_str()) { return None; }
        let mut end = i + 1;
        if matches!(tag.as_str(), "script" | "style" | "pre") {
            let close = format!("</{tag}>");
            while end < to && !self.line(end).text.to_ascii_lowercase().contains(&close) { end += 1; }
            end = (end + 1).min(to);
        } else {
            while end < to && !self.blank(end) { end += 1; }
        }
        Some(end)
    }

    fn quote(&mut self, i: usize, to: usize) -> (Vec<Block>, usize) {
        let mut j = i;
        let mut inner = String::new();
        while j < to {
            let t = self.line(j).text;
            if let Some(rest) = t.strip_prefix('>') {
                if !inner.is_empty() { inner.push('\n'); }
                inner.push_str(rest.strip_prefix(' ').unwrap_or(rest));
                j += 1;
            } else if t.trim().is_empty() {
                let mut k = j + 1;
                while k < to && self.blank(k) { k += 1; }
                if k < to && self.line(k).text.starts_with('>') {
                    inner.push('\n');
                    j += 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        let children = parse(&inner);
        let b = Block::new(BlockKind::Quote, RichText::default()).with_children(children);
        (vec![self.finish(b, i, j, MdKind::Quote)], j)
    }

    /// A list is a run of sibling blocks (one per item), each holding its
    /// indented continuation as children.
    fn list(&mut self, i: usize, to: usize) -> (Vec<Block>, usize) {
        let mut items = vec![];
        let mut extents: Vec<(usize, usize)> = vec![];
        let mut j = i;
        let first = list_marker(self.line(i).text).unwrap();
        let ordered = first.ordered;
        while j < to {
            if self.blank(j) {
                let mut k = j + 1;
                while k < to && self.blank(k) { k += 1; }
                if k < to && let Some(m) = list_marker(self.line(k).text) && m.ordered == ordered && self.line(k).indent <= self.line(i).indent + 1 {
                    j = k;
                    continue;
                }
                break;
            }
            let l = self.line(j);
            let Some(m) = list_marker(l.text) else { break };
            if m.ordered != ordered || l.indent > self.line(i).indent + 1 { break; }
            let mut rt = RichText::default();
            let content = m.task.map(|_| &l.text[m.content_at..]).unwrap_or(&l.text[m.content_at..]);
            inline(content.trim_end(), Marks::default(), &mut rt);
            let kind = match (ordered, m.task) {
                (_, Some(done)) => BlockKind::Todo { done },
                (true, None) => BlockKind::Ordered { seq: m.seq },
                (false, None) => BlockKind::Bullet,
            };
            let mut item = Block::new(kind, rt);
            item.origin = Some(Origin::clean(
                Source { range: 0..0, text: String::new(), gap: String::new(), fingerprint: Fingerprint::default() },
                MdKind::ListItem { marker: m.marker },
            ));
            let start = j;
            items.push(item);
            j += 1;
            // Indented lines under the item are its children. Blank lines
            // inside the item belong to it; blank lines that end the list do
            // not, so the item's last content line is tracked separately.
            let child_base = l.indent + m.width;
            let mut k = j;
            let mut end = j;
            while k < to {
                if self.blank(k) {
                    k += 1;
                } else if self.line(k).indent >= child_base {
                    k += 1;
                    end = k;
                } else {
                    break;
                }
            }
            if end > j {
                let (nested, _) = self.blocks(j, end, child_base);
                if let Some(last) = items.last_mut() { last.children = nested; }
            }
            j = end;
            extents.push((start, end));
        }
        // Each item keeps its own bytes, blank tail included, so an
        // untouched list reproduces its spacing.
        for (item, (a, z)) in items.iter_mut().zip(extents) {
            // `split` finds the blank lines after the item itself; passing the
            // item's own end keeps them out of its text.
            let (sp, text, gap) = self.split(a, z);
            let md = item.origin.as_ref().and_then(|o| o.md).unwrap_or(MdKind::ListItem { marker: Marker::Dash });
            let fp = fingerprint(item);
            item.origin = Some(Origin::clean(Source { range: sp, text, gap, fingerprint: fp }, md));
        }
        (items, j)
    }

    /// `![alt](src)` alone on a line: an image block. A following
    /// `<!-- hush:size WxH crop -->` carries its display size.
    fn image(&self, i: usize, to: usize) -> Option<(Block, usize)> {
        let line = self.line(i).text.trim();
        let rest = line.strip_prefix("![")?;
        let close = rest.find("](")?;
        let alt = &rest[..close];
        let inner = rest[close + 2..].strip_suffix(')')?;
        let src = inner.trim();
        if src.is_empty() || src.contains(' ') {
            return None;
        }
        let mut end = i + 1;
        let mut width = None;
        let mut height = None;
        let mut crop = None;
        if end < to
            && let Some(meta) = self.line(end).text.trim().strip_prefix("<!-- hush:")
        {
            let meta = meta.trim_end_matches("-->").trim();
            if let Some(rest) = meta.strip_prefix("size ") {
                let mut parts = rest.split_whitespace();
                if let Some(size) = parts.next()
                    && let Some((w, h)) = size.split_once('x')
                {
                    width = w.parse::<f32>().ok();
                    height = h.parse::<f32>().ok();
                }
                if let Some(c) = parts.next() {
                    let vals: Vec<f32> = c.split(',').filter_map(|v| v.parse().ok()).collect();
                    if vals.len() == 4 {
                        crop = Some([vals[0], vals[1], vals[2], vals[3]]);
                    }
                }
                end += 1;
            }
        }
        let b = Block::new(
            BlockKind::Image { src_kind: "src".into(), src: src.to_string(), width, height, caption: (!alt.is_empty()).then(|| alt.to_string()), crop },
            RichText::default(),
        );
        Some((b, end))
    }

    /// Placeholder for the paragraph reader.
    fn paragraph(&self, i: usize, to: usize) -> (Block, usize) {
        let mut j = i;
        let mut lines: Vec<&str> = vec![];
        while j < to {
            let t = self.line(j).text;
            if t.trim().is_empty() { break; }
            if j > i && (is_block_start(t) || setext_level(t).is_some()) { break; }
            lines.push(t.trim_end());
            j += 1;
        }
        let mut rt = RichText::default();
        for (k, line) in lines.iter().enumerate() {
            if k > 0 {
                let prev = lines[k - 1];
                let hard = prev.ends_with("  ") || prev.ends_with('\\');
                rt.push(if hard { "\n" } else { " " }, Marks::default());
            }
            inline(line, Marks::default(), &mut rt);
        }
        (Block::new(BlockKind::Text, rt), j.max(i + 1))
    }

    fn table(&self, i: usize, to: usize) -> Option<(Block, usize)> {
        if i + 1 >= to { return None; }
        if !self.line(i).text.contains('|') || !is_table_delim(self.line(i + 1).text) { return None; }
        let cols = split_row(self.line(i + 1).text).len().max(1);
        let aligns: Vec<Option<Align>> = split_row(self.line(i + 1).text).iter().take(cols).map(|d| {
            let d = d.trim();
            // A plain `---` keeps its spelling; `:---` records the explicit left.
            match (d.starts_with(':'), d.ends_with(':')) {
                (true, true) => Some(Align::Center),
                (true, false) => Some(Align::Left),
                (false, true) => Some(Align::Right),
                (false, false) => None,
            }
        }).collect();
        let mut rows = vec![];
        let mut j = i;
        while j < to {
            let t = self.line(j).text;
            if t.trim().is_empty() || (j > i + 1 && !t.contains('|')) { break; }
            if j == i + 1 { j += 1; continue; }
            let cells = split_row(t);
            let mut row = vec![];
            for c in 0..cols {
                let mut rt = RichText::default();
                inline(cells.get(c).map(|s| s.trim()).unwrap_or(""), Marks::default(), &mut rt);
                row.push(TableCell { blocks: vec![Block::new(BlockKind::Text, rt)], ..TableCell::empty() });
            }
            rows.push(row);
            j += 1;
        }
        let mut t = Table { col_widths: vec![Table::DEFAULT_WIDTH; cols], header_rows: 1, header_col: false, rows, aligns };
        t.repair();
        Some((Block::new(BlockKind::Table(t), RichText::default()), j))
    }
}

fn strip_cols(line: &str, cols: usize) -> &str {
    let mut col = 0;
    for (i, c) in line.char_indices() {
        if col >= cols { return &line[i..]; }
        match c {
            ' ' => col += 1,
            '\t' => col += 4 - col % 4,
            _ => return &line[i..],
        }
    }
    ""
}

// ---- block syntax

struct Fence {
    ch: char,
    chars: String,
    info: String,
}

fn fence_open(t: &str) -> Option<Fence> {
    let t = t.trim_start();
    let ch = t.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let n = t.chars().take_while(|&x| x == ch).count();
    if n < 3 { return None; }
    let info = t[n..].trim().to_string();
    if ch == '`' && info.contains('`') { return None; }
    Some(Fence { ch, chars: t[..n].to_string(), info })
}

fn fence_close(t: &str, open: &Fence) -> bool {
    let t = t.trim();
    let n = t.chars().take_while(|&x| x == open.ch).count();
    n >= open.chars.chars().count() && t[n..].trim().is_empty()
}

fn atx(t: &str) -> Option<(u8, &str)> {
    let t = t.trim_start();
    let n = t.chars().take_while(|&c| c == '#').count();
    if n == 0 || n > 6 { return None; }
    let rest = &t[n..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) { return None; }
    Some((n as u8, rest.trim().trim_end_matches('#').trim_end()))
}

fn setext_level(t: &str) -> Option<u8> {
    let t = t.trim();
    let c = t.chars().next()?;
    if !matches!(c, '=' | '-') || !t.chars().all(|x| x == c) { return None; }
    Some(if c == '=' { 1 } else { 2 })
}

fn is_thematic(t: &str) -> bool {
    let t = t.trim();
    let c = match t.chars().next() {
        Some(c @ ('-' | '*' | '_')) => c,
        _ => return false,
    };
    t.chars().filter(|&x| x != ' ' && x != '\t').count() >= 3 && t.chars().all(|x| x == c || x == ' ' || x == '\t')
}

fn is_table_delim(t: &str) -> bool {
    let cells = split_row(t);
    !cells.is_empty()
        && cells.iter().all(|c| {
            let c = c.trim().trim_start_matches(':').trim_end_matches(':');
            c.len() >= 3 && c.chars().all(|x| x == '-')
        })
}

fn split_row(t: &str) -> Vec<String> {
    let t = t.trim().trim_start_matches('|').trim_end_matches('|');
    let mut out = vec![];
    let mut cur = String::new();
    let mut esc = false;
    for c in t.chars() {
        if esc {
            cur.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == '|' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

struct ListMark {
    ordered: bool,
    seq: Option<u32>,
    task: Option<bool>,
    marker: Marker,
    /// Byte offset in the line where the content starts.
    content_at: usize,
    /// Columns from the line start to the content.
    width: usize,
}

fn list_marker(t: &str) -> Option<ListMark> {
    let (ordered, seq, marker, mlen) = match t.chars().next()? {
        c @ ('-' | '*' | '+') => {
            if t.len() > 1 && !t[1..].starts_with([' ', '\t']) { return None; }
            let marker = match c {
                '-' => Marker::Dash,
                '*' => Marker::Star,
                _ => Marker::Plus,
            };
            (false, None, marker, 1)
        }
        c if c.is_ascii_digit() => {
            let digits = t.bytes().take_while(u8::is_ascii_digit).count();
            let d = t[digits..].chars().next().filter(|c| matches!(c, '.' | ')'))?;
            if t.len() > digits + 1 && !t[digits + 1..].starts_with([' ', '\t']) { return None; }
            (true, t[..digits].parse().ok(), Marker::Ordered { paren: d == ')' }, digits + 1)
        }
        _ => return None,
    };
    if is_thematic(t) { return None; }
    let body_start = mlen;
    let body = &t[body_start..];
    let pad = body.len() - body.trim_start_matches([' ', '\t']).len();
    let mut content_at = body_start + pad;
    // Task marker: `[ ]` / `[x]` followed by a space.
    let rest = &t[content_at..];
    let task = if rest.len() >= 4 && rest.as_bytes()[0] == b'[' && rest.as_bytes()[2] == b']' && rest.as_bytes()[3] == b' ' {
        match rest.as_bytes()[1].to_ascii_lowercase() {
            b' ' => Some(false),
            b'x' => Some(true),
            _ => None,
        }
    } else {
        None
    };
    if task.is_some() { content_at += 4; }
    let width = content_at.max(2);
    Some(ListMark { ordered, seq, task, marker, content_at, width })
}

fn is_block_start(t: &str) -> bool {
    atx(t).is_some()
        || fence_open(t).is_some()
        || is_thematic(t)
        || t.starts_with('>')
        || list_marker(t).is_some()
        || t.trim_start().starts_with('<')
}

// ---- writing

pub fn serialize(b: &Block, out: &mut String, depth: usize) {
    serialize_shallow(b, out, depth);
    // List items are siblings: children are a nested list under the item.
    for c in &b.children {
        let nested = matches!(b.kind, BlockKind::Bullet | BlockKind::Ordered { .. } | BlockKind::Todo { .. });
        if nested {
            serialize(c, out, depth + 1);
        } else {
            serialize(c, out, 0);
        }
    }
}

/// One block's own line(s), without children.
fn serialize_shallow(b: &Block, out: &mut String, depth: usize) {
    let pad = "  ".repeat(depth);
    let marker = |b: &Block| -> String {
        match b.origin.as_ref().and_then(|o| o.md) {
            Some(MdKind::ListItem { marker }) => match marker {
                Marker::Dash => "-".into(),
                Marker::Star => "*".into(),
                Marker::Plus => "+".into(),
                Marker::Ordered { paren } => format!("{}{}", match b.kind { BlockKind::Ordered { seq } => seq.unwrap_or(1), _ => 1 }, if paren { ")" } else { "." }),
            },
            _ => match b.kind {
                BlockKind::Ordered { seq } => format!("{}.", seq.unwrap_or(1)),
                BlockKind::Todo { .. } => "-".into(),
                _ => "-".into(),
            },
        }
    };
    match &b.kind {
        BlockKind::Heading { level, .. } => {
            out.push_str(&format!("{pad}{} {}\n", "#".repeat((*level).clamp(1, 6) as usize), inline_md(&b.text)));
        }
        BlockKind::Text => {
            out.push_str(&pad);
            out.push_str(&inline_md(&b.text));
            out.push('\n');
        }
        BlockKind::Bullet | BlockKind::Ordered { .. } | BlockKind::Todo { .. } => {
            let m = marker(b);
            let task = match b.kind {
                BlockKind::Todo { done } => format!(" [{}]", if done { 'x' } else { ' ' }),
                _ => String::new(),
            };
            out.push_str(&format!("{pad}{m}{task} {}\n", inline_md(&b.text)));
        }
        BlockKind::Quote => {
            let mut inner = String::new();
            for c in &b.children { serialize(c, &mut inner, 0); }
            for line in inner.lines() {
                out.push_str(&format!("{pad}> {line}\n"));
            }
            if inner.is_empty() { out.push_str(&format!("{pad}>\n")); }
        }
        BlockKind::Code { lang, .. } => {
            let fence = match b.origin.as_ref().and_then(|o| o.md) {
                Some(MdKind::Fenced { fence }) if !b.text.text.contains(&fence.ch.to_string().repeat(fence.len)) => {
                    fence.ch.to_string().repeat(fence.len)
                }
                _ if b.text.text.contains("```") => "````".into(),
                _ => "```".into(),
            };
            out.push_str(&format!("{pad}{fence}{lang}\n"));
            if !b.text.text.is_empty() {
                out.push_str(&b.text.text);
                if !b.text.text.ends_with('\n') { out.push('\n'); }
            }
            out.push_str(&format!("{pad}{fence}\n"));
        }
        BlockKind::Divider => out.push_str(&format!("{pad}---\n")),
        BlockKind::Table(t) => {
            // A cell's blocks all go on its one Markdown line: joining their
            // text with spaces keeps every word (a newline inside a cell
            // would end the row).
            let cell_md = |cell: &crate::model::TableCell| -> String {
                cell.blocks
                    .iter()
                    .map(|b| inline_md(&b.text))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
                    .replace('\n', " ")
                    .replace('|', "\\|")
            };
            for (r, row) in t.rows.iter().enumerate() {
                out.push_str(&pad);
                out.push('|');
                for cell in row {
                    out.push_str(&format!(" {} |", cell_md(cell)));
                }
                out.push('\n');
                if r == 0 && t.header_rows > 0 {
                    out.push_str(&pad);
                    out.push('|');
                    for c in 0..t.ncols() {
                        out.push_str(match t.aligns.get(c).copied().flatten() {
                            None => " --- |",
                            Some(Align::Left) => " :--- |",
                            Some(Align::Center) => " :---: |",
                            Some(Align::Right) => " ---: |",
                        });
                    }
                    out.push('\n');
                }
            }
        }
        BlockKind::Raw { .. } => {
            if let Some(text) = b.origin.as_ref().and_then(|o| o.source.as_ref()).map(|s| s.text.as_str()) {
                out.push_str(text);
            }
        }
        BlockKind::Image { src, caption, width, height, crop, .. } => {
            let alt = caption.as_deref().unwrap_or("");
            out.push_str(&format!("{pad}![{alt}]({src})\n"));
            // Markdown has no place for the displayed size or the crop; an
            // HTML comment carries them so re-opening the file keeps them,
            // and every other Markdown tool ignores the line.
            if let Some(w) = width {
                let h = height.unwrap_or(0.);
                let c = crop.map(|c| c.iter().map(|v| format!("{v:.4}")).collect::<Vec<_>>().join(",")).unwrap_or_default();
                out.push_str(&format!("{pad}<!-- hush:size {w:.1}x{h:.1} {c} -->\n"));
            }
        }
        BlockKind::Callout { .. } => {
            // No Markdown syntax exists for a callout: keep its content as a
            // quote, which is the closest thing and never loses text.
            let mut inner = String::new();
            for c in &b.children {
                serialize(c, &mut inner, 0);
            }
            for line in inner.lines() {
                out.push_str(&format!("{pad}> {line}\n"));
            }
            if inner.is_empty() {
                out.push_str(&format!("{pad}>\n"));
            }
        }
        BlockKind::Grid | BlockKind::Column { .. } => {
            // Columns stack: their content is still there, just not side by side.
            for c in &b.children {
                serialize(c, out, depth);
            }
        }
        _ => {
            let t = inline_md(&b.text);
            if !t.is_empty() {
                out.push_str(&pad);
                out.push_str(&t);
                out.push('\n');
            }
        }
    }
}

/// Inline Markdown for one line of rich text.
pub fn inline_md(t: &RichText) -> String {
    let mut out = String::new();
    for (r, m) in t.spans() {
        let s = &t.text[r];
        if m.inline.is_some() { continue; }
        if m.code {
            let ticks = "`".repeat(longest_run(s, '`') + 1);
            let pad = if s.starts_with('`') || s.ends_with('`') { " " } else { "" };
            out.push_str(&format!("{ticks}{pad}{s}{pad}{ticks}"));
            continue;
        }
        let mut piece = escape_inline(s);
        if m.strike { piece = format!("~~{piece}~~"); }
        if m.italic { piece = format!("*{piece}*"); }
        if m.bold { piece = format!("**{piece}**"); }
        if let Some(link) = &m.link {
            piece = if piece == *link { format!("<{link}>") } else { format!("[{piece}]({link})") };
        }
        out.push_str(&piece);
    }
    out
}

fn longest_run(s: &str, c: char) -> usize {
    let mut best = 0;
    let mut cur = 0;
    for x in s.chars() {
        if x == c {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

fn escape_inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.char_indices() {
        let special = matches!(c, '\\' | '`' | '*' | '_' | '[' | ']');
        if special || (c == '<' && s[i + 1..].starts_with(|x: char| x.is_ascii_alphanumeric() || x == '/')) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

// ---- inline reading

/// Parse inline Markdown in `s`, appending to `out`.
pub fn inline(s: &str, marks: Marks, out: &mut RichText) {
    let mut i = 0;
    let mut plain = String::new();
    macro_rules! flush {
        () => {
            if !plain.is_empty() {
                out.push(&plain, marks.clone());
                plain.clear();
            }
        };
    }
    while i < s.len() {
        let rest = &s[i..];
        let c = rest.chars().next().unwrap();
        if c == '\\' && rest[1..].starts_with(|x: char| x.is_ascii_punctuation()) {
            let next = rest[1..].chars().next().unwrap();
            plain.push(next);
            i += 1 + next.len_utf8();
            continue;
        }
        if c == '`' {
            let n = rest.bytes().take_while(|&b| b == b'`').count();
            let delim = &rest[..n];
            if let Some(end) = rest[n..].find(delim) {
                let inner = &rest[n..n + end];
                if !inner.is_empty() {
                    flush!();
                    let t = if inner.len() > 2 && inner.starts_with(' ') && inner.ends_with(' ') { &inner[1..inner.len() - 1] } else { inner };
                    out.push(t, Marks { code: true, ..marks.clone() });
                    i += n + end + n;
                    continue;
                }
            }
            plain.push_str(delim);
            i += n;
            continue;
        }
        if c == '!'
            && let Some((label, url, used)) = rest.strip_prefix("![").and_then(|r| link_at(&format!("[{r}")))
        {
            flush!();
            out.push(&label, Marks { link: Some(url), ..marks.clone() });
            i += 1 + used;
            continue;
        }
        if c == '['
            && let Some((label, url, used)) = link_at(rest)
        {
            flush!();
            let mut m = marks.clone();
            m.link = Some(url);
            let mut sub = RichText::default();
            inline(&label, m, &mut sub);
            out.append(&sub);
            i += used;
            continue;
        }
        if c == '<' && let Some(end) = rest.find('>') {
            let url = &rest[1..end];
            if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
                flush!();
                out.push(url, Marks { link: Some(url.to_string()), ..marks.clone() });
                i += end + 1;
                continue;
            }
        }
        if matches!(c, '*' | '_' | '~') {
            let n = rest.chars().take_while(|&x| x == c).count();
            let run = if n >= 2 { 2 } else { 1 };
            let delim = &rest[..run];
            if let Some(end) = emphasis_end(&rest[run..], delim) {
                let inner = &rest[run..run + end];
                if !inner.is_empty() && !inner.starts_with(char::is_whitespace) {
                    flush!();
                    let mut m = marks.clone();
                    match (c, run) {
                        ('~', _) => m.strike = true,
                        (_, 2) => m.bold = true,
                        _ => m.italic = true,
                    }
                    let mut sub = RichText::default();
                    inline(inner, m, &mut sub);
                    out.append(&sub);
                    i += run + end + run;
                    continue;
                }
            }
            plain.push_str(&rest[..n]);
            i += n;
            continue;
        }
        plain.push(c);
        i += c.len_utf8();
    }
    flush!();
}

fn emphasis_end(s: &str, delim: &str) -> Option<usize> {
    let c = delim.chars().next().unwrap();
    let mut i = 0;
    while i < s.len() {
        if s[i..].starts_with(delim) {
            let after = s[i + delim.len()..].chars().next();
            let before = s[..i].chars().next_back();
            if after != Some(c) && before.is_some_and(|x| !x.is_whitespace()) { return Some(i); }
        }
        i += s[i..].chars().next().unwrap().len_utf8();
    }
    None
}

fn link_at(s: &str) -> Option<(String, String, usize)> {
    if !s.starts_with('[') { return None; }
    let mut depth = 0;
    let mut close = None;
    for (i, ch) in s.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let label = &s[1..close];
    let rest = &s[close + 1..];
    let r = rest.strip_prefix('(')?;
    let end = r.find(')')?;
    let inner = r[..end].trim();
    let url = inner.split_whitespace().next().unwrap_or("").trim_matches(['<', '>']).to_string();
    Some((label.to_string(), url, close + 2 + end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loading a file and saving it without edits must give back the same
    /// bytes. This is the property the whole design rests on.
    fn round_trip(src: &str) -> String {
        write(&read(src))
    }

    #[test]
    fn plain_paragraphs_round_trip() {
        for src in ["hello\n", "hello\n\nworld\n", "a\nb\n\n\nc\n", "", "\n\n\n", "trailing spaces  \nx\n"] {
            assert_eq!(round_trip(src), src, "source: {src:?}");
        }
    }

    #[test]
    fn headings_round_trip() {
        for src in [
            "# One\n",
            "## Two ##\n",
            "#NoSpace\n",
            "####### seven\n",
            "Title\n=====\n",
            "Title\n-----\n\nnext\n",
            "## 中文标题\n",
        ] {
            assert_eq!(round_trip(src), src, "source: {src:?}");
        }
    }

    #[test]
    fn code_and_fences_round_trip() {
        for src in [
            "```rust\nfn main() {}\n```\n",
            "```\n```\n",
            "~~~\na\n~~~\n",
            "````md\n```\nnested\n```\n````\n",
            "    indented code\n    more\n",
            "```\nno closing fence\n",
            "```py\na\n\nb\n```\n\nafter\n",
        ] {
            assert_eq!(round_trip(src), src, "source: {src:?}");
        }
    }

    #[test]
    fn lists_round_trip() {
        for src in [
            "- a\n- b\n",
            "* a\n+ b\n",
            "1. a\n2. b\n",
            "- [ ] todo\n- [x] done\n",
            "- a\n  - nested\n- b\n",
            "- a\n\n- b\n",
            "- a\n  continued\n",
            "1) a\n2) b\n",
        ] {
            assert_eq!(round_trip(src), src, "source: {src:?}");
        }
    }

    #[test]
    fn quotes_tables_and_rules_round_trip() {
        for src in [
            "> quoted\n> more\n",
            "> a\n>\n> b\n",
            "| a | b | c |\n| :--- | :---: | ---: |\n| 1 | 2 | 3 |\n",
            "| a |\n| --- |\n",
            // A diagram fence is ordinary code to the format: the source is
            // what gets saved, never a rendering of it.
            "```mermaid\nflowchart LR\n  A --> B\n```\n",
            "---\n",
            "***\n",
            "___\n",
            "| a | b |\n| --- | --- |\n| 1 | 2 |\n",
            "| a | b |\n|:---|---:|\n| 1 | 2 |\n| 3 | 4 |\n",
        ] {
            assert_eq!(round_trip(src), src, "source: {src:?}");
        }
    }

    #[test]
    fn front_matter_and_html_round_trip() {
        for src in [
            "---\ntitle: x\ntags: [a, b]\n---\n\nbody\n",
            "<div class=\"x\">\nraw\n</div>\n\ntext\n",
            "<script>\nvar x = 1;\n</script>\n",
        ] {
            assert_eq!(round_trip(src), src, "source: {src:?}");
        }
    }

    #[test]
    fn table_cell_text_is_never_dropped() {
        // Two paragraphs in one cell: Markdown has one line per cell, so the
        // save joins them rather than keeping only the first.
        let src = "| a |\n| --- |\n| 1 |\n";
        let mut doc = read(src);
        let BlockKind::Table(t) = &mut doc.blocks[0].kind else { panic!("no table") };
        t.rows[1][0].blocks.push(Block::text("2"));
        t.rows[1][0].blocks.push(Block::text("3"));
        t.rows[1][0].blocks[0].text = RichText::plain("1");
        // The edit kernel marks the containing block dirty; do the same so the
        // writer does not hand back the source bytes.
        doc.blocks[0].origin.as_mut().unwrap().dirty = true;
        let out = write(&doc);
        assert_eq!(out, "| a |\n| --- |\n| 1 2 3 |\n", "{out}");
        let again = read(&out);
        let BlockKind::Table(t) = &again.blocks[0].kind else { panic!("no table") };
        assert_eq!(t.rows[1][0].blocks[0].text.text, "1 2 3");
    }

    #[test]
    fn editing_a_table_keeps_column_alignment() {
        let src = "| left | middle | right |\n| :--- | :---: | ---: |\n| 1 | 2 | 3 |\n";
        let mut doc = read(src);
        let BlockKind::Table(t) = &mut doc.blocks[0].kind else { panic!("no table") };
        assert_eq!(t.aligns, vec![Some(Align::Left), Some(Align::Center), Some(Align::Right)]);
        t.rows[1][0].blocks[0].text = RichText::plain("changed");
        doc.blocks[0].origin.as_mut().unwrap().dirty = true;
        assert_eq!(write(&doc), "| left | middle | right |\n| :--- | :---: | ---: |\n| changed | 2 | 3 |\n");
    }

    #[test]
    fn inline_parsing() {
        let blocks = parse("**bold** and *italic* and `code` and ~~gone~~\n");
        let t = &blocks[0].text;
        assert!(t.spans().any(|(_, m)| m.bold));
        assert!(t.spans().any(|(_, m)| m.italic));
        assert!(t.spans().any(|(_, m)| m.code));
        assert!(t.spans().any(|(_, m)| m.strike));
        let plain: String = t.text.clone();
        assert_eq!(plain, "bold and italic and code and gone");
    }

    #[test]
    fn links_and_autolinks() {
        let blocks = parse("[label](https://example.com) <https://a.b>\n");
        let links: Vec<_> = blocks[0].text.spans().filter_map(|(r, m)| m.link.as_ref().map(|l| (blocks[0].text.text[r].to_string(), l.clone()))).collect();
        assert_eq!(links[0].0, "label");
        assert_eq!(links[0].1, "https://example.com");
        assert_eq!(links[1].1, "https://a.b");
    }

    #[test]
    fn editing_one_block_leaves_the_rest_byte_identical() {
        let src = "---\ntitle: x\n---\n\n# Head\n\nfirst\n\nsecond\n";
        let mut doc = read(src);
        let blocks = &mut doc.blocks;
        // Change the second paragraph.
        let b = blocks.iter_mut().find(|b| b.text.text == "second").unwrap();
        b.text = RichText::plain("changed");
        let out = write(&doc);
        assert!(out.contains("---\ntitle: x\n---\n"), "{out}");
        assert!(out.contains("# Head\n"), "{out}");
        assert!(out.contains("first\n"), "{out}");
        assert!(out.contains("changed"), "{out}");
        assert!(!out.contains("second"), "{out}");
    }

    /// Real documents must survive a load → save unchanged. Point
    /// `HUSH_MD_ROUNDTRIP_DIR` at a folder of `.md` files to widen the check.
    #[test]
    fn real_documents_round_trip() {
        let mut files: Vec<(String, String)> = vec![
            ("readme".into(), include_str!("../../../README.md").to_string()),
            ("architecture".into(), include_str!("../../../docs/ARCHITECTURE.md").to_string()),
            ("contributing".into(), include_str!("../../../CONTRIBUTING.md").to_string()),
            ("format".into(), include_str!("../../../docs/FORMAT.md").to_string()),
            ("themes".into(), include_str!("../../../docs/THEMES.md").to_string()),
        ];
        if let Some(dir) = std::env::var_os("HUSH_MD_ROUNDTRIP_DIR")
            && let Ok(rd) = std::fs::read_dir(dir)
        {
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "md")
                    && let Ok(t) = std::fs::read_to_string(e.path())
                {
                    files.push((e.file_name().to_string_lossy().into(), t));
                }
            }
        }
        for (name, src) in files {
            let out = round_trip(&src);
            if out != src {
                let at = src.bytes().zip(out.bytes()).position(|(a, b)| a != b).unwrap_or(src.len().min(out.len()));
                let lo = src.floor_char_boundary(at.saturating_sub(60));
                let hi = src.ceil_char_boundary((at + 60).min(src.len()));
                let ohi = out.ceil_char_boundary((at + 60).min(out.len()));
                panic!("{name}: differs at byte {at}\n  in:  {:?}\n  out: {:?}", &src[lo..hi], &out[lo.min(out.len())..ohi]);
            }
        }
    }

    #[test]
    fn fence_then_rule() {
        let src = "````x\na\n````\n\n---\n\nb\n";
        let doc = read(src);
        for (i, b) in doc.blocks.iter().enumerate() {
            eprintln!("{i}: {:?} text={:?} gap={:?}", b.kind, b.origin.as_ref().and_then(|o| o.source.as_ref()).map(|s| s.text.clone()), b.origin.as_ref().and_then(|o| o.source.as_ref()).map(|s| s.gap.clone()));
        }
        assert_eq!(write(&doc), src);
    }

    /// Editing a block rewrites only that block; everything else keeps its
    /// exact bytes, and the result reads back to the same model.
    #[test]
    fn edits_are_local_and_stable() {
        let src = "---\ntitle: t\n---\n\n# Head ##\n\npara one\n\n* item a\n* item b\n\n```py\nx = 1\n```\n\n> quote\n";
        // Rewrite the first paragraph.
        let mut doc = read(src);
        doc.blocks[2].text = RichText::plain("para two");
        let out = write(&doc);
        assert_eq!(out, src.replace("para one", "para two"));

        // Turn a bullet into a heading; the marker style of the other stays.
        let mut doc = read(src);
        doc.blocks[3].kind = BlockKind::Heading { level: 2, seq_auto: false };
        let out = write(&doc);
        assert!(out.contains("## item a\n"), "{out}");
        assert!(out.contains("* item b\n"), "the untouched item keeps its marker: {out}");

        // Toggling a checkbox.
        let src = "- [ ] a\n- [x] b\n";
        let mut doc = read(src);
        doc.blocks[0].kind = BlockKind::Todo { done: true };
        assert_eq!(write(&doc), "- [x] a\n- [x] b\n");

        // A toolbar bold on a plain paragraph.
        let src = "hello world\n";
        let mut doc = read(src);
        doc.blocks[0].text.update_marks(0..5, |m| m.bold = true);
        let out = write(&doc);
        assert_eq!(out, "**hello** world\n");
        // …and it reads back with the bold intact.
        let round = read(&out);
        assert!(round.blocks[0].text.spans().any(|(r, m)| m.bold && &round.blocks[0].text.text[r] == "hello"));
    }

    #[test]
    fn the_title_is_the_first_heading() {
        let d = read("# Real Title\n\nbody\n");
        assert_eq!(title_of(&d), "Real Title");
        // A level-2 heading is used when there is no level-1 one.
        let d = read("intro\n\n## Section\n");
        assert_eq!(title_of(&d), "Section");
        // Otherwise the first text stands in.
        let d = read("just some text\n");
        assert_eq!(title_of(&d), "just some text");
        assert_eq!(title_of(&read("")), "");
        // A heading inside a fence is not a title.
        let d = read("```\n# no\n```\n\n# yes\n");
        assert_eq!(title_of(&d), "yes");
    }

    /// Every block keeps the blank lines that follow it, including the last
    /// one in the file.
    #[test]
    fn block_gaps_cover_their_trailing_blank_lines() {
        let src = "a\n\n- one\n- two\n\nb\n\n";
        let d = read(src);
        let gaps: Vec<_> = d.blocks.iter().map(|b| b.origin.as_ref().and_then(|o| o.source.as_ref()).map(|s| s.gap.clone())).collect();
        // One gap per block: `a`, the first item, the last item (which carries
        // the blank line that ends the list), and `b`.
        assert_eq!(gaps, vec![Some("\n".into()), Some("".into()), Some("\n".into()), Some("\n".into())], "{gaps:?}");
        assert_eq!(write(&d), src);
    }



    #[test]
    fn document_structure() {
        let blocks = parse("# H1\n\npara\n\n- item\n  - nested\n");
        assert_eq!(blocks.len(), 3);
        assert!(matches!(blocks[0].kind, BlockKind::Heading { level: 1, .. }));
        assert!(matches!(blocks[1].kind, BlockKind::Text));
        assert!(matches!(blocks[2].kind, BlockKind::Bullet));
        assert_eq!(blocks[2].children.len(), 1);
        assert!(matches!(blocks[2].children[0].kind, BlockKind::Bullet));
    }
}

#[cfg(test)]
mod source_edit_tests {

    /// Editing through the source view means: take the file text, change one
    /// line, re-read the whole thing. Everything the line did not touch must
    /// come back identical.
    fn edit_line(src: &str, old: &str, new: &str) -> String {
        assert!(src.contains(old), "{old:?} not in source");
        super::write(&super::read(&src.replace(old, new)))
    }

    #[test]
    fn editing_one_source_line_keeps_the_rest() {
        let src = "---\ntitle: t\n---\n\n# Head\n\npara one\n\n- a\n- b\n\n```py\nx = 1\n```\n";
        let out = edit_line(src, "para one", "para 1");
        assert_eq!(out, src.replace("para one", "para 1"));
    }

    #[test]
    fn retyping_the_same_text_changes_nothing() {
        let src = "# T\n\n- [x] a\n\n> q\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n";
        assert_eq!(edit_line(src, "# T", "# T"), src);
    }

    #[test]
    fn changing_a_block_type_writes_the_new_shape() {
        let src = "plain\n";
        assert_eq!(edit_line(src, "plain", "## heading"), "## heading\n");
        let src = "- item\n";
        assert_eq!(edit_line(src, "- item", "1. item"), "1. item\n");
        let src = "todo\n";
        assert_eq!(edit_line(src, "todo", "- [x] todo"), "- [x] todo\n");
    }

    #[test]
    fn adding_and_removing_blocks() {
        let src = "a\n\nb\n";
        assert_eq!(edit_line(src, "a\n\nb", "a\n\nbetween\n\nb"), "a\n\nbetween\n\nb\n");
        assert_eq!(edit_line(src, "a\n\nb", "b"), "b\n");
    }

    #[test]
    fn a_paragraph_keeps_its_inline_marks_through_a_model_round_trip() {
        let src = "some **bold** and a [link](https://x.y) here\n";
        let doc = super::read(src);
        assert_eq!(super::write(&doc), src);
        let t = &doc.blocks[0].text;
        assert!(t.spans().any(|(r, m)| m.bold && &t.text[r] == "bold"));
        assert!(t.spans().any(|(_, m)| m.link.as_deref() == Some("https://x.y")));
    }
}

/// What the source view does to a file's text: insert, delete, split, undo.
/// These are the operations the source surface performs, checked against the
/// parser and writer without a window.
#[cfg(test)]
mod source_edit_unit_tests {
    use super::*;

    /// A minimal stand-in for the source view's caret, selection and undo.
    struct T {
        text: String,
        caret: usize,
        anchor: usize,
        undo: Vec<(String, usize)>,
        redo: Vec<(String, usize)>,
    }

    impl T {
        fn new(s: &str) -> Self {
            T { text: s.into(), caret: 0, anchor: 0, undo: vec![], redo: vec![] }
        }
        fn sel(&self) -> std::ops::Range<usize> {
            self.anchor.min(self.caret)..self.anchor.max(self.caret)
        }
        fn replace(&mut self, r: std::ops::Range<usize>, s: &str) {
            self.undo.push((self.text.clone(), self.caret));
            self.redo.clear();
            self.text.replace_range(r.clone(), s);
            self.caret = r.start + s.len();
            self.anchor = self.caret;
        }
        fn undo(&mut self) {
            let Some((t, c)) = self.undo.pop() else { return };
            self.redo.push((std::mem::replace(&mut self.text, t), self.caret));
            self.caret = c;
            self.anchor = c;
        }
        fn line_bounds(&self, at: usize) -> (usize, usize) {
            let start = self.text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let end = self.text[at..].find('\n').map(|i| at + i).unwrap_or(self.text.len());
            (start, end)
        }
    }

    #[test]
    fn typing_inserts_at_the_caret() {
        let mut t = T::new("hello\n");
        t.replace(5..5, " world");
        assert_eq!(t.text, "hello world\n");
        assert_eq!(t.caret, 11);
    }

    #[test]
    fn enter_starts_a_new_block() {
        let mut t = T::new("one paragraph\n");
        t.replace(13..13, "\n\ntwo");
        let d = read(&t.text);
        assert_eq!(d.blocks.len(), 2, "{:?}", d.blocks.iter().map(|b| b.text.text.clone()).collect::<Vec<_>>());
        assert_eq!(write(&d), t.text);
    }

    #[test]
    fn typing_a_marker_changes_the_block_kind() {
        let mut t = T::new("title\n- item\n");
        t.replace(0..0, "## ");
        let d = read(&t.text);
        assert!(matches!(d.blocks[0].kind, BlockKind::Heading { level: 2, .. }));
        assert!(matches!(d.blocks[1].kind, BlockKind::Bullet));
        assert_eq!(write(&d), t.text);
    }

    #[test]
    fn undo_restores_the_exact_text() {
        let mut t = T::new("# a\n\nb\n");
        let before = t.text.clone();
        t.replace(4..4, "XY");
        assert_ne!(t.text, before);
        t.undo();
        assert_eq!(t.text, before);
    }

    #[test]
    fn deleting_a_selection_removes_exactly_that_range() {
        let mut t = T::new("one two three\n");
        // "one two three": remove "two " (bytes 4..8).
        t.anchor = 4;
        t.caret = 8;
        t.replace(t.sel(), "");
        assert_eq!(t.text, "one three\n");
    }

    #[test]
    fn line_bounds_find_the_current_line() {
        let t = T::new("aa\nbbb\ncc");
        assert_eq!(t.line_bounds(0), (0, 2));
        assert_eq!(t.line_bounds(4), (3, 6));
        assert_eq!(t.line_bounds(8), (7, 9));
        assert_eq!(t.line_bounds(9), (7, 9));
    }

    /// An edit anywhere leaves every other block byte-identical.
    #[test]
    fn edits_round_trip_through_the_parser() {
        let src = "---\ntitle: t\n---\n\n# H\n\n- [ ] a\n\n> q\n";
        let mut t = T::new(src);
        t.replace(30..30, "!");
        let d = read(&t.text);
        assert_eq!(write(&d), t.text);
        // The front matter and the rest are still there.
        assert!(t.text.contains("title: t"));
        assert!(t.text.contains("- [ ] a"));
    }
}
