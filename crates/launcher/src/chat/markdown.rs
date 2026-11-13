//! Splits an answer's Markdown into the blocks the chat view draws. Slint's styled text
//! handles paragraphs, lists, emphasis, inline code and links, but not headings, code
//! blocks, quotes or tables, so those become blocks of their own.
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text = 0,
    Heading = 1,
    Code = 2,
    Quote = 3,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    /// Markdown for styled text; empty for code.
    pub markdown: String,
    /// The block as plain text: the code of a code block, the source otherwise.
    pub plain: String,
    pub language: String,
}

impl Block {
    fn new(kind: Kind, markdown: &str, plain: &str) -> Self {
        Self {
            kind,
            markdown: markdown.trim().into(),
            plain: plain.trim_end().into(),
            language: String::new(),
        }
    }
}

/// One top-level element while it is read.
struct Open {
    tag: Tag<'static>,
    start: usize,
    /// Where the element's content starts and ends, e.g. a heading's text without `#`.
    inner: Option<Range<usize>>,
    code: String,
}

pub fn blocks(markdown: &str) -> Vec<Block> {
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let mut blocks = Vec::new();
    let mut depth = 0usize;
    let mut open: Option<Open> = None;
    for (event, range) in Parser::new_ext(markdown, options).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    open = Some(Open {
                        tag: tag.into_static(),
                        start: range.start,
                        inner: None,
                        code: String::new(),
                    });
                } else {
                    widen(&mut open, &range);
                }
                depth += 1;
            }
            Event::End(end) => {
                depth = depth.saturating_sub(1);
                if depth > 0 {
                    widen(&mut open, &range);
                    continue;
                }
                if let Some(element) = open.take() {
                    blocks.extend(finish(markdown, element, range.end, end));
                }
            }
            Event::Text(text) if depth > 0 => {
                if let Some(element) = &mut open {
                    element.code.push_str(&text);
                }
                widen(&mut open, &range);
            }
            _ if depth > 0 => widen(&mut open, &range),
            // A rule or stray HTML between blocks.
            _ => {}
        }
    }
    // An answer cut off mid-element (while streaming) still shows what arrived.
    if let Some(element) = open {
        let end = markdown.len();
        blocks.extend(finish(markdown, element, end, TagEnd::Paragraph));
    }
    blocks
}

fn widen(open: &mut Option<Open>, range: &Range<usize>) {
    if let Some(element) = open {
        let inner = element.inner.get_or_insert(range.clone());
        inner.start = inner.start.min(range.start);
        inner.end = inner.end.max(range.end);
    }
}

fn finish(markdown: &str, element: Open, end: usize, _: TagEnd) -> Option<Block> {
    let source = &markdown[element.start..end.min(markdown.len())];
    let inner = element
        .inner
        .clone()
        .map(|inner| &markdown[inner.start..inner.end.min(markdown.len())])
        .unwrap_or_default();
    let block = match &element.tag {
        Tag::Heading { .. } => Block::new(Kind::Heading, inner, inner),
        Tag::CodeBlock(kind) => Block {
            language: match kind {
                CodeBlockKind::Fenced(info) => {
                    info.split_whitespace().next().unwrap_or_default().into()
                }
                CodeBlockKind::Indented => String::new(),
            },
            ..Block::new(Kind::Code, "", element.code.trim_end_matches('\n'))
        },
        Tag::BlockQuote(_) => {
            let unquoted: Vec<&str> = source
                .lines()
                .map(|line| {
                    let line = line.trim_start();
                    let line = line.strip_prefix('>').unwrap_or(line);
                    line.strip_prefix(' ').unwrap_or(line)
                })
                .collect();
            let text = unquoted.join("\n");
            Block::new(Kind::Quote, &text, &text)
        }
        // Columns line up in a monospace font.
        Tag::Table(_) => Block::new(Kind::Code, "", source),
        Tag::HtmlBlock => Block::new(Kind::Text, "", source),
        _ => Block::new(Kind::Text, source, source),
    };
    (!block.plain.trim().is_empty()).then_some(block)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(markdown: &str) -> Vec<(Kind, String)> {
        blocks(markdown)
            .into_iter()
            .map(|block| {
                let text = match block.kind {
                    Kind::Code => format!("{}|{}", block.language, block.plain),
                    _ => block.markdown,
                };
                (block.kind, text)
            })
            .collect()
    }

    #[test]
    fn splits_headings_code_quotes_and_text() {
        let answer = "## Title *x*\n\nSome **bold** text\nnext line.\n\n- one\n- two\n\n```rust\nfn main() {}\n```\n\n> quoted\n> more\n\n---\n\nEnd.";
        assert_eq!(
            kinds(answer),
            [
                (Kind::Heading, "Title *x*".into()),
                (Kind::Text, "Some **bold** text\nnext line.".into()),
                (Kind::Text, "- one\n- two".into()),
                (Kind::Code, "rust|fn main() {}".into()),
                (Kind::Quote, "quoted\nmore".into()),
                (Kind::Text, "End.".into()),
            ]
        );
    }

    #[test]
    fn tables_are_monospace_and_unfinished_code_still_shows() {
        let table = "| a | b |\n|---|---|\n| 1 | 2 |";
        assert_eq!(blocks(table)[0].kind, Kind::Code);
        assert_eq!(blocks(table)[0].plain, table);
        // While streaming, the closing fence has not arrived yet.
        assert_eq!(
            kinds("Try:\n\n```py\nprint(1)\nprint("),
            [
                (Kind::Text, "Try:".into()),
                (Kind::Code, "py|print(1)\nprint(".into())
            ]
        );
        assert!(blocks("").is_empty());
    }
}
