//! Turns an answer's Markdown into the blocks the chat view draws: paragraphs, lists,
//! headings, code, quotes and tables. Every block is plain text, so it can be selected and
//! copied; list markers stay (beside their items), emphasis and link targets do not.
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text = 0,
    Heading = 1,
    Code = 2,
    Quote = 3,
    Table = 4,
    List = 5,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    /// What the block shows; the code of a code block. Empty for a table.
    pub text: String,
    /// A code block's language, e.g. "rust".
    pub language: String,
    /// A table's rows, the header first; a list's items as `[marker, text]`.
    pub rows: Vec<Vec<String>>,
}

impl Block {
    fn new(kind: Kind, text: String) -> Self {
        Self {
            kind,
            text: text.trim_end().to_owned(),
            language: String::new(),
            rows: Vec::new(),
        }
    }
}

fn options() -> Options {
    Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS
}

pub fn blocks(markdown: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut depth = 0usize;
    let mut open: Option<(Tag<'static>, usize)> = None;
    for (event, range) in Parser::new_ext(markdown, options()).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    open = Some((tag.into_static(), range.start));
                }
                depth += 1;
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0
                    && let Some((tag, start)) = open.take()
                {
                    blocks.extend(finish(
                        &tag,
                        &markdown[start..range.end.min(markdown.len())],
                    ));
                }
            }
            // A rule or stray HTML between blocks.
            _ => {}
        }
    }
    blocks
}

fn finish(tag: &Tag, source: &str) -> Option<Block> {
    let block = match tag {
        Tag::Heading { .. } => Block::new(Kind::Heading, plain(source)),
        Tag::CodeBlock(kind) => Block {
            language: match kind {
                CodeBlockKind::Fenced(info) => {
                    info.split_whitespace().next().unwrap_or_default().into()
                }
                CodeBlockKind::Indented => String::new(),
            },
            ..Block::new(Kind::Code, code(source))
        },
        Tag::BlockQuote(_) => Block::new(Kind::Quote, plain(source)),
        Tag::Table(_) => Block {
            rows: table(source),
            ..Block::new(Kind::Table, String::new())
        },
        Tag::List(start) => Block {
            rows: list(source, *start),
            ..Block::new(Kind::List, String::new())
        },
        Tag::HtmlBlock => Block::new(Kind::Text, source.to_owned()),
        _ => Block::new(Kind::Text, plain(source)),
    };
    (!block.text.trim().is_empty() || !block.rows.is_empty()).then_some(block)
}

/// The code of a code block, without its fences.
fn code(source: &str) -> String {
    let mut code = String::new();
    for event in Parser::new_ext(source, options()) {
        if let Event::Text(text) = event {
            code.push_str(&text);
        }
    }
    code.trim_end_matches('\n').to_owned()
}

/// The text of a paragraph, list, heading or quote: line breaks kept, list items marked
/// with "•" or their number and indented by level.
fn plain(source: &str) -> String {
    let mut text = String::new();
    // Each open list's next number; `None` for bullets.
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut paragraphs = 0;
    let new_line = |text: &mut String| {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
    };
    for event in Parser::new_ext(source, options()) {
        match event {
            Event::Start(Tag::List(start)) => {
                new_line(&mut text);
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                new_line(&mut text);
                text.push_str(&"  ".repeat(lists.len().saturating_sub(1)));
                match lists.last_mut() {
                    Some(Some(number)) => {
                        text.push_str(&format!("{number}. "));
                        *number += 1;
                    }
                    _ => text.push_str("• "),
                }
            }
            Event::Start(Tag::Paragraph) => {
                if lists.is_empty() && paragraphs > 0 {
                    new_line(&mut text);
                    text.push('\n');
                } else if !lists.is_empty() && !text.ends_with(". ") && !text.ends_with("• ") {
                    new_line(&mut text);
                }
                paragraphs += 1;
            }
            Event::Start(Tag::CodeBlock(_)) => new_line(&mut text),
            Event::Text(part) | Event::Code(part) | Event::InlineHtml(part) | Event::Html(part) => {
                text.push_str(&part)
            }
            Event::SoftBreak | Event::HardBreak => text.push('\n'),
            Event::TaskListMarker(done) => text.push_str(if done { "☑ " } else { "☐ " }),
            Event::Rule => {
                new_line(&mut text);
                text.push_str("———");
            }
            _ => {}
        }
    }
    text.trim().to_owned()
}

/// A list's top-level items, each as its marker ("•" or its number) and its text, so the
/// view can space them apart and indent wrapped lines under the text. Nested lists stay in
/// their item's text, marked and indented as [`plain`] does.
fn list(source: &str, start: Option<u64>) -> Vec<Vec<String>> {
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut item_start = 0;
    for (event, range) in Parser::new_ext(source, options()).into_offset_iter() {
        match event {
            Event::Start(Tag::List(_)) => depth += 1,
            Event::End(TagEnd::List(_)) => depth = depth.saturating_sub(1),
            Event::Start(Tag::Item) if depth == 1 => item_start = range.start,
            Event::End(TagEnd::Item) if depth == 1 => {
                let marker = match start {
                    Some(first) => format!("{}.", first + items.len() as u64),
                    None => "•".to_owned(),
                };
                let text = plain(&source[item_start..range.end.min(source.len())]);
                // An item that starts with a nested list has nothing on its first line.
                let text = without_marker(&text).trim_start_matches('\n');
                items.push(vec![marker, text.to_owned()]);
            }
            _ => {}
        }
    }
    items
}

/// An item's text without the "• " or "3. " that [`plain`] put in front of it.
fn without_marker(text: &str) -> &str {
    if let Some(rest) = text.strip_prefix("• ") {
        return rest;
    }
    let digits = text.len() - text.trim_start_matches(|ch: char| ch.is_ascii_digit()).len();
    match text[digits..].strip_prefix(". ") {
        Some(rest) if digits > 0 => rest,
        _ => text,
    }
}

/// A table's cells as plain text, row by row.
fn table(source: &str) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut cell: Option<String> = None;
    for event in Parser::new_ext(source, options()) {
        match event {
            Event::Start(Tag::TableHead | Tag::TableRow) => rows.push(Vec::new()),
            Event::Start(Tag::TableCell) => cell = Some(String::new()),
            Event::End(TagEnd::TableCell) => {
                if let (Some(row), Some(text)) = (rows.last_mut(), cell.take()) {
                    row.push(text.trim().to_owned());
                }
            }
            Event::Text(part) | Event::Code(part) | Event::InlineHtml(part) => {
                if let Some(cell) = &mut cell {
                    cell.push_str(&part);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(cell) = &mut cell {
                    cell.push(' ');
                }
            }
            _ => {}
        }
    }
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    for row in &mut rows {
        row.resize(columns, String::new());
    }
    rows
}

/// How wide each column should be, as shares of the table: by its longest cell (wide
/// characters count double), within limits so short columns stay readable.
pub fn column_shares(rows: &[Vec<String>]) -> Vec<f32> {
    let columns = rows.first().map_or(0, Vec::len);
    let widths: Vec<f32> = (0..columns)
        .map(|column| {
            rows.iter()
                .map(|row| {
                    row[column]
                        .chars()
                        .map(|ch| if ch.is_ascii() { 1.0 } else { 2.0 })
                        .sum::<f32>()
                })
                .fold(0.0, f32::max)
                .clamp(4.0, 40.0)
        })
        .collect();
    let total: f32 = widths.iter().sum();
    widths.iter().map(|width| width / total).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(markdown: &str) -> Vec<(Kind, String)> {
        blocks(markdown)
            .into_iter()
            .map(|block| match block.kind {
                Kind::Code => (block.kind, format!("{}|{}", block.language, block.text)),
                Kind::List => (block.kind, items(&block)),
                _ => (block.kind, block.text),
            })
            .collect()
    }

    /// A list's items as "marker|text", one per line.
    fn items(block: &Block) -> String {
        let items: Vec<String> = block.rows.iter().map(|item| item.join("|")).collect();
        items.join("\n")
    }

    #[test]
    fn splits_headings_code_quotes_and_text_as_plain_text() {
        let answer = "## Title *x*\n\nSome **bold** `code` and [a link](https://x.y)\nnext line.\n\n- one\n- two\n  1. sub\n  2. sub two\n\n```rust\nfn main() {}\n```\n\n> quoted\n> more\n\n---\n\nEnd.";
        assert_eq!(
            kinds(answer),
            [
                (Kind::Heading, "Title x".into()),
                (Kind::Text, "Some bold code and a link\nnext line.".into()),
                (Kind::List, "•|one\n•|two\n  1. sub\n  2. sub two".into()),
                (Kind::Code, "rust|fn main() {}".into()),
                (Kind::Quote, "quoted\nmore".into()),
                (Kind::Text, "End.".into()),
            ]
        );
    }

    #[test]
    fn numbered_lists_keep_their_start_and_tasks_their_boxes() {
        assert_eq!(kinds("3. c\n4. d")[0].1, "3.|c\n4.|d");
        assert_eq!(kinds("1. a\n1. b")[0].1, "1.|a\n2.|b");
        assert_eq!(kinds("- [x] done\n- [ ] todo")[0].1, "•|☑ done\n•|☐ todo");
        // A loose list keeps each item's paragraphs on their own lines.
        assert_eq!(kinds("- **a**\n\n  more\n\n- b")[0].1, "•|a\nmore\n•|b");
    }

    #[test]
    fn tables_become_rows_of_cells_and_unfinished_code_still_shows() {
        let table = "| Name | 说明 |\n|---|---|\n| **a** | 第一 |\n| b |";
        let blocks = blocks(table);
        assert_eq!(blocks[0].kind, Kind::Table);
        assert_eq!(
            blocks[0].rows,
            [["Name", "说明"], ["a", "第一"], ["b", ""]].map(|row| row.map(String::from).to_vec())
        );
        let shares = column_shares(&blocks[0].rows);
        assert!((shares.iter().sum::<f32>() - 1.0).abs() < 0.001);
        // While streaming, the closing fence has not arrived yet.
        assert_eq!(
            kinds("Try:\n\n```py\nprint(1)\nprint("),
            [
                (Kind::Text, "Try:".into()),
                (Kind::Code, "py|print(1)\nprint(".into())
            ]
        );
        assert!(super::blocks("").is_empty());
    }
}
