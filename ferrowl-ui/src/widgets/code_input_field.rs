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
use crate::state::CodeInputFieldState;
use crate::style::{InputFieldStyle, SyntaxTheme};
use crate::traits::Margins;
use crate::widgets::Title;

/// A multi-line text editor (e.g. for Lua snippets) rendered from a
/// [`CodeInputFieldState`](crate::state::CodeInputFieldState), with line
/// numbers and vertical/horizontal scrolling. Configure border, title,
/// margins, and [`InputFieldStyle`] via [`CodeInputFieldBuilder`].
#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct CodeInputField {
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
}

impl Margins for CodeInputField {
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

impl StatefulWidget for &CodeInputField {
    type State = CodeInputFieldState;

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
            // A focused field shows the focused border even when disabled (read-only): a disabled
            // viewer can still hold focus for scrolling, and the border must reflect that.
            let border_style = if state.focused() {
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
        state.set_visible_height(visible_height);

        let line_count = state.lines().len();
        let active = state.active_line();

        // Show placeholder when empty and not focused
        let is_empty = line_count == 1 && state.lines()[0].is_empty();
        if is_empty && !state.focused() {
            if let Some(ph) = state.placeholder() {
                let para = Paragraph::new(Text::from(ph.as_str()).style(self.style.placeholder));
                para.render(area, buf);
            }
            return;
        }

        // Adjust scroll so active_line is always visible. Only a focused field
        // follows its cursor: unfocused fields keep their viewport (set_content
        // parks the cursor at the end of the text, and following it would show
        // an unfocused pane scrolled to the bottom/right).
        let scroll = state.scroll_offset();
        let scroll = if !state.focused() {
            scroll.min(line_count.saturating_sub(1))
        } else if active < scroll {
            active
        } else if active >= scroll + visible_height {
            active + 1 - visible_height
        } else {
            scroll
        };
        state.set_scroll_offset(scroll);

        // Gutter width (UI-R-167): one separator space plus the widest label-list entry
        // (surplus entries past the end of the buffer included, UI-E-080) and, if any row
        // falls back to its line index (UI-R-168), the widest such fallback index too.
        let gutter_text_width = match state.gutter_labels() {
            Some(labels) => labels
                .iter()
                .map(|l| l.chars().count())
                .max()
                .unwrap_or(0)
                .max(if labels.len() < line_count {
                    line_count.to_string().len()
                } else {
                    0
                }),
            None => line_count.to_string().len(),
        };
        // UI-R-172: never let the gutter exceed the field's area width, so it stays inside
        // the widget's area even if that leaves the content zero columns (UI-E-083).
        // Clamp in `usize` before the `u16` cast so an oversized label cannot wrap.
        let gutter_width = (gutter_text_width + 1).min(area.width as usize) as u16;
        let content_x = area.x + gutter_width;
        let content_width = area.width.saturating_sub(gutter_width) as usize;
        state.set_content_width(content_width.max(1));

        // Adjust horizontal scroll so the cursor stays in view on the active line.
        let cursor_col = state.cursor_col();
        let h_scroll = state.h_scroll();
        let h_scroll = if content_width == 0 {
            0
        } else if !state.focused() {
            h_scroll
        } else if cursor_col < h_scroll {
            cursor_col
        } else if cursor_col >= h_scroll + content_width {
            cursor_col + 1 - content_width
        } else {
            h_scroll
        };
        state.set_h_scroll(h_scroll);

        // Fold `LineState` from line 0 through the last visible line, stashing the spans
        // for the visible window. Recomputed every render; buffers are small.
        let visible_spans: Option<Vec<Vec<(usize, usize, ferrowl_syntax::SyntaxKind)>>> =
            state.language().map(|lang| {
                let last_visible = (scroll + visible_height - 1).min(line_count - 1);
                let mut carry = ferrowl_syntax::LineState::default();
                let mut spans = Vec::with_capacity(visible_height);
                for i in 0..=last_visible {
                    let (line_spans, next_carry) =
                        ferrowl_syntax::highlight_line(lang, &state.lines()[i], carry);
                    carry = next_carry;
                    if i >= scroll {
                        spans.push(line_spans);
                    }
                }
                spans
            });

        let selection = state.selection_range();

        for (row, line_idx) in (scroll..scroll + visible_height).enumerate() {
            let y = area.y + row as u16;
            if line_idx >= line_count {
                break;
            }

            let gutter_style: Style = if line_idx == active && state.focused() {
                self.style.focused.reversed().bold()
            } else {
                self.style.general
            };
            let gutter_text = match state.gutter_labels() {
                Some(labels) if line_idx < labels.len() => labels[line_idx].clone(),
                _ => (line_idx + 1).to_string(),
            };
            let gutter_str = format!(
                "{:>width$}",
                gutter_text,
                width = gutter_width.saturating_sub(1) as usize
            );
            let gutter_rect = Rect::new(area.x, y, gutter_width.saturating_sub(1), 1);
            Paragraph::new(Text::from(gutter_str).style(gutter_style)).render(gutter_rect, buf);

            if content_width == 0 {
                continue;
            }

            let line = &state.lines()[line_idx];
            let chars: Vec<char> = line.chars().collect();
            let content_rect = Rect::new(content_x, y, content_width as u16, 1);

            if let Some(spans) = visible_spans.as_ref() {
                let window_start = h_scroll;
                let window_end = h_scroll.saturating_add(content_width).min(chars.len());
                let mut line_spans = Vec::new();
                let mut cursor = window_start;
                for &(start, end, kind) in &spans[row] {
                    let s = start.max(window_start);
                    let e = end.min(window_end);
                    if s >= e {
                        continue;
                    }
                    if cursor < s {
                        let gap: String = chars[cursor..s].iter().collect();
                        line_spans.push(Span::styled(gap, self.style.general));
                    }
                    let text: String = chars[s..e].iter().collect();
                    line_spans.push(Span::styled(text, self.syntax_theme.style(kind)));
                    cursor = e;
                }
                if cursor < window_end {
                    let gap: String = chars[cursor..window_end].iter().collect();
                    line_spans.push(Span::styled(gap, self.style.general));
                }
                Paragraph::new(Text::from(Line::from(line_spans))).render(content_rect, buf);
            } else {
                let visible: String = chars
                    .get(h_scroll..h_scroll.saturating_add(content_width).min(chars.len()))
                    .unwrap_or(&[])
                    .iter()
                    .collect();
                Paragraph::new(Text::from(visible).style(self.style.general))
                    .render(content_rect, buf);
            }

            if let Some(((sl, sc), (el, ec))) = selection
                && line_idx >= sl
                && line_idx <= el
            {
                let line_start = if line_idx == sl { sc } else { 0 };
                let line_end = if line_idx == el { ec + 1 } else { chars.len() };
                let start = line_start.max(h_scroll);
                let end = line_end.min(h_scroll + content_width).min(chars.len());
                for col in start..end {
                    let x = content_x + (col - h_scroll) as u16;
                    buf[(x, y)].set_style(self.style.selection);
                }
            }

            if state.focused() && !state.disabled() && line_idx == active {
                let cursor_in_view = cursor_col.saturating_sub(h_scroll) as u16;
                if (cursor_in_view as usize) < content_width {
                    buf[(content_x + cursor_in_view, y)].set_style(self.style.cursor);
                }
            }
        }
    }
}

impl StatefulWidget for CodeInputField {
    type State = CodeInputFieldState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::CodeInputFieldStateBuilder;
    use crate::traits::HandleEvents;
    use crossterm::event::{KeyCode, KeyModifiers};

    fn full_border() -> Border {
        Border::Full(Margin::new(0, 0))
    }

    fn buffer(w: u16, h: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, w, h))
    }

    fn title_row(b: &Buffer, w: u16) -> String {
        (0..w)
            .map(|x| b[(x, 0)].symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    /// UI-R-110 — the code editor's border is styled from focus alone: no validation term, and
    /// a disabled editor keeps the focused border, since a read-only viewer still holds focus for
    /// scrolling. This is the deliberate exception to the single-line input's validation-first
    /// rule, so it is pinned rather than left to the widget's own comment.
    #[test]
    fn ut_border_style_follows_focus_only_including_while_disabled() {
        let style = InputFieldStyle::default();
        let focused = style.focused().fg.expect("focused style sets a foreground");
        let normal = style.border().fg.expect("border style sets a foreground");
        assert_ne!(focused, normal, "focused and normal borders must differ");

        let border_fg = |is_focused: bool, disabled: bool| {
            let w = CodeInputFieldBuilder::default()
                .border(full_border())
                .build()
                .unwrap();
            let mut st = CodeInputFieldStateBuilder::default()
                .disabled(disabled)
                .build()
                .unwrap();
            crate::traits::SetFocus::set_focused(&mut st, is_focused);
            let mut b = buffer(20, 4);
            StatefulWidget::render(&w, Rect::new(0, 0, 20, 4), &mut b, &mut st);
            b[(0, 0)].fg
        };

        assert_eq!(border_fg(true, false), focused, "focused");
        assert_eq!(border_fg(false, false), normal, "unfocused");
        assert_eq!(
            border_fg(true, true),
            focused,
            "focused + disabled keeps the focused border"
        );
        assert_eq!(border_fg(false, true), normal, "unfocused + disabled");
    }

    #[test]
    /// UI-R-028 — a focused vim editor shows its current mode tag in the title.
    fn focused_vim_field_appends_mode_tag_to_title() {
        let w = CodeInputFieldBuilder::default()
            .border(full_border())
            .title(Some("code".into()))
            .build()
            .unwrap();
        let mut st = CodeInputFieldStateBuilder::default().build().unwrap();
        let mut b = buffer(20, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 4), &mut b, &mut st);
        assert!(title_row(&b, 20).contains("code [NORMAL]"));
    }

    #[test]
    /// UI-R-028 — an unfocused editor shows no mode tag.
    fn unfocused_field_has_no_mode_tag() {
        let w = CodeInputFieldBuilder::default()
            .border(full_border())
            .title(Some("code".into()))
            .build()
            .unwrap();
        let mut st = CodeInputFieldStateBuilder::default()
            .focused(false)
            .build()
            .unwrap();
        let mut b = buffer(20, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 4), &mut b, &mut st);
        let row = title_row(&b, 20);
        assert!(row.contains("code"));
        assert!(!row.contains('['));
    }

    #[test]
    /// UI-R-028 — the mode tag tracks Insert and Visual mode transitions.
    fn mode_tag_tracks_insert_and_visual_after_events() {
        // No configured title -> bare "[LABEL]" title.
        let w = CodeInputFieldBuilder::default()
            .border(full_border())
            .build()
            .unwrap();
        let mut st = CodeInputFieldStateBuilder::default().build().unwrap();

        st.handle_events(KeyModifiers::NONE, KeyCode::Char('i'));
        let mut b = buffer(20, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 4), &mut b, &mut st);
        assert!(title_row(&b, 20).contains("[INSERT]"));

        st.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        st.handle_events(KeyModifiers::NONE, KeyCode::Char('v'));
        let mut b = buffer(20, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 4), &mut b, &mut st);
        assert!(title_row(&b, 20).contains("[VISUAL]"));
    }

    #[test]
    /// UI-R-028 — a charwise visual selection is highlighted across two lines, with the cursor cell winning.
    fn selection_highlights_charwise_span_two_lines() {
        let w = CodeInputFieldBuilder::default().build().unwrap();
        let mut st = CodeInputFieldStateBuilder::default().build().unwrap();
        st.set_content("abcdef\nghijkl");
        st.set_active_line(0);
        st.set_cursor_col(2);
        st.handle_events(KeyModifiers::NONE, KeyCode::Char('v'));
        st.set_active_line(1);
        st.set_cursor_col(3);

        let mut b = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b, &mut st);

        // gutter_width = "2".len() + 1 = 2.
        let content_x = 2u16;
        let sel = w.style().selection();
        // Line 0: selection runs from col 2 to the end of the line.
        for col in 2..6u16 {
            assert_eq!(b[(content_x + col, 0)].fg, sel.fg.unwrap());
            assert_eq!(b[(content_x + col, 0)].bg, sel.bg.unwrap());
        }
        assert_ne!(b[(content_x, 0)].fg, sel.fg.unwrap());
        // Line 1: selection runs from col 0 up to (but not overwriting) the cursor at col 3.
        for col in 0..3u16 {
            assert_eq!(b[(content_x + col, 1)].fg, sel.fg.unwrap());
        }
        // The cursor cell wins over the selection highlight.
        let cursor = w.style().cursor();
        assert_eq!(b[(content_x + 3, 1)].fg, cursor.fg.unwrap());
        assert_eq!(b[(content_x + 3, 1)].bg, cursor.bg.unwrap());
    }

    #[test]
    /// UI-R-028 — a visual selection highlight is clipped to the horizontal-scroll window.
    fn selection_clips_to_h_scroll_window() {
        let w = CodeInputFieldBuilder::default().build().unwrap();
        let mut st = CodeInputFieldStateBuilder::default().build().unwrap();
        st.set_content("abcdefghijklmnop");
        st.set_cursor_col(2);
        st.handle_events(KeyModifiers::NONE, KeyCode::Char('v'));
        st.set_cursor_col(9);
        st.set_h_scroll(5);

        let mut b = buffer(10, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 10, 1), &mut b, &mut st);

        // gutter_width = "1".len() + 1 = 2; content window covers cols [5, 13).
        let content_x = 2u16;
        let sel = w.style().selection();
        // Selection cols 2..=9 clipped on the left to the h_scroll window: cols 5..9 show up.
        for col in 5..9u16 {
            let x = content_x + (col - 5);
            assert_eq!(b[(x, 0)].fg, sel.fg.unwrap());
        }
        // The cursor sits at col 9 (last selected col) and wins over the selection style.
        let cursor = w.style().cursor();
        let cursor_x = content_x + (9 - 5);
        assert_eq!(b[(cursor_x, 0)].fg, cursor.fg.unwrap());
    }

    fn gutter_cell(b: &Buffer, y: u16, width: u16) -> String {
        (0..width)
            .map(|x| b[(x, y)].symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    #[test]
    /// UI-R-165 — with gutter labels set, each row renders its label in place of the line index.
    fn ut_gutter_labels_replace_line_indices() {
        let w = CodeInputFieldBuilder::default().build().unwrap();
        let mut st = CodeInputFieldStateBuilder::default()
            .gutter_labels(Some(vec!["X".into(), "Y".into(), "Z".into()]))
            .build()
            .unwrap();
        st.set_content("a\nb\nc");
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        // gutter_width = 1 + 1 = 2 (widest label is 1 char, list covers the whole buffer).
        assert_eq!(gutter_cell(&b, 0, 1), "X");
        assert_eq!(gutter_cell(&b, 1, 1), "Y");
        assert_eq!(gutter_cell(&b, 2, 1), "Z");
    }

    #[test]
    /// UI-R-166 — an empty gutter label renders as a blank cell of the full gutter width.
    fn ut_empty_gutter_label_renders_blank_cell() {
        let w = CodeInputFieldBuilder::default().build().unwrap();
        let mut st = CodeInputFieldStateBuilder::default()
            .gutter_labels(Some(vec![String::new(), "100".into()]))
            .build()
            .unwrap();
        st.set_content("a\nb");
        let mut b = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b, &mut st);
        // gutter_width = 3 + 1 = 4; the blank row must be blank across the whole width.
        assert_eq!(gutter_cell(&b, 0, 3), "   ");
        assert_eq!(gutter_cell(&b, 1, 3), "100");
    }

    #[test]
    /// UI-R-167 — gutter width fits the widest label-list entry, so a wide label is never truncated.
    fn ut_gutter_width_fits_widest_label_plus_separator() {
        let w = CodeInputFieldBuilder::default().build().unwrap();
        let mut st = CodeInputFieldStateBuilder::default()
            .gutter_labels(Some(vec!["100".into()]))
            .build()
            .unwrap();
        st.set_content("a");
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut st);
        // gutter_width = 3 + 1 = 4; content starts right after.
        assert_eq!(gutter_cell(&b, 0, 3), "100");
        assert_eq!(b[(4, 0)].symbol(), "a");
    }

    #[test]
    /// UI-R-167 — when a row falls back to its line index (UI-R-168), the widest such fallback
    /// index enters the width formula: a short, narrow label list on an 10+-line buffer must not
    /// truncate the 2-digit fallback index on the unlabelled rows.
    fn ut_gutter_width_fits_widest_fallback_index() {
        let w = CodeInputFieldBuilder::default().build().unwrap();
        let mut st = CodeInputFieldStateBuilder::default()
            .gutter_labels(Some(vec!["X".into()]))
            .build()
            .unwrap();
        st.set_content("a\nb\nc\nd\ne\nf\ng\nh\ni\nj");
        let mut b = buffer(20, 10);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 10), &mut b, &mut st);
        // gutter_text_width = max(label width 1, fallback index digit count 2) = 2; width = 3.
        assert_eq!(gutter_cell(&b, 0, 2), " X");
        assert_eq!(gutter_cell(&b, 9, 2), "10");
    }

    #[test]
    /// UI-R-168 — a buffer row with no entry in a shorter label list falls back to its line index.
    fn ut_short_label_list_falls_back_to_line_index() {
        let w = CodeInputFieldBuilder::default().build().unwrap();
        let mut st = CodeInputFieldStateBuilder::default()
            .gutter_labels(Some(vec!["X".into()]))
            .build()
            .unwrap();
        st.set_content("a\nb\nc");
        let mut b = buffer(20, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 3), &mut b, &mut st);
        // gutter_width = max(label width 1, digit count of line_count 3 -> 1) + 1 = 2.
        assert_eq!(gutter_cell(&b, 0, 1), "X");
        assert_eq!(gutter_cell(&b, 1, 1), "2");
        assert_eq!(gutter_cell(&b, 2, 1), "3");
    }

    #[test]
    /// UI-E-080 — surplus labels past the end of the buffer are never rendered, but still widen the gutter.
    fn ut_surplus_labels_are_unrendered_but_widen_gutter() {
        let w = CodeInputFieldBuilder::default().build().unwrap();
        let mut st = CodeInputFieldStateBuilder::default()
            .gutter_labels(Some(vec![
                "AAAA".into(),
                "B".into(),
                "C".into(),
                "D".into(),
            ]))
            .build()
            .unwrap();
        st.set_content("a\nb");
        let mut b = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b, &mut st);
        // gutter_width = 4 (widest label, "AAAA", counted even though only 2 rows exist) + 1 = 5.
        assert_eq!(gutter_cell(&b, 0, 4), "AAAA");
        assert_eq!(gutter_cell(&b, 1, 4), "   B");
        assert_eq!(b[(5, 0)].symbol(), "a");
    }

    #[test]
    /// UI-R-172/UI-E-083 — a gutter label wider than the field's whole area clamps the gutter to
    /// the area width instead of overflowing it, leaving the content zero columns and not panicking.
    fn ut_gutter_wider_than_area_clamps_to_area_width() {
        let w = CodeInputFieldBuilder::default().build().unwrap();
        let mut st = CodeInputFieldStateBuilder::default()
            .gutter_labels(Some(vec!["1234567890".into()]))
            .build()
            .unwrap();
        st.set_content("a");
        let mut b = buffer(4, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 4, 1), &mut b, &mut st);
        // gutter_text_width = 10, uncapped gutter_width = 11, area.width = 4: clamps to 4,
        // leaving one separator column past the 3-wide gutter cell and zero content columns.
        assert_eq!(gutter_cell(&b, 0, 4), "123 ");
    }

    #[test]
    /// UI-R-172 — the gutter width clamp is applied before the `u16` cast, so a label wide
    /// enough to overflow `u16` (>= 65535 chars) clamps to the area width instead of wrapping.
    fn ut_gutter_width_clamp_does_not_wrap_on_oversized_label() {
        let w = CodeInputFieldBuilder::default().build().unwrap();
        let huge_label = "x".repeat(65_535);
        let mut st = CodeInputFieldStateBuilder::default()
            .gutter_labels(Some(vec![huge_label]))
            .build()
            .unwrap();
        st.set_content("a");
        let mut b = buffer(4, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 4, 1), &mut b, &mut st);
        assert_eq!(gutter_cell(&b, 0, 4), "xxx ");
    }

    #[test]
    /// UI-R-160 — a Diff context line yields no span, so it renders in the field's general
    /// style, not a syntax-kind style; a `+`-prefixed line on the same field renders in a
    /// distinct (diff-added) style, proving the general-style row is not a coincidence.
    fn ut_diff_context_line_renders_in_general_style() {
        let w = CodeInputFieldBuilder::default().build().unwrap();
        let mut st = CodeInputFieldStateBuilder::default()
            .language(Some(ferrowl_syntax::Language::Diff))
            .build()
            .unwrap();
        st.set_content("+added\n ctx");
        let mut b = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b, &mut st);
        // gutter_width = "2".len() + 1 = 2; content starts at x = 2.
        let general = w.style().general;
        assert_eq!(b[(2, 1)].fg, general.fg.unwrap());
        assert_eq!(b[(2, 1)].bg, general.bg.unwrap());
        assert_ne!(
            b[(2, 0)].fg,
            general.fg.unwrap(),
            "an added line must not use the general fg"
        );
    }

    #[test]
    /// UI-R-169 — gutter styling is independent of gutter content: labelled and indexed rows style identically.
    fn ut_gutter_style_is_independent_of_gutter_content() {
        let w = CodeInputFieldBuilder::default().build().unwrap();

        let style_at = |focused: bool, labels: Option<Vec<String>>, row: u16| -> (Style, Style) {
            let mut with_labels = CodeInputFieldStateBuilder::default()
                .focused(focused)
                .gutter_labels(labels)
                .build()
                .unwrap();
            with_labels.set_content("a\nb");
            with_labels.set_active_line(1);
            let mut without_labels = CodeInputFieldStateBuilder::default()
                .focused(focused)
                .build()
                .unwrap();
            without_labels.set_content("a\nb");
            without_labels.set_active_line(1);

            let mut b1 = buffer(20, 2);
            StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b1, &mut with_labels);
            let mut b2 = buffer(20, 2);
            StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b2, &mut without_labels);
            (b1[(0, row)].style(), b2[(0, row)].style())
        };

        let labels = Some(vec!["X".into(), "Y".into()]);
        for focused in [true, false] {
            for row in [0u16, 1u16] {
                let (labelled, indexed) = style_at(focused, labels.clone(), row);
                assert_eq!(
                    labelled, indexed,
                    "focused={focused} row={row}: labelled gutter style must match indexed"
                );
            }
        }
    }

    #[test]
    /// UI-R-293 — a render records the visible height in rows of the area it drew into.
    fn ut_render_records_visible_height() {
        let w = CodeInputFieldBuilder::default().build().unwrap();
        let mut st = CodeInputFieldStateBuilder::default().build().unwrap();
        let mut b = buffer(20, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 5), &mut b, &mut st);
        assert_eq!(st.visible_height(), 5);
    }

    #[test]
    /// UI-R-303 — zero before the first render, and a render scrolled to bring the
    /// active line into view records that scroll on the state.
    fn ut_vertical_scroll_offset_defaults_to_zero_and_follows_a_render() {
        let w = CodeInputFieldBuilder::default().build().unwrap();
        let mut st = CodeInputFieldStateBuilder::default().build().unwrap();
        assert_eq!(st.scroll_offset(), 0);
        st.set_content("0\n1\n2\n3\n4\n5\n6\n7\n8\n9");
        st.set_active_line(9);
        let mut b = buffer(20, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 5), &mut b, &mut st);
        assert_eq!(st.scroll_offset(), 5);
    }
}
