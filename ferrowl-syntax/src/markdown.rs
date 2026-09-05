//! Markdown inline model: pure text-to-spans parsing of the inline constructs a
//! consumer needs to render (hide markers) or highlight (UI-R-178).

use std::collections::HashSet;

/// Inline construct kinds recognized on one source line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineKind {
    Bold,
    Italic,
    Code,
    Strike,
    Link,
    Image,
}

/// One inline construct, in source character columns of its line: the columns that carry
/// markup (`markers`) and the columns a renderer shows (`content`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSpan {
    pub kind: InlineKind,
    pub markers: Vec<(usize, usize)>,
    pub content: (usize, usize),
}

/// Inline constructs of one source line, sorted by content start, non-overlapping except for
/// the deliberate Bold/Italic pair `***x***` resolves to over the same text (UI-E-092).
pub fn inline_spans(line: &str) -> Vec<InlineSpan> {
    let chars: Vec<char> = line.chars().collect();
    let escaped = escaped_positions(&chars);
    let mut spans = Vec::new();
    let mut i = 0usize;
    let len = chars.len();

    while i < len {
        if escaped.contains(&i) {
            i += 1;
            continue;
        }

        if chars[i] == '!'
            && i + 1 < len
            && chars[i + 1] == '['
            && let Some(span) = try_link_or_image(&chars, i, &escaped, true)
        {
            i = span.markers.last().expect("link/image has markers").1;
            spans.push(span);
            continue;
        }

        if chars[i] == '['
            && let Some(span) = try_link_or_image(&chars, i, &escaped, false)
        {
            i = span.markers.last().expect("link/image has markers").1;
            spans.push(span);
            continue;
        }

        if chars[i] == '`'
            && let Some(span) = try_delim(&chars, i, &['`'], InlineKind::Code, &escaped)
        {
            i = span.markers[1].1;
            spans.push(span);
            continue;
        }

        if chars[i] == '~'
            && i + 1 < len
            && chars[i + 1] == '~'
            && let Some(span) = try_delim(&chars, i, &['~', '~'], InlineKind::Strike, &escaped)
        {
            i = span.markers[1].1;
            spans.push(span);
            continue;
        }

        if chars[i] == '*'
            && i + 2 < len
            && chars[i + 1] == '*'
            && chars[i + 2] == '*'
            && let Some((bold, italic, end)) = try_triple(&chars, i, &escaped)
        {
            spans.push(bold);
            spans.push(italic);
            i = end;
            continue;
        }

        if chars[i] == '*'
            && i + 1 < len
            && chars[i + 1] == '*'
            && let Some(span) = try_delim(&chars, i, &['*', '*'], InlineKind::Bold, &escaped)
        {
            i = span.markers[1].1;
            spans.push(span);
            continue;
        }

        if chars[i] == '*'
            && let Some(span) = try_delim(&chars, i, &['*'], InlineKind::Italic, &escaped)
        {
            i = span.markers[1].1;
            spans.push(span);
            continue;
        }

        if chars[i] == '_'
            && !(i > 0 && is_word_char(chars[i - 1]))
            && let Some(span) = try_underscore_italic(&chars, i, &escaped)
        {
            i = span.markers[1].1;
            spans.push(span);
            continue;
        }

        i += 1;
    }

    spans.sort_by_key(|s| s.content.0);
    spans
}

/// Backslash-escape marker columns of a line: each `\` that makes the following character
/// literal. The escaped character itself never opens or closes an inline construct.
pub fn escape_markers(line: &str) -> Vec<usize> {
    let chars: Vec<char> = line.chars().collect();
    backslash_positions(&chars)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn backslash_positions(chars: &[char]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            out.push(i);
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

fn escaped_positions(chars: &[char]) -> HashSet<usize> {
    backslash_positions(chars)
        .into_iter()
        .flat_map(|i| [i, i + 1])
        .collect()
}

fn find_delim(
    chars: &[char],
    from: usize,
    delim: &[char],
    escaped: &HashSet<usize>,
) -> Option<usize> {
    let dl = delim.len();
    if dl == 0 || from + dl > chars.len() {
        return None;
    }
    let mut j = from;
    while j + dl <= chars.len() {
        if !escaped.contains(&j) && chars[j..j + dl] == *delim {
            return Some(j);
        }
        j += 1;
    }
    None
}

fn try_delim(
    chars: &[char],
    start: usize,
    delim: &[char],
    kind: InlineKind,
    escaped: &HashSet<usize>,
) -> Option<InlineSpan> {
    let dl = delim.len();
    let content_start = start + dl;
    let close = find_delim(chars, content_start, delim, escaped)?;
    if close == content_start {
        return None;
    }
    let marker_end = close + dl;
    Some(InlineSpan {
        kind,
        markers: vec![(start, content_start), (close, marker_end)],
        content: (content_start, close),
    })
}

/// `_..._` italic, but only where both markers sit at a non-word boundary (UI-E-078): a
/// closing `_` immediately followed by a word character does not close, just as an opening
/// `_` immediately preceded by one does not open.
fn try_underscore_italic(
    chars: &[char],
    start: usize,
    escaped: &HashSet<usize>,
) -> Option<InlineSpan> {
    let mut from = start + 1;
    loop {
        let close = find_delim(chars, from, &['_'], escaped)?;
        if close == start + 1 {
            return None;
        }
        if close + 1 < chars.len() && is_word_char(chars[close + 1]) {
            from = close + 1;
            continue;
        }
        return Some(InlineSpan {
            kind: InlineKind::Italic,
            markers: vec![(start, start + 1), (close, close + 1)],
            content: (start + 1, close),
        });
    }
}

fn try_triple(
    chars: &[char],
    start: usize,
    escaped: &HashSet<usize>,
) -> Option<(InlineSpan, InlineSpan, usize)> {
    let delim = ['*', '*', '*'];
    let content_start = start + 3;
    let close = find_delim(chars, content_start, &delim, escaped)?;
    if close == content_start {
        return None;
    }
    let bold = InlineSpan {
        kind: InlineKind::Bold,
        markers: vec![(start, start + 2), (close + 1, close + 3)],
        content: (start + 2, close + 1),
    };
    let italic = InlineSpan {
        kind: InlineKind::Italic,
        markers: vec![(start + 2, start + 3), (close, close + 1)],
        content: (start + 3, close),
    };
    Some((bold, italic, close + 3))
}

fn try_link_or_image(
    chars: &[char],
    start: usize,
    escaped: &HashSet<usize>,
    is_image: bool,
) -> Option<InlineSpan> {
    let bracket_pos = if is_image { start + 1 } else { start };
    if bracket_pos >= chars.len() || chars[bracket_pos] != '[' {
        return None;
    }
    let text_start = bracket_pos + 1;
    let close_bracket = find_delim(chars, text_start, &[']'], escaped)?;
    let paren_open = close_bracket + 1;
    if paren_open >= chars.len() || chars[paren_open] != '(' {
        return None;
    }
    let url_start = paren_open + 1;
    let close_paren = find_delim(chars, url_start, &[')'], escaped)?;
    let end = close_paren + 1;
    let kind = if is_image {
        InlineKind::Image
    } else {
        InlineKind::Link
    };
    let markers = vec![
        (start, bracket_pos + 1),
        (close_bracket, paren_open + 1),
        (url_start, close_paren),
        (close_paren, end),
    ];
    Some(InlineSpan {
        kind,
        markers,
        content: (text_start, close_bracket),
    })
}

/// The block a source line forms, in the line-preserving model of the markdown widget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    Heading { level: u8 },
    UnorderedItem { depth: usize, marker: char },
    OrderedItem { depth: usize },
    TaskItem { depth: usize, checked: bool },
    Quote { depth: usize },
    Rule,
    FenceDelimiter { info: String },
    FenceBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpenFence {
    info: String,
    len: usize,
}

/// Carried block state: holds the open fence's info string and delimiter length while
/// inside a fenced code block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockState {
    fence: Option<OpenFence>,
}

impl BlockState {
    /// The info string of the fence currently open, if any.
    pub fn fence_info(&self) -> Option<&str> {
        self.fence.as_ref().map(|f| f.info.as_str())
    }
}

/// One classified source line: its block kind, the source column its content starts at
/// (the column after the block marker), and its inline constructs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockLine {
    pub kind: BlockKind,
    pub content_start: usize,
    pub inline: Vec<InlineSpan>,
}

/// Classifies one source line given the state carried from the previous line, returning
/// the state to carry into the next.
pub fn block_line(line: &str, state: &BlockState) -> (BlockLine, BlockState) {
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut i = 0usize;
    while i < len && chars[i] == ' ' {
        i += 1;
    }
    let leading = i;

    if let Some(open) = &state.fence {
        if is_closing_fence(&chars, i, len, open.len) {
            let bl = BlockLine {
                kind: BlockKind::FenceDelimiter {
                    info: String::new(),
                },
                content_start: len,
                inline: Vec::new(),
            };
            return (bl, BlockState { fence: None });
        }
        let bl = BlockLine {
            kind: BlockKind::FenceBody,
            content_start: 0,
            inline: Vec::new(),
        };
        return (bl, state.clone());
    }

    if is_rule(&chars) {
        let bl = BlockLine {
            kind: BlockKind::Rule,
            content_start: len,
            inline: Vec::new(),
        };
        return (bl, state.clone());
    }

    if i < len && chars[i] == '`' {
        let mut j = i;
        while j < len && chars[j] == '`' {
            j += 1;
        }
        let opening_len = j - i;
        if opening_len >= 3 {
            let info: String = chars[j..].iter().collect::<String>().trim().to_string();
            let bl = BlockLine {
                kind: BlockKind::FenceDelimiter { info: info.clone() },
                content_start: len,
                inline: Vec::new(),
            };
            return (
                bl,
                BlockState {
                    fence: Some(OpenFence {
                        info,
                        len: opening_len,
                    }),
                },
            );
        }
    }

    if i < len && chars[i] == '#' {
        let mut j = i;
        while j < len && chars[j] == '#' && j - i < 6 {
            j += 1;
        }
        let level = (j - i) as u8;
        if j < len && chars[j] == ' ' {
            let content_start = j + 1;
            let inline = offset_inline(&chars, content_start);
            let bl = BlockLine {
                kind: BlockKind::Heading { level },
                content_start,
                inline,
            };
            return (bl, state.clone());
        }
    }

    if i < len && chars[i] == '>' {
        let mut depth = 0usize;
        let mut k = i;
        while k < len && chars[k] == '>' {
            depth += 1;
            k += 1;
            if k < len && chars[k] == ' ' {
                k += 1;
            }
        }
        let content_start = k;
        let inline = offset_inline(&chars, content_start);
        let bl = BlockLine {
            kind: BlockKind::Quote { depth },
            content_start,
            inline,
        };
        return (bl, state.clone());
    }

    let depth = leading / 2;

    if let Some(bullet_end) = unordered_bullet_end(&chars, i) {
        let marker = chars[i];
        if bullet_end + 2 < len
            && chars[bullet_end] == '['
            && matches!(chars[bullet_end + 1], ' ' | 'x' | 'X')
            && chars[bullet_end + 2] == ']'
        {
            let checked = matches!(chars[bullet_end + 1], 'x' | 'X');
            let mut content_start = bullet_end + 3;
            if content_start < len && chars[content_start] == ' ' {
                content_start += 1;
            }
            let inline = offset_inline(&chars, content_start);
            let bl = BlockLine {
                kind: BlockKind::TaskItem { depth, checked },
                content_start,
                inline,
            };
            return (bl, state.clone());
        }
        let content_start = bullet_end;
        let inline = offset_inline(&chars, content_start);
        let bl = BlockLine {
            kind: BlockKind::UnorderedItem { depth, marker },
            content_start,
            inline,
        };
        return (bl, state.clone());
    }

    if let Some(content_start) = ordered_item_end(&chars, i) {
        let inline = offset_inline(&chars, content_start);
        let bl = BlockLine {
            kind: BlockKind::OrderedItem { depth },
            content_start,
            inline,
        };
        return (bl, state.clone());
    }

    let content_start = leading;
    let inline = offset_inline(&chars, content_start);
    let bl = BlockLine {
        kind: BlockKind::Paragraph,
        content_start,
        inline,
    };
    (bl, state.clone())
}

fn is_closing_fence(chars: &[char], from: usize, len: usize, opening_len: usize) -> bool {
    let mut j = from;
    let mut n = 0usize;
    while j < len && chars[j] == '`' {
        n += 1;
        j += 1;
    }
    while j < len && chars[j] == ' ' {
        j += 1;
    }
    n >= opening_len && j == len
}

fn is_rule(chars: &[char]) -> bool {
    let cs: Vec<char> = chars
        .iter()
        .copied()
        .filter(|c| !c.is_whitespace())
        .collect();
    if cs.len() < 3 {
        return false;
    }
    let c0 = cs[0];
    if !matches!(c0, '-' | '*') {
        return false;
    }
    cs.iter().all(|c| *c == c0)
}

fn unordered_bullet_end(chars: &[char], from: usize) -> Option<usize> {
    let len = chars.len();
    if from < len
        && matches!(chars[from], '-' | '*' | '+')
        && from + 1 < len
        && chars[from + 1] == ' '
    {
        return Some(from + 2);
    }
    None
}

fn ordered_item_end(chars: &[char], from: usize) -> Option<usize> {
    let len = chars.len();
    let mut d = from;
    while d < len && chars[d].is_ascii_digit() {
        d += 1;
    }
    if d > from && d < len && matches!(chars[d], '.' | ')') && d + 1 < len && chars[d + 1] == ' ' {
        return Some(d + 2);
    }
    None
}

fn offset_inline(chars: &[char], offset: usize) -> Vec<InlineSpan> {
    let text: String = chars[offset..].iter().collect();
    inline_spans(&text)
        .into_iter()
        .map(|s| InlineSpan {
            kind: s.kind,
            markers: s
                .markers
                .into_iter()
                .map(|(a, b)| (a + offset, b + offset))
                .collect(),
            content: (s.content.0 + offset, s.content.1 + offset),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<InlineKind> {
        inline_spans(line).into_iter().map(|s| s.kind).collect()
    }

    #[test]
    /// UI-R-178 — bold, code and strike each separate their marker columns from their content columns.
    fn ut_emphasis_code_and_strike_separate_markers_from_content() {
        let spans = inline_spans("**bold**");
        assert_eq!(spans.len(), 1);
        let s = &spans[0];
        assert_eq!(s.kind, InlineKind::Bold);
        assert_eq!(s.markers, vec![(0, 2), (6, 8)]);
        assert_eq!(s.content, (2, 6));

        let spans = inline_spans("*italic*");
        assert_eq!(spans[0].kind, InlineKind::Italic);
        assert_eq!(spans[0].markers, vec![(0, 1), (7, 8)]);
        assert_eq!(spans[0].content, (1, 7));

        let spans = inline_spans("_italic_");
        assert_eq!(spans[0].kind, InlineKind::Italic);
        assert_eq!(spans[0].markers, vec![(0, 1), (7, 8)]);
        assert_eq!(spans[0].content, (1, 7));

        let spans = inline_spans("`code`");
        assert_eq!(spans[0].kind, InlineKind::Code);
        assert_eq!(spans[0].markers, vec![(0, 1), (5, 6)]);
        assert_eq!(spans[0].content, (1, 5));

        let spans = inline_spans("~~strike~~");
        assert_eq!(spans[0].kind, InlineKind::Strike);
        assert_eq!(spans[0].markers, vec![(0, 2), (8, 10)]);
        assert_eq!(spans[0].content, (2, 8));
    }

    #[test]
    /// UI-R-178 — a link and an image each separate bracket/paren/URL marker columns from their text content.
    fn ut_link_and_image_separate_markers_from_content() {
        let spans = inline_spans("[text](url)");
        assert_eq!(spans.len(), 1);
        let s = &spans[0];
        assert_eq!(s.kind, InlineKind::Link);
        assert_eq!(s.content, (1, 5));
        assert_eq!(s.markers, vec![(0, 1), (5, 7), (7, 10), (10, 11)]);

        let spans = inline_spans("![alt](url)");
        let s = &spans[0];
        assert_eq!(s.kind, InlineKind::Image);
        assert_eq!(s.content, (2, 5));
        assert_eq!(s.markers, vec![(0, 2), (5, 7), (7, 10), (10, 11)]);
    }

    #[test]
    /// UI-R-179 — an escaped `*` neither opens nor closes an italic construct; the backslash is a marker column.
    fn ut_backslash_escape_neither_opens_nor_closes_a_construct() {
        let line = r"\*not italic\*";
        assert!(inline_spans(line).is_empty());
        let escapes = escape_markers(line);
        assert_eq!(escapes, vec![0, 12]);
    }

    #[test]
    /// Constructs the block model does not recognize report no inline spans on the inline path.
    fn ut_unsupported_inline_constructs_report_no_spans() {
        let cases = [
            "| a | b |",
            "raw <b>html</b>",
            "footnote [^1] ref",
            "[ref][1]",
            "<https://example.com>",
        ];
        for case in cases {
            let spans = inline_spans(case);
            assert!(spans.is_empty(), "unexpected span in {case:?}: {spans:?}");
        }
    }

    #[test]
    /// UI-E-078 — `_` preceded by a word character never opens italic, so an identifier's
    /// underscores stay visible instead of a spurious pair hiding as markers.
    fn ut_intraword_underscore_does_not_open_italic() {
        let spans = inline_spans("snake_case_word");
        assert!(
            spans.is_empty(),
            "intraword underscores must not be treated as emphasis markers: {spans:?}"
        );
    }

    #[test]
    /// UI-E-078 — `_` immediately followed by a word character never closes italic either,
    /// so a leading underscore before an identifier stays visible.
    fn ut_underscore_followed_by_word_char_does_not_close_italic() {
        let spans = inline_spans("_snake_case_name");
        assert!(
            spans.is_empty(),
            "underscore adjacent to a word character on either side must not be emphasis: {spans:?}"
        );
    }

    #[test]
    /// UI-E-092 — `***x***` resolves best-effort as nested Bold and Italic over the same text.
    fn ut_triple_marker_yields_bold_and_italic() {
        let ks = kinds("***x***");
        assert_eq!(ks, vec![InlineKind::Bold, InlineKind::Italic]);
        let spans = inline_spans("***x***");
        let bold = spans
            .iter()
            .find(|s| s.kind == InlineKind::Bold)
            .expect("bold span");
        let italic = spans
            .iter()
            .find(|s| s.kind == InlineKind::Italic)
            .expect("italic span");
        assert_eq!(italic.content, (3, 4));
        assert!(bold.content.0 <= italic.content.0 && bold.content.1 >= italic.content.1);
    }

    #[test]
    /// UI-R-176 — headings, list items, quotes and rules classify to their `BlockKind`.
    fn ut_block_kinds_cover_headings_lists_quotes_and_rules() {
        let (bl, _) = block_line("### Title", &BlockState::default());
        assert_eq!(bl.kind, BlockKind::Heading { level: 3 });
        assert_eq!(bl.content_start, 4);

        let (bl, _) = block_line("- item", &BlockState::default());
        assert_eq!(
            bl.kind,
            BlockKind::UnorderedItem {
                depth: 0,
                marker: '-'
            }
        );
        assert_eq!(bl.content_start, 2);

        let (bl, _) = block_line("  * nested", &BlockState::default());
        assert_eq!(
            bl.kind,
            BlockKind::UnorderedItem {
                depth: 1,
                marker: '*'
            }
        );

        let (bl, _) = block_line("2. second", &BlockState::default());
        assert_eq!(bl.kind, BlockKind::OrderedItem { depth: 0 });
        assert_eq!(bl.content_start, 3);

        let (bl, _) = block_line("> quoted", &BlockState::default());
        assert_eq!(bl.kind, BlockKind::Quote { depth: 1 });
        assert_eq!(bl.content_start, 2);

        let (bl, _) = block_line("> > nested quote", &BlockState::default());
        assert_eq!(bl.kind, BlockKind::Quote { depth: 2 });

        let (bl, _) = block_line("---", &BlockState::default());
        assert_eq!(bl.kind, BlockKind::Rule);

        let (bl, _) = block_line("plain text", &BlockState::default());
        assert_eq!(bl.kind, BlockKind::Paragraph);
        assert_eq!(bl.content_start, 0);
    }

    #[test]
    /// UI-R-176 — a task item reports its checked state, case-insensitively.
    fn ut_task_item_reports_checked_state() {
        let (bl, _) = block_line("- [ ] todo", &BlockState::default());
        assert_eq!(
            bl.kind,
            BlockKind::TaskItem {
                depth: 0,
                checked: false
            }
        );

        let (bl, _) = block_line("- [x] done", &BlockState::default());
        assert_eq!(
            bl.kind,
            BlockKind::TaskItem {
                depth: 0,
                checked: true
            }
        );

        let (bl, _) = block_line("- [X] done", &BlockState::default());
        assert_eq!(
            bl.kind,
            BlockKind::TaskItem {
                depth: 0,
                checked: true
            }
        );
    }

    #[test]
    /// UI-R-176 — `inline` carries the source line's inline constructs, offset by `content_start`.
    fn ut_block_line_reports_inline_spans_of_its_content() {
        let (bl, _) = block_line("- a **bold** item", &BlockState::default());
        assert_eq!(bl.inline.len(), 1);
        let span = &bl.inline[0];
        assert_eq!(span.kind, InlineKind::Bold);
        let chars: Vec<char> = "- a **bold** item".chars().collect();
        let content: String = chars[span.content.0..span.content.1].iter().collect();
        assert_eq!(content, "bold");
    }

    #[test]
    /// UI-R-177 — every line inside an open fence is `FenceBody` regardless of content, and a
    /// matching closing delimiter clears the carry.
    fn ut_fence_carry_classifies_every_body_line_and_closes_on_matching_delimiter() {
        let state = BlockState::default();
        let (open, state) = block_line("```lua", &state);
        assert_eq!(
            open.kind,
            BlockKind::FenceDelimiter {
                info: "lua".to_string()
            }
        );
        assert_eq!(state.fence_info(), Some("lua"));

        let (body1, state) = block_line("# not a heading here", &state);
        assert_eq!(body1.kind, BlockKind::FenceBody);
        assert!(body1.inline.is_empty());

        let (body2, state) = block_line("- not a list either", &state);
        assert_eq!(body2.kind, BlockKind::FenceBody);

        let (close, state) = block_line("```", &state);
        assert_eq!(
            close.kind,
            BlockKind::FenceDelimiter {
                info: String::new()
            }
        );
        assert_eq!(state.fence_info(), None);

        let (after, _) = block_line("# back to a heading", &state);
        assert_eq!(after.kind, BlockKind::Heading { level: 1 });
    }

    #[test]
    /// UI-R-177 — a closing fence delimiter must be at least as long as the opening one; a
    /// shorter backtick run stays fence body.
    fn ut_fence_closing_run_must_be_at_least_the_opening_length() {
        let state = BlockState::default();
        let (open, state) = block_line("````lua", &state);
        assert_eq!(state.fence_info(), Some("lua"));
        assert!(matches!(open.kind, BlockKind::FenceDelimiter { .. }));

        let (short_close, state) = block_line("```", &state);
        assert_eq!(
            short_close.kind,
            BlockKind::FenceBody,
            "a 3-backtick run must not close a 4-backtick fence"
        );
        assert_eq!(state.fence_info(), Some("lua"));

        let (long_close, state) = block_line("````", &state);
        assert_eq!(
            long_close.kind,
            BlockKind::FenceDelimiter {
                info: String::new()
            }
        );
        assert_eq!(state.fence_info(), None);
    }

    #[test]
    /// UI-E-090 — a fence opened and never closed keeps every following line a fence body.
    fn ut_unclosed_fence_keeps_every_following_line_a_fence_body() {
        let mut state = BlockState::default();
        let (open, next) = block_line("```", &state);
        assert!(matches!(open.kind, BlockKind::FenceDelimiter { .. }));
        state = next;

        for line in [
            "one",
            "```not-closing-because-trailing-text extra",
            "",
            "still inside",
        ] {
            let (bl, next) = block_line(line, &state);
            assert_eq!(
                bl.kind,
                BlockKind::FenceBody,
                "line {line:?} should stay fence body"
            );
            assert!(state.fence_info().is_some());
            state = next;
        }
    }

    #[test]
    /// UI-R-180 — tables, raw HTML, footnotes, reference links, setext headings and autolinks
    /// classify as `Paragraph` with no inline spans over them.
    fn ut_tables_html_footnotes_reference_links_setext_and_autolinks_are_paragraphs() {
        let cases = [
            "| a | b |",
            "raw <b>html</b>",
            "footnote [^1] ref",
            "[ref][1]",
            "<https://example.com>",
            "===",
        ];
        for case in cases {
            let (bl, _) = block_line(case, &BlockState::default());
            assert_eq!(bl.kind, BlockKind::Paragraph, "{case:?}");
            assert!(bl.inline.is_empty(), "{case:?}: {:?}", bl.inline);
        }
    }
}
