//! Assistant markdown laid out to a width: concealed markers, wrapped paragraphs, padded list
//! markers, highlighted fences, `│` quotes, rules and grid tables.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use unicode_segmentation::UnicodeSegmentation;

use super::highlight;
use crate::text::width;
use crate::theme::Theme;

pub type Line = Vec<Span<'static>>;

/// Word-wraps styled spans to `max` columns.
pub fn wrap_spans(spans: &[Span<'static>], max: usize) -> Vec<Line> {
    let max = max.max(1);
    let mut lines: Vec<Line> = vec![Vec::new()];
    let mut used = 0;
    for span in spans {
        for (i, source) in span.content.split('\n').enumerate() {
            if i > 0 {
                lines.push(Vec::new());
                used = 0;
            }
            for word in source.split_word_bounds() {
                let w = width(word);
                if used + w > max && used > 0 {
                    if let Some(last) = lines.last_mut() {
                        trim_end(last);
                    }
                    lines.push(Vec::new());
                    used = 0;
                    if word.trim().is_empty() {
                        continue;
                    }
                }
                let mut rest = word;
                while width(rest) > max - used {
                    let take = crate::text::clip(rest, max - used);
                    if take.is_empty() {
                        break;
                    }
                    push(lines.last_mut().expect("line"), &take, span.style);
                    rest = &rest[take.len()..];
                    lines.push(Vec::new());
                    used = 0;
                }
                push(lines.last_mut().expect("line"), rest, span.style);
                used += width(rest);
            }
        }
    }
    lines
}

fn push(line: &mut Line, text: &str, style: Style) {
    if text.is_empty() {
        return;
    }
    match line.last_mut() {
        Some(last) if last.style == style => {
            let mut joined = last.content.to_string();
            joined.push_str(text);
            last.content = joined.into();
        }
        _ => line.push(Span::styled(text.to_string(), style)),
    }
}

fn trim_end(line: &mut Line) {
    while let Some(last) = line.last_mut() {
        let trimmed = last.content.trim_end().to_string();
        if trimmed.is_empty() {
            line.pop();
        } else {
            last.content = trimmed.into();
            break;
        }
    }
}

pub fn plain(line: &Line) -> String {
    line.iter().map(|s| s.content.as_ref()).collect()
}

struct List {
    next: Option<u64>,
}

struct Table {
    rows: Vec<Vec<Line>>,
    head: usize,
}

struct Builder<'t> {
    theme: &'t Theme,
    width: usize,
    out: Vec<Line>,
    inline: Line,
    styles: Vec<Style>,
    lists: Vec<List>,
    /// Marker of the list item whose first block is not written yet.
    marker: Option<String>,
    quote: usize,
    code: Option<(String, String)>,
    table: Option<Table>,
    link: Option<String>,
}

impl Builder<'_> {
    fn style(&self) -> Style {
        self.styles
            .last()
            .copied()
            .unwrap_or_else(|| Style::new().fg(self.theme.fg("markdown.text")))
    }

    fn with(&mut self, path: &str, modifier: Modifier) {
        let style = self.style().fg(self.theme.fg(path)).add_modifier(modifier);
        self.styles.push(style);
    }

    fn indent(&self) -> usize {
        self.lists.len().saturating_sub(1) * 2 + self.quote * 2
    }

    /// Starts a block: a blank line between top-level blocks.
    fn gap(&mut self) {
        if self.lists.is_empty() && self.quote == 0 && !self.out.is_empty() {
            self.out.push(Vec::new());
        }
    }

    fn emit(&mut self, lines: Vec<Line>) {
        let marker = self.marker.take();
        let marker_width = marker.as_deref().map_or(0, width);
        let list_marker_width = if self.lists.is_empty() {
            0
        } else {
            2.max(marker_width)
        };
        for (i, line) in lines.into_iter().enumerate() {
            let mut prefix: Line = Vec::new();
            let quote_style = Style::new().fg(self.theme.fg("markdown.blockQuote"));
            for _ in 0..self.quote {
                prefix.push(Span::styled("│ ", quote_style));
            }
            let pad = self.lists.len().saturating_sub(1) * 2;
            if pad > 0 {
                prefix.push(Span::raw(" ".repeat(pad)));
            }
            if !self.lists.is_empty() {
                let text = match (&marker, i) {
                    (Some(m), 0) => format!("{m:<list_marker_width$}"),
                    _ => " ".repeat(list_marker_width),
                };
                prefix.push(Span::styled(
                    text,
                    Style::new().fg(self.theme.fg("markdown.listItem")),
                ));
            }
            prefix.extend(line);
            self.out.push(prefix);
        }
    }

    fn flush_inline(&mut self) {
        let spans = std::mem::take(&mut self.inline);
        let room = self
            .width
            .saturating_sub(self.indent() + if self.lists.is_empty() { 0 } else { 2 });
        let lines = wrap_spans(&spans, room.max(1));
        self.emit(lines);
    }

    fn text(&mut self, text: &str) {
        let style = self.style();
        if let Some((_, code)) = &mut self.code {
            code.push_str(text);
            return;
        }
        if let Some(table) = &mut self.table {
            if let Some(cell) = table.rows.last_mut().and_then(|r| r.last_mut()) {
                push(cell, text, style);
            }
            return;
        }
        push(&mut self.inline, text, style);
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                if self.marker.is_none() {
                    self.gap();
                }
            }
            Tag::Heading { level, .. } => {
                self.gap();
                let modifier = if level == HeadingLevel::H1 {
                    Modifier::BOLD | Modifier::UNDERLINED
                } else {
                    Modifier::BOLD
                };
                self.with("markdown.heading", modifier);
            }
            Tag::BlockQuote(_) => {
                self.gap();
                self.quote += 1;
                self.with("markdown.blockQuote", Modifier::ITALIC);
            }
            Tag::CodeBlock(kind) => {
                if self.marker.is_none() {
                    self.gap();
                }
                let language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((language, String::new()));
            }
            Tag::List(start) => {
                if self.lists.is_empty() {
                    self.gap();
                } else if !self.inline.is_empty() {
                    self.flush_inline();
                }
                self.lists.push(List { next: start });
            }
            Tag::Item => {
                if !self.inline.is_empty() {
                    self.flush_inline();
                }
                let list = self.lists.last_mut();
                self.marker = Some(match list {
                    Some(List { next: Some(n) }) => {
                        let marker = format!("{n}.");
                        *n += 1;
                        marker
                    }
                    _ => "-".to_string(),
                });
            }
            Tag::Table(_) => {
                self.gap();
                self.table = Some(Table {
                    rows: Vec::new(),
                    head: 0,
                });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = &mut self.table {
                    table.rows.push(Vec::new());
                }
                if matches!(tag, Tag::TableHead) {
                    self.with("markdown.heading", Modifier::BOLD);
                }
            }
            Tag::TableCell => {
                if let Some(row) = self.table.as_mut().and_then(|t| t.rows.last_mut()) {
                    row.push(Vec::new());
                }
            }
            Tag::Emphasis => self.with("markdown.emphasis", Modifier::ITALIC),
            Tag::Strong => self.with("markdown.strong", Modifier::BOLD),
            Tag::Strikethrough => {
                let style = self.style().add_modifier(Modifier::CROSSED_OUT);
                self.styles.push(style);
            }
            Tag::Link { dest_url, .. } => {
                self.link = Some(dest_url.to_string());
                self.with("markdown.linkText", Modifier::UNDERLINED);
            }
            Tag::Image { .. } => self.with("markdown.imageText", Modifier::empty()),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush_inline(),
            TagEnd::Heading(_) => {
                self.flush_inline();
                self.styles.pop();
            }
            TagEnd::BlockQuote(_) => {
                if !self.inline.is_empty() {
                    self.flush_inline();
                }
                self.quote -= 1;
                self.styles.pop();
            }
            TagEnd::CodeBlock => {
                if let Some((language, code)) = self.code.take() {
                    let room = self.width.saturating_sub(self.indent()).max(1);
                    let drawn = match language.as_str() {
                        "latex" | "math" | "tex" => super::latex::render(&code).ok(),
                        "mermaid" => super::mermaid::render(&code).ok(),
                        _ => None,
                    };
                    if let Some(rows) = drawn {
                        let style = Style::new().fg(self.theme.fg("text.base"));
                        let lines = rows
                            .into_iter()
                            .map(|row| vec![Span::styled(row, style)])
                            .collect();
                        self.emit(lines);
                        return;
                    }
                    let lines =
                        highlight::highlight(code.trim_end_matches('\n'), &language, self.theme)
                            .into_iter()
                            .flat_map(|line| wrap_code(line, room))
                            .collect();
                    self.emit(lines);
                }
            }
            TagEnd::List(_) => {
                if !self.inline.is_empty() {
                    self.flush_inline();
                }
                self.lists.pop();
            }
            TagEnd::Item => {
                if !self.inline.is_empty() || self.marker.is_some() {
                    self.flush_inline();
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = &mut self.table {
                    table.head = table.rows.len();
                }
                self.styles.pop();
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    let lines = self.layout_table(table);
                    self.emit(lines);
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Image => {
                self.styles.pop();
            }
            TagEnd::Link => {
                self.styles.pop();
                if let Some(url) = self.link.take() {
                    let label: String = plain(&self.inline);
                    if !label.ends_with(&url) {
                        let style = Style::new().fg(self.theme.fg("markdown.link"));
                        push(&mut self.inline, &format!(" ({url})"), style);
                    }
                }
            }
            _ => {}
        }
    }

    /// Grid table filling the width: each column gets its natural width plus an equal share of
    /// the rest (earlier columns take the remainder); cells word-wrap when space is short.
    fn layout_table(&self, table: Table) -> Vec<Line> {
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return Vec::new();
        }
        let available = self.width.saturating_sub(self.indent() + columns + 1);
        let natural: Vec<usize> = (0..columns)
            .map(|c| {
                table
                    .rows
                    .iter()
                    .filter_map(|r| r.get(c))
                    .map(|cell| width(&plain(cell)))
                    .max()
                    .unwrap_or(0)
                    + 2
            })
            .collect();
        let total: usize = natural.iter().sum();
        let widths: Vec<usize> = if total <= available {
            let extra = available - total;
            natural
                .iter()
                .enumerate()
                .map(|(i, w)| w + extra / columns + usize::from(i < extra % columns))
                .collect()
        } else {
            natural
                .iter()
                .map(|w| (w * available / total.max(1)).max(3))
                .collect()
        };
        let border = Style::new().fg(self.theme.fg("border.base"));
        let rule = |left: &str, mid: &str, right: &str| -> Line {
            let body: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
            vec![Span::styled(
                format!("{left}{}{right}", body.join(mid)),
                border,
            )]
        };
        let mut out = vec![rule("┌", "┬", "┐")];
        for (index, row) in table.rows.iter().enumerate() {
            let cells: Vec<Vec<Line>> = widths
                .iter()
                .enumerate()
                .map(|(c, w)| {
                    let cell = row.get(c).cloned().unwrap_or_default();
                    wrap_spans(&cell, w.saturating_sub(2).max(1))
                })
                .collect();
            let height = cells.iter().map(Vec::len).max().unwrap_or(1);
            for line in 0..height {
                let mut spans: Line = vec![Span::styled("│", border)];
                for (c, w) in widths.iter().enumerate() {
                    let content = cells[c].get(line).cloned().unwrap_or_default();
                    let used = width(&plain(&content));
                    spans.push(Span::raw(" "));
                    spans.extend(content);
                    spans.push(Span::raw(" ".repeat(w.saturating_sub(used + 1))));
                    spans.push(Span::styled("│", border));
                }
                out.push(spans);
            }
            if index + 1 == table.head && table.rows.len() > table.head {
                out.push(rule("├", "┼", "┤"));
            }
        }
        out.push(rule("└", "┴", "┘"));
        out
    }
}

fn wrap_code(line: Line, room: usize) -> Vec<Line> {
    let mut out = vec![Vec::new()];
    let mut used = 0;
    for span in line {
        for grapheme in span.content.graphemes(true) {
            let w = width(grapheme);
            if used + w > room {
                out.push(Vec::new());
                used = 0;
            }
            push(out.last_mut().expect("line"), grapheme, span.style);
            used += w;
        }
    }
    out
}

/// Lays out `text`; `source` keeps the markdown markers (`session.markdown = source`).
pub fn render(text: &str, width: usize, theme: &Theme, source: bool) -> Vec<Line> {
    if source {
        let style = Style::new().fg(theme.fg("markdown.text"));
        return wrap_spans(&[Span::styled(text.to_string(), style)], width);
    }
    let mut builder = Builder {
        theme,
        width,
        out: Vec::new(),
        inline: Vec::new(),
        styles: Vec::new(),
        lists: Vec::new(),
        marker: None,
        quote: 0,
        code: None,
        table: None,
        link: None,
    };
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    for event in Parser::new_ext(text, options) {
        match event {
            Event::Start(tag) => builder.start(tag),
            Event::End(tag) => builder.end(tag),
            Event::Text(t) => builder.text(&t),
            Event::Code(code) => {
                let style = builder.style().fg(theme.fg("markdown.code"));
                builder.styles.push(style);
                builder.text(&code);
                builder.styles.pop();
            }
            Event::SoftBreak => builder.text(" "),
            Event::HardBreak => builder.text("\n"),
            Event::Rule => {
                builder.gap();
                let style = Style::new().fg(theme.fg("markdown.horizontalRule"));
                let room = width.saturating_sub(builder.indent());
                builder.emit(vec![vec![Span::styled("─".repeat(room), style)]]);
            }
            Event::Html(html) | Event::InlineHtml(html) => builder.text(&html),
            _ => {}
        }
    }
    if !builder.inline.is_empty() {
        builder.flush_inline();
    }
    builder.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Catalog, Mode};

    fn lines(text: &str, width: usize) -> Vec<String> {
        let theme = Catalog::bundled().load("anysola", Mode::Dark).unwrap();
        render(text, width, &theme, false)
            .iter()
            .map(plain)
            .collect()
    }

    #[test]
    fn math_blocks_render_or_stay_source() {
        assert_eq!(lines("```latex\nE = mc^2\n```", 40), vec!["E = mc²"]);
        let source = lines("```math\n\\begin{x}\n```", 40);
        assert!(source.iter().any(|l| l.contains("\\begin{x}")));
    }

    #[test]
    fn conceals_markers_and_pads_lists() {
        let out = lines(
            "# Fixed\n\nThe **typo** is `fixed`:\n\n- one\n- two\n\n> Run it.",
            40,
        );
        assert_eq!(
            out,
            [
                "Fixed",
                "",
                "The typo is fixed:",
                "",
                "- one",
                "- two",
                "",
                "│ Run it."
            ]
        );
    }

    #[test]
    fn tables_fill_the_width() {
        let out = lines("| file | change |\n|---|---|\n| hello.txt | 1 line |", 113);
        assert_eq!(out[0].chars().count(), 113);
        assert_eq!(out[0].chars().position(|c| c == '┬'), Some(58));
    }

    #[test]
    fn links_show_their_url() {
        assert_eq!(lines("[docs](https://x.y)", 40), ["docs (https://x.y)"]);
    }
}
