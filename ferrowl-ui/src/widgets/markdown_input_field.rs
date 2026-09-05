use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Margin, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, StatefulWidget, Widget},
};

use ferrowl_syntax::markdown::{BlockState, block_line};
use ferrowl_syntax::{Language, LineState};

use super::markdown_render::{RenderedLine, render_line, wrap_line};
use crate::Border;
use crate::state::{MarkdownInputFieldState, VimMode};
use crate::style::{InputFieldStyle, MarkdownTheme, SyntaxTheme};
use crate::traits::Margins;
use crate::widgets::Title;

/// A markdown editor rendered from a
/// [`MarkdownInputFieldState`](crate::state::MarkdownInputFieldState): source lines are
/// shown rendered except where editing requires revealing their markup (UI-R-182..UI-R-184),
/// always wrapped to the widget width (UI-R-130). Configure border, title, margins, and the
/// syntax/markdown themes via [`MarkdownInputFieldBuilder`].
#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct MarkdownInputField {
    #[getset(get = "pub")]
    #[builder(default = "Border::None")]
    border: Border,
    #[getset(get = "pub")]
    #[builder(default = "InputFieldStyle::default()")]
    style: InputFieldStyle,
    #[getset(get = "pub")]
    #[builder(default = "None")]
    title: Option<Title>,
    #[getset(get = "pub")]
    #[builder(default = "Margin::default()")]
    margin: Margin,
    #[getset(get = "pub")]
    #[builder(default = "SyntaxTheme::default()")]
    syntax_theme: SyntaxTheme,
    #[getset(get = "pub")]
    #[builder(default = "MarkdownTheme::default()")]
    markdown_theme: MarkdownTheme,
    /// Off by default; prints the source line number on each source line's first display
    /// row when enabled (UI-R-140).
    #[getset(get_copy = "pub")]
    #[builder(default = "false")]
    line_numbers: bool,
}

impl Margins for MarkdownInputField {
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

impl MarkdownInputField {
    /// Builds the styled-source form of `line` (UI-R-129) as a word-wrapped
    /// [`RenderedLine`] — the same wrap layout a rendered paragraph gets (UI-R-131: a
    /// break only at a space, falling back to a character break only when a single word is
    /// itself wider than the line, UI-E-087), so the source and rendered paths never
    /// disagree on where a line breaks.
    fn styled_source_line(&self, line: &str) -> RenderedLine {
        let (spans, _) =
            ferrowl_syntax::highlight_line(Language::Markdown, line, LineState::default());
        let chars: Vec<char> = line.chars().collect();
        let mut styled = Vec::new();
        let mut cursor = 0usize;
        for &(start, end, kind) in &spans {
            if cursor < start {
                styled.push((
                    chars[cursor..start].iter().collect::<String>(),
                    self.style.general,
                ));
            }
            let end = end.min(chars.len());
            styled.push((
                chars[start..end].iter().collect::<String>(),
                self.syntax_theme.style(kind),
            ));
            cursor = end;
        }
        if cursor < chars.len() {
            styled.push((
                chars[cursor..].iter().collect::<String>(),
                self.style.general,
            ));
        }
        if styled.is_empty() {
            styled.push((String::new(), self.style.general));
        }
        RenderedLine {
            spans: styled,
            hanging_indent: 0,
            char_wrap: false,
            rule: false,
        }
    }
}

/// Locates `cursor_col` (a char index into the unwrapped source line) within `rows`, the
/// output of wrapping that same line's chars: word-wrap only ever drops a run of spaces
/// exactly at a row break (UI-R-131), never reorders or alters other characters, so a row's
/// text is otherwise a verbatim slice of `original` — walking both in lockstep and skipping
/// `original` past a dropped run finds the row/column pair (UI-E-088). Clamps to the last
/// row/column so a cursor at the exact end of the line is always drawn (UI-E-088).
fn locate_wrapped_position(
    original: &[char],
    rows: &[Vec<(String, Style)>],
    cursor_col: usize,
) -> (usize, usize) {
    let mut orig_idx = 0usize;
    for (row_idx, row) in rows.iter().enumerate() {
        let row_chars: Vec<char> = row.iter().flat_map(|(s, _)| s.chars()).collect();
        if row_idx > 0 {
            let first = row_chars.first().copied();
            while orig_idx < original.len() && Some(original[orig_idx]) != first {
                orig_idx += 1;
            }
        }
        let row_start = orig_idx;
        let row_end = row_start + row_chars.len();
        let is_last_row = row_idx == rows.len() - 1;
        if cursor_col < row_end || (is_last_row && cursor_col >= row_start) {
            let last_cell = row_chars.len().saturating_sub(1);
            return (
                row_idx,
                (cursor_col.saturating_sub(row_start)).min(last_cell),
            );
        }
        orig_idx = row_end;
    }
    (0, 0)
}

/// Whether `line_idx` is drawn in its rendered form or as styled source, per UI-R-182..UI-R-184.
fn reveal_as_source(
    line_idx: usize,
    active_line: usize,
    focused: bool,
    disabled: bool,
    mode: VimMode,
) -> bool {
    if !focused || disabled {
        return false;
    }
    match mode {
        VimMode::Insert | VimMode::Visual { .. } => true,
        VimMode::Normal => line_idx == active_line,
    }
}

impl StatefulWidget for &MarkdownInputField {
    type State = MarkdownInputFieldState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        buf.set_style(area, self.style.general);

        let area = Layout::vertical([
            Constraint::Length(self.margin.vertical),
            Constraint::Min(1),
            Constraint::Length(self.margin.vertical),
        ])
        .split(area)[1];

        let mut area = Layout::horizontal([
            Constraint::Length(self.margin.horizontal),
            Constraint::Min(1),
            Constraint::Length(self.margin.horizontal),
        ])
        .split(area)[1];

        if let Border::Full(m) = &self.border {
            let border_style = if state.inner().focused() {
                self.style.focused
            } else {
                self.style.border
            };
            let mut block = Block::bordered().style(border_style);
            match (self.title.as_ref(), state.mode_label()) {
                (Some(t), Some(label)) => {
                    block = block
                        .title(format!("{} [{}]", t.name, label))
                        .title_alignment(t.alignment);
                }
                (Some(t), None) => {
                    block = block.title(t.name.as_str()).title_alignment(t.alignment);
                }
                (None, Some(label)) => {
                    block = block.title(format!("[{label}]"));
                }
                (None, None) => {}
            }
            let inner = block.inner(area);
            block.render(area, buf);
            area = inner.inner(*m);
        }

        let visible_height = area.height as usize;
        if visible_height == 0 {
            return;
        }

        let lines = state.lines().clone();
        let line_count = lines.len();
        let active = state.active_line();
        let focused = state.inner().focused();
        let disabled = state.read_only();
        let mode = state.vim_mode();

        let gutter_width = if self.line_numbers {
            line_count.to_string().len() as u16 + 1
        } else {
            0
        };
        let content_x = area.x + gutter_width;
        let content_width = area.width.saturating_sub(gutter_width) as usize;
        if content_width == 0 {
            return;
        }

        let mut block_state = BlockState::default();
        let mut fence_carry = LineState::default();
        let mut rows_per_line = Vec::with_capacity(line_count);
        let mut rows_data: Vec<Vec<Vec<(String, Style)>>> = Vec::with_capacity(line_count);
        let mut is_source_line: Vec<bool> = Vec::with_capacity(line_count);

        for (line_idx, line) in lines.iter().enumerate() {
            let (block, next_block_state) = block_line(line, &block_state);
            let fence_info = block_state.fence_info().map(str::to_string);
            block_state = next_block_state;

            let (rendered, next_carry) = render_line(
                &block,
                line,
                fence_info.as_deref(),
                fence_carry,
                &self.markdown_theme,
                &self.syntax_theme,
                self.style.general,
            );
            fence_carry = next_carry;

            let source_flag = reveal_as_source(line_idx, active, focused, disabled, mode);
            let rows = if source_flag {
                wrap_line(&self.styled_source_line(line), content_width)
            } else {
                wrap_line(&rendered, content_width)
            };

            rows_per_line.push(rows.len());
            rows_data.push(rows);
            is_source_line.push(source_flag);
        }

        let cursor_position = if active < is_source_line.len() && is_source_line[active] {
            let active_chars: Vec<char> = lines[active].chars().collect();
            let (row, col) =
                locate_wrapped_position(&active_chars, &rows_data[active], state.cursor_col());
            state.sync_cursor_row(row);
            Some((row, col))
        } else {
            None
        };

        state.sync_layout(rows_per_line.clone(), visible_height);
        let row_scroll = state.row_scroll();

        let mut display_rows: Vec<(usize, usize)> = Vec::new();
        for (line_idx, &n) in rows_per_line.iter().enumerate() {
            for r in 0..n {
                display_rows.push((line_idx, r));
            }
        }

        for (row, &(line_idx, sub_row)) in display_rows
            .iter()
            .enumerate()
            .skip(row_scroll)
            .take(visible_height)
        {
            let y = area.y + (row - row_scroll) as u16;

            if gutter_width > 0 {
                let gutter_str = if sub_row == 0 {
                    format!(
                        "{:>width$}",
                        line_idx + 1,
                        width = gutter_width as usize - 1
                    )
                } else {
                    " ".repeat(gutter_width as usize - 1)
                };
                let gutter_rect = Rect::new(area.x, y, gutter_width - 1, 1);
                Paragraph::new(Text::from(gutter_str).style(self.style.general))
                    .render(gutter_rect, buf);
            }

            let content_rect = Rect::new(content_x, y, content_width as u16, 1);
            let row_spans = &rows_data[line_idx][sub_row];
            let line_spans: Vec<Span> = row_spans
                .iter()
                .map(|(text, style)| Span::styled(text.clone(), *style))
                .collect();
            Paragraph::new(Text::from(Line::from(line_spans))).render(content_rect, buf);

            if disabled && line_idx == active {
                buf.set_style(
                    Rect::new(area.x, y, area.width, 1),
                    *self.markdown_theme.highlighted_row(),
                );
            }

            if focused
                && !disabled
                && line_idx == active
                && let Some((cursor_row, col_in_row)) = cursor_position
                && cursor_row == sub_row
            {
                buf[(content_x + col_in_row as u16, y)].set_style(self.style.cursor);
            }
        }
    }
}

impl StatefulWidget for MarkdownInputField {
    type State = MarkdownInputFieldState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}
