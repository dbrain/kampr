//! Markdown. The node passes it through verbatim so that the client can render it — which is the
//! whole reason a table is still a table by the time it gets here.

use super::{Block, Laying, Piece, lay_block, pad};
use crate::theme::Theme;
use pulldown_cmark::{Alignment, CodeBlockKind, Event as Md, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

const MIN_COLUMN: usize = 3;

#[derive(Default)]
struct Grid {
    aligns: Vec<Alignment>,
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    head: usize,
}

struct Doc<'t> {
    width: usize,
    t: &'t Theme,
    out: Vec<Piece>,
    inline: Vec<Span<'static>>,
    bold: usize,
    italic: usize,
    strike: usize,
    link: usize,
    marker: Option<String>,
    lists: Vec<Option<u64>>,
    quote: usize,
    fence: Option<(Option<String>, String)>,
    grid: Option<Grid>,
    cell: Option<String>,
}

impl Doc<'_> {
    fn style(&self) -> Style {
        let mut style = Style::default().fg(self.t.text);
        if self.bold > 0 {
            style = style.add_modifier(Modifier::BOLD);
        }
        if self.italic > 0 {
            style = style.add_modifier(Modifier::ITALIC);
        }
        if self.strike > 0 {
            style = style.add_modifier(Modifier::CROSSED_OUT);
        }
        if self.link > 0 {
            style = style.fg(self.t.accent).add_modifier(Modifier::UNDERLINED);
        }
        style
    }

    fn write(&mut self, text: &str) {
        if let Some((_, body)) = self.fence.as_mut() {
            body.push_str(text);
            return;
        }
        if let Some(cell) = self.cell.as_mut() {
            cell.push_str(text);
            return;
        }
        let style = self.style();
        self.inline.push(Span::styled(text.to_string(), style));
    }

    fn indent(&self) -> String {
        format!(
            "  {}{}",
            "│ ".repeat(self.quote),
            "  ".repeat(self.lists.len().saturating_sub(1))
        )
    }

    fn flush(&mut self) {
        if self.inline.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.inline);
        let indent = self.indent();
        let (first, rest) = match self.marker.take() {
            Some(marker) => {
                let pad = " ".repeat(Span::raw(marker.as_str()).width());
                (format!("{indent}{marker}"), format!("{indent}{pad}"))
            }
            None => (indent.clone(), indent),
        };
        wrap(&spans, self.width, &first, &rest, self.t, &mut self.out);
    }

    fn rule(&mut self) {
        let width = self.width.saturating_sub(4);
        self.out.push(Piece::Line(Line::styled(
            format!("  {}", "─".repeat(width)),
            Style::default().fg(self.t.line),
        )));
    }
}

fn wrap(spans: &[Span<'static>], width: usize, first: &str, rest: &str, theme: &Theme, out: &mut Vec<Piece>) {
    let room = |prefix: &str| width.saturating_sub(Span::raw(prefix).width()).max(8);
    let ground = Style::default().fg(theme.text);
    let mut line: Vec<Span<'static>> = vec![Span::styled(first.to_string(), ground)];
    let mut used = 0usize;
    let mut limit = room(first);
    let mut opened = false;
    let push = |line: &mut Vec<Span<'static>>, out: &mut Vec<Piece>| {
        out.push(Piece::Line(Line::from(std::mem::take(line))));
    };
    for span in spans {
        for word in span.content.split_whitespace() {
            if word == "\n" {
                push(&mut line, out);
                line.push(Span::styled(rest.to_string(), ground));
                used = 0;
                limit = room(rest);
                opened = false;
                continue;
            }
            let size = Span::raw(word).width();
            let lead = usize::from(opened);
            if opened && used + lead + size > limit {
                push(&mut line, out);
                line.push(Span::styled(rest.to_string(), ground));
                used = 0;
                limit = room(rest);
                opened = false;
            }
            let text = match opened {
                true => format!(" {word}"),
                false => word.to_string(),
            };
            used += usize::from(opened) + size;
            line.push(Span::styled(text, span.style));
            opened = true;
        }
    }
    if opened {
        push(&mut line, out);
    }
}

pub fn markdown(text: &str, width: u16, theme: &Theme, out: &mut Vec<Piece>) {
    let mut doc = Doc {
        width: width as usize,
        t: theme,
        out: Vec::new(),
        inline: Vec::new(),
        bold: 0,
        italic: 0,
        strike: 0,
        link: 0,
        marker: None,
        lists: Vec::new(),
        quote: 0,
        fence: None,
        grid: None,
        cell: None,
    };
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    for event in Parser::new_ext(text, options) {
        step(&mut doc, event);
    }
    doc.flush();
    out.append(&mut doc.out);
}

fn step(doc: &mut Doc<'_>, event: Md<'_>) {
    match event {
        Md::Start(Tag::Paragraph) => {}
        Md::End(TagEnd::Paragraph) => doc.flush(),
        Md::Start(Tag::Heading { .. }) => doc.bold += 1,
        Md::End(TagEnd::Heading(_)) => {
            let style = Style::default().fg(doc.t.accent_hi).add_modifier(Modifier::BOLD);
            for span in doc.inline.iter_mut() {
                span.style = style;
            }
            doc.flush();
            doc.bold = doc.bold.saturating_sub(1);
        }
        Md::Start(Tag::BlockQuote(_)) => doc.quote += 1,
        Md::End(TagEnd::BlockQuote(_)) => doc.quote = doc.quote.saturating_sub(1),
        Md::Start(Tag::List(first)) => doc.lists.push(first),
        Md::End(TagEnd::List(_)) => {
            doc.lists.pop();
        }
        Md::Start(Tag::Item) => {
            doc.marker = Some(match doc.lists.last_mut() {
                Some(Some(n)) => {
                    let marker = format!("{n}. ");
                    *n += 1;
                    marker
                }
                _ => "• ".to_string(),
            });
        }
        Md::End(TagEnd::Item) => doc.flush(),
        Md::Start(Tag::CodeBlock(kind)) => {
            let lang = match kind {
                CodeBlockKind::Fenced(name) if !name.is_empty() => Some(name.to_string()),
                _ => None,
            };
            doc.flush();
            doc.fence = Some((lang, String::new()));
        }
        Md::End(TagEnd::CodeBlock) => {
            if let Some((lang, body)) = doc.fence.take() {
                let width = doc.width as u16;
                let theme = doc.t;
                lay_block(
                    &Block::Code {
                        lang,
                        text: body,
                        role: None,
                    },
                    &Laying::bare(width, theme),
                    None,
                    &mut doc.out,
                );
            }
        }
        Md::Start(Tag::Emphasis) => doc.italic += 1,
        Md::End(TagEnd::Emphasis) => doc.italic = doc.italic.saturating_sub(1),
        Md::Start(Tag::Strong) => doc.bold += 1,
        Md::End(TagEnd::Strong) => doc.bold = doc.bold.saturating_sub(1),
        Md::Start(Tag::Strikethrough) => doc.strike += 1,
        Md::End(TagEnd::Strikethrough) => doc.strike = doc.strike.saturating_sub(1),
        Md::Start(Tag::Link { .. }) => doc.link += 1,
        Md::End(TagEnd::Link) => doc.link = doc.link.saturating_sub(1),
        Md::Start(Tag::Image { title, .. }) => doc.write(&format!("[{title}]")),
        Md::Start(Tag::Table(aligns)) => {
            doc.flush();
            doc.grid = Some(Grid {
                aligns,
                ..Grid::default()
            });
        }
        Md::End(TagEnd::Table) => {
            if let Some(grid) = doc.grid.take() {
                table(grid, doc.width, doc.t, &mut doc.out);
            }
        }
        Md::Start(Tag::TableHead) => {}
        Md::End(TagEnd::TableHead) => {
            if let Some(grid) = doc.grid.as_mut() {
                let row = std::mem::take(&mut grid.row);
                grid.rows.push(row);
                grid.head = grid.rows.len();
            }
        }
        Md::End(TagEnd::TableRow) => {
            if let Some(grid) = doc.grid.as_mut() {
                let row = std::mem::take(&mut grid.row);
                grid.rows.push(row);
            }
        }
        Md::Start(Tag::TableCell) => doc.cell = Some(String::new()),
        Md::End(TagEnd::TableCell) => {
            if let (Some(cell), Some(grid)) = (doc.cell.take(), doc.grid.as_mut()) {
                grid.row.push(cell);
            }
        }
        Md::Text(text) | Md::Html(text) | Md::InlineHtml(text) => doc.write(&text),
        Md::Code(text) => {
            let style = Style::default().fg(doc.t.accent).bg(doc.t.surface);
            match (doc.fence.is_some(), doc.cell.as_mut()) {
                (true, _) => {}
                (false, Some(cell)) => cell.push_str(&text),
                (false, None) => doc.inline.push(Span::styled(text.to_string(), style)),
            }
        }
        Md::SoftBreak => doc.write(" "),
        Md::HardBreak => doc.write(" \n "),
        Md::Rule => {
            doc.flush();
            doc.rule();
        }
        _ => {}
    }
}

fn table(grid: Grid, width: usize, theme: &Theme, out: &mut Vec<Piece>) {
    let columns = grid.rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return;
    }
    let mut widths: Vec<usize> = (0..columns)
        .map(|column| {
            grid.rows
                .iter()
                .filter_map(|row| row.get(column))
                .map(|cell| Span::raw(cell.as_str()).width())
                .max()
                .unwrap_or(1)
                .max(1)
        })
        .collect();
    let room = width.saturating_sub(2);
    let frame = |widths: &[usize]| widths.iter().sum::<usize>() + 3 * widths.len() + 1;
    while frame(&widths) > room {
        let Some(widest) = widths
            .iter()
            .enumerate()
            .filter(|(_, w)| **w > MIN_COLUMN)
            .max_by_key(|(_, w)| **w)
            .map(|(i, _)| i)
        else {
            break;
        };
        widths[widest] -= 1;
    }
    let border = Style::default().fg(theme.line);
    let rule = |left: &str, mid: &str, right: &str| {
        let bars: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
        Line::styled(
            format!("  {left}{}{right}", bars.join(mid)),
            Style::default().fg(theme.line),
        )
    };
    out.push(Piece::Line(rule("┌", "┬", "┐")));
    for (index, row) in grid.rows.iter().enumerate() {
        if index == grid.head && grid.head > 0 {
            out.push(Piece::Line(rule("├", "┼", "┤")));
        }
        let ink = match index < grid.head {
            true => Style::default().fg(theme.accent_hi).add_modifier(Modifier::BOLD),
            false => Style::default().fg(theme.text),
        };
        let mut spans = vec![Span::styled("  │".to_string(), border)];
        for (column, size) in widths.iter().enumerate() {
            let cell = row.get(column).map(String::as_str).unwrap_or("");
            let align = grid.aligns.get(column).copied().unwrap_or(Alignment::None);
            spans.push(Span::styled(format!(" {} ", pad(cell, *size, align)), ink));
            spans.push(Span::styled("│".to_string(), border));
        }
        out.push(Piece::Line(Line::from(spans)));
    }
    out.push(Piece::Line(rule("└", "┴", "┘")));
}
