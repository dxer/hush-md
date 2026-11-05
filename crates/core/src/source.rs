//! Where a block sits in the Markdown file it came from, and how to write it
//! back.
//!
//! Every block loaded from disk remembers its byte range and the Markdown
//! fragment it was read from. A canonical serialization of the block is
//! fingerprinted at load time; the writer fingerprints the block again and,
//! when the two match, emits the original fragment instead. Editing one
//! paragraph therefore rewrites that paragraph and nothing else — front
//! matter, HTML, unknown syntax and deliberate whitespace elsewhere stay
//! byte-identical.

use std::ops::Range;

/// `(hash, length)` of a block's canonical Markdown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fingerprint {
    pub hash: u64,
    pub len: usize,
}

/// The block's span on disk and the text it was read from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Source {
    pub range: Range<usize>,
    /// The block's own bytes, without the blank lines that follow it.
    /// Written back verbatim while the block is pristine.
    pub text: String,
    /// The blank lines between this block and the next, byte for byte. Kept
    /// through an edit: changing a paragraph does not change the space after
    /// it.
    pub gap: String,
    pub fingerprint: Fingerprint,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Origin {
    pub source: Option<Source>,
    /// The Markdown construct this block was read as, kept so the writer can
    /// reproduce its shape (fence length, list marker, …).
    pub md: Option<MdKind>,
    /// Set when the block was edited, so the writer rebuilds it.
    pub dirty: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MdKind {
    /// `# heading` or its setext underline form.
    Heading,
    Paragraph,
    /// `- item`, `1. item`, `- [x] item`; `marker` is the exact marker text
    /// (`-`, `*`, `1.`, …) so an untouched item keeps its bullet character.
    ListItem { marker: Marker },
    Quote,
    /// A fenced block; `fence` is the opening run (` ``` `, `~~~~`).
    Fenced { fence: FenceStyle },
    /// A four-space indented code block.
    Indented,
    Divider,
    /// A `---` thematic break written as three or more characters.
    Table,
    /// An HTML block, kept verbatim.
    Html,
    /// Front matter, or anything else kept verbatim.
    Raw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Dash,
    Star,
    Plus,
    /// Ordered: `1.` or `1)`.
    Ordered { paren: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FenceStyle {
    pub ch: char,
    pub len: usize,
}

impl Origin {
    /// A block read from the file, still untouched.
    pub fn clean(source: Source, md: MdKind) -> Self {
        Origin { source: Some(source), md: Some(md), dirty: false }
    }

    /// A block the editor created; the writer serializes it from the model.
    pub fn fresh(md: MdKind) -> Self {
        Origin { source: None, md: Some(md), dirty: true }
    }

    /// No origin at all (a document that was never on disk).
    pub fn none() -> Self {
        Origin::default()
    }
}

/// Bytes of the file that are not part of any block: blank lines before the
/// first block. (Blank lines after the last block are held by that block.)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocSource {
    pub front: String,
}

/// Write everything about a block that a reader could observe — its kind's
/// data and its text, but not how it happens to be written. Used to tell
/// whether a block still matches what was loaded from disk.
pub fn model_fingerprint(b: &crate::model::Block, out: &mut String) {
    use crate::model::{BlockKind, Table};
    out.push_str(b.text.text.as_str());
    for run in &b.text.runs {
        out.push_str(&format!(
            "\u{1}{:?}\u{1}{:?}\u{1}{:?}\u{1}{:?}\u{1}{:?}\u{1}{:?}\u{1}{:?}{}",
            run.len, run.marks.bold, run.marks.italic, run.marks.strike, run.marks.code, run.marks.link, run.marks.underline,
            run.marks.inline.as_ref().map(|i| i.xml.as_str()).unwrap_or("")
        ));
    }
    out.push_str(&format!("\u{2}{:?}\u{2}{:?}", b.align, b.bg));
    match &b.kind {
        BlockKind::Heading { level, seq_auto } => out.push_str(&format!("h{level}{seq_auto}")),
        BlockKind::Ordered { seq } => out.push_str(&format!("o{seq:?}")),
        BlockKind::Todo { done } => out.push_str(&format!("t{done}")),
        BlockKind::Code { lang, caption, wrap } => out.push_str(&format!("c{lang}{caption:?}{wrap}")),
        BlockKind::Divider => out.push('d'),
        BlockKind::Bullet => out.push('b'),
        BlockKind::Text => out.push('p'),
        BlockKind::Raw { tag, xml } => out.push_str(&format!("raw{tag}{xml}")),
        BlockKind::Table(Table { header_rows, header_col, rows, .. }) => {
            out.push_str(&format!("tbl{header_rows}{header_col}{}", rows.len()));
            for row in rows {
                for cell in row {
                    out.push_str(&format!("|{}{}", cell.colspan, cell.rowspan));
                    for b in &cell.blocks {
                        model_fingerprint(b, out);
                    }
                }
            }
        }
        other => out.push_str(&format!("{other:?}")),
    }
}
