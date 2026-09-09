use std::ops::Range;

use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, StatefulWidget, Widget},
};

use crate::Border;
use crate::state::{
    Annotation, DiffEntry, DiffKind, DiffLayout, DiffMode, DiffRow, DiffViewState,
    MarkdownInputFieldStateBuilder, MarkedRange, RowPart, Side,
};
use crate::style::{DiffViewStyle, InputFieldStyleBuilder, MarkdownThemeBuilder, SyntaxTheme};
use crate::traits::{IsFocus, Margins};
use crate::widgets::{MarkdownInputFieldBuilder, Title};

/// A read-only side-by-side or unified diff viewer rendered from a
/// [`DiffViewState`](crate::state::DiffViewState). Configure border, title, margins, and
/// [`DiffViewStyle`] via [`DiffViewBuilder`]. The split/unified layout is not a builder
/// field: it lives on the state (`DiffViewState::layout`), since it can be toggled at
/// runtime and a widget rebuilt fresh each frame cannot carry it.
#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct DiffView {
    #[getset(get = "pub")]
    #[builder(default = "Border::None")]
    border: Border,
    #[getset(get = "pub")]
    #[builder(default = "DiffViewStyle::default()")]
    style: DiffViewStyle,
    #[getset(get = "pub")]
    #[builder(default = "SyntaxTheme::default()")]
    syntax_theme: SyntaxTheme,
    #[getset(get = "pub")]
    #[builder(default = "None")]
    title: Option<Title>,
    #[getset(get = "pub")]
    #[builder(default = "Margin::default()")]
    margin: Margin,
}

impl Default for DiffView {
    fn default() -> Self {
        DiffViewBuilder::default()
            .build()
            .expect("DiffViewBuilder fields all default")
    }
}

impl Margins for DiffView {
    fn margins(&self) -> Margin {
        let horizontal = if let Border::Full(m) = &self.border {
            4 + m.horizontal * 2
        } else {
            0
        } + 2 * self.margin.horizontal
            + 1;
        let vertical = if let Border::Full(m) = &self.border {
            2 + m.vertical * 2
        } else if self.title.is_some() {
            1
        } else {
            0
        } + self.margin.vertical;
        Margin {
            horizontal,
            vertical,
        }
    }
}

/// The widest label width, plus the widest fallback line-number digit width for a row
/// with no label entry (the code editor's UI-R-167 rule, adapted to aligned rows rather
/// than buffer lines: a filler or meta row contributes nothing, since it never falls back
/// to a number, UI-R-212).
fn side_gutter_text_width(rows: &[DiffRow], labels: Option<&Vec<String>>, side: Side) -> usize {
    fn entry_at(row: &DiffRow, side: Side) -> Option<&DiffEntry> {
        match row {
            DiffRow::Meta { .. } => None,
            DiffRow::Pair { old, new, .. } => match side {
                Side::Old => old.as_ref(),
                Side::New => new.as_ref(),
            },
        }
    }
    let label_width = labels.map_or(0, |l| {
        l.iter().map(|s| s.chars().count()).max().unwrap_or(0)
    });
    let needs_fallback = |i: usize| labels.is_none_or(|l| i >= l.len());
    let fallback_width = rows
        .iter()
        .enumerate()
        .filter(|(i, _)| needs_fallback(*i))
        .filter_map(|(_, row)| entry_at(row, side).map(|e| e.line_no.to_string().len()))
        .max()
        .unwrap_or(0);
    label_width.max(fallback_width)
}

/// Clamps a gutter's combined width — its digits/label plus their one separator space —
/// so it never exceeds `pane_width` (UI-R-172, amended UI-R-216/UI-R-218's inheritance of
/// the code editor's UI-R-167): the separator column that once sat between the gutter and
/// the removed marker column is now the gutter's own trailing blank column.
fn clamp_gutter(text_width: usize, pane_width: u16) -> u16 {
    (text_width + 1).min(pane_width as usize) as u16
}

/// The gutter text for one row's side entry: the label at `row_idx` if the label list
/// covers it, else the entry's file line number, else blank for a filler or meta row
/// (UI-R-166, UI-R-212).
fn gutter_text_for(
    row_idx: usize,
    entry: Option<&DiffEntry>,
    labels: Option<&Vec<String>>,
) -> String {
    match entry {
        None => String::new(),
        Some(e) => labels
            .and_then(|l| l.get(row_idx))
            .cloned()
            .unwrap_or_else(|| e.line_no.to_string()),
    }
}

/// The diff-kind style of UI-R-219, with its foreground overridden by the syntax theme's
/// span style when a language is set (UI-R-220); the diff style's background and
/// modifiers survive since `Style::fg` only ever replaces the foreground. With no spans
/// (no language, or a span-free line) the diff-kind style alone applies (UI-R-221).
/// Highlighting is computed per line, not threaded across rows: the old and new sides
/// interleave and a hunk starts mid-file, so carrying a `LineState` between them would mix
/// two unrelated documents' running state.
fn styled_chars(
    text: &str,
    diff_style: Style,
    language: Option<ferrowl_syntax::Language>,
    syntax_theme: &SyntaxTheme,
) -> Vec<(char, Style)> {
    let chars: Vec<char> = text.chars().collect();
    let Some(lang) = language else {
        return chars.into_iter().map(|c| (c, diff_style)).collect();
    };
    let (spans, _) =
        ferrowl_syntax::highlight_line(lang, text, ferrowl_syntax::LineState::default());
    let mut styles = vec![diff_style; chars.len()];
    for (start, end, kind) in spans {
        let end = end.min(chars.len());
        let style = match syntax_theme.style(kind).fg {
            Some(fg) => diff_style.fg(fg),
            None => diff_style,
        };
        for slot in styles.iter_mut().take(end).skip(start) {
            *slot = style;
        }
    }
    chars.into_iter().zip(styles).collect()
}

/// Groups consecutive same-style characters into `(text, style)` runs, cutting `Span`
/// count without changing what is drawn.
fn group_runs(chars: Vec<(char, Style)>) -> Vec<(String, Style)> {
    let mut out: Vec<(String, Style)> = Vec::new();
    for (c, style) in chars {
        match out.last_mut() {
            Some((s, last_style)) if *last_style == style => s.push(c),
            _ => out.push((c.to_string(), style)),
        }
    }
    out
}

/// Splits `text` into the word tokens of UI-R-281 — one token per maximal run of word
/// characters (letter, digit or `_`), one per maximal run of anything else — as
/// half-open char-index ranges into `text`.
fn word_tokens(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut current: Option<bool> = None;
    for (i, c) in text.chars().enumerate() {
        let is_word = c.is_alphanumeric() || c == '_';
        match current {
            Some(w) if w == is_word => {}
            Some(_) => {
                out.push(start..i);
                start = i;
            }
            None => {}
        }
        current = Some(is_word);
    }
    let len = text.chars().count();
    if len > 0 {
        out.push(start..len);
    }
    out
}

/// The char-index ranges of `text` holding tokens (UI-R-281) that `other` does not,
/// after a longest-common-subsequence match over the two token texts: the tokens present
/// on only this side (UI-R-280, UI-R-283). Adjacent surviving ranges are merged so one
/// emphasised stretch is one span.
fn word_diff_spans(text: &str, other: &str) -> Vec<Range<usize>> {
    let chars: Vec<char> = text.chars().collect();
    let other_chars: Vec<char> = other.chars().collect();
    let a = word_tokens(text);
    let b = word_tokens(other);
    /// Above this many word tokens on either side the pair gets no emphasis at all and
    /// both rows stay plain full-width bands (UI-E-129): the longest-common-subsequence
    /// table below costs the product of the two token counts, which one minified or
    /// base64 line makes quadratic on every frame that draws the row.
    const MAX_WORD_TOKENS: usize = 512;
    if a.len() > MAX_WORD_TOKENS || b.len() > MAX_WORD_TOKENS {
        return Vec::new();
    }
    let text_of = |chars: &[char], r: &Range<usize>| chars[r.clone()].iter().collect::<String>();
    let a_tok: Vec<String> = a.iter().map(|r| text_of(&chars, r)).collect();
    let b_tok: Vec<String> = b.iter().map(|r| text_of(&other_chars, r)).collect();

    let (la, lb) = (a_tok.len(), b_tok.len());
    let mut dp = vec![vec![0usize; lb + 1]; la + 1];
    for i in (0..la).rev() {
        for j in (0..lb).rev() {
            dp[i][j] = if a_tok[i] == b_tok[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut matched = vec![false; la];
    let (mut i, mut j) = (0usize, 0usize);
    while i < la && j < lb {
        if a_tok[i] == b_tok[j] && dp[i][j] == dp[i + 1][j + 1] + 1 {
            matched[i] = true;
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }

    let mut spans: Vec<Range<usize>> = Vec::new();
    for (idx, range) in a.into_iter().enumerate() {
        if matched[idx] {
            continue;
        }
        match spans.last_mut() {
            Some(last) if last.end == range.start => last.end = range.end,
            _ => spans.push(range),
        }
    }
    spans
}

impl DiffView {
    /// The per-side diff style of UI-R-219: the widget's own removed/added style
    /// for a changed row's old/new side respectively, the general text style for context.
    /// A `DiffRow::Pair` carries one shared `kind` even for a genuine two-sided change
    /// (both sides present, text differing), so the style is derived from which side is
    /// being drawn, not from `kind` alone — otherwise a combined change row's new (added)
    /// side would wrongly inherit the row's `Removed` tag.
    fn side_style(&self, kind: &DiffKind, side: Side) -> Style {
        match kind {
            DiffKind::Context => self.style.general,
            DiffKind::Meta => self.style.meta,
            DiffKind::Added | DiffKind::Removed => match side {
                Side::Old => self.style.removed,
                Side::New => self.style.added,
            },
        }
    }

    /// Draws one side's gutter and text into `rect` (whose width is exactly
    /// `gutter_width + text_width`). A filler entry (UI-R-212) draws a blank gutter of the
    /// full width and no text. `sub_row` selects which wrapped display row of the entry's
    /// text this call draws (UI-R-260); `sub_row > 0` draws a continuation, with a blank
    /// gutter of its own (UI-R-261) rather than `draw_entry`'s own text-absent blank, so a
    /// continuation is visually distinct from a filler even though both leave the gutter
    /// empty. Called once per display row of the same logical row with a rising
    /// `sub_row`, so an added/removed row's band (UI-R-278) paints every continuation
    /// display row of a wrapped entry too (UI-R-279), its blank gutter drawn on top of
    /// the band rather than leaving a hole in the colour.
    // One argument per independently-varying render input (row position, kind, side,
    // entry, labels, gutter width, language, the counterpart text for word emphasis);
    // grouping them into a context struct would just move the same count into field
    // access without reducing it.
    #[allow(clippy::too_many_arguments)]
    fn draw_entry(
        &self,
        buf: &mut Buffer,
        rect: Rect,
        row_idx: usize,
        kind: &DiffKind,
        side: Side,
        entry: Option<&DiffEntry>,
        labels: Option<&Vec<String>>,
        gutter_width: u16,
        language: Option<ferrowl_syntax::Language>,
        h_scroll: usize,
        wrap: bool,
        sub_row: usize,
        marked_ranges: &[MarkedRange],
        counterpart: Option<&str>,
    ) {
        if rect.width == 0 {
            return;
        }
        let style = self.side_style(kind, side);
        // UI-R-278: an added/removed row's style paints this side's whole pane width —
        // gutter digits, separator, text and any trailing blank cells past a short
        // line — but only on the side that actually holds the entry (UI-E-122): a filler
        // side of an added/removed row keeps the general style. A wrapped row's
        // continuation display rows reuse this same call (UI-R-279), so the band covers
        // them too without a separate branch.
        let row_style = if entry.is_some() && matches!(kind, DiffKind::Added | DiffKind::Removed) {
            style
        } else {
            self.style.general
        };
        buf.set_style(rect, row_style);
        let continuation = sub_row > 0;
        let gutter_text_width = gutter_width.saturating_sub(1);
        if gutter_text_width > 0 {
            let text = if continuation {
                String::new()
            } else {
                gutter_text_for(row_idx, entry, labels)
            };
            let gutter_rect = Rect::new(rect.x, rect.y, gutter_text_width, 1);
            // UI-R-267: a marked range's colour fills the whole gutter cell as a
            // background, under the label or line-number text (UI-R-268) — the label's
            // own style keeps its foreground but drops its background so the fill shows
            // through. Painted after `row_style` above, so a marked range's colour wins
            // on the digit cells while the rest of the row keeps the added/removed band
            // (UI-E-121).
            let marked = entry.and_then(|e| {
                marked_ranges
                    .iter()
                    .find(|m| m.side == side && m.lines.contains(&e.line_no))
            });
            let gutter_style = if let Some(m) = marked {
                buf.set_style(gutter_rect, Style::default().bg(m.color));
                Style {
                    bg: None,
                    ..row_style
                }
            } else {
                row_style
            };
            let gutter_str = format!("{text:>width$}", width = gutter_text_width as usize);
            Paragraph::new(Text::from(gutter_str).style(gutter_style)).render(gutter_rect, buf);
        }
        let text_x = rect.x + gutter_width;
        let text_width = rect.width.saturating_sub(gutter_width);
        if text_width == 0 {
            return;
        }
        if let Some(e) = entry {
            // Highlighting is computed against the entry's full text, then `h_scroll`
            // drops leading characters (UI-R-232): highlighting a pre-truncated string
            // would shift every span's start against the language's real column.
            let mut chars = styled_chars(&e.text, style, language, &self.syntax_theme);
            // UI-R-281, UI-R-283: the emphasis is applied before wrapping (UI-E-125) and
            // before the `h_scroll` skip below, so both paths inherit it style-preserving.
            let emphasis = match (kind, counterpart) {
                (DiffKind::Added | DiffKind::Removed, Some(other)) => {
                    word_diff_spans(&e.text, other)
                }
                _ => Vec::new(),
            };
            if !emphasis.is_empty() {
                let emph_style = match side {
                    Side::Old => self.style.removed_word,
                    Side::New => self.style.added_word,
                };
                if let Some(bg) = emph_style.bg {
                    for span in &emphasis {
                        for s in chars.iter_mut().take(span.end).skip(span.start) {
                            s.1 = s.1.bg(bg);
                        }
                    }
                }
            }
            if wrap {
                let wrapped =
                    crate::widgets::markdown_render::word_wrap(&chars, text_width as usize, 0);
                let Some(chunk) = wrapped.get(sub_row) else {
                    return;
                };
                let line = Line::from(
                    chunk
                        .iter()
                        .map(|(t, s)| Span::styled(t.clone(), *s))
                        .collect::<Vec<_>>(),
                );
                Paragraph::new(Text::from(line))
                    .render(Rect::new(text_x, rect.y, text_width, 1), buf);
            } else {
                let visible: Vec<(char, Style)> = chars.into_iter().skip(h_scroll).collect();
                let line = Line::from(
                    group_runs(visible)
                        .into_iter()
                        .map(|(t, s)| Span::styled(t, s))
                        .collect::<Vec<_>>(),
                );
                Paragraph::new(Text::from(line))
                    .render(Rect::new(text_x, rect.y, text_width, 1), buf);
            }
        }
    }

    /// Draws one display row of a meta row (UI-R-210): the full width, in the meta style.
    /// A meta row wraps like any other row (UI-R-260 exempts none), so with `wrap` on
    /// `sub_row` selects which of its wrapped chunks this call draws.
    fn draw_meta(&self, buf: &mut Buffer, rect: Rect, text: &str, wrap: bool, sub_row: usize) {
        // The meta style covers the whole rect first: a `Paragraph` only paints the cells
        // its text occupies, so a row wider than `text` would otherwise show a trailing
        // run of unstyled (`general`) cells past the end of the line (UI-R-210).
        buf.set_style(rect, self.style.meta);
        if wrap {
            let chars: Vec<(char, Style)> = text.chars().map(|c| (c, self.style.meta)).collect();
            let wrapped =
                crate::widgets::markdown_render::word_wrap(&chars, rect.width.max(1) as usize, 0);
            let Some(chunk) = wrapped.get(sub_row) else {
                return;
            };
            let line = Line::from(
                chunk
                    .iter()
                    .map(|(t, s)| Span::styled(t.clone(), *s))
                    .collect::<Vec<_>>(),
            );
            Paragraph::new(Text::from(line)).render(rect, buf);
        } else if sub_row == 0 {
            Paragraph::new(Text::from(text.to_string()).style(self.style.meta)).render(rect, buf);
        }
    }

    /// Measures every annotation's own text height at `inner_width` (UI-R-272), using a
    /// read-only, line-numberless [`MarkdownInputField`] — the same field `draw_annotation`
    /// draws with, so the measured and drawn layouts cannot differ.
    fn measure_annotations(&self, annotations: &[Annotation], inner_width: u16) -> Vec<usize> {
        let field = MarkdownInputFieldBuilder::default()
            .line_numbers(false)
            .style(
                InputFieldStyleBuilder::default()
                    .general(self.style.general)
                    .build()
                    .expect("InputFieldStyleBuilder fields all default"),
            )
            .build()
            .expect("MarkdownInputFieldBuilder fields all default");
        annotations
            .iter()
            .map(|a| field.measure(&a.text, inner_width))
            .collect()
    }

    /// Each annotation's height in the bordered split layout (UI-R-309): measured at both
    /// panes' inner widths and reserved at the greater, so a block occupies the same display
    /// rows in both panes and no aligned row below it is off by a row between them.
    fn measure_annotations_paired(
        &self,
        annotations: &[Annotation],
        old_inner: u16,
        new_inner: u16,
    ) -> Vec<usize> {
        self.measure_annotations(annotations, old_inner)
            .into_iter()
            .zip(self.measure_annotations(annotations, new_inner))
            .map(|(old, new)| std::cmp::max(old, new))
            .collect()
    }

    /// Draws one annotation's bordered block (UI-R-270) into `rect`, whose own top row is
    /// the block's `skip_rows`'th row rather than always its own row zero (UI-R-265): a
    /// window scrolled to display rows past the block's own top border must still show
    /// the portion of the block that falls inside it, not skip the whole block because
    /// its own first row is off the top. `skip_rows == 0` draws the ordinary top-bordered
    /// block; `skip_rows >= 1` omits the top border (already scrolled past) and starts
    /// the body `skip_rows - 1` of its own display rows in. The body is a read-only
    /// [`MarkdownInputField`] over the annotation's text, so it renders markdown rather
    /// than showing the source (UI-R-271).
    fn draw_annotation(
        &self,
        buf: &mut Buffer,
        rect: Rect,
        text: &str,
        total_height: u16,
        skip_rows: u16,
    ) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        // Rendered once into a scratch buffer sized to the block's own full height,
        // starting at its own row zero, then only the window's visible slice is copied
        // into `buf` at `rect` (UI-R-265): a window scrolled to somewhere past the
        // block's own top border must still show the portion that falls inside it, which
        // a render confined to `rect` alone (whose height is already the clipped
        // remainder) cannot reconstruct on its own.
        let scratch_rect = Rect::new(0, 0, rect.width, total_height);
        let mut scratch = Buffer::empty(scratch_rect);
        let block = Block::bordered().style(self.style.border);
        let inner = block.inner(scratch_rect);
        block.render(scratch_rect, &mut scratch);
        let mut state = MarkdownInputFieldStateBuilder::default()
            .build()
            .expect("MarkdownInputFieldStateBuilder fields all default");
        state.set_content(text);
        state.set_read_only(true);
        // `MarkdownInputField::render` opens with its own `buf.set_style(area, general)`
        // over the whole rect it is given (UI-E-143): that is the surplus-column fill, so
        // it must paint in the diff widget's own general style, not the field's default.
        // A read-only, unfocusable block also has no meaningful active row, so its
        // active-row highlight (`markdown_theme.highlighted_row`) is styled to the same
        // general color: every body row, including its first, stays in general (UI-E-143
        // holds with no exception), not just the rows past the active one.
        let field = MarkdownInputFieldBuilder::default()
            .line_numbers(false)
            .style(
                InputFieldStyleBuilder::default()
                    .general(self.style.general)
                    .build()
                    .expect("InputFieldStyleBuilder fields all default"),
            )
            .markdown_theme(
                MarkdownThemeBuilder::default()
                    .highlighted_row(self.style.general)
                    .build()
                    .expect("MarkdownThemeBuilder fields all default"),
            )
            .build()
            .expect("MarkdownInputFieldBuilder fields all default");
        StatefulWidget::render(&field, inner, &mut scratch, &mut state);
        // `rect` is already clipped to the pane's own remaining height/width, but not
        // necessarily to `buf`'s own area: a direct `buf[(x, y)]` index, unlike
        // `Paragraph`/`buf.set_style`, does not clip on its own and panics past it.
        let dest = rect.intersection(buf.area);
        for row in 0..dest.height {
            let src_y = skip_rows + (dest.y - rect.y) + row;
            if src_y >= total_height {
                break;
            }
            for col in 0..dest.width {
                let src_x = (dest.x - rect.x) + col;
                buf[(dest.x + col, dest.y + row)] = scratch[(src_x, src_y)].clone();
            }
        }
    }

    fn pane_border_style(&self, focused: bool) -> Style {
        if focused {
            self.style.focused
        } else {
            self.style.border
        }
    }

    /// Renders `border`/`margin`/`title` around `rect` for one pane, styled by whether
    /// the widget itself is focused (UI-R-306, UI-R-307), and returns the inner content
    /// area. `title` is drawn only when `show_title`, since the split layout has two
    /// panes sharing one widget-level title and it belongs on one of them, not
    /// duplicated on both.
    fn pane_area(&self, area: Rect, buf: &mut Buffer, focused: bool, show_title: bool) -> Rect {
        let Border::Full(m) = &self.border else {
            return area;
        };
        let mut block = Block::bordered().style(self.pane_border_style(focused));
        if show_title && let Some(t) = &self.title {
            block = block.title(t.name.as_str()).title_alignment(t.alignment);
        }
        let inner = block.inner(area);
        block.render(area, buf);
        inner.inner(*m)
    }
}

impl StatefulWidget for &DiffView {
    type State = DiffViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        buf.set_style(area, self.style.general);

        let area = Layout::vertical([
            Constraint::Length(self.margin.vertical),
            Constraint::Min(1),
            Constraint::Length(self.margin.vertical),
        ])
        .split(area)[1];

        let area = Layout::horizontal([
            Constraint::Length(self.margin.horizontal),
            Constraint::Min(1),
            Constraint::Length(self.margin.horizontal),
        ])
        .split(area)[1];

        let rows = state.rows().to_vec();
        let focused = state.is_focused();
        let old_labels = state.old_labels().clone();
        let new_labels = state.new_labels().clone();
        let marked_ranges = state.marked_ranges().clone();
        let language = state.language;
        let scroll_offset = state.scroll_offset();
        let h_scroll = state.h_scroll();
        let wrap = state.wrap();

        match state.layout() {
            DiffLayout::Split => {
                // `Constraint::Percentage(50)` twice rounds unevenly on an odd width
                // (UI-R-211 requires equal panes), so split the width explicitly instead.
                // The separator absorbs whatever the two equal panes leave over, rather
                // than one pane taking the odd column (UI-R-211, UI-E-130); under three
                // columns there is nothing to spare and the panes abut (UI-E-132).
                // Bordered, the two pane borders are the seam already (UI-R-287), and
                // every row keeps the general style across the seam by construction,
                // since every band and gutter rect below is built from `old_area`/
                // `new_area`, never from the outer `area` (UI-R-288).
                let (half, gap) = match self.border {
                    Border::None if area.width >= 3 => {
                        let half = (area.width - 1) / 2;
                        (half, area.width - 2 * half)
                    }
                    _ => (area.width / 2, 0),
                };
                let panes = Layout::horizontal([
                    Constraint::Length(half),
                    Constraint::Length(gap),
                    Constraint::Length(half),
                ])
                .split(area);
                let old_area = self.pane_area(panes[0], buf, focused, true);
                let new_area = self.pane_area(panes[2], buf, focused, false);

                if old_area.height == 0 {
                    return;
                }
                state.set_visible_height(old_area.height as usize);

                let old_gutter = clamp_gutter(
                    side_gutter_text_width(&rows, old_labels.as_ref(), Side::Old),
                    old_area.width,
                );
                let new_gutter = clamp_gutter(
                    side_gutter_text_width(&rows, new_labels.as_ref(), Side::New),
                    new_area.width,
                );
                state.set_content_widths(
                    old_area.width.saturating_sub(old_gutter).max(1) as usize,
                    new_area.width.saturating_sub(new_gutter).max(1) as usize,
                );
                // The borderless full span (both panes and any separator), used by
                // annotations without a border (UI-R-308) since the separator column(s)
                // between the panes and an odd unused column at odd widths both sit
                // outside `old_area`/`new_area`.
                let full_width = (area.x + area.width).saturating_sub(old_area.x);

                // A row drawn inside the area it is drawn in: one pane's inner width when
                // bordered (UI-R-304, UI-R-308), each pane's border cells its own
                // boundary; the full outer width when borderless (UI-E-131), since there
                // the panes share no border to stay inside of. One binding serves meta
                // rows and annotations alike.
                let block_width = if matches!(self.border, Border::Full(_)) {
                    old_area.width
                } else {
                    full_width
                };
                state.set_meta_width(block_width as usize);

                let annotations = state.annotations().clone();
                let annotation_heights = if matches!(self.border, Border::Full(_)) {
                    self.measure_annotations_paired(
                        &annotations,
                        old_area.width.saturating_sub(2).max(1),
                        new_area.width.saturating_sub(2).max(1),
                    )
                } else {
                    self.measure_annotations(&annotations, full_width.saturating_sub(2).max(1))
                };
                state.set_annotation_heights(annotation_heights.clone());

                let visible_height = old_area.height as usize;
                let display = state.display_rows();
                // Tracks which annotations this render has already drawn: a block spans
                // several display rows, and the window can start mid-block (UI-R-265),
                // so "first `RowPart::Annotation` seen for this index" — not "`sub_row ==
                // 0`" — is what triggers the one draw.
                let mut drawn_annotations = std::collections::HashSet::new();
                // The rendered row window starts at `scroll_offset` display rows, not
                // display row zero (amended UI-R-231); `display_idx` is a display row's
                // position within that window.
                for (display_idx, d) in display
                    .iter()
                    .enumerate()
                    .skip(scroll_offset)
                    .take(visible_height)
                {
                    let row_idx = d.logical;
                    let y_old = old_area.y + (display_idx - scroll_offset) as u16;
                    let y_new = new_area.y + (display_idx - scroll_offset) as u16;
                    match d.part {
                        RowPart::Meta { sub_row } => {
                            let DiffRow::Meta { text } = &rows[row_idx] else {
                                unreachable!(
                                    "display_rows() pairs RowPart::Meta with DiffRow::Meta"
                                )
                            };
                            self.draw_meta(
                                buf,
                                Rect::new(old_area.x, y_old, block_width, 1),
                                text,
                                wrap,
                                sub_row,
                            );
                            if matches!(self.border, Border::Full(_)) {
                                self.draw_meta(
                                    buf,
                                    Rect::new(new_area.x, y_new, block_width, 1),
                                    text,
                                    wrap,
                                    sub_row,
                                );
                            }
                        }
                        RowPart::Pair { old_sub, new_sub } => {
                            let DiffRow::Pair { kind, old, new } = &rows[row_idx] else {
                                unreachable!(
                                    "display_rows() pairs RowPart::Pair with DiffRow::Pair"
                                )
                            };
                            let old_rect = Rect::new(old_area.x, y_old, old_area.width, 1);
                            if let Some(sub) = old_sub {
                                self.draw_entry(
                                    buf,
                                    old_rect,
                                    row_idx,
                                    kind,
                                    Side::Old,
                                    old.as_ref(),
                                    old_labels.as_ref(),
                                    old_gutter,
                                    language,
                                    h_scroll,
                                    wrap,
                                    sub,
                                    &marked_ranges,
                                    new.as_ref().map(|e| e.text.as_str()),
                                );
                            } else {
                                // UI-R-262: the shorter side pads its remaining rows
                                // blank, once the taller side has wrapped past it.
                                buf.set_style(old_rect, self.style.general);
                            }
                            let new_rect = Rect::new(new_area.x, y_new, new_area.width, 1);
                            if let Some(sub) = new_sub {
                                self.draw_entry(
                                    buf,
                                    new_rect,
                                    row_idx,
                                    kind,
                                    Side::New,
                                    new.as_ref(),
                                    new_labels.as_ref(),
                                    new_gutter,
                                    language,
                                    h_scroll,
                                    wrap,
                                    sub,
                                    &marked_ranges,
                                    old.as_ref().map(|e| e.text.as_str()),
                                );
                            } else {
                                buf.set_style(new_rect, self.style.general);
                            }
                        }
                        RowPart::Annotation { index, sub_row } => {
                            // Drawn once per pane in the bordered split, once across the
                            // widget otherwise (UI-R-270, UI-R-308); later `sub_row`s of
                            // the same block draw nothing further (UI-E-116).
                            if drawn_annotations.insert(index) {
                                let total = annotation_heights[index] as u16 + 2;
                                let own_remaining = total.saturating_sub(sub_row as u16);
                                let old_remaining =
                                    (old_area.y + old_area.height).saturating_sub(y_old);
                                let new_remaining =
                                    (new_area.y + new_area.height).saturating_sub(y_new);
                                self.draw_annotation(
                                    buf,
                                    Rect::new(
                                        old_area.x,
                                        y_old,
                                        block_width,
                                        own_remaining.min(old_remaining),
                                    ),
                                    &annotations[index].text,
                                    total,
                                    sub_row as u16,
                                );
                                if matches!(self.border, Border::Full(_)) {
                                    self.draw_annotation(
                                        buf,
                                        Rect::new(
                                            new_area.x,
                                            y_new,
                                            new_area.width,
                                            own_remaining.min(new_remaining),
                                        ),
                                        &annotations[index].text,
                                        total,
                                        sub_row as u16,
                                    );
                                }
                            }
                            continue;
                        }
                    }
                    self.paint_row_highlight(
                        buf,
                        state,
                        row_idx,
                        &[
                            Rect::new(old_area.x, y_old, old_area.width, 1),
                            Rect::new(new_area.x, y_new, new_area.width, 1),
                        ],
                    );
                }
            }
            DiffLayout::Unified => {
                let pane = self.pane_area(area, buf, focused, true);
                if pane.height == 0 {
                    return;
                }
                // Screen rows, not aligned rows: a changed pair draws two of them here, so
                // paging code must count display rows in this layout and aligned rows
                // (`state.rows().len()`) in split, rather than assuming the two agree.
                state.set_visible_height(pane.height as usize);

                let gutter = clamp_gutter(
                    side_gutter_text_width(&rows, old_labels.as_ref(), Side::Old).max(
                        side_gutter_text_width(&rows, new_labels.as_ref(), Side::New),
                    ),
                    pane.width,
                );
                state.set_content_widths(
                    pane.width.saturating_sub(gutter).max(1) as usize,
                    pane.width.saturating_sub(gutter).max(1) as usize,
                );
                state.set_meta_width(pane.width as usize);

                let annotations = state.annotations().clone();
                let annotation_heights =
                    self.measure_annotations(&annotations, pane.width.saturating_sub(2).max(1));
                state.set_annotation_heights(annotation_heights.clone());

                let display = state.display_rows();
                let visible_height = pane.height as usize;
                let mut drawn_annotations = std::collections::HashSet::new();
                for (display_idx, d) in display
                    .iter()
                    .enumerate()
                    .skip(scroll_offset)
                    .take(visible_height)
                {
                    let row_idx = d.logical;
                    let y = pane.y + (display_idx - scroll_offset) as u16;
                    let rect = Rect::new(pane.x, y, pane.width, 1);
                    match d.part {
                        RowPart::Meta { sub_row } => {
                            let DiffRow::Meta { text } = &rows[row_idx] else {
                                unreachable!(
                                    "display_rows() pairs RowPart::Meta with DiffRow::Meta"
                                )
                            };
                            self.draw_meta(buf, rect, text, wrap, sub_row);
                        }
                        RowPart::Pair { old_sub, new_sub } => {
                            let DiffRow::Pair { kind, old, new } = &rows[row_idx] else {
                                unreachable!(
                                    "display_rows() pairs RowPart::Pair with DiffRow::Pair"
                                )
                            };
                            if let Some(sub) = old_sub {
                                self.draw_entry(
                                    buf,
                                    rect,
                                    row_idx,
                                    kind,
                                    Side::Old,
                                    old.as_ref(),
                                    old_labels.as_ref(),
                                    gutter,
                                    language,
                                    h_scroll,
                                    wrap,
                                    sub,
                                    &marked_ranges,
                                    new.as_ref().map(|e| e.text.as_str()),
                                );
                            }
                            if let Some(sub) = new_sub {
                                self.draw_entry(
                                    buf,
                                    rect,
                                    row_idx,
                                    kind,
                                    Side::New,
                                    new.as_ref(),
                                    new_labels.as_ref(),
                                    gutter,
                                    language,
                                    h_scroll,
                                    wrap,
                                    sub,
                                    &marked_ranges,
                                    old.as_ref().map(|e| e.text.as_str()),
                                );
                            }
                        }
                        RowPart::Annotation { index, sub_row } => {
                            if drawn_annotations.insert(index) {
                                let total = annotation_heights[index] as u16 + 2;
                                let own_remaining = total.saturating_sub(sub_row as u16);
                                let pane_remaining = (pane.y + pane.height).saturating_sub(y);
                                self.draw_annotation(
                                    buf,
                                    Rect::new(
                                        pane.x,
                                        y,
                                        pane.width,
                                        own_remaining.min(pane_remaining),
                                    ),
                                    &annotations[index].text,
                                    total,
                                    sub_row as u16,
                                );
                            }
                            continue;
                        }
                    }
                    self.paint_row_highlight(buf, state, row_idx, &[rect]);
                }
            }
        }
    }
}

impl DiffView {
    /// Restyles `row_idx`'s cells in every given screen rect at once (UI-R-224, UI-R-225):
    /// the active row in the read-only highlighted-row style, and, in Visual mode, every
    /// row in the selection range in the selection style. Painting every rect from one row
    /// index is what keeps a row that spans two screen rows (the unified layout's changed
    /// pairs) highlighted on both, and both panes agreeing in the split layout.
    fn paint_row_highlight(
        &self,
        buf: &mut Buffer,
        state: &DiffViewState,
        row_idx: usize,
        rects: &[Rect],
    ) {
        let selected = state.mode() == DiffMode::Visual
            && state.selected_rows().is_some_and(|r| r.contains(&row_idx));
        let is_active = row_idx == state.active_row();
        if selected {
            for rect in rects {
                buf.set_style(*rect, self.style.selection);
            }
        }
        if is_active {
            for rect in rects {
                buf.set_style(*rect, self.style.highlighted_row);
            }
        }
    }
}

impl StatefulWidget for DiffView {
    type State = DiffViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DiffViewStateBuilder;
    use crate::style::DiffViewStyleBuilder;
    use crate::traits::SetFocus;
    use ratatui::style::Color;

    fn buffer(w: u16, h: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, w, h))
    }

    fn row_text(b: &Buffer, y: u16, w: u16) -> String {
        (0..w)
            .map(|x| b[(x, y)].symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    fn state_with(text: &str) -> DiffViewState {
        DiffViewStateBuilder::default()
            .build_with_diff(text)
            .unwrap()
    }

    #[test]
    /// UI-R-210, UI-R-276 — a meta row (the hunk header itself, here) spans the
    /// full width in the meta style the *builder* set on `DiffViewStyle`, with blank
    /// gutters on both sides.
    fn ut_meta_row_spans_the_full_width_in_the_meta_style_with_blank_gutters() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n context\n");
        let meta = Style::default().fg(ratatui::style::Color::Magenta);
        let w = DiffViewBuilder::default()
            .style(DiffViewStyleBuilder::default().meta(meta).build().unwrap())
            .build()
            .unwrap();
        let mut b = buffer(21, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 21, 2), &mut b, &mut st);
        let line = row_text(&b, 0, 21);
        assert!(line.starts_with("@@ -1,1 +1,1 @@"));
        // Full width (UI-R-210), including the odd trailing column an even split leaves
        // unused by either pane, and the builder's own meta style across the whole row,
        // not just its first cell, and not `SyntaxTheme::default().meta`.
        for x in 0..21 {
            assert_eq!(
                b[(x, 0)].fg,
                meta.fg.unwrap(),
                "column {x} not in the builder's meta style"
            );
        }
        // Blank gutter on every side: the content row below carries a gutter digit at
        // column 0, the meta row above it does not.
        assert_ne!(b[(0, 0)].symbol(), b[(0, 1)].symbol());
        assert_eq!(b[(0, 0)].symbol(), "@");
    }

    #[test]
    /// UI-R-211 — split layout draws the old and new sides of the same aligned row on the
    /// same screen row.
    fn ut_split_layout_draws_corresponding_lines_on_the_same_screen_row() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n-old text\n+new text\n");
        let w = DiffView::default();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut st);
        let line = row_text(&b, 1, 40);
        assert!(line[..20].contains("old text"));
        assert!(line[20..].contains("new text"));
    }

    #[test]
    /// UI-R-209, UI-R-212 — a filler side renders a blank gutter and no text.
    fn ut_filler_side_renders_a_blank_gutter_and_no_text() {
        let mut st = state_with("@@ -1,1 +1,2 @@\n-a\n+x\n+y\n");
        let w = DiffView::default();
        let mut b = buffer(40, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 3), &mut b, &mut st);
        let line = row_text(&b, 2, 40);
        assert!(line[..20].trim().is_empty());
        assert!(line[20..].contains('y'));
    }

    #[test]
    /// UI-R-213 — unified layout draws a changed pair as old-then-new across two screen
    /// rows, and everything else as a single row.
    fn ut_unified_layout_draws_old_then_new_for_a_changed_row_and_one_row_otherwise() {
        let mut st = DiffViewStateBuilder::default()
            .layout(DiffLayout::Unified)
            .build_with_diff("@@ -1,2 +1,2 @@\n same\n-old text\n+new text\n")
            .unwrap();
        let w = DiffView::default();
        let mut b = buffer(20, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 5), &mut b, &mut st);
        assert!(row_text(&b, 1, 20).contains("same"));
        assert!(row_text(&b, 2, 20).contains("old text"));
        assert!(row_text(&b, 3, 20).contains("new text"));
    }

    #[test]
    /// UI-R-216, UI-E-127 — no marker column is drawn between gutter and text: the cell
    /// immediately after the gutter digits is blank, no `-` or `+` appears anywhere in
    /// either rendered row, and the first text character sits at the same column it does
    /// on a context row.
    fn ut_no_marker_column_is_drawn_between_gutter_and_text() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n-old\n+new\n context\n");
        let w = DiffView::default();
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        let changed = row_text(&b, 1, 20);
        let context = row_text(&b, 2, 20);
        assert_eq!(&changed[1..2], " ", "old pane separator is blank");
        assert_eq!(&changed[12..13], " ", "new pane separator is blank");
        assert!(!changed.contains('-'));
        assert!(!changed.contains('+'));
        assert!(changed[2..10].starts_with("old"));
        assert!(changed[13..].starts_with("new"));
        assert!(
            context[2..10].starts_with("context"),
            "text starts at the same column as a changed row's"
        );
    }

    #[test]
    /// UI-R-278 — an added/removed row paints every cell of the row: gutter digits,
    /// separator column, text and the blank cells past the end of a short line.
    fn ut_added_and_removed_rows_paint_every_cell_of_the_row() {
        for layout in [DiffLayout::Split, DiffLayout::Unified] {
            let mut st = DiffViewStateBuilder::default()
                .layout(layout)
                .build_with_diff("@@ -1,1 +1,1 @@\n-a\n+b\n")
                .unwrap();
            // "a" and "b" share no token (UI-E-126), so word emphasis would otherwise
            // paint the whole cell; pin it to the same colors as the row bands so this
            // test still pins the plain band, unaffected by the (separately tested)
            // emphasis.
            let mut w = DiffView::default();
            let removed = w.style.removed;
            let added = w.style.added;
            w.style.set_removed_word(removed);
            w.style.set_added_word(added);
            let mut b = buffer(20, 3);
            StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
            // Row 0 is the meta header. Split draws old and new on the same row 1;
            // unified draws old on row 1 and new on row 2.
            let (removed_y, added_y) = if layout == DiffLayout::Split {
                (1u16, 1u16)
            } else {
                (1u16, 2u16)
            };
            let old_range: std::ops::Range<u16> = if layout == DiffLayout::Split {
                0..9
            } else {
                0..20
            };
            let new_range: std::ops::Range<u16> = if layout == DiffLayout::Split {
                11..20
            } else {
                0..20
            };
            for x in old_range {
                assert_eq!(
                    b[(x, removed_y)].bg,
                    w.style.removed.bg.unwrap(),
                    "layout {layout:?} old pane column {x}"
                );
            }
            for x in new_range {
                assert_eq!(
                    b[(x, added_y)].bg,
                    w.style.added.bg.unwrap(),
                    "layout {layout:?} new pane column {x}"
                );
            }
        }
    }

    #[test]
    /// UI-R-279 — a wrapped added row paints every continuation display row, including
    /// its blank gutter, in the added band.
    fn ut_wrapped_added_row_paints_every_continuation_display_row() {
        let mut st = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@ -1,1 +1,1 @@\n+aaaa bbbb\n")
            .unwrap();
        let w = DiffView::default();
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        // Row 0 is the meta header, row 1 the pair row's first display row, row 2 its
        // wrapped continuation.
        for x in 11..20 {
            assert_eq!(
                b[(x, 1)].bg,
                w.style.added.bg.unwrap(),
                "first display row column {x}"
            );
            assert_eq!(
                b[(x, 2)].bg,
                w.style.added.bg.unwrap(),
                "continuation display row column {x}"
            );
        }
    }

    #[test]
    /// UI-E-121 — a marked range's colour wins on the gutter of a painted row, the rest
    /// of the row keeping the added/removed style.
    fn ut_marked_range_colour_wins_on_the_gutter_of_a_painted_row() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n-a\n+b\n");
        st.set_marked_ranges(vec![MarkedRange {
            side: Side::Old,
            lines: 1..=1,
            color: ratatui::style::Color::Yellow,
        }]);
        let w = DiffView::default();
        let mut b = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b, &mut st);
        assert_eq!(b[(0, 1)].bg, ratatui::style::Color::Yellow);
        assert_eq!(
            b[(1, 1)].bg,
            w.style.removed.bg.unwrap(),
            "separator column keeps the row style, not the range colour"
        );
        assert_eq!(b[(5, 1)].bg, w.style.removed.bg.unwrap());
    }

    #[test]
    /// UI-E-122 — a filler side of a painted row stays unpainted: the old pane keeps the
    /// general style end to end, including its blank gutter cells, when the new side
    /// carries an added row and the old side has no entry.
    fn ut_filler_side_of_a_painted_row_stays_unpainted() {
        let mut st = state_with("@@ -1,1 +1,2 @@\n-a\n+x\n+y\n");
        let w = DiffView::default();
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        for x in 0..10 {
            assert_eq!(
                b[(x, 2)].bg,
                w.style.general.bg.unwrap(),
                "old pane column {x}"
            );
        }
    }

    #[test]
    /// UI-E-123, UI-R-288 — the split layout's shorter-side padding stays in the general
    /// style: UI-R-278/UI-R-279 paint only the display rows an entry actually occupies,
    /// so with wrapping on, the old side's padded display rows past its own text never
    /// carry the added/removed band, and neither does the separator column between the
    /// panes.
    fn ut_split_padding_rows_stay_in_the_general_style() {
        let mut st = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@ -1,1 +1,1 @@\n-x\n+aaaa bbbb cccc\n")
            .unwrap();
        let w = DiffView::default();
        let mut b = buffer(20, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 4), &mut b, &mut st);
        // Row 0 is the meta header, row 1 the real pair row (old side's own real text,
        // in the removed style), rows 2 and 3 the padding rows past the old side's own
        // text (the new side's "aaaa bbbb cccc" wraps to three display rows at this
        // width, the old side's "x" to one). The old pane is 9 columns (0..9); columns
        // 9 and 10 are the separator.
        for y in [2u16, 3u16] {
            for x in 0..11 {
                assert_eq!(
                    b[(x, y)].bg,
                    w.style.general.bg.unwrap(),
                    "old pane padded row {y} column {x}"
                );
            }
        }
    }

    #[test]
    /// UI-R-218 — gutter labels replace file line numbers per side, when set.
    fn ut_gutter_labels_replace_line_numbers_per_side() {
        let mut st = state_with("@@ -5,1 +9,1 @@\n context\n");
        st.set_old_labels(Some(vec![String::new(), "OLD".into()]));
        st.set_new_labels(Some(vec![String::new(), "NEW".into()]));
        let w = DiffView::default();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut st);
        let line = row_text(&b, 1, 40);
        assert!(line.contains("OLD"));
        assert!(line.contains("NEW"));
    }

    #[test]
    /// UI-R-218 — gutter labels are also settable through the state builder at build time,
    /// not only through the post-build setter.
    fn ut_gutter_labels_are_settable_through_the_state_builder() {
        let mut st = DiffViewStateBuilder::default()
            .old_labels(Some(vec![String::new(), "OLD".into()]))
            .new_labels(Some(vec![String::new(), "NEW".into()]))
            .build_with_diff("@@ -5,1 +9,1 @@\n context\n")
            .unwrap();
        let w = DiffView::default();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut st);
        let line = row_text(&b, 1, 40);
        assert!(line.contains("OLD"));
        assert!(line.contains("NEW"));
    }

    #[test]
    /// UI-R-267 — a marked range fills the gutter background of every row whose file line
    /// falls inside it, on the side it names, and leaves the rest of that side's gutter
    /// unpainted.
    fn ut_marked_range_fills_the_gutter_cells_of_every_row_it_covers() {
        let mut st = state_with("@@ -1,3 +1,3 @@\n a\n b\n c\n");
        st.set_marked_ranges(vec![MarkedRange {
            side: Side::Old,
            lines: 1..=2,
            color: ratatui::style::Color::Yellow,
        }]);
        let w = DiffView::default();
        // Row 0 is the `@@` header (a meta row); rows 1-3 hold file lines 1-3.
        let mut b = buffer(20, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 4), &mut b, &mut st);
        assert_eq!(b[(0, 1)].bg, ratatui::style::Color::Yellow);
        assert_eq!(b[(0, 2)].bg, ratatui::style::Color::Yellow);
        assert_ne!(b[(0, 3)].bg, ratatui::style::Color::Yellow);
    }

    #[test]
    /// UI-R-268 — a gutter label's text still shows over a marked range's colour; neither
    /// swallows the other.
    fn ut_a_gutter_label_keeps_its_text_under_a_marked_range_colour() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n a\n");
        st.set_old_labels(Some(vec![String::new(), "L".into()]));
        st.set_marked_ranges(vec![MarkedRange {
            side: Side::Old,
            lines: 1..=1,
            color: ratatui::style::Color::Blue,
        }]);
        let w = DiffView::default();
        // Row 0 is the `@@` header (a meta row); row 1 holds the labeled line.
        let mut b = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b, &mut st);
        assert_eq!(b[(0, 1)].symbol(), "L");
        assert_eq!(b[(0, 1)].bg, ratatui::style::Color::Blue);
    }

    #[test]
    /// UI-E-117 — a marked range covers only rows with a real line number on that side; a
    /// filler row's gutter stays blank and unpainted, visibly interrupting the block.
    fn ut_marked_range_leaves_a_filler_rows_gutter_blank() {
        let mut st = state_with("@@ -1,1 +1,2 @@\n-a\n+x\n+y\n");
        st.set_marked_ranges(vec![MarkedRange {
            side: Side::Old,
            lines: 1..=5,
            color: ratatui::style::Color::Yellow,
        }]);
        let w = DiffView::default();
        // Row 0 is the `@@` header (a meta row); row 1 is the paired line ("a"/"x"); row 2
        // is the filler on the old side ("y" has no old-side entry).
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        assert_eq!(b[(0, 1)].bg, ratatui::style::Color::Yellow);
        assert_ne!(b[(0, 2)].bg, ratatui::style::Color::Yellow);
    }

    #[test]
    /// UI-R-270 — an annotation block's scratch-buffer copy stays inside the real
    /// buffer's own bounds even when the rendered `area` extends past them, the same
    /// clipping every other draw path in this widget already gets from `Paragraph`/
    /// `buf.set_style`, rather than panicking on an out-of-bounds index.
    fn ut_annotation_block_copy_does_not_panic_past_the_buffers_own_bounds() {
        let mut st = DiffViewStateBuilder::default()
            .annotations(vec![Annotation {
                side: Side::Old,
                lines: 1..=1,
                text: "hi".into(),
            }])
            .build_with_diff("@@ -1,1 +1,1 @@\n a\n")
            .unwrap();
        let w = DiffView::default();
        // The buffer is narrower and shorter than the area handed to `render`: every
        // other draw call clips against the buffer's own bounds internally, but a direct
        // `buf[(x, y)]` index does not, so this must not panic.
        let mut b = buffer(10, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 6), &mut b, &mut st);
    }

    #[test]
    /// UI-R-270, UI-R-265 — an annotation block scrolled to somewhere past its own first
    /// row still draws its visible portion, clipped to the pane, rather than vanishing
    /// because its own top row is off the top of the window.
    fn ut_annotation_block_stays_visible_when_scrolled_to_its_middle() {
        let mut st = DiffViewStateBuilder::default()
            .annotations(vec![Annotation {
                side: Side::Old,
                lines: 1..=1,
                text: "a\nb\nc\nd".into(),
            }])
            .build_with_diff("@@ -1,1 +1,1 @@\n a\n")
            .unwrap();
        // Display rows: 0 = meta, 1 = the pair row, 2..=7 = the annotation's own 6 rows
        // (top border, then "a","b","c","d", then bottom border). Scrolling to 4 puts
        // the window's first visible row on "b", well past the block's own top border.
        st.set_scroll_offset(4);
        let w = DiffView::default();
        let mut b = buffer(40, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 4), &mut b, &mut st);
        assert!(
            row_text(&b, 0, 40).contains('b'),
            "the block's own second text line should be the window's first visible row"
        );
        assert_eq!(
            b[(0, 3)].symbol(),
            "└",
            "the block's own last visible row is its bottom border"
        );
    }

    #[test]
    /// UI-R-270 — an annotation renders as one bordered block directly beneath the last
    /// row of its range, spanning the full width in both layouts (both panes in split).
    fn ut_annotation_block_spans_the_width_beneath_the_last_row_of_its_range() {
        for layout in [DiffLayout::Split, DiffLayout::Unified] {
            let mut st = DiffViewStateBuilder::default()
                .layout(layout)
                .annotations(vec![Annotation {
                    side: Side::Old,
                    lines: 1..=2,
                    text: "hi".into(),
                }])
                .build_with_diff("@@ -1,2 +1,2 @@\n a\n b\n")
                .unwrap();
            let w = DiffView::default();
            let mut b = buffer(40, 6);
            StatefulWidget::render(&w, Rect::new(0, 0, 40, 6), &mut b, &mut st);
            // Rows 0-2 are the header and the two aligned lines; the block starts right
            // beneath row 2, spanning the full width as one block, not two separate
            // per-pane blocks.
            assert_eq!(b[(0, 3)].symbol(), "┌", "layout {layout:?}");
            assert_eq!(b[(39, 3)].symbol(), "┐", "layout {layout:?}");
        }
    }

    #[test]
    /// UI-R-308, UI-R-270, UI-E-144 — in a bordered split layout an annotation block is
    /// drawn once inside each pane's border, each spanning only that pane's inner width,
    /// so no annotation block ever crosses or overwrites a pane border.
    fn ut_bordered_split_draws_the_annotation_inside_each_pane() {
        let mut st = DiffViewStateBuilder::default()
            .annotations(vec![Annotation {
                side: Side::Old,
                lines: 1..=1,
                text: "hi".into(),
            }])
            .build_with_diff("@@ -1,1 +1,1 @@\n a\n")
            .unwrap();
        let w = DiffViewBuilder::default()
            .border(Border::Full(Margin::new(0, 0)))
            .build()
            .unwrap();
        let mut b = buffer(40, 8);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 8), &mut b, &mut st);

        // Row 0 is the pane border's own top row; row 1 = meta header, row 2 = the pair
        // row, rows 3..=5 = the block (top border, one-line body, bottom border), both
        // panes. Columns 0/19 and 20/39 are the panes' own border columns; the block's
        // own left border sits one column inside each pane, at 1 and 21.
        assert_eq!(b[(1, 3)].symbol(), "┌", "old pane's block top-left corner");
        assert_eq!(b[(21, 3)].symbol(), "┌", "new pane's block top-left corner");
        let border = w.style().border().fg.expect("style sets a color");
        for y in 3..=5 {
            assert_eq!(b[(0, y)].fg, border, "old pane's left border, row {y}");
            assert_eq!(b[(19, y)].fg, border, "old pane's right border, row {y}");
            assert_eq!(b[(20, y)].fg, border, "new pane's left border, row {y}");
            assert_eq!(b[(39, y)].fg, border, "new pane's right border, row {y}");
        }
    }

    #[test]
    /// UI-E-143, UI-E-144 — an annotation block's inner row whose text does not fill its
    /// inner width leaves the surplus columns in the widget's general style, not the
    /// block's border style, and the block's own border cells still carry the border
    /// style; both hold for a surplus row too, not just a surplus column.
    fn ut_annotation_block_fills_surplus_inner_columns_with_the_general_style() {
        let style = DiffViewStyleBuilder::default()
            .border(Style::default().fg(Color::Red).bg(Color::Blue))
            .general(Style::default().fg(Color::Green).bg(Color::Magenta))
            .build()
            .unwrap();
        let w = DiffViewBuilder::default().style(style).build().unwrap();
        let mut b = buffer(20, 5);
        // A read-only, unfocusable block has no meaningful active row, so its highlight
        // is suppressed (draw_annotation styles it to general): "hi" on line 0, the
        // field's active line, is asserted the same way as its second line "yo". `5`
        // reserves a third body row beyond both source lines, a wholly surplus row.
        w.draw_annotation(&mut b, Rect::new(0, 0, 20, 5), "hi\nyo", 5, 0);
        assert_eq!(b[(0, 0)].fg, Color::Red, "top border, left corner");
        assert_eq!(b[(19, 0)].fg, Color::Red, "top border, right corner");
        assert_eq!(b[(0, 4)].fg, Color::Red, "bottom border, left corner");
        let general_bg = w.style().general().bg.expect("style sets a color");
        assert_eq!(
            b[(10, 1)].bg,
            general_bg,
            "surplus column past \"hi\" on the active row"
        );
        assert_eq!(
            b[(10, 2)].bg,
            general_bg,
            "surplus column past \"yo\" on its own row"
        );
        assert_eq!(
            b[(10, 3)].bg,
            general_bg,
            "wholly surplus row, no source line at all"
        );
        assert_ne!(b[(10, 2)].bg, Color::Reset, "not a bare reset cell");
        assert_ne!(b[(10, 3)].bg, Color::Reset, "not a bare reset cell");
    }

    #[test]
    /// UI-R-271 — an annotation's body is rendered markdown, not its raw source: emphasis
    /// markers do not appear in the drawn text.
    fn ut_annotation_body_renders_markdown_not_its_source() {
        let mut st = DiffViewStateBuilder::default()
            .annotations(vec![Annotation {
                side: Side::Old,
                lines: 1..=1,
                text: "**bold**".into(),
            }])
            .build_with_diff("@@ -1,1 +1,1 @@\n a\n")
            .unwrap();
        let w = DiffView::default();
        let mut b = buffer(40, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 5), &mut b, &mut st);
        let body = row_text(&b, 3, 40);
        assert!(body.contains("bold"));
        assert!(!body.contains('*'));
    }

    #[test]
    /// UI-R-272 — an annotation block's height is its measured text rows, at the block's
    /// own inner width, plus its two border rows.
    fn ut_annotation_height_is_the_measured_rows_plus_its_border() {
        let text = "one two three four five six seven eight nine ten";
        let mut st = DiffViewStateBuilder::default()
            .annotations(vec![Annotation {
                side: Side::Old,
                lines: 1..=1,
                text: text.into(),
            }])
            .build_with_diff("@@ -1,1 +1,1 @@\n a\n")
            .unwrap();
        let w = DiffView::default();
        let mut b = buffer(20, 12);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 12), &mut b, &mut st);
        let inner_width = 20u16.saturating_sub(2);
        let field = MarkdownInputFieldBuilder::default()
            .line_numbers(false)
            .build()
            .unwrap();
        let measured = field.measure(text, inner_width);
        // Row 2 is the block's top border, row 1 the header/line above it: the bottom
        // border sits `measured + 1` rows below the top border.
        assert_eq!(b[(0, 2)].symbol(), "┌");
        assert_eq!(b[(0, 2 + measured as u16 + 1)].symbol(), "└");
    }

    #[test]
    /// UI-R-309 — a bordered split's annotation block is reserved at the greater of the
    /// two panes' own measurements, not one pane's alone, so a rendered frame — where the
    /// two inner widths always agree — never has to rely on that agreement to hold.
    fn ut_paired_annotation_measurement_takes_the_greater_height() {
        let w = DiffView::default();
        let annotations = vec![Annotation {
            side: Side::Old,
            lines: 1..=1,
            text: "one two three".into(),
        }];
        // At width 13 "one two three" fits in a single row; at width 6 it greedily wraps
        // to "one" / "two" / "three", three rows. The paired measurement must take the
        // greater of the two.
        let heights = w.measure_annotations_paired(&annotations, 13, 6);
        assert_eq!(heights[0], 3);
    }

    #[test]
    /// UI-E-115 — an annotation naming a side and line range no row covers renders
    /// nothing and does not panic.
    fn ut_out_of_range_annotation_and_marked_range_are_silently_not_rendered() {
        let mut st = DiffViewStateBuilder::default()
            .annotations(vec![Annotation {
                side: Side::New,
                lines: 999..=999,
                text: "note".into(),
            }])
            .marked_ranges(vec![MarkedRange {
                side: Side::New,
                lines: 999..=999,
                color: ratatui::style::Color::Yellow,
            }])
            .build_with_diff("@@ -1,1 +1,1 @@\n a\n")
            .unwrap();
        let w = DiffView::default();
        let mut b = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b, &mut st);
        assert_ne!(b[(0, 1)].bg, ratatui::style::Color::Yellow);
    }

    #[test]
    /// UI-E-116 — several annotations anchored to the same row draw one block after
    /// another, in the order they were supplied.
    fn ut_several_annotations_on_one_row_stack_in_supplied_order() {
        let mut st = DiffViewStateBuilder::default()
            .annotations(vec![
                Annotation {
                    side: Side::Old,
                    lines: 1..=1,
                    text: "first".into(),
                },
                Annotation {
                    side: Side::Old,
                    lines: 1..=1,
                    text: "second".into(),
                },
            ])
            .build_with_diff("@@ -1,1 +1,1 @@\n a\n")
            .unwrap();
        let w = DiffView::default();
        let mut b = buffer(40, 9);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 9), &mut b, &mut st);
        let mut first_row = None;
        let mut second_row = None;
        for y in 0..9 {
            let line = row_text(&b, y, 40);
            if line.contains("first") {
                first_row = Some(y);
            }
            if line.contains("second") {
                second_row = Some(y);
            }
        }
        assert!(first_row.unwrap() < second_row.unwrap());
    }

    #[test]
    /// UI-E-102 — a surplus gutter label past the row count is unrendered but still
    /// widens the gutter.
    fn ut_surplus_gutter_labels_are_unrendered_but_widen_the_gutter() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n context\n");
        st.set_old_labels(Some(vec![String::new(), "A".into(), "WIDE-LABEL".into()]));
        let w = DiffView::default();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut st);
        assert_eq!(b[(9, 1)].symbol(), "A");
    }

    #[test]
    /// UI-R-219 — added, removed and context text take their diff styles.
    fn ut_added_removed_and_context_text_take_their_diff_styles() {
        let mut st = state_with("@@ -1,3 +1,3 @@\n context\n-removed\n+added\n");
        let w = DiffView::default();
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        assert_eq!(
            b[(2, 1)].fg,
            w.style.general.fg.expect("style sets a color")
        );
        assert_eq!(
            b[(2, 2)].fg,
            w.style.removed.fg.expect("style sets a color")
        );
        assert_eq!(b[(12, 2)].fg, w.style.added.fg.expect("style sets a color"));
    }

    #[test]
    /// UI-R-219 — the added/removed/meta row styles are the widget's own
    /// (`DiffViewStyle`), not the syntax theme's; setting a distinctive style through the
    /// builder must be what the rendered cells take.
    fn ut_row_text_takes_the_widgets_own_styles_not_the_syntax_themes() {
        let mut st = state_with("@@ -1,3 +1,3 @@\n context\n-removed\n+added\n");
        // Row 0 (the meta header) must not be the active row: UI-R-224's highlighted-row
        // overlay paints its own background over whatever kind style drew there, which
        // would otherwise mask the meta assertion below.
        st.set_active_row(1);
        let mut w = DiffView::default();
        w.style.set_removed(
            Style::default()
                .fg(ratatui::style::Color::Magenta)
                .bg(ratatui::style::Color::Cyan),
        );
        w.style.set_added(
            Style::default()
                .fg(ratatui::style::Color::Yellow)
                .bg(ratatui::style::Color::Gray),
        );
        // "removed" and "added" share no token, so UI-E-126 emphasises the whole word;
        // pin the emphasis backgrounds to the same colors so the assertions below still
        // pin the widget's own row styles rather than the (separately tested) emphasis.
        w.style
            .set_removed_word(Style::default().bg(ratatui::style::Color::Cyan));
        w.style
            .set_added_word(Style::default().bg(ratatui::style::Color::Gray));
        w.style.set_meta(
            Style::default()
                .fg(ratatui::style::Color::Green)
                .bg(ratatui::style::Color::Black),
        );
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        assert_eq!(b[(2, 2)].fg, ratatui::style::Color::Magenta);
        assert_eq!(b[(2, 2)].bg, ratatui::style::Color::Cyan);
        assert_eq!(b[(12, 2)].fg, ratatui::style::Color::Yellow);
        assert_eq!(b[(12, 2)].bg, ratatui::style::Color::Gray);
        assert_eq!(b[(0, 0)].fg, ratatui::style::Color::Green);
        assert_eq!(b[(0, 0)].bg, ratatui::style::Color::Black);
    }

    #[test]
    /// UI-R-220 — with a language set, syntax highlighting supplies a foreground only over
    /// the diff-kind style; the diff style's background survives.
    fn ut_language_highlight_supplies_foreground_only_over_the_diff_style() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n-local x = 1\n+local x = 2\n");
        st.language = Some(ferrowl_syntax::Language::Lua);
        let mut w = DiffView::default();
        // A `removed` style carrying its own background and a modifier, distinct from
        // `general`'s: an implementation that used the syntax theme's span style wholesale
        // (dropping the diff style's background/modifiers) would still pass an assertion
        // against `general`'s background alone, since `keyword` sets no background either.
        w.style.set_removed(
            Style::default()
                .fg(ratatui::style::Color::Red)
                .bg(ratatui::style::Color::Blue)
                .add_modifier(ratatui::style::Modifier::BOLD),
        );
        let mut b = buffer(30, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 30, 2), &mut b, &mut st);
        assert_eq!(b[(2, 1)].bg, ratatui::style::Color::Blue);
        assert!(b[(2, 1)].modifier.contains(ratatui::style::Modifier::BOLD));
        assert_eq!(
            b[(2, 1)].fg,
            w.syntax_theme.keyword.fg.expect("style sets a color")
        );
    }

    #[test]
    /// UI-R-220 — the one `language` option (the per-side pair's collapse) highlights both
    /// sides: an added line on the new side and a removed line on the old side.
    fn ut_one_language_highlights_both_sides() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n-local x = 1\n+local y = 2\n");
        st.language = Some(ferrowl_syntax::Language::Lua);
        let w = DiffView::default();
        let mut b = buffer(30, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 30, 2), &mut b, &mut st);
        // The removed line's old side and the added line's new side (one aligned row,
        // both sides present) each take the keyword foreground on their `local`.
        assert_eq!(
            b[(2, 1)].fg,
            w.syntax_theme.keyword.fg.expect("style sets a color")
        );
        assert_eq!(
            b[(18, 1)].fg,
            w.syntax_theme.keyword.fg.expect("style sets a color")
        );
    }

    #[test]
    /// UI-R-221 — without a language, the diff style alone applies to the whole line.
    fn ut_without_a_language_the_diff_style_alone_applies() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n-local x = 1\n+local x = 2\n");
        let w = DiffView::default();
        let mut b = buffer(30, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 30, 2), &mut b, &mut st);
        assert_eq!(
            b[(2, 1)].fg,
            w.style.removed.fg.expect("style sets a color")
        );
        assert_eq!(
            b[(6, 1)].fg,
            w.style.removed.fg.expect("style sets a color")
        );
    }

    #[test]
    /// UI-R-224 — the active row is highlighted on both panes.
    fn ut_active_row_is_highlighted_on_both_panes() {
        let mut st = state_with("@@ -1,3 +1,3 @@\n a\n b\n c\n");
        st.set_active_row(3);
        let w = DiffView::default();
        let mut b = buffer(20, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 4), &mut b, &mut st);
        assert_eq!(
            b[(0, 3)].bg,
            w.style.highlighted_row.bg.expect("style sets a color")
        );
        assert_eq!(
            b[(11, 3)].bg,
            w.style.highlighted_row.bg.expect("style sets a color")
        );
        // A row drawn before the active one, never redrawn afterward, must stay in the
        // general style: a bug that painted the whole pane rect for the active row
        // (rather than just its own row) would leave this earlier row highlighted too.
        assert_eq!(
            b[(0, 1)].bg,
            w.style.general.bg.expect("style sets a color")
        );
        assert_eq!(
            b[(10, 1)].bg,
            w.style.general.bg.expect("style sets a color")
        );
    }

    #[test]
    /// UI-R-225 — a Visual-mode selection is painted on both panes.
    fn ut_visual_selection_is_painted_on_both_panes() {
        let mut st = state_with("@@ -1,3 +1,3 @@\n a\n b\n c\n");
        st.set_mode(DiffMode::Visual);
        st.set_anchor(Some(1));
        st.set_active_row(2);
        let mut w = DiffView::default();
        // Distinct from `highlighted_row`'s default (both default to `hi_bg`), so a row
        // painted selected-then-active can't accidentally read back as selection-colored.
        w.style
            .set_selection(Style::default().bg(ratatui::style::Color::Magenta));
        let mut b = buffer(20, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 4), &mut b, &mut st);
        // Row 1 ("a") is selected but not active.
        assert_eq!(
            b[(0, 1)].bg,
            w.style.selection.bg.expect("style sets a color")
        );
        assert_eq!(
            b[(11, 1)].bg,
            w.style.selection.bg.expect("style sets a color")
        );
        // Row 2 ("b") is the active row: highlighted, not selection-colored, even though
        // it is also inside the selection range.
        assert_eq!(
            b[(0, 2)].bg,
            w.style.highlighted_row.bg.expect("style sets a color")
        );
        // Row 3 ("c") is outside the selection.
        assert_eq!(
            b[(0, 3)].bg,
            w.style.general.bg.expect("style sets a color")
        );
    }

    #[test]
    /// UI-R-306 — both split-pane borders paint the focused style once the widget itself
    /// is focused, regardless of which side `focused_side` names.
    fn ut_focused_widget_paints_both_split_pane_borders_focused() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n a\n");
        st.set_focused_side(Side::New);
        SetFocus::set_focused(&mut st, true);
        let w = DiffViewBuilder::default()
            .border(Border::Full(Margin::new(0, 0)))
            .build()
            .unwrap();
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        assert_eq!(
            b[(0, 0)].fg,
            w.style.focused.fg.expect("style sets a color"),
            "old pane's border, despite focused_side naming New"
        );
        assert_eq!(
            b[(10, 0)].fg,
            w.style.focused.fg.expect("style sets a color"),
            "new pane's border"
        );
    }

    #[test]
    /// UI-R-307 — with the widget unfocused, both split-pane borders paint the normal
    /// style, even when `focused_side` names one of them.
    fn ut_unfocused_widget_paints_both_borders_normal() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n a\n");
        st.set_focused_side(Side::New);
        let w = DiffViewBuilder::default()
            .border(Border::Full(Margin::new(0, 0)))
            .build()
            .unwrap();
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        assert_eq!(
            b[(0, 0)].fg,
            w.style.border.fg.expect("style sets a color"),
            "old pane's border"
        );
        assert_eq!(
            b[(10, 0)].fg,
            w.style.border.fg.expect("style sets a color"),
            "new pane's border, despite focused_side naming it"
        );
    }

    #[test]
    /// UI-R-306 — in unified layout, with one pane, its border paints the focused style
    /// once the widget's focus flag is set.
    fn ut_focused_widget_paints_the_unified_pane_border_focused() {
        let mut st = DiffViewStateBuilder::default()
            .layout(DiffLayout::Unified)
            .build_with_diff("@@ -1,1 +1,1 @@\n a\n")
            .unwrap();
        SetFocus::set_focused(&mut st, true);
        let w = DiffViewBuilder::default()
            .border(Border::Full(Margin::new(0, 0)))
            .build()
            .unwrap();
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        assert_eq!(
            b[(0, 0)].fg,
            w.style.focused.fg.expect("style sets a color")
        );
    }

    #[test]
    /// UI-E-142 — a focus change on a borderless widget cannot repaint anything: the
    /// rendered buffer is identical focused or not.
    fn ut_focus_change_does_not_repaint_a_borderless_widget() {
        let mut unfocused = state_with("@@ -1,1 +1,1 @@\n a\n");
        let mut focused = state_with("@@ -1,1 +1,1 @@\n a\n");
        SetFocus::set_focused(&mut focused, true);
        let w = DiffView::default();
        let mut b_unfocused = buffer(20, 3);
        let mut b_focused = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b_unfocused, &mut unfocused);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b_focused, &mut focused);
        assert_eq!(b_unfocused, b_focused);
    }

    #[test]
    /// UI-R-230 (rendering half) — a nonzero vertical scroll offset selects the rendered
    /// row window on both panes: the first drawn row is `scroll_offset`, not row zero.
    fn ut_vertical_scroll_offset_selects_the_rendered_row_window_on_both_panes() {
        let mut st = state_with("@@ -1,3 +1,3 @@\n a\n b\n c\n");
        st.set_scroll_offset(2);
        let w = DiffView::default();
        let mut b = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b, &mut st);
        assert!(row_text(&b, 0, 20).contains('b'));
        assert!(row_text(&b, 1, 20).contains('c'));
    }

    #[test]
    /// UI-R-232 (rendering half) — a nonzero horizontal offset shifts the text of every
    /// pane, dropping that many leading characters, while the gutter columns stay put.
    fn ut_horizontal_offset_shifts_the_text_of_every_pane_leaving_gutters_in_place() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n-abcdefgh\n+xyzuvwtq\n");
        st.set_h_scroll(2);
        let w = DiffView::default();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut st);
        let line = row_text(&b, 1, 40);
        // Old pane: gutter "1", separator still blank at its column; text starts with 'c'
        // (the third character), the first two dropped.
        assert_eq!(&line[0..1], "1");
        assert_eq!(&line[1..2], " ");
        assert!(line[2..19].starts_with('c'));
        // New pane: same shift applied independently, at its own gutter columns.
        assert_eq!(&line[21..22], "1");
        assert_eq!(&line[22..23], " ");
        assert!(line[23..].starts_with('z'));
    }

    #[test]
    /// UI-R-261 — a continuation display row of a wrapped entry carries a blank
    /// gutter, its text starting at the same column as the row's first display row.
    fn ut_continuation_row_has_a_blank_gutter_and_aligned_text() {
        let mut st = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@ -1,1 +1,1 @@\n-aaaa bbbb\n+short\n")
            .unwrap();
        let w = DiffView::default();
        // Each 10-wide pane: gutter "1" (1 col, digit) + separator (1 col) + 8 text
        // columns, so "aaaa bbbb" (9 chars) wraps to "aaaa" then "bbbb" and "short" (5
        // chars) does not.
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        let first = row_text(&b, 1, 20);
        assert_eq!(&first[0..1], "1", "first display row carries the gutter");
        assert_eq!(&first[1..2], " ", "separator is blank");
        assert!(first[2..10].starts_with("aaaa"));
        let continuation = row_text(&b, 2, 20);
        assert_eq!(&continuation[0..1], " ", "continuation gutter is blank");
        assert_eq!(&continuation[1..2], " ", "continuation separator is blank");
        assert!(
            continuation[2..10].starts_with("bbbb"),
            "continuation text starts at the same column as the first row's text"
        );
    }

    #[test]
    /// UI-E-112 — a pane too narrow for the gutter treats the available text
    /// width as one column, wrapping one character per display row, rendered.
    fn ut_pane_too_narrow_for_the_gutter_wraps_one_character_per_row_when_rendered() {
        let mut st = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@1@@\n-abcd\n+x\n")
            .unwrap();
        let w = DiffView::default();
        // Each 3-wide pane: gutter "1" (1 col, digit) + separator (1 col) + 1 text column,
        // so "abcd" wraps one character per display row. The header ("@@1@@", 5 chars) also wraps at
        // this width, so scan for the old side's column (2) rather than fixing row indices.
        let mut b = buffer(8, 10);
        StatefulWidget::render(&w, Rect::new(0, 0, 8, 10), &mut b, &mut st);
        let old_column: String = (0..10)
            .map(|y| row_text(&b, y, 8).chars().nth(2).unwrap_or(' '))
            .collect();
        assert!(
            old_column.contains("abcd"),
            "old side's text wraps one character per display row: {old_column:?}"
        );
    }

    #[test]
    /// UI-R-262 — in the split layout a logical row occupies as many display rows as the
    /// taller side needs when wrapped, the shorter side padded blank so both sides keep
    /// starting on the same display row.
    fn ut_split_layout_pads_the_shorter_side_so_both_sides_start_on_the_same_display_row() {
        let mut st = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@ -1,1 +1,1 @@\n-aaaa bbbb\n+short\n")
            .unwrap();
        let w = DiffView::default();
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        let first = row_text(&b, 1, 20);
        assert!(first[13..20].starts_with("short"), "new side's one row");
        let continuation = row_text(&b, 2, 20);
        assert_eq!(
            &continuation[11..20],
            "         ",
            "new side has nothing more to draw, so its padded row is blank"
        );
    }

    #[test]
    /// UI-R-281 — `word_tokens` splits a line into maximal word-character runs and
    /// maximal runs of anything else, as char-index ranges.
    fn ut_word_tokens_split_word_runs_from_everything_else() {
        assert_eq!(
            word_tokens("let a = 1;"),
            vec![0..3, 3..4, 4..5, 5..8, 8..9, 9..10]
        );
        assert_eq!(word_tokens(""), Vec::<std::ops::Range<usize>>::new());
        assert_eq!(word_tokens("ab_1 cd"), vec![0..4, 4..5, 5..7]);
    }

    #[test]
    /// UI-R-280, UI-R-281, UI-R-283 — `word_diff_spans` returns the char-index ranges of
    /// `text` holding tokens `other` does not, after an LCS match over the two token lists.
    fn ut_word_diff_spans_are_the_tokens_only_this_side_holds() {
        let old = "let a = 1;";
        let new = "let b = 1;";
        assert_eq!(word_diff_spans(old, new), vec![4..5]);
        assert_eq!(word_diff_spans(new, old), vec![4..5]);
    }

    #[test]
    /// UI-E-126 — two rows sharing no token are emphasised end to end, one span
    /// covering the whole text.
    fn ut_word_diff_spans_with_no_shared_tokens_covers_the_whole_text() {
        assert_eq!(word_diff_spans("aaa", "bbb"), vec![0..3]);
    }

    #[test]
    /// UI-E-126 — rendered: two rows sharing no token ("aaa" against "bbb") are
    /// emphasised end to end, every text cell carrying the emphasis style, while the
    /// gutter and the cells past the end of the text still carry the plain band.
    fn ut_rows_sharing_no_token_are_emphasised_end_to_end() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n-aaa\n+bbb\n");
        let w = DiffView::default();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut st);
        let style = DiffViewStyle::default();
        // Row 0 is the meta header, row 1 the pair row. Old pane: gutter+separator at
        // columns 0..2, text "aaa" at columns 2..5, trailing blank cells past it.
        assert_eq!(
            b[(0, 1)].bg,
            style.removed.bg.unwrap(),
            "gutter keeps the band"
        );
        for x in 2..5 {
            assert_eq!(b[(x, 1)].bg, style.removed_word.bg.unwrap());
        }
        assert_eq!(
            b[(10, 1)].bg,
            style.removed.bg.unwrap(),
            "past the end of the text keeps the band"
        );
        // New pane starts at column 21.
        assert_eq!(
            b[(21, 1)].bg,
            style.added.bg.unwrap(),
            "gutter keeps the band"
        );
        for x in 23..26 {
            assert_eq!(b[(x, 1)].bg, style.added_word.bg.unwrap());
        }
        assert_eq!(
            b[(31, 1)].bg,
            style.added.bg.unwrap(),
            "past the end of the text keeps the band"
        );
    }

    #[test]
    /// UI-E-129 — above 512 word tokens on either side, `word_diff_spans` returns no
    /// spans at all; at exactly 512 it still diffs, so the boundary itself is pinned,
    /// not just the far side of it.
    fn ut_word_diff_spans_are_empty_above_the_token_cap() {
        // `word_tokens` alternates word and non-word runs, so "x!" repeated N times
        // yields 2N tokens: 300 repeats crosses the 512-token cap (600 tokens), 256
        // repeats sits exactly at it (512 tokens). The `!` tokens always match across
        // sides, the `x`/`y` ones never do, so a within-cap diff is never empty.
        let over_a: String = "x!".repeat(300);
        let over_b: String = "y!".repeat(300);
        assert!(
            word_diff_spans(&over_a, &over_b).is_empty(),
            "600 tokens exceeds the 512 cap"
        );
        let at_a: String = "x!".repeat(256);
        let at_b: String = "y!".repeat(256);
        assert!(
            !word_diff_spans(&at_a, &at_b).is_empty(),
            "512 tokens is still within the cap"
        );
    }

    #[test]
    /// UI-E-129 — rendered: a paired removed/added row whose texts exceed the token
    /// cap gets no word-diff emphasis, every text cell carrying the plain row band.
    fn ut_capped_pair_rows_are_plain_bands() {
        let old_line: String = "x ".repeat(600);
        let new_line: String = "y ".repeat(600);
        let mut st = DiffViewStateBuilder::default()
            .build_with_diff(&format!("@@ -1,1 +1,1 @@\n-{old_line}\n+{new_line}\n"))
            .unwrap();
        let w = DiffView::default();
        let mut b = buffer(2200, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 2200, 2), &mut b, &mut st);
        let style = DiffViewStyle::default();
        // Row 0 is the meta header, row 1 the pair row. Old pane: gutter+separator
        // at columns 0..2, text from column 2. No cell should carry removed_word.
        for x in 0..1090 {
            assert_ne!(b[(x, 1)].bg, style.removed_word.bg.unwrap());
        }
        assert_eq!(b[(2, 1)].bg, style.removed.bg.unwrap());
        for x in 1100..2200 {
            assert_ne!(b[(x, 1)].bg, style.added_word.bg.unwrap());
        }
        assert_eq!(b[(1102, 1)].bg, style.added.bg.unwrap());
    }

    #[test]
    /// UI-R-283 — in the split layout, a paired removed/added row differing in one word
    /// has that word's cells carrying the emphasis style and every other text cell (plus
    /// the gutter and any cells past the text) carrying the row band.
    fn ut_changed_words_carry_the_emphasis_style_and_the_rest_the_band() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n-let a = 1;\n+let b = 1;\n");
        let w = DiffView::default();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut st);
        let style = DiffViewStyle::default();
        // Row 0 is the meta header, row 1 the pair row. Old pane: gutter "1" +
        // separator at columns 0..2, text from column 2. The changed word "a" is the
        // fourth text character ("let a = 1;"), at column 6.
        assert_eq!(b[(6, 1)].bg, style.removed_word.bg.unwrap());
        assert_eq!(
            b[(2, 1)].bg,
            style.removed.bg.unwrap(),
            "unchanged text keeps the band"
        );
        assert_eq!(
            b[(0, 1)].bg,
            style.removed.bg.unwrap(),
            "gutter keeps the band"
        );
        assert_eq!(
            b[(15, 1)].bg,
            style.removed.bg.unwrap(),
            "past the end of the text keeps the band"
        );
        // New pane starts at column 21 (half of 39, past the 2-column separator).
        assert_eq!(b[(27, 1)].bg, style.added_word.bg.unwrap());
        assert_eq!(
            b[(23, 1)].bg,
            style.added.bg.unwrap(),
            "unchanged text keeps the band"
        );
    }

    #[test]
    /// UI-R-284 — an emphasised span keeps the syntax foreground the language computed;
    /// only the background changes.
    fn ut_word_emphasis_keeps_the_syntax_foreground() {
        let mut st = DiffViewStateBuilder::default()
            .language(Some(ferrowl_syntax::Language::Lua))
            .build_with_diff("@@ -1,1 +1,1 @@\n-local a = 1\n+local b = 1\n")
            .unwrap();
        let mut w = DiffView::default();
        w.style.set_removed_word(
            Style::default()
                .fg(ratatui::style::Color::Magenta)
                .bg(ratatui::style::Color::Cyan),
        );
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut st);
        let plain = state_with("@@ -1,1 +1,1 @@\n-local a = 1\n+local b = 1\n");
        let mut plain_st = plain;
        let plain_w = DiffView::default();
        let mut plain_b = buffer(40, 2);
        StatefulWidget::render(
            &plain_w,
            Rect::new(0, 0, 40, 2),
            &mut plain_b,
            &mut plain_st,
        );
        // Row 0 is the meta header, row 1 the pair row. The "local" keyword at
        // columns 2..7 gets a syntax foreground under highlighting, different from the
        // plain no-language render, while the emphasised cell (column 8, the changed
        // word "a"/"b") keeps that same syntax-computed foreground.
        assert_ne!(
            b[(2, 1)].fg,
            plain_b[(2, 1)].fg,
            "language changes the foreground"
        );
        assert_eq!(b[(8, 1)].bg, w.style.removed_word.bg.unwrap());
        assert_ne!(
            b[(8, 1)].bg,
            plain_b[(8, 1)].bg,
            "emphasis background differs from the plain removed band"
        );
        assert_ne!(
            b[(8, 1)].fg,
            w.style.removed_word.fg.unwrap(),
            "foreground stays the syntax theme's, not the emphasis style's own"
        );
    }

    #[test]
    /// UI-E-124 — an unpaired row (the surplus side has no counterpart) carries no word
    /// emphasis: it is one plain band end to end.
    fn ut_unpaired_removed_row_carries_no_word_emphasis() {
        let mut st = state_with("@@ -1,2 +1,1 @@\n-a\n-b\n+x\n");
        let w = DiffView::default();
        let mut b = buffer(40, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 3), &mut b, &mut st);
        let style = DiffViewStyle::default();
        // Row 0 is the meta header, row 1 the first pair ("a" against "x", which is
        // fully emphasised since it shares no token), row 2 the surplus removed line
        // "b" with no added counterpart.
        for x in 0..19 {
            assert_eq!(
                b[(x, 2)].bg,
                style.removed.bg.unwrap(),
                "column {x} should be the plain band, no emphasis"
            );
        }
    }

    #[test]
    /// UI-E-125 — wrapping on, a differing word straddling the wrap point stays
    /// emphasised on both display rows it lands on.
    fn ut_word_emphasis_continues_across_a_wrap_point() {
        // A 14-char token wholly unmatched on either side (UI-E-126: "xxxxxxxxxxxxxx"
        // and "yyyyyyyyyyyyyy" share no token) against a 10-char text width forces
        // `word_wrap` to hard-split the token mid-word (it exceeds the cap, so the
        // token-preserving path never applies): 10 chars land on the entry's first
        // display row, the remaining 4 on its continuation. The single emphasis span
        // covers the whole token, so both display rows must carry it.
        let mut st = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@ -1,1 +1,1 @@\n-xxxxxxxxxxxxxx\n+yyyyyyyyyyyyyy\n")
            .unwrap();
        let w = DiffView::default();
        let mut b = buffer(25, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 25, 3), &mut b, &mut st);
        let style = DiffViewStyle::default();
        // Row 0 is the meta header. Row 1 is the entry's first display row, row 2 its
        // wrapped continuation. Old pane text starts at column 2 ("1 " gutter).
        for x in 2..12 {
            assert_eq!(
                b[(x, 1)].bg,
                style.removed_word.bg.unwrap(),
                "old pane row 1 column {x} should carry the emphasis"
            );
        }
        for x in 2..6 {
            assert_eq!(
                b[(x, 2)].bg,
                style.removed_word.bg.unwrap(),
                "old pane row 2 (continuation) column {x} should carry the emphasis"
            );
        }
        // New pane starts at column 13, past the 1-column separator; gutter at 13..15.
        for x in 15..25 {
            assert_eq!(
                b[(x, 1)].bg,
                style.added_word.bg.unwrap(),
                "new pane row 1 column {x} should carry the emphasis"
            );
        }
        for x in 15..19 {
            assert_eq!(
                b[(x, 2)].bg,
                style.added_word.bg.unwrap(),
                "new pane row 2 (continuation) column {x} should carry the emphasis"
            );
        }
        // Continuation gutters stay blank (UI-R-261), which the row band still covers.
        assert_eq!(b[(0, 2)].bg, style.removed.bg.unwrap());
        assert_eq!(b[(13, 2)].bg, style.added.bg.unwrap());
    }

    #[test]
    /// UI-R-282 — the word-emphasis styles are builder-settable on the widget, and a
    /// caller-set style is what actually paints (regression against a hardcoded default).
    fn ut_word_emphasis_styles_are_builder_settable() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n-a\n+b\n");
        let custom = Style::default().bg(ratatui::style::Color::Rgb(9, 9, 9));
        let w = DiffViewBuilder::default()
            .style(
                DiffViewStyleBuilder::default()
                    .added_word(custom)
                    .removed_word(custom)
                    .build()
                    .unwrap(),
            )
            .build()
            .unwrap();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut st);
        // Row 0 is the meta header, row 1 the pair row.
        assert_eq!(b[(2, 1)].bg, custom.bg.unwrap());
        assert_eq!(b[(23, 1)].bg, custom.bg.unwrap());
    }

    #[test]
    /// UI-R-286 — the diff widget's border defaults to no border.
    fn ut_diff_view_defaults_to_no_border() {
        let w = DiffViewBuilder::default().build().unwrap();
        assert!(matches!(w.border(), Border::None));
    }

    #[test]
    /// UI-R-287 — a borderless split layout draws a separator column between the two
    /// panes: the new side's gutter starts past it, not directly after the old side.
    fn ut_borderless_split_puts_a_separator_column_between_the_panes() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n context\n");
        let w = DiffView::default();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut st);
        assert_eq!(b[(21, 1)].symbol(), "1", "new side's gutter digit");
        assert_eq!(b[(19, 1)].symbol(), " ", "seam carries no gutter or text");
        assert_eq!(b[(20, 1)].symbol(), " ", "seam carries no gutter or text");
    }

    #[test]
    /// UI-R-286, UI-R-287 — a bordered split layout draws no separator column: the two
    /// pane borders already part the panes and abut directly.
    fn ut_bordered_split_has_no_separator() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n a\n");
        let w = DiffViewBuilder::default()
            .border(Border::Full(Margin::new(0, 0)))
            .build()
            .unwrap();
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        assert_ne!(b[(9, 1)].symbol(), " ", "old pane's right border");
        assert_ne!(
            b[(10, 1)].symbol(),
            " ",
            "new pane's left border, directly adjacent"
        );
    }

    #[test]
    /// UI-R-288 — the separator column between the panes carries the general style on
    /// every row, never an added or removed band.
    fn ut_separator_column_stays_in_the_general_style_on_every_row() {
        let mut st = state_with(
            "@@ -1,1 +1,1 @@\n-old\n@@ -5,1 +6,1 @@\n context\n@@ -10,0 +11,1 @@\n+new\n",
        );
        let w = DiffView::default();
        let mut b = buffer(40, 6);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 6), &mut b, &mut st);
        for y in [1u16, 3u16, 5u16] {
            assert_eq!(
                b[(20, y)].bg,
                w.style.general.bg.unwrap(),
                "row {y} separator column"
            );
        }
    }

    #[test]
    /// UI-E-130 — on an even width the separator widens to two columns so the two panes
    /// stay equal (UI-R-211); the old pane's band stops at column 18, not column 19.
    fn ut_even_width_widens_the_separator_and_keeps_the_panes_equal() {
        let mut st = state_with("@@ -1,1 +1,0 @@\n-old\n");
        let w = DiffView::default();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut st);
        let style = DiffViewStyle::default();
        assert_eq!(
            b[(18, 1)].bg,
            style.removed.bg.unwrap(),
            "old pane's last column"
        );
        assert_eq!(
            b[(19, 1)].bg,
            w.style.general.bg.unwrap(),
            "separator, not the old pane's band"
        );
        assert_eq!(b[(20, 1)].bg, w.style.general.bg.unwrap(), "separator");
    }

    #[test]
    /// UI-E-131 — a full-width meta row spans the separator column too, since it spans
    /// the widget's whole width.
    fn ut_meta_row_spans_the_separator_column() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n context\n");
        let meta = Style::default().fg(ratatui::style::Color::Magenta);
        let w = DiffViewBuilder::default()
            .style(DiffViewStyleBuilder::default().meta(meta).build().unwrap())
            .build()
            .unwrap();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut st);
        for x in 0..40 {
            assert_eq!(
                b[(x, 0)].fg,
                meta.fg.unwrap(),
                "column {x} of the meta row, including the separator"
            );
        }
    }

    #[test]
    /// UI-R-210, UI-R-304 — in a bordered split layout the meta row is drawn once inside
    /// each pane's border, spanning only that pane's inner width, so the amended "area it
    /// is drawn in" wording pins the same rendered row.
    fn ut_bordered_split_draws_the_meta_row_inside_each_pane() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n a\n");
        let w = DiffViewBuilder::default()
            .border(Border::Full(Margin::new(0, 0)))
            .build()
            .unwrap();
        let mut b = buffer(40, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 4), &mut b, &mut st);
        let chars: Vec<char> = row_text(&b, 1, 40).chars().collect();
        let old_inner: String = chars[1..19].iter().collect();
        let new_inner: String = chars[21..39].iter().collect();
        assert!(
            old_inner.contains("@@ -1,1 +1,1 @@"),
            "old pane's inner columns carry the header: {old_inner:?}"
        );
        assert!(
            new_inner.contains("@@ -1,1 +1,1 @@"),
            "new pane's inner columns carry the header too: {new_inner:?}"
        );
        assert_ne!(b[(0, 1)].symbol(), " ", "old pane's left border");
        assert_ne!(b[(19, 1)].symbol(), " ", "old pane's right border");
        assert_ne!(b[(20, 1)].symbol(), " ", "new pane's left border");
        assert_ne!(b[(39, 1)].symbol(), " ", "new pane's right border");
        let border = w.style.border.fg.expect("style sets a color");
        assert_eq!(b[(0, 1)].fg, border, "old pane's left border style");
        assert_eq!(b[(19, 1)].fg, border, "old pane's right border style");
        assert_eq!(b[(20, 1)].fg, border, "new pane's left border style");
        assert_eq!(b[(39, 1)].fg, border, "new pane's right border style");
    }

    #[test]
    /// UI-E-141 — a meta row wider than a pane's inner width in the bordered split layout
    /// clips at that pane's inner width, independently in each pane, with no ellipsis and
    /// no spill onto either pane's border.
    fn ut_bordered_split_meta_row_is_clipped_at_each_pane_border() {
        let mut st = state_with(
            "@@ -1,1 +1,1 @@ a very long hunk header that exceeds one pane's width\n a\n",
        );
        let w = DiffViewBuilder::default()
            .border(Border::Full(Margin::new(0, 0)))
            .build()
            .unwrap();
        let mut b = buffer(40, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 4), &mut b, &mut st);
        assert_eq!(b[(0, 1)].symbol(), "│", "old pane's left border glyph");
        assert_eq!(
            b[(19, 1)].symbol(),
            "│",
            "old pane's right border untouched by the clipped text"
        );
        assert_eq!(
            b[(20, 1)].symbol(),
            "│",
            "new pane's left border untouched by the clipped text"
        );
        assert_eq!(
            b[(39, 1)].symbol(),
            "│",
            "new pane's right border untouched by the clipped text"
        );
        assert_ne!(b[(18, 1)].symbol(), "…", "no ellipsis, text simply stops");
    }

    #[test]
    /// UI-E-132 — a borderless split layout narrower than three columns drops the
    /// separator so both panes keep at least one column, and the two panes abut: each
    /// column carries only its own pane's band, with no dropped column between them.
    fn ut_width_under_three_columns_drops_the_separator() {
        let mut st = state_with("@@ -1,1 +1,0 @@\n-old\n@@ -5,0 +5,1 @@\n+new\n");
        let w = DiffView::default();
        let mut b = buffer(2, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 2, 4), &mut b, &mut st);
        let style = DiffViewStyle::default();
        // Row 1: unpaired removed line, old pane only, new pane filler. Row 3: unpaired
        // added line, new pane only, old pane filler.
        assert_eq!(
            b[(0, 1)].bg,
            style.removed.bg.unwrap(),
            "old pane keeps its column"
        );
        assert_eq!(
            b[(1, 1)].bg,
            w.style.general.bg.unwrap(),
            "new pane's filler, directly adjacent, no dropped column"
        );
        assert_eq!(
            b[(0, 3)].bg,
            w.style.general.bg.unwrap(),
            "old pane's filler, directly adjacent, no dropped column"
        );
        assert_eq!(
            b[(1, 3)].bg,
            style.added.bg.unwrap(),
            "new pane keeps its column"
        );
    }
}
