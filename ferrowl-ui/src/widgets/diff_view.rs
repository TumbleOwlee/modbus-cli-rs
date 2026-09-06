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
    DiffEntry, DiffKind, DiffLayout, DiffMode, DiffRow, DiffViewState, RowPart, Side,
};
use crate::style::{DiffViewStyle, SyntaxTheme};
use crate::traits::Margins;
use crate::widgets::Title;

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

/// Clamps a gutter's digit/label width so the digits plus their one separator space never
/// exceed `pane_width` (UI-R-172), the same rule the code editor applies to its own
/// gutter: clamp the combined `text_width + 1`, then drop the separator back off.
fn clamp_gutter(text_width: usize, pane_width: u16) -> u16 {
    ((text_width + 1).min(pane_width as usize) as u16).saturating_sub(1)
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

/// One side's marker character (UI-R-216): `-` on the old side and `+` on the new side of
/// a changed row's present entry, space for a context row or a filler. `DiffRow::Pair`
/// carries one shared `kind` even when both sides are present and their text differs (a
/// genuine two-sided change, not two independent rows), so the marker is derived from
/// which side holds the entry, not from `kind` alone.
fn marker_for(kind: &DiffKind, side: Side, has_entry: bool) -> char {
    if !has_entry || *kind == DiffKind::Context {
        return ' ';
    }
    match side {
        Side::Old => '-',
        Side::New => '+',
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

impl DiffView {
    /// The per-side diff style of UI-R-219: the syntax theme's removed/added style for a
    /// changed row's old/new side respectively, the general text style for context. A
    /// `DiffRow::Pair` carries one shared `kind` even for a genuine two-sided change (both
    /// sides present, text differing), so the style is derived from which side is being
    /// drawn, not from `kind` alone — otherwise a combined change row's new (added) side
    /// would wrongly inherit the row's `Removed` tag.
    fn side_style(&self, kind: &DiffKind, side: Side) -> Style {
        match kind {
            DiffKind::Context => self.style.general,
            DiffKind::Meta => self.syntax_theme.meta,
            DiffKind::Added | DiffKind::Removed => match side {
                Side::Old => self.syntax_theme.removed,
                Side::New => self.syntax_theme.added,
            },
        }
    }

    /// Draws one side's gutter, marker and text into `rect` (whose width is exactly
    /// `gutter_width + 1 + text_width`). A filler entry (UI-R-212) draws a blank gutter of
    /// the full width, a space marker and no text. `sub_row` selects which wrapped display
    /// row of the entry's text this call draws (UI-R-260); `sub_row > 0` draws a
    /// continuation, with a blank gutter and marker of its own (UI-R-261) rather than
    /// `draw_entry`'s own text-absent blank, so a continuation is visually distinct from a
    /// filler even though both leave the gutter and marker empty.
    // One argument per independently-varying render input (row position, kind, side,
    // entry, labels, gutter width, language); grouping them into a context struct would
    // just move the same count into field access without reducing it.
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
    ) {
        if rect.width == 0 {
            return;
        }
        let style = self.side_style(kind, side);
        buf.set_style(rect, self.style.general);
        let continuation = sub_row > 0;
        if gutter_width > 0 {
            let text = if continuation {
                String::new()
            } else {
                gutter_text_for(row_idx, entry, labels)
            };
            let gutter_str = format!("{text:>width$}", width = gutter_width as usize);
            Paragraph::new(Text::from(gutter_str).style(self.style.general))
                .render(Rect::new(rect.x, rect.y, gutter_width, 1), buf);
        }
        let marker_x = rect.x + gutter_width;
        let marker = if continuation {
            ' '
        } else {
            marker_for(kind, side, entry.is_some())
        };
        Paragraph::new(Text::from(marker.to_string()).style(style))
            .render(Rect::new(marker_x, rect.y, 1, 1), buf);
        let text_x = marker_x + 1;
        let text_width = rect.width.saturating_sub(gutter_width + 1);
        if text_width == 0 {
            return;
        }
        if let Some(e) = entry {
            // Highlighting is computed against the entry's full text, then `h_scroll`
            // drops leading characters (UI-R-232): highlighting a pre-truncated string
            // would shift every span's start against the language's real column.
            let chars = styled_chars(&e.text, style, language, &self.syntax_theme);
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

    /// Draws a meta row (UI-R-210): one screen row across the full width, meta style,
    /// blank gutter on every side, no marker column.
    /// Draws one display row of a meta row (UI-R-210): the full width, in the meta style.
    /// A meta row wraps like any other row (UI-R-260 exempts none), so with `wrap` on
    /// `sub_row` selects which of its wrapped chunks this call draws.
    fn draw_meta(&self, buf: &mut Buffer, rect: Rect, text: &str, wrap: bool, sub_row: usize) {
        // The meta style covers the whole rect first: a `Paragraph` only paints the cells
        // its text occupies, so a row wider than `text` would otherwise show a trailing
        // run of unstyled (`general`) cells past the end of the line (UI-R-210).
        buf.set_style(rect, self.syntax_theme.meta);
        if wrap {
            let chars: Vec<(char, Style)> =
                text.chars().map(|c| (c, self.syntax_theme.meta)).collect();
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
            Paragraph::new(Text::from(text.to_string()).style(self.syntax_theme.meta))
                .render(rect, buf);
        }
    }

    fn pane_border_style(&self, side: Side, focused_side: Side) -> Style {
        if side == focused_side {
            self.style.focused
        } else {
            self.style.border
        }
    }

    /// Renders `border`/`margin`/`title` around `rect` for one pane, styled by whether
    /// `side` is the focused one (UI-R-228), and returns the inner content area. `title`
    /// is drawn only when `show_title`, since the split layout has two panes sharing one
    /// widget-level title and it belongs on one of them, not duplicated on both.
    fn pane_area(
        &self,
        area: Rect,
        buf: &mut Buffer,
        side: Side,
        focused_side: Side,
        show_title: bool,
    ) -> Rect {
        let Border::Full(m) = &self.border else {
            return area;
        };
        let mut block = Block::bordered().style(self.pane_border_style(side, focused_side));
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
        let focused_side = state.focused_side();
        let old_labels = state.old_labels().clone();
        let new_labels = state.new_labels().clone();
        let language = state.language;
        let scroll_offset = state.scroll_offset();
        let h_scroll = state.h_scroll();
        let wrap = state.wrap();

        match state.layout() {
            DiffLayout::Split => {
                // `Constraint::Percentage(50)` twice rounds unevenly on an odd width
                // (UI-R-211 requires equal panes), so split the width explicitly instead.
                let half = area.width / 2;
                let panes =
                    Layout::horizontal([Constraint::Length(half), Constraint::Length(half)])
                        .split(area);
                let old_area = self.pane_area(panes[0], buf, Side::Old, focused_side, true);
                let new_area = self.pane_area(panes[1], buf, Side::New, focused_side, false);

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
                    old_area.width.saturating_sub(old_gutter + 1).max(1) as usize,
                    new_area.width.saturating_sub(new_gutter + 1).max(1) as usize,
                );
                // Without a border, extend to the outer area's own right edge: an odd
                // inner width leaves one column unused by either pane (`Length(half)`
                // twice), and UI-R-210 spans the full width. With a border each pane
                // already owns its border cells, so stop at the new pane's inner edge.
                let meta_right = if matches!(self.border, Border::Full(_)) {
                    new_area.x + new_area.width
                } else {
                    area.x + area.width
                };
                let meta_width = meta_right.saturating_sub(old_area.x);
                state.set_meta_width(meta_width as usize);

                let visible_height = old_area.height as usize;
                let display = state.display_rows();
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
                                Rect::new(old_area.x, y_old, meta_width, 1),
                                text,
                                wrap,
                                sub_row,
                            );
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
                                );
                            } else {
                                buf.set_style(new_rect, self.style.general);
                            }
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
                let pane = self.pane_area(area, buf, focused_side, focused_side, true);
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
                    pane.width.saturating_sub(gutter + 1).max(1) as usize,
                    pane.width.saturating_sub(gutter + 1).max(1) as usize,
                );
                state.set_meta_width(pane.width as usize);

                let display = state.display_rows();
                let visible_height = pane.height as usize;
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
                                );
                            }
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
    /// UI-R-210 — a meta row (the hunk header itself, here) spans the full width in the
    /// meta style, with blank gutters on both sides.
    fn ut_meta_row_spans_the_full_width_in_the_meta_style_with_blank_gutters() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n context\n");
        let w = DiffView::default();
        let mut b = buffer(21, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 21, 2), &mut b, &mut st);
        let line = row_text(&b, 0, 21);
        assert!(line.starts_with("@@ -1,1 +1,1 @@"));
        // Full width (UI-R-210), including the odd trailing column an even split leaves
        // unused by either pane, and the meta style across the whole row, not just its
        // first cell.
        for x in 0..21 {
            assert_eq!(
                b[(x, 0)].fg,
                w.syntax_theme.meta.fg.expect("style sets a color"),
                "column {x} not in the meta style"
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
    /// UI-R-216 — the marker column holds `-`, `+` or space.
    fn ut_marker_column_holds_minus_plus_or_space() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n-old\n+new\n");
        let w = DiffView::default();
        let mut b = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b, &mut st);
        assert_eq!(b[(1, 1)].symbol(), "-");
        assert_eq!(b[(11, 1)].symbol(), "+");
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
            w.syntax_theme.removed.fg.expect("style sets a color")
        );
        assert_eq!(
            b[(12, 2)].fg,
            w.syntax_theme.added.fg.expect("style sets a color")
        );
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
        w.syntax_theme.set_removed(
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
            b[(17, 1)].fg,
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
            w.syntax_theme.removed.fg.expect("style sets a color")
        );
        assert_eq!(
            b[(6, 1)].fg,
            w.syntax_theme.removed.fg.expect("style sets a color")
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
            b[(10, 3)].bg,
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
            b[(10, 1)].bg,
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
    /// UI-R-228 — the focused side paints the focused border style, the other the normal
    /// border style.
    fn ut_focused_side_paints_the_focused_border_and_the_other_the_normal_one() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n a\n");
        st.set_focused_side(Side::New);
        let w = DiffViewBuilder::default()
            .border(Border::Full(Margin::new(0, 0)))
            .build()
            .unwrap();
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        assert_eq!(b[(0, 0)].fg, w.style.border.fg.expect("style sets a color"));
        assert_eq!(
            b[(10, 0)].fg,
            w.style.focused.fg.expect("style sets a color")
        );
    }

    #[test]
    /// UI-R-228 — in unified layout, with one pane, its border paints the focused style.
    fn ut_focused_side_paints_the_focused_style_in_unified_layout() {
        let mut st = DiffViewStateBuilder::default()
            .layout(DiffLayout::Unified)
            .build_with_diff("@@ -1,1 +1,1 @@\n a\n")
            .unwrap();
        st.set_focused_side(Side::New);
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
    /// pane, dropping that many leading characters, while the gutter and marker columns
    /// stay put.
    fn ut_horizontal_offset_shifts_the_text_of_every_pane_leaving_gutters_in_place() {
        let mut st = state_with("@@ -1,1 +1,1 @@\n-abcdefgh\n+xyzuvwtq\n");
        st.set_h_scroll(2);
        let w = DiffView::default();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut st);
        let line = row_text(&b, 1, 40);
        // Old pane: gutter "1", marker '-' still at their columns; text starts with 'c'
        // (the third character), the first two dropped.
        assert_eq!(&line[0..1], "1");
        assert_eq!(&line[1..2], "-");
        assert!(line[2..20].starts_with('c'));
        // New pane: same shift applied independently, at its own gutter/marker columns.
        assert_eq!(&line[20..21], "1");
        assert_eq!(&line[21..22], "+");
        assert!(line[22..].starts_with('z'));
    }

    #[test]
    /// UI-R-261 — a continuation display row of a wrapped entry carries a blank gutter
    /// and a blank marker column, its text starting at the same column as the row's first
    /// display row.
    fn ut_continuation_row_has_a_blank_gutter_and_marker_and_aligned_text() {
        let mut st = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@ -1,1 +1,1 @@\n-aaaa bbbb\n+short\n")
            .unwrap();
        let w = DiffView::default();
        // Each 10-wide pane: gutter "1" (1 col) + marker (1 col) + 8 text columns, so
        // "aaaa bbbb" (9 chars) wraps to "aaaa" then "bbbb" and "short" (5 chars) does not.
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        let first = row_text(&b, 1, 20);
        assert_eq!(&first[0..1], "1", "first display row carries the gutter");
        assert_eq!(&first[1..2], "-", "and the marker");
        assert!(first[2..10].starts_with("aaaa"));
        let continuation = row_text(&b, 2, 20);
        assert_eq!(&continuation[0..1], " ", "continuation gutter is blank");
        assert_eq!(&continuation[1..2], " ", "continuation marker is blank");
        assert!(
            continuation[2..10].starts_with("bbbb"),
            "continuation text starts at the same column as the first row's text"
        );
    }

    #[test]
    /// UI-E-112 — a pane too narrow for the gutter and marker column treats the available
    /// text width as one column, wrapping one character per display row, rendered.
    fn ut_pane_too_narrow_for_the_gutter_wraps_one_character_per_row_when_rendered() {
        let mut st = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@1@@\n-abcd\n+x\n")
            .unwrap();
        let w = DiffView::default();
        // Each 3-wide pane: gutter "1" (1 col) + marker (1 col) + 1 text column, so "abcd"
        // wraps one character per display row. The header ("@@1@@", 5 chars) also wraps at
        // this width, so scan for the old side's column (2) rather than fixing row indices.
        let mut b = buffer(6, 10);
        StatefulWidget::render(&w, Rect::new(0, 0, 6, 10), &mut b, &mut st);
        let old_column: String = (0..10)
            .map(|y| row_text(&b, y, 6).chars().nth(2).unwrap_or(' '))
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
        assert!(first[12..20].starts_with("short"), "new side's one row");
        let continuation = row_text(&b, 2, 20);
        assert_eq!(
            &continuation[10..20],
            "          ",
            "new side has nothing more to draw, so its padded row is blank"
        );
    }
}
