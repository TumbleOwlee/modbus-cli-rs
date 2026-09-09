use derive_builder::Builder;
use getset::{Getters, Setters, WithSetters};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, StatefulWidget, Widget};

use crate::COLOR_SCHEME;
use crate::state::CommandLineState;
use crate::style::InputFieldStyle;
use crate::widgets::{InputField, InputFieldBuilder};

/// A single-row `:`-prompted command line, rendered from a
/// [`CommandLineState`]: the prompt and input while open, otherwise the
/// error, notice or hint in that order (UI-R-194). An optional help list
/// (UI-R-196, UI-R-197) draws a bordered box above the row while open.
#[derive(Builder, Debug, Clone, Getters, Setters, WithSetters)]
#[getset(set = "pub")]
pub struct CommandLine {
    #[getset(get = "pub")]
    #[builder(
        default = "InputFieldBuilder::default().build().expect(\"InputFieldBuilder fields all default\")"
    )]
    input: InputField<String>,
    #[getset(get = "pub")]
    #[builder(default = "InputFieldStyle::default()")]
    style: InputFieldStyle,
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(COLOR_SCHEME.error).bg(COLOR_SCHEME.bg)")]
    error_style: Style,
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(COLOR_SCHEME.hi).bg(COLOR_SCHEME.bg)")]
    highlight_style: Style,
    #[getset(get = "pub")]
    #[builder(default = "Vec::new()")]
    help: Vec<(String, String)>,
}

impl Default for CommandLine {
    fn default() -> Self {
        CommandLineBuilder::default()
            .build()
            .expect("CommandLineBuilder fields all default")
    }
}

impl CommandLine {
    /// UI-R-196 — anchored to the bottom of `area`, clipped to the rows above it
    /// (UI-E-093); UI-R-197 handled by the caller, which skips this when `help` is empty.
    fn render_help(&self, area: Rect, buf: &mut Buffer) {
        let lines: Vec<Line> = self
            .help
            .iter()
            .map(|(usage, description)| {
                Line::from(vec![
                    Span::styled(usage.clone(), self.highlight_style.bold()),
                    Span::raw(" "),
                    Span::styled(description.clone(), *self.style.general()),
                ])
            })
            .collect();
        let popup_h = lines.len() as u16 + 2;
        let popup = Rect {
            x: area.x,
            y: area.y.saturating_sub(popup_h),
            width: area.width,
            height: popup_h.min(area.y),
        };
        Widget::render(Clear, popup, buf);
        let block = Block::bordered().style(*self.style.border());
        let inner = block.inner(popup);
        Widget::render(block, popup, buf);
        Widget::render(Paragraph::new(lines), inner, buf);
    }
}

impl StatefulWidget for CommandLine {
    type State = CommandLineState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

impl StatefulWidget for &CommandLine {
    type State = CommandLineState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        buf.set_style(area, *self.style.general());

        if state.is_open() {
            if !self.help.is_empty() {
                self.render_help(area, buf);
            }
            buf.set_string(area.x, area.y, ":", *self.highlight_style());
            let input_area = Rect {
                x: area.x.saturating_add(1),
                y: area.y,
                width: area.width.saturating_sub(1),
                height: area.height,
            };
            StatefulWidget::render(&self.input, input_area, buf, state.input_mut());
        } else if let Some(err) = state.error() {
            buf.set_string(area.x, area.y, err, *self.error_style());
        } else if let Some(notice) = state.notice() {
            buf.set_string(area.x, area.y, notice, *self.style.general());
        } else {
            buf.set_string(area.x, area.y, state.hint(), *self.style.general());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::CommandLineStateBuilder;
    use ratatui::layout::Rect as RRect;

    fn buffer(w: u16, h: u16) -> Buffer {
        Buffer::empty(RRect::new(0, 0, w, h))
    }

    fn row_text(b: &Buffer, y: u16, w: u16) -> String {
        (0..w)
            .map(|x| b[(x, y)].symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    #[test]
    /// UI-R-194 — precedence: open prompt, then error, then notice, then hint.
    fn ut_render_precedence_prompt_then_error_then_notice_then_hint() {
        let w = CommandLine::default();

        let mut open_state = CommandLineStateBuilder::default().build().unwrap();
        open_state.set_error(Some("bad".to_string()));
        open_state.set_notice(Some("saved".to_string()));
        open_state.open();
        let mut b = buffer(40, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 1), &mut b, &mut open_state);
        assert!(row_text(&b, 0, 40).starts_with(':'));

        let mut error_state = CommandLineStateBuilder::default().build().unwrap();
        error_state.set_error(Some("bad".to_string()));
        error_state.set_notice(Some("saved".to_string()));
        let mut b = buffer(40, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 1), &mut b, &mut error_state);
        assert!(row_text(&b, 0, 40).starts_with("bad"));

        let mut notice_state = CommandLineStateBuilder::default().build().unwrap();
        notice_state.set_notice(Some("saved".to_string()));
        let mut b = buffer(40, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 1), &mut b, &mut notice_state);
        assert!(row_text(&b, 0, 40).starts_with("saved"));

        let mut hint_state = CommandLineStateBuilder::default().build().unwrap();
        hint_state.set_hint("type :help".to_string());
        let mut b = buffer(40, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 1), &mut b, &mut hint_state);
        assert!(row_text(&b, 0, 40).starts_with("type :help"));
    }

    #[test]
    /// UI-R-196 — the help box renders above the line with a bold usage column.
    fn ut_help_box_renders_above_the_line_with_bold_usage_column() {
        let w = CommandLineBuilder::default()
            .help(vec![("cmd".to_string(), "does a thing".to_string())])
            .build()
            .unwrap();
        let mut state = CommandLineStateBuilder::default().build().unwrap();
        state.open();
        let mut b = buffer(40, 8);
        StatefulWidget::render(&w, Rect::new(0, 7, 40, 1), &mut b, &mut state);
        let popup_row = row_text(&b, 5, 40);
        assert!(popup_row.contains("cmd"));
        let usage_x = popup_row.find('c').unwrap() as u16;
        assert!(
            b[(usage_x, 5)]
                .modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
    }

    #[test]
    /// UI-R-197 — an empty help list renders no box.
    fn ut_empty_help_list_renders_no_box() {
        let w = CommandLine::default();
        let mut state = CommandLineStateBuilder::default().build().unwrap();
        state.open();
        let mut b = buffer(40, 8);
        StatefulWidget::render(&w, Rect::new(0, 7, 40, 1), &mut b, &mut state);
        for y in 0..7 {
            assert_eq!(row_text(&b, y, 40).trim(), "");
        }
    }

    #[test]
    /// UI-E-093 — a help box taller than the space available above the line is clipped to
    /// the rows it has and stays anchored to the row: with 2 rows above the line and a
    /// help list that would need 5, the box occupies rows 0-1, and the row directly above
    /// the line is its bottom border, not the box's own uncut bottom.
    fn ut_help_box_taller_than_the_space_above_is_clipped_and_stays_anchored() {
        let w = CommandLineBuilder::default()
            .help(vec![
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "2".to_string()),
                ("c".to_string(), "3".to_string()),
            ])
            .build()
            .unwrap();
        let mut state = CommandLineStateBuilder::default().build().unwrap();
        state.open();
        let mut b = buffer(40, 3);
        StatefulWidget::render(&w, Rect::new(0, 2, 40, 1), &mut b, &mut state);
        assert_eq!(b[(0, 0)].symbol(), ratatui::symbols::border::PLAIN.top_left);
        assert_eq!(
            b[(0, 1)].symbol(),
            ratatui::symbols::border::PLAIN.bottom_left
        );
        assert!(row_text(&b, 2, 40).starts_with(':'));
    }
}
