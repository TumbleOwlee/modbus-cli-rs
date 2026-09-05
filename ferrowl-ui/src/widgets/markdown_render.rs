use std::collections::HashSet;

use ferrowl_syntax::markdown::{BlockKind, BlockLine, InlineKind, InlineSpan, escape_markers};
use ferrowl_syntax::{Language, LineState, highlight_line};
use ratatui::style::{Modifier, Style};

use crate::style::{MarkdownTheme, SyntaxTheme};

/// One source line rendered to display cells, before wrapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RenderedLine {
    /// Styled runs of the rendered text, markers already hidden.
    pub spans: Vec<(String, Style)>,
    /// Display columns continuation rows are indented by (UI-R-132).
    pub hanging_indent: usize,
    /// Fence delimiter/body: wrap at a character boundary, no hanging indent (UI-R-133).
    pub char_wrap: bool,
    /// Horizontal rule: fill the text width, after the gutter when enabled, with the rule
    /// style (UI-R-140, UI-R-148).
    pub rule: bool,
}

/// Renders one classified source line. `carry` is the syntax-highlighter state threaded
/// through a `lua`/`json` fence body (UI-R-151); it is returned updated.
pub(crate) fn render_line(
    block: &BlockLine,
    source: &str,
    fence_info: Option<&str>,
    carry: LineState,
    md: &MarkdownTheme,
    syntax: &SyntaxTheme,
    base: Style,
) -> (RenderedLine, LineState) {
    let chars: Vec<char> = source.chars().collect();

    match &block.kind {
        BlockKind::Paragraph => {
            let mut spans = inline_render(&chars, 0, &block.inline, md, base);
            if spans.is_empty() {
                spans.push((String::new(), base));
            }
            (
                RenderedLine {
                    spans,
                    hanging_indent: 0,
                    char_wrap: false,
                    rule: false,
                },
                carry,
            )
        }
        BlockKind::Heading { level } => {
            let style = md.heading(*level).add_modifier(Modifier::BOLD);
            let mut spans = inline_render(&chars, block.content_start, &block.inline, md, style);
            if spans.is_empty() {
                spans.push((String::new(), style));
            }
            (
                RenderedLine {
                    spans,
                    hanging_indent: 0,
                    char_wrap: false,
                    rule: false,
                },
                carry,
            )
        }
        BlockKind::UnorderedItem { .. } => {
            let leading = leading_spaces(&chars);
            let mut spans = Vec::new();
            let mut hanging_indent = 0usize;
            if leading > 0 {
                spans.push((" ".repeat(leading), base));
                hanging_indent += leading;
            }
            spans.push(("•".to_string(), md.bullet));
            spans.push((" ".to_string(), base));
            hanging_indent += 2;
            spans.extend(inline_render(
                &chars,
                block.content_start,
                &block.inline,
                md,
                base,
            ));
            (
                RenderedLine {
                    spans,
                    hanging_indent,
                    char_wrap: false,
                    rule: false,
                },
                carry,
            )
        }
        BlockKind::OrderedItem { .. } => {
            let prefix: String = chars[..block.content_start].iter().collect();
            let hanging_indent = prefix.chars().count();
            let mut spans = vec![(prefix, base)];
            spans.extend(inline_render(
                &chars,
                block.content_start,
                &block.inline,
                md,
                base,
            ));
            (
                RenderedLine {
                    spans,
                    hanging_indent,
                    char_wrap: false,
                    rule: false,
                },
                carry,
            )
        }
        BlockKind::TaskItem { checked, .. } => {
            let leading = leading_spaces(&chars);
            let box_ch = if *checked { "☑" } else { "☐" };
            let mut spans = Vec::new();
            let mut hanging_indent = 0usize;
            if leading > 0 {
                spans.push((" ".repeat(leading), base));
                hanging_indent += leading;
            }
            spans.push((box_ch.to_string(), md.bullet));
            spans.push((" ".to_string(), base));
            hanging_indent += 2;
            spans.extend(inline_render(
                &chars,
                block.content_start,
                &block.inline,
                md,
                base,
            ));
            (
                RenderedLine {
                    spans,
                    hanging_indent,
                    char_wrap: false,
                    rule: false,
                },
                carry,
            )
        }
        BlockKind::Quote { depth } => {
            let mut spans = Vec::new();
            for d in 1..=*depth {
                spans.push(("▎".to_string(), md.quote_bar(d)));
            }
            let hanging_indent = *depth;
            spans.extend(inline_render(
                &chars,
                block.content_start,
                &block.inline,
                md,
                md.quote_text()
                    .add_modifier(Modifier::DIM | Modifier::ITALIC),
            ));
            (
                RenderedLine {
                    spans,
                    hanging_indent,
                    char_wrap: false,
                    rule: false,
                },
                carry,
            )
        }
        BlockKind::Rule => (
            RenderedLine {
                spans: vec![(String::new(), *md.rule())],
                hanging_indent: 0,
                char_wrap: false,
                rule: true,
            },
            carry,
        ),
        BlockKind::FenceDelimiter { .. } => (
            RenderedLine {
                spans: vec![(String::new(), *md.code())],
                hanging_indent: 0,
                char_wrap: true,
                rule: false,
            },
            carry,
        ),
        BlockKind::FenceBody => render_fence_body(&chars, source, fence_info, carry, md, syntax),
    }
}

fn render_fence_body(
    chars: &[char],
    source: &str,
    fence_info: Option<&str>,
    carry: LineState,
    md: &MarkdownTheme,
    syntax: &SyntaxTheme,
) -> (RenderedLine, LineState) {
    let lang = match fence_info {
        Some("lua") => Some(Language::Lua),
        Some("json") => Some(Language::Json),
        _ => None,
    };
    let Some(lang) = lang else {
        return (
            RenderedLine {
                spans: vec![(source.to_string(), *md.code())],
                hanging_indent: 0,
                char_wrap: true,
                rule: false,
            },
            carry,
        );
    };
    let (hl_spans, next_carry) = highlight_line(lang, source, carry);
    let mut spans = Vec::new();
    let mut prev = 0usize;
    for (start, end, kind) in hl_spans {
        if start > prev {
            spans.push((chars[prev..start].iter().collect(), *md.code()));
        }
        spans.push((chars[start..end].iter().collect(), syntax.style(kind)));
        prev = end;
    }
    if prev < chars.len() {
        spans.push((chars[prev..].iter().collect(), *md.code()));
    }
    (
        RenderedLine {
            spans,
            hanging_indent: 0,
            char_wrap: true,
            rule: false,
        },
        next_carry,
    )
}

fn leading_spaces(chars: &[char]) -> usize {
    chars.iter().take_while(|c| **c == ' ').count()
}

fn inline_render(
    chars: &[char],
    from: usize,
    inline: &[InlineSpan],
    md: &MarkdownTheme,
    base: Style,
) -> Vec<(String, Style)> {
    let len = chars.len();
    if from >= len {
        return Vec::new();
    }
    let text: String = chars[from..].iter().collect();
    let mut hidden: HashSet<usize> = escape_markers(&text)
        .into_iter()
        .map(|i| i + from)
        .collect();
    for s in inline {
        for (a, b) in &s.markers {
            for p in *a..*b {
                hidden.insert(p);
            }
        }
    }

    let mut style_at: Vec<Option<Style>> = vec![None; len - from];
    for s in inline {
        let style = match s.kind {
            InlineKind::Bold => base.add_modifier(Modifier::BOLD),
            InlineKind::Italic => base.add_modifier(Modifier::ITALIC),
            InlineKind::Code => *md.code(),
            InlineKind::Strike => base.add_modifier(Modifier::CROSSED_OUT),
            InlineKind::Link => md.link().add_modifier(Modifier::UNDERLINED),
            InlineKind::Image => *md.image(),
        };
        for p in s.content.0..s.content.1 {
            if p >= from && p < len {
                style_at[p - from] = Some(match style_at[p - from] {
                    Some(existing) => existing.patch(style),
                    None => style,
                });
            }
        }
    }

    let mut spans = Vec::new();
    let mut i = from;
    while i < len {
        if hidden.contains(&i) {
            i += 1;
            continue;
        }
        let style = style_at[i - from].unwrap_or(base);
        let start = i;
        while i < len && !hidden.contains(&i) && style_at[i - from].unwrap_or(base) == style {
            i += 1;
        }
        spans.push((chars[start..i].iter().collect(), style));
    }
    spans
}

/// Wraps one rendered line to `width` display columns, returning its display rows
/// (never empty: a blank line is one empty row). Wrapping is always on and content never
/// overflows horizontally (UI-R-130).
pub(crate) fn wrap_line(line: &RenderedLine, width: usize) -> Vec<Vec<(String, Style)>> {
    if width == 0 {
        return vec![Vec::new()];
    }
    if line.rule {
        let style = line.spans.first().map_or_else(Style::default, |(_, s)| *s);
        return vec![vec![("─".repeat(width), style)]];
    }

    let chars: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|(s, style)| s.chars().map(move |c| (c, *style)))
        .collect();

    if chars.is_empty() {
        return vec![Vec::new()];
    }

    if line.char_wrap {
        return chunk_by_char(&chars, width);
    }

    let indent = if line.hanging_indent >= width.saturating_sub(1) {
        0
    } else {
        line.hanging_indent
    };
    word_wrap(&chars, width, indent)
}

fn compress_runs(chars: &[(char, Style)]) -> Vec<(String, Style)> {
    let mut out: Vec<(String, Style)> = Vec::new();
    for &(c, style) in chars {
        if let Some(last) = out.last_mut()
            && last.1 == style
        {
            last.0.push(c);
            continue;
        }
        out.push((c.to_string(), style));
    }
    out
}

fn chunk_by_char(chars: &[(char, Style)], width: usize) -> Vec<Vec<(String, Style)>> {
    let mut rows = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let end = (i + width).min(chars.len());
        rows.push(compress_runs(&chars[i..end]));
        i = end;
    }
    rows
}

fn tokenize(chars: &[(char, Style)]) -> Vec<Vec<(char, Style)>> {
    let mut tokens = Vec::new();
    let mut cur: Vec<(char, Style)> = Vec::new();
    let mut cur_is_space: Option<bool> = None;
    for &(c, style) in chars {
        let is_space = c == ' ';
        if cur_is_space.is_none() || cur_is_space == Some(is_space) {
            cur.push((c, style));
        } else {
            tokens.push(std::mem::take(&mut cur));
            cur.push((c, style));
        }
        cur_is_space = Some(is_space);
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn word_wrap(chars: &[(char, Style)], width: usize, indent: usize) -> Vec<Vec<(String, Style)>> {
    let tokens = tokenize(chars);
    let mut rows: Vec<Vec<(char, Style)>> = Vec::new();
    let mut cur: Vec<(char, Style)> = Vec::new();

    let cap_for = |row_index: usize| -> usize {
        if row_index == 0 {
            width
        } else {
            width.saturating_sub(indent)
        }
        .max(1)
    };

    for token in tokens {
        let is_space_token = token.first().is_some_and(|(c, _)| *c == ' ');
        let cap = cap_for(rows.len());
        if cur.len() + token.len() <= cap {
            cur.extend(token);
            continue;
        }
        if is_space_token && !cur.is_empty() {
            rows.push(std::mem::take(&mut cur));
            continue;
        }
        if is_space_token {
            let mut remaining = token.as_slice();
            loop {
                let cap = cap_for(rows.len());
                if remaining.len() <= cap {
                    cur.extend(remaining.iter().copied());
                    break;
                }
                let (head, tail) = remaining.split_at(cap.max(1));
                rows.push(head.to_vec());
                remaining = tail;
            }
            continue;
        }
        if !cur.is_empty() {
            while cur.last().is_some_and(|(c, _)| *c == ' ') {
                cur.pop();
            }
            rows.push(std::mem::take(&mut cur));
        }
        let mut remaining = token.as_slice();
        loop {
            let cap = cap_for(rows.len());
            if remaining.len() <= cap {
                cur.extend(remaining.iter().copied());
                break;
            }
            let (head, tail) = remaining.split_at(cap.max(1));
            rows.push(head.to_vec());
            remaining = tail;
        }
    }
    if !cur.is_empty() {
        rows.push(cur);
    }

    rows.into_iter()
        .enumerate()
        .map(|(i, row)| {
            let mut r = Vec::new();
            if i > 0 && indent > 0 {
                let base = row.first().map_or_else(Style::default, |(_, s)| *s);
                r.push((" ".repeat(indent), base));
            }
            r.extend(compress_runs(&row));
            r
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::MarkdownThemeBuilder;
    use ferrowl_syntax::markdown::{BlockState, block_line};

    fn render(line: &str) -> RenderedLine {
        let (block, _) = block_line(line, &BlockState::default());
        let md = MarkdownTheme::default();
        let syntax = SyntaxTheme::default();
        render_line(
            &block,
            line,
            None,
            LineState::default(),
            &md,
            &syntax,
            Style::default(),
        )
        .0
    }

    fn text(rl: &RenderedLine) -> String {
        rl.spans.iter().map(|(t, _)| t.as_str()).collect()
    }

    #[test]
    /// UI-E-092 — `***x***` renders with bold and italic merged over the same text, not one
    /// style overwriting the other.
    fn ut_triple_marker_merges_bold_and_italic_over_the_same_text() {
        let rl = render("***x***");
        assert_eq!(text(&rl), "x");
        let style = rl.spans[0].1;
        assert!(
            style.add_modifier.contains(Modifier::BOLD),
            "expected bold in {style:?}"
        );
        assert!(
            style.add_modifier.contains(Modifier::ITALIC),
            "expected italic in {style:?}"
        );
    }

    #[test]
    /// UI-R-142 — every classified source line renders to exactly one `RenderedLine`.
    fn ut_each_source_line_renders_to_exactly_one_line() {
        let lines = ["first paragraph", "second paragraph", "third paragraph"];
        let md = MarkdownTheme::default();
        let syntax = SyntaxTheme::default();
        let mut state = BlockState::default();
        let mut carry = LineState::default();
        let mut rendered = Vec::new();
        for line in lines {
            let (block, next_state) = block_line(line, &state);
            state = next_state;
            let (rl, next_carry) =
                render_line(&block, line, None, carry, &md, &syntax, Style::default());
            carry = next_carry;
            rendered.push(rl);
        }
        assert_eq!(rendered.len(), lines.len());
        for (rl, line) in rendered.iter().zip(lines.iter()) {
            let t = text(rl);
            assert!(
                !t.contains('\n'),
                "rendered line joined multiple source lines: {t:?}"
            );
            assert_eq!(t, *line, "line {line:?} was not preserved 1:1");
        }
    }

    #[test]
    /// UI-R-143 — a heading hides its `#` markers and draws the text bold in the level's style.
    fn ut_heading_hides_markers_and_uses_level_style_bold() {
        let md = MarkdownTheme::default();
        let rl = render("### Title");
        assert_eq!(text(&rl), "Title");
        assert_eq!(rl.spans[0].1, md.heading(3));

        let bare = MarkdownThemeBuilder::default()
            .heading([Style::default(); 6])
            .build()
            .expect("builder");
        let (block, _) = block_line("### Title", &BlockState::default());
        let (rl, _) = render_line(
            &block,
            "### Title",
            None,
            LineState::default(),
            &bare,
            &SyntaxTheme::default(),
            Style::default(),
        );
        assert!(
            rl.spans[0].1.add_modifier.contains(Modifier::BOLD),
            "heading must be bold even when the theme style carries no modifier"
        );
    }

    #[test]
    /// UI-R-144 — an unordered item's marker becomes a bullet; leading indentation is kept.
    fn ut_unordered_item_marker_becomes_bullet_keeping_indent() {
        let md = MarkdownTheme::default();
        let rl = render("  - item");
        assert_eq!(text(&rl), "  • item");
        let bullet_span = rl
            .spans
            .iter()
            .find(|(t, _)| t == "•")
            .expect("bullet span");
        assert_eq!(bullet_span.1, md.bullet);
    }

    #[test]
    /// UI-R-145 — an ordered item keeps its number and delimiter exactly as written.
    fn ut_ordered_item_keeps_number_and_delimiter() {
        let rl = render("12. item");
        assert_eq!(text(&rl), "12. item");
    }

    #[test]
    /// UI-R-146 — a task item's brackets become a checkbox glyph, checked state preserved.
    fn ut_task_item_markers_become_boxes() {
        assert_eq!(text(&render("- [ ] todo")), "☐ todo");
        assert_eq!(text(&render("- [x] done")), "☑ done");
    }

    #[test]
    /// UI-R-147 — quote markers become bars in the theme's per-depth color; text is dim+italic.
    fn ut_quote_markers_become_bars_and_text_is_dim_italic() {
        let md = MarkdownTheme::default();
        let rl = render("> quoted text");
        assert_eq!(text(&rl), "▎quoted text");
        let bar_span = rl.spans.iter().find(|(t, _)| t == "▎").expect("bar span");
        assert_eq!(bar_span.1, md.quote_bar(1));
        let text_span = rl
            .spans
            .iter()
            .find(|(t, _)| t == "quoted text")
            .expect("text span");
        assert_eq!(text_span.1, *md.quote_text());
        assert!(text_span.1.add_modifier.contains(Modifier::DIM));
        assert!(text_span.1.add_modifier.contains(Modifier::ITALIC));

        let bare = MarkdownThemeBuilder::default()
            .quote_text(Style::default())
            .build()
            .expect("builder");
        let (block, _) = block_line("> quoted text", &BlockState::default());
        let (rl, _) = render_line(
            &block,
            "> quoted text",
            None,
            LineState::default(),
            &bare,
            &SyntaxTheme::default(),
            Style::default(),
        );
        let text_span = rl
            .spans
            .iter()
            .find(|(t, _)| t == "quoted text")
            .expect("text span");
        assert!(
            text_span.1.add_modifier.contains(Modifier::DIM)
                && text_span.1.add_modifier.contains(Modifier::ITALIC),
            "quoted text must be dim+italic even when the theme style carries no modifier"
        );
    }

    #[test]
    /// UI-R-148 — a horizontal rule renders as `RenderedLine { rule: true, .. }`.
    fn ut_horizontal_rule_is_a_rule_line() {
        let rl = render("---");
        assert!(rl.rule);
    }

    #[test]
    /// UI-R-149 — a fence delimiter renders empty in the theme's code style.
    fn ut_fence_delimiter_renders_empty_in_code_style() {
        let md = MarkdownTheme::default();
        let rl = render("```lua");
        assert_eq!(text(&rl), "");
        assert_eq!(rl.spans[0].1, *md.code());
        assert!(rl.char_wrap);
    }

    #[test]
    /// UI-R-150 — a fence body with no recognized info string is drawn verbatim in the code style.
    fn ut_fence_body_is_verbatim_in_code_style() {
        let (open, state) = block_line("```", &BlockState::default());
        assert!(matches!(open.kind, BlockKind::FenceDelimiter { .. }));
        let (body, _) = block_line("plain text here", &state);
        let md = MarkdownTheme::default();
        let syntax = SyntaxTheme::default();
        let (rl, _) = render_line(
            &body,
            "plain text here",
            None,
            LineState::default(),
            &md,
            &syntax,
            Style::default(),
        );
        assert_eq!(text(&rl), "plain text here");
        assert_eq!(rl.spans[0].1, *md.code());
        assert!(rl.char_wrap);
    }

    #[test]
    /// UI-R-151 — a `lua` fence body is highlighted through the syntax highlighter, with the
    /// carry-over line state threaded across the block (a long string opened on one line and
    /// closed on the next proves the carry).
    fn ut_lua_and_json_fence_bodies_are_syntax_highlighted_with_threaded_carry() {
        let (open, mut state) = block_line("```lua", &BlockState::default());
        assert!(matches!(open.kind, BlockKind::FenceDelimiter { .. }));
        let fence_info = Some("lua");
        let md = MarkdownTheme::default();
        let syntax = SyntaxTheme::default();
        let mut carry = LineState::default();

        let (body1, next_state) = block_line("s = [[", &state);
        state = next_state;
        let (rl1, next_carry) = render_line(
            &body1,
            "s = [[",
            fence_info,
            carry,
            &md,
            &syntax,
            Style::default(),
        );
        carry = next_carry;
        assert_eq!(text(&rl1), "s = [[");

        let (body2, _) = block_line("still a string]]", &state);
        let (rl2, _) = render_line(
            &body2,
            "still a string]]",
            fence_info,
            carry,
            &md,
            &syntax,
            Style::default(),
        );
        assert_eq!(text(&rl2), "still a string]]");
        let string_span = rl2
            .spans
            .iter()
            .find(|(t, _)| t.contains("still a string"))
            .expect("carried string span");
        assert_eq!(string_span.1, *syntax.string());

        let (open, state) = block_line("```json", &BlockState::default());
        assert!(matches!(open.kind, BlockKind::FenceDelimiter { .. }));
        let (body, _) = block_line(r#"{"a": true}"#, &state);
        let (rl, _) = render_line(
            &body,
            r#"{"a": true}"#,
            Some("json"),
            LineState::default(),
            &md,
            &syntax,
            Style::default(),
        );
        assert_eq!(text(&rl), r#"{"a": true}"#);
        let literal_span = rl
            .spans
            .iter()
            .find(|(t, _)| t == "true")
            .expect("json literal span");
        assert_eq!(literal_span.1, *syntax.literal());
    }

    #[test]
    /// UI-R-152 — inline bold, italic, code and strike hide their markers and style the content.
    fn ut_inline_emphasis_code_and_strike_hide_markers() {
        let md = MarkdownTheme::default();
        let rl = render("**bold** *italic* `code` ~~strike~~");
        assert_eq!(text(&rl), "bold italic code strike");
        let bold_span = rl
            .spans
            .iter()
            .find(|(t, _)| t == "bold")
            .expect("bold span");
        assert!(bold_span.1.add_modifier.contains(Modifier::BOLD));
        let italic_span = rl
            .spans
            .iter()
            .find(|(t, _)| t == "italic")
            .expect("italic span");
        assert!(italic_span.1.add_modifier.contains(Modifier::ITALIC));
        let code_span = rl
            .spans
            .iter()
            .find(|(t, _)| t == "code")
            .expect("code span");
        assert_eq!(code_span.1, *md.code());
        let strike_span = rl
            .spans
            .iter()
            .find(|(t, _)| t == "strike")
            .expect("strike span");
        assert!(strike_span.1.add_modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    /// UI-R-153 — a link renders its text underlined in the theme's link style; markers hidden.
    fn ut_link_renders_text_underlined_in_link_style() {
        let md = MarkdownTheme::default();
        let rl = render("[a link](http://x)");
        assert_eq!(text(&rl), "a link");
        assert_eq!(rl.spans[0].1, *md.link());
        assert!(rl.spans[0].1.add_modifier.contains(Modifier::UNDERLINED));

        let bare = MarkdownThemeBuilder::default()
            .link(Style::default())
            .build()
            .expect("builder");
        let (block, _) = block_line("[a link](http://x)", &BlockState::default());
        let (rl, _) = render_line(
            &block,
            "[a link](http://x)",
            None,
            LineState::default(),
            &bare,
            &SyntaxTheme::default(),
            Style::default(),
        );
        assert!(
            rl.spans[0].1.add_modifier.contains(Modifier::UNDERLINED),
            "link text must be underlined even when the theme style carries no modifier"
        );
    }

    #[test]
    /// UI-R-154 — an image renders its alt text in the theme's image style; markers hidden.
    fn ut_image_renders_alt_in_image_style() {
        let md = MarkdownTheme::default();
        let rl = render("![alt](img.png)");
        assert_eq!(text(&rl), "alt");
        assert_eq!(rl.spans[0].1, *md.image());
    }

    #[test]
    /// UI-E-091 — tables, raw HTML, footnotes, reference links and autolinks render as plain text.
    fn ut_unsupported_constructs_render_as_plain_text() {
        for line in [
            "| a | b |",
            "raw <b>html</b>",
            "[ref][1]",
            "<https://example.com>",
            "  indented paragraph text",
            "===",
        ] {
            let rl = render(line);
            assert_eq!(text(&rl), line);
        }
    }

    fn row_text(row: &[(String, Style)]) -> String {
        row.iter().map(|(t, _)| t.as_str()).collect()
    }

    #[test]
    /// UI-R-130 — wrapping is always on: no display row exceeds the available width.
    fn ut_no_row_exceeds_the_available_width() {
        let rl = render("the quick brown fox jumps over the lazy dog");
        for width in [1usize, 4, 5, 10, 20] {
            let rows = wrap_line(&rl, width);
            for row in &rows {
                assert!(
                    row_text(row).chars().count() <= width,
                    "width {width}: {row:?}"
                );
            }
        }
    }

    #[test]
    /// UI-R-148 — the wrapped rule row is styled in the theme's rule style, not a default.
    fn ut_rule_row_uses_the_theme_rule_style() {
        let md = MarkdownTheme::default();
        let rl = render("---");
        let rows = wrap_line(&rl, 8);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0].1, *md.rule());
        assert_eq!(
            row_text(&rows[0]).chars().count(),
            8,
            "rule row must span the full text width"
        );
    }

    #[test]
    /// UI-R-130 — width `0` and a blank line each yield exactly one empty row, never a panic.
    fn ut_width_zero_and_blank_line_yield_one_empty_row() {
        let rl = render("plain text");
        let rows = wrap_line(&rl, 0);
        assert_eq!(rows.len(), 1);
        assert_eq!(row_text(&rows[0]), "");

        let blank = render("");
        let rows = wrap_line(&blank, 10);
        assert_eq!(rows.len(), 1);
        assert_eq!(row_text(&rows[0]), "");
    }

    #[test]
    /// UI-R-131 — a line breaks at the last word boundary that fits the width.
    fn ut_wraps_at_the_last_word_boundary_that_fits() {
        let rl = render("the quick brown fox");
        let rows = wrap_line(&rl, 10);
        assert_eq!(row_text(&rows[0]), "the quick");
        assert_eq!(row_text(&rows[1]), "brown fox");
    }

    #[test]
    /// UI-R-131 — a single word longer than the available width breaks at a character boundary.
    fn ut_word_longer_than_width_breaks_at_a_character_boundary() {
        let rl = render("abcdefghij");
        let rows = wrap_line(&rl, 4);
        assert_eq!(row_text(&rows[0]), "abcd");
        assert_eq!(row_text(&rows[1]), "efgh");
        assert_eq!(row_text(&rows[2]), "ij");
    }

    #[test]
    /// UI-R-132 — continuation rows of a wrapped list item are indented to the content start.
    fn ut_list_and_quote_continuation_rows_align_under_the_content() {
        let rl = render("- one two three four five");
        assert_eq!(rl.hanging_indent, 2);
        let rows = wrap_line(&rl, 10);
        assert!(rows.len() > 1);
        for row in &rows[1..] {
            assert_eq!(
                row[0].0, "  ",
                "continuation row indent must be exactly hanging_indent spaces"
            );
        }

        let rl = render("> one two three four five");
        assert_eq!(rl.hanging_indent, 1);
        let rows = wrap_line(&rl, 10);
        assert!(rows.len() > 1);
        for row in &rows[1..] {
            assert_eq!(
                row[0].0, " ",
                "quote continuation row indent must be exactly hanging_indent spaces"
            );
        }
    }

    #[test]
    /// UI-R-133 — fence delimiter/body lines wrap at a character boundary with no hanging indent.
    fn ut_fence_lines_wrap_by_character_with_no_hanging_indent() {
        let rl = RenderedLine {
            spans: vec![("abcdefghij".to_string(), Style::default())],
            hanging_indent: 5,
            char_wrap: true,
            rule: false,
        };
        let rows = wrap_line(&rl, 4);
        assert_eq!(row_text(&rows[0]), "abcd");
        assert_eq!(row_text(&rows[1]), "efgh");
        assert_eq!(row_text(&rows[2]), "ij");
    }

    #[test]
    /// UI-E-086 — when the hanging indent leaves no room for content, it is dropped and
    /// continuation rows start at column zero.
    fn ut_hanging_indent_is_dropped_when_the_width_is_too_narrow() {
        let rl = RenderedLine {
            spans: vec![("one two three".to_string(), Style::default())],
            hanging_indent: 8,
            char_wrap: false,
            rule: false,
        };
        let rows = wrap_line(&rl, 9);
        assert!(rows.len() > 1);
        for row in &rows[1..] {
            let t = row_text(row);
            assert!(
                !t.starts_with(" "),
                "indent should have been dropped: {t:?}"
            );
        }
    }

    #[test]
    /// UI-E-087 — a long word is never truncated: every character reappears across the rows.
    fn ut_long_word_is_never_truncated() {
        let word = "abcdefghijklmnopqrstuvwxyz";
        let rl = render(word);
        let rows = wrap_line(&rl, 5);
        let joined: String = rows.iter().map(|r| row_text(r)).collect();
        assert_eq!(joined, word);
    }

    #[test]
    /// UI-E-087 — a leading whitespace run wider than the row cap is kept, never silently
    /// dropped into an empty row.
    fn ut_leading_space_run_wider_than_width_is_kept_not_dropped() {
        let spaces = " ".repeat(10);
        let rl = render(&spaces);
        let rows = wrap_line(&rl, 4);
        let joined: String = rows.iter().map(|r| row_text(r)).collect();
        assert_eq!(joined, spaces);
        assert!(
            rows.iter().all(|r| !row_text(r).is_empty()),
            "no row should be spuriously empty: {rows:?}"
        );
    }
}
