//! Markdown for message bodies: pulldown-cmark's event stream folded into the block structure
//! the apps' native text views render (a UITextView on iOS, a TextView on Android, and the same
//! shape for the Mac). The views only style what they are handed, so every screen reads a
//! message the same way.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

uniffi::setup_scaffolding!();

/// A run of text with one style.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct ListItem {
    pub blocks: Vec<Block>,
    /// A task item's box; `None` for a plain item.
    pub checked: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct Cell {
    pub spans: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct TableRow {
    pub cells: Vec<Cell>,
}

/// How a table column lines up its text, from the `:` marks of the delimiter row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Auto,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Paragraph { spans: Vec<Span> },
    Heading { level: u8, spans: Vec<Span> },
    Code { language: Option<String>, text: String },
    /// A bullet or numbered list. (`Listing`, because a `List` variant shadows Kotlin's `List`.)
    Listing { ordered: bool, start: u32, items: Vec<ListItem> },
    Quote { blocks: Vec<Block> },
    Table { alignments: Vec<Align>, header: Vec<Cell>, rows: Vec<TableRow> },
    Rule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct Document {
    pub blocks: Vec<Block>,
}

/// Parses a message body. Tables, strikethrough, and task lists are on; everything else is
/// CommonMark.
pub fn parse(text: &str) -> Document {
    parse_markdown(text.to_owned())
}

/// The same, over the FFI: what the Swift and Kotlin views call.
#[uniffi::export]
pub fn parse_markdown(text: String) -> Document {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut builder = Builder::new();
    for event in Parser::new_ext(&text, options) {
        builder.event(event);
    }
    builder.finish()
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Style {
    bold: u8,
    italic: u8,
    strike: u8,
    links: Vec<String>,
}

/// The inline leaf being filled.
enum Leaf {
    Paragraph,
    /// A tight list item's text arrives with no paragraph around it.
    Implicit,
    Heading(u8),
    Code(Option<String>, String),
    Cell,
}

struct ListBuilder {
    ordered: bool,
    start: u32,
    items: Vec<ListItem>,
    checked: Option<bool>,
}

struct TableBuilder {
    alignments: Vec<Align>,
    header: Vec<Cell>,
    rows: Vec<TableRow>,
    row: Vec<Cell>,
    in_head: bool,
}

struct Builder {
    /// Block lists being filled: the document, then each open quote or list item.
    containers: Vec<Vec<Block>>,
    lists: Vec<ListBuilder>,
    table: Option<TableBuilder>,
    leaf: Option<Leaf>,
    spans: Vec<Span>,
    style: Style,
}

impl Builder {
    fn new() -> Self {
        Builder { containers: vec![Vec::new()], lists: Vec::new(), table: None, leaf: None, spans: Vec::new(), style: Style::default() }
    }

    fn finish(mut self) -> Document {
        self.close_implicit();
        while self.containers.len() > 1 {
            let blocks = self.containers.pop().unwrap();
            self.push(Block::Quote { blocks });
        }
        Document { blocks: self.containers.pop().unwrap_or_default() }
    }

    fn top(&mut self) -> &mut Vec<Block> {
        self.containers.last_mut().expect("the document container is always open")
    }

    fn push(&mut self, block: Block) {
        self.top().push(block);
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => match &mut self.leaf {
                Some(Leaf::Code(_, buffer)) => buffer.push_str(&text),
                _ => self.text(&text, false),
            },
            Event::Code(text) => self.text(&text, true),
            Event::InlineMath(text) | Event::DisplayMath(text) => self.text(&text, true),
            Event::Html(html) => {
                self.close_implicit();
                let spans = vec![plain(html.trim_end_matches('\n'))];
                self.push(Block::Paragraph { spans });
            }
            Event::InlineHtml(html) => self.text(&html, false),
            Event::FootnoteReference(name) => self.text(&format!("[{name}]"), false),
            Event::SoftBreak | Event::HardBreak => self.text("\n", false),
            Event::Rule => {
                self.close_implicit();
                self.push(Block::Rule);
            }
            Event::TaskListMarker(checked) => {
                if let Some(list) = self.lists.last_mut() {
                    list.checked = Some(checked);
                }
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.open(Leaf::Paragraph),
            Tag::Heading { level, .. } => self.open(Leaf::Heading(level as u8)),
            Tag::CodeBlock(kind) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().map(str::to_owned),
                    CodeBlockKind::Indented => None,
                };
                self.open(Leaf::Code(language, String::new()));
            }
            Tag::BlockQuote(_) => {
                self.close_implicit();
                self.containers.push(Vec::new());
            }
            Tag::List(start) => {
                self.close_implicit();
                self.lists.push(ListBuilder { ordered: start.is_some(), start: start.unwrap_or(1).min(u32::MAX as u64) as u32, items: Vec::new(), checked: None });
            }
            Tag::Item => {
                self.close_implicit();
                self.containers.push(Vec::new());
            }
            Tag::Table(alignments) => {
                self.close_implicit();
                let alignments = alignments
                    .iter()
                    .map(|a| match a {
                        Alignment::None => Align::Auto,
                        Alignment::Left => Align::Left,
                        Alignment::Center => Align::Center,
                        Alignment::Right => Align::Right,
                    })
                    .collect();
                self.table = Some(TableBuilder { alignments, header: Vec::new(), rows: Vec::new(), row: Vec::new(), in_head: false });
            }
            Tag::TableHead => {
                if let Some(table) = &mut self.table {
                    table.in_head = true;
                }
            }
            Tag::TableRow => {}
            Tag::TableCell => self.open(Leaf::Cell),
            Tag::Emphasis => self.style.italic += 1,
            Tag::Strong => self.style.bold += 1,
            Tag::Strikethrough => self.style.strike += 1,
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => self.style.links.push(dest_url.into_string()),
            Tag::HtmlBlock | Tag::FootnoteDefinition(_) | Tag::MetadataBlock(_) | Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition | Tag::Superscript | Tag::Subscript => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock => self.close_leaf(),
            TagEnd::BlockQuote(_) => {
                self.close_implicit();
                if self.containers.len() > 1 {
                    let blocks = self.containers.pop().unwrap();
                    self.push(Block::Quote { blocks });
                }
            }
            TagEnd::Item => {
                self.close_implicit();
                if self.containers.len() > 1 {
                    let blocks = self.containers.pop().unwrap();
                    if let Some(list) = self.lists.last_mut() {
                        let checked = list.checked.take();
                        list.items.push(ListItem { blocks, checked });
                    }
                }
            }
            TagEnd::List(_) => {
                if let Some(list) = self.lists.pop() {
                    self.push(Block::Listing { ordered: list.ordered, start: list.start, items: list.items });
                }
            }
            TagEnd::TableCell => {
                let spans = self.take_spans();
                self.leaf = None;
                if let Some(table) = &mut self.table {
                    table.row.push(Cell { spans });
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = &mut self.table {
                    table.header = std::mem::take(&mut table.row);
                    table.in_head = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = &mut self.table {
                    let cells = std::mem::take(&mut table.row);
                    table.rows.push(TableRow { cells });
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.push(Block::Table { alignments: table.alignments, header: table.header, rows: table.rows });
                }
            }
            TagEnd::Emphasis => self.style.italic = self.style.italic.saturating_sub(1),
            TagEnd::Strong => self.style.bold = self.style.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.style.strike = self.style.strike.saturating_sub(1),
            TagEnd::Link | TagEnd::Image => {
                self.style.links.pop();
            }
            TagEnd::HtmlBlock | TagEnd::FootnoteDefinition | TagEnd::MetadataBlock(_) | TagEnd::DefinitionList | TagEnd::DefinitionListTitle | TagEnd::DefinitionListDefinition | TagEnd::Superscript | TagEnd::Subscript => {}
        }
    }

    fn open(&mut self, leaf: Leaf) {
        self.close_implicit();
        self.spans.clear();
        self.leaf = Some(leaf);
    }

    /// Text outside any leaf is a tight list item's line: it gets a paragraph of its own.
    fn text(&mut self, text: &str, code: bool) {
        if self.leaf.is_none() {
            self.leaf = Some(Leaf::Implicit);
        }
        let span = Span {
            text: text.to_owned(),
            bold: self.style.bold > 0,
            italic: self.style.italic > 0,
            code,
            strike: self.style.strike > 0,
            link: self.style.links.last().cloned(),
        };
        match self.spans.last_mut() {
            Some(last) if same_style(last, &span) => last.text.push_str(&span.text),
            _ => self.spans.push(span),
        }
    }

    fn close_implicit(&mut self) {
        if matches!(self.leaf, Some(Leaf::Implicit)) {
            self.close_leaf();
        }
    }

    fn close_leaf(&mut self) {
        let Some(leaf) = self.leaf.take() else { return };
        let block = match leaf {
            Leaf::Paragraph | Leaf::Implicit => Block::Paragraph { spans: self.take_spans() },
            Leaf::Heading(level) => Block::Heading { level, spans: self.take_spans() },
            Leaf::Code(language, text) => Block::Code { language, text: text.trim_end_matches('\n').to_owned() },
            Leaf::Cell => return,
        };
        self.push(block);
    }

    /// The leaf's spans, trimmed at both ends so a stray line break never opens or closes a block.
    fn take_spans(&mut self) -> Vec<Span> {
        let mut spans = std::mem::take(&mut self.spans);
        if let Some(first) = spans.first_mut() {
            first.text = first.text.trim_start().to_owned();
        }
        if let Some(last) = spans.last_mut() {
            last.text = last.text.trim_end().to_owned();
        }
        spans.retain(|span| !span.text.is_empty());
        spans
    }
}

fn same_style(a: &Span, b: &Span) -> bool {
    a.bold == b.bold && a.italic == b.italic && a.code == b.code && a.strike == b.strike && a.link == b.link
}

fn plain(text: &str) -> Span {
    Span { text: text.to_owned(), bold: false, italic: false, code: false, strike: false, link: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn paragraphs_keep_their_line_breaks_and_inline_styles() {
        let doc = parse("Hello **bold** and *it* with `code`\nsecond line\n\nNext ~~gone~~ [Docs](https://x.y)");
        assert_eq!(doc.blocks.len(), 2);
        let Block::Paragraph { spans } = &doc.blocks[0] else { panic!("{:?}", doc.blocks[0]) };
        assert_eq!(text(spans), "Hello bold and it with code\nsecond line");
        assert!(spans.iter().any(|s| s.bold && s.text == "bold"));
        assert!(spans.iter().any(|s| s.italic && s.text == "it"));
        assert!(spans.iter().any(|s| s.code && s.text == "code"));
        let Block::Paragraph { spans } = &doc.blocks[1] else { panic!() };
        assert!(spans.iter().any(|s| s.strike && s.text == "gone"));
        assert_eq!(spans.last().unwrap().link.as_deref(), Some("https://x.y"));
        assert_eq!(spans.last().unwrap().text, "Docs");
    }

    #[test]
    fn headings_code_quotes_and_rules() {
        let doc = parse("## Title\n\n```rust\nfn main() {}\n```\n\n> quoted **line**\n> more\n\n---\n");
        assert_eq!(doc.blocks.len(), 4);
        assert!(matches!(&doc.blocks[0], Block::Heading { level: 2, spans } if text(spans) == "Title"));
        assert!(matches!(&doc.blocks[1], Block::Code { language: Some(l), text } if l == "rust" && text == "fn main() {}"));
        let Block::Quote { blocks } = &doc.blocks[2] else { panic!() };
        assert!(matches!(&blocks[0], Block::Paragraph { spans } if text(spans) == "quoted line\nmore"));
        assert_eq!(doc.blocks[3], Block::Rule);
    }

    #[test]
    fn lists_tight_loose_nested_ordered_and_tasks() {
        let doc = parse("- one\n- two **b**\n  - nested\n\n3. three\n4. four\n\n- [x] done\n- [ ] todo\n");
        let Block::Listing { ordered: false, items, .. } = &doc.blocks[0] else { panic!("{:?}", doc.blocks[0]) };
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0].blocks[0], Block::Paragraph { spans } if text(spans) == "one"));
        assert!(matches!(&items[1].blocks[1], Block::Listing { items, .. } if items.len() == 1));
        let Block::Listing { ordered: true, start: 3, items, .. } = &doc.blocks[1] else { panic!("{:?}", doc.blocks[1]) };
        assert_eq!(items.len(), 2);
        let Block::Listing { items, .. } = &doc.blocks[2] else { panic!() };
        assert_eq!(items[0].checked, Some(true));
        assert_eq!(items[1].checked, Some(false));
    }

    #[test]
    fn tables() {
        let doc = parse("| a | b |\n|---|--:|\n| 1 | **2** |\n");
        let Block::Table { alignments, header, rows } = &doc.blocks[0] else { panic!("{:?}", doc.blocks[0]) };
        assert_eq!(alignments, &[Align::Auto, Align::Right]);
        assert_eq!(header.len(), 2);
        assert_eq!(text(&header[1].spans), "b");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].cells[1].spans[0].bold);
    }

    #[test]
    fn plain_text_and_empty_input_survive() {
        assert_eq!(parse("").blocks, vec![]);
        let doc = parse("just words");
        assert!(matches!(&doc.blocks[0], Block::Paragraph { spans } if text(spans) == "just words"));
    }
}
