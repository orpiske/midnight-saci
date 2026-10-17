//! First pass: GitHub Flavored Markdown into a flat list of top-level blocks.
//!
//! Only top-level blocks matter for plan structure, so a heading-like line
//! inside a code fence, block quote or list item can never become an
//! execution heading.

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// A top-level Markdown block with its byte range in the source.
#[derive(Debug, Clone)]
pub struct Block {
    pub range: Range<usize>,
    pub kind: BlockKind,
}

#[derive(Debug, Clone)]
pub enum BlockKind {
    Heading { level: u8, text: String },
    Paragraph(Paragraph),
    List { items: Vec<(Range<usize>, String)> },
    Code { language: String, code: String },
    Other,
}

/// A paragraph with the details the plan conventions look at.
#[derive(Debug, Clone, Default)]
pub struct Paragraph {
    /// Plain text with inline code kept in backticks.
    pub text: String,
    /// Text of a leading `**strong**` span, if the paragraph starts with one.
    pub leading_strong: Option<String>,
    /// Plain text before the first link.
    pub before_first_link: String,
    /// Destination of the first link.
    pub first_link: Option<String>,
}

pub fn blocks(source: &str) -> Vec<Block> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES;
    let mut events = Parser::new_ext(source, options)
        .into_offset_iter()
        .peekable();
    let mut blocks = Vec::new();
    while let Some((event, range)) = events.next() {
        if let Event::Start(tag) = event {
            let inner = take_until_end(&mut events);
            blocks.push(Block {
                range,
                kind: classify(&tag, &inner),
            });
        }
    }
    blocks
}

type Spanned<'a> = (Event<'a>, Range<usize>);

/// Collects the events of one block, consuming its matching end event.
fn take_until_end<'a>(events: &mut impl Iterator<Item = Spanned<'a>>) -> Vec<Spanned<'a>> {
    let mut depth = 1usize;
    let mut inner = Vec::new();
    for (event, range) in events.by_ref() {
        match &event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            break;
        }
        inner.push((event, range));
    }
    inner
}

fn classify(tag: &Tag<'_>, inner: &[Spanned<'_>]) -> BlockKind {
    match tag {
        Tag::Heading { level, .. } => BlockKind::Heading {
            level: heading_level(*level),
            text: plain_text(inner).trim().to_owned(),
        },
        Tag::Paragraph => BlockKind::Paragraph(paragraph(inner)),
        Tag::List(_) => BlockKind::List {
            items: list_items(inner),
        },
        Tag::CodeBlock(kind) => BlockKind::Code {
            language: match kind {
                CodeBlockKind::Fenced(info) => info
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
                CodeBlockKind::Indented => String::new(),
            },
            code: plain_text(inner),
        },
        _ => BlockKind::Other,
    }
}

const fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Concatenates text, keeping inline code in backticks and turning line
/// breaks into spaces.
fn plain_text(events: &[Spanned<'_>]) -> String {
    let mut text = String::new();
    for (event, _) in events {
        match event {
            Event::Text(value) => text.push_str(value),
            Event::Code(value) => {
                text.push('`');
                text.push_str(value);
                text.push('`');
            }
            Event::SoftBreak | Event::HardBreak => text.push(' '),
            _ => {}
        }
    }
    text
}

fn paragraph(inner: &[Spanned<'_>]) -> Paragraph {
    let leading_strong = match inner.first() {
        Some((Event::Start(Tag::Strong), _)) => {
            let end = inner
                .iter()
                .position(|(event, _)| matches!(event, Event::End(TagEnd::Strong)))
                .unwrap_or(inner.len());
            Some(plain_text(&inner[1..end]).trim().to_owned())
        }
        _ => None,
    };
    let link_at = inner
        .iter()
        .position(|(event, _)| matches!(event, Event::Start(Tag::Link { .. })));
    let first_link = link_at.and_then(|index| match &inner[index].0 {
        Event::Start(Tag::Link { dest_url, .. }) => Some(dest_url.to_string()),
        _ => None,
    });
    Paragraph {
        text: plain_text(inner).trim().to_owned(),
        leading_strong,
        before_first_link: plain_text(&inner[..link_at.unwrap_or(0)]),
        first_link,
    }
}

/// Top-level items of a list, with their ranges and plain text.
fn list_items(inner: &[Spanned<'_>]) -> Vec<(Range<usize>, String)> {
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut current: Option<(Range<usize>, Vec<Spanned<'_>>)> = None;
    for (event, range) in inner {
        let is_item = matches!(event, Event::Start(Tag::Item));
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth -= 1,
            _ => {}
        }
        if is_item && depth == 1 {
            current = Some((range.clone(), Vec::new()));
        } else if depth == 0 {
            if let Some((item_range, events)) = current.take() {
                items.push((item_range, plain_text(&events).trim().to_owned()));
            }
        } else if let Some((_, events)) = current.as_mut() {
            events.push((event.clone(), range.clone()));
        }
    }
    items
}
