//! The `:` command line — parsing typed commands and the embedded
//! documents they open.

use std::fmt;
use std::sync::OnceLock;

/// A parsed `:` command.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Open(Doc),
    Quit,
}

/// The input the prompt could not parse; displays as the offending
/// text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UnknownCommand(String);

impl fmt::Display for UnknownCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Command {
    /// Parses a non-empty prompt line. Callers decide what an empty
    /// line means (vim closes the prompt silently).
    pub fn parse(input: &str) -> Result<Self, UnknownCommand> {
        match input.trim() {
            "help" => Ok(Self::Open(Doc::Help)),
            "readme" => Ok(Self::Open(Doc::Readme)),
            "q" | "quit" => Ok(Self::Quit),
            other => Err(UnknownCommand(other.to_owned())),
        }
    }
}

/// A document embedded in the binary, viewable with `:help` /
/// `:readme`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Doc {
    Help,
    Readme,
}

impl Doc {
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Help => " Help ",
            Self::Readme => " README ",
        }
    }

    /// The document's markdown, preprocessed so the viewer shows the
    /// authored ~72-column line layout instead of parser-joined
    /// paragraphs (the viewer renders without wrapping, keeping
    /// tables and scroll math exact).
    #[must_use]
    pub fn markdown(self) -> &'static str {
        static HELP: OnceLock<String> = OnceLock::new();
        static README: OnceLock<String> = OnceLock::new();
        let (cell, source) = match self {
            Self::Help => (&HELP, include_str!("../help.md")),
            Self::Readme => (&README, include_str!("../../../README.md")),
        };
        cell.get_or_init(|| preserve_line_breaks(source))
    }
}

/// Turns every soft line break outside a code fence into a markdown
/// hard break (trailing double space), so pulldown-cmark stops
/// joining hand-wrapped paragraph lines. Fenced code keeps its exact
/// bytes; trailing spaces are inert on tables, headings, and fences.
fn preserve_line_breaks(markdown: &str) -> String {
    let mut in_fence = false;
    let mut out = String::with_capacity(markdown.len() + markdown.len() / 16);
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        out.push_str(line);
        if !in_fence && !line.is_empty() {
            out.push_str("  ");
        }
        out.push('\n');
    }
    out
}

/// An open document plus its scroll position, clamped so the last
/// line can reach the top of the viewport but never scroll past it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DocView {
    pub doc: Doc,
    pub scroll: u16,
    line_count: u16,
}

/// One PgUp/PgDn stride. The viewer doesn't know the terminal
/// height, so this is a fixed stride rather than a viewport page.
const PAGE_LINES: u16 = 15;

impl DocView {
    #[must_use]
    pub fn open(doc: Doc) -> Self {
        let line_count = u16::try_from(tui_markdown::from_str(doc.markdown()).lines.len())
            .expect("embedded docs are far shorter than 65k rendered lines");
        Self {
            doc,
            scroll: 0,
            line_count,
        }
    }

    pub fn scroll_lines(&mut self, delta: i16) {
        self.scroll = self
            .scroll
            .saturating_add_signed(delta)
            .min(self.max_scroll());
    }

    pub fn scroll_pages(&mut self, delta: i16) {
        self.scroll_lines(delta.saturating_mul(i16::try_from(PAGE_LINES).expect("small constant")));
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll = self.max_scroll();
    }

    fn max_scroll(&self) -> u16 {
        self.line_count.saturating_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_parse_with_synonyms_and_trimmed_whitespace() {
        assert_eq!(Command::parse("help"), Ok(Command::Open(Doc::Help)));
        assert_eq!(Command::parse(" readme "), Ok(Command::Open(Doc::Readme)));
        assert_eq!(Command::parse("q"), Ok(Command::Quit));
        assert_eq!(Command::parse("quit"), Ok(Command::Quit));
        let unknown = Command::parse("frobnicate").unwrap_err();
        assert_eq!(unknown.to_string(), "frobnicate");
    }

    #[test]
    fn embedded_docs_render_to_nonempty_markdown() {
        for doc in [Doc::Help, Doc::Readme] {
            assert!(!tui_markdown::from_str(doc.markdown()).lines.is_empty());
        }
    }

    #[test]
    fn preprocessing_preserves_authored_breaks_outside_code_fences() {
        let source = "para line one\npara line two\n\n```\ncode stays\nverbatim\n```\n";
        let processed = preserve_line_breaks(source);
        assert!(processed.contains("para line one  \n"));
        assert!(processed.contains("code stays\n"));
        assert!(!processed.contains("code stays  "));

        let rendered: Vec<String> = tui_markdown::from_str(&processed)
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        assert!(rendered.contains(&"para line one".to_owned()));
        assert!(rendered.contains(&"para line two".to_owned()));
    }

    #[test]
    fn rendered_docs_stay_narrow_enough_for_a_terminal() {
        use ratatui::text::Line;
        for doc in [Doc::Help, Doc::Readme] {
            let text = tui_markdown::from_str(doc.markdown());
            let widest = text.lines.iter().max_by_key(|line| line.width());
            let width = widest.map_or(0, Line::width);
            assert!(
                width <= 100,
                "{doc:?} renders a {width}-column line; re-wrap the source:\n{}",
                widest
                    .map(|line| line
                        .spans
                        .iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>())
                    .unwrap_or_default()
            );
        }
    }

    #[test]
    fn scrolling_clamps_to_both_ends() {
        let mut view = DocView::open(Doc::Readme);
        assert_eq!(view.scroll, 0);

        view.scroll_lines(-1);
        assert_eq!(view.scroll, 0, "cannot scroll above the top");

        view.scroll_lines(3);
        assert_eq!(view.scroll, 3);
        view.scroll_pages(1);
        assert_eq!(view.scroll, 3 + PAGE_LINES);

        view.scroll_to_bottom();
        let bottom = view.scroll;
        assert!(bottom > 0);
        view.scroll_lines(1);
        assert_eq!(view.scroll, bottom, "cannot scroll past the last line");

        view.scroll_to_top();
        assert_eq!(view.scroll, 0);
    }
}
