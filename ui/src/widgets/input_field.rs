use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::text::Text;
use ratatui::widgets::Widget;
use ratatui::widgets::{Block, Paragraph, StatefulWidget};

use crate::state::InputFieldState;
use crate::style::InputFieldStyle;
use crate::traits::AsConstraint;

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct InputField {
    #[getset(get_copy = "pub")]
    #[builder(default = "false")]
    border: bool,
    #[getset(get = "pub")]
    #[builder(default = "InputFieldStyle::default()")]
    style: InputFieldStyle,
    #[getset(get = "pub")]
    #[builder(default = "None")]
    title: Option<String>,
    #[getset(get = "pub")]
    #[builder(default = "Margin::default()")]
    margins: Margin,
}

impl AsConstraint for InputField {
    fn horizontal(&self) -> Constraint {
        let width = if self.border { 2 } else { 0 };
        Constraint::Min(width)
    }

    fn vertical(&self) -> Constraint {
        let height = if self.border { 3 } else { 1 };
        Constraint::Length(height)
    }
}

impl Widget for InputField {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Widget::render(&self, area, buf);
    }
}

impl Widget for &InputField {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut state = InputFieldState::default();
        StatefulWidget::render(self, area, buf, &mut state);
    }
}

impl StatefulWidget for InputField {
    type State = InputFieldState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

impl StatefulWidget for &InputField {
    type State = InputFieldState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        buf.set_style(area, self.style.default);

        let height = if self.border { 3 } else { 1 };

        let area = Layout::vertical([
            Constraint::Length(self.margins.vertical),
            Constraint::Length(height),
            Constraint::Length(self.margins.vertical),
        ])
        .split(area)[1];

        let mut area = Layout::horizontal([
            Constraint::Length(self.margins.horizontal),
            Constraint::Min(1),
            Constraint::Length(self.margins.horizontal),
        ])
        .split(area)[1];

        // Create block if border is required
        if self.border {
            let style = if state.focused() && !state.disabled() {
                self.style.focused
            } else {
                self.style.default
            };
            let mut block = Block::bordered().style(style);
            if let Some(title) = self.title.as_ref() {
                block = block.title(title.clone());
            }
            let inner = block.inner(area);
            block.render(area, buf);
            area = inner.inner(Margin {
                vertical: 0,
                horizontal: 1,
            });
        }

        let input = state.input();
        let mut text = if input.is_empty() {
            state.placeholder().clone().unwrap_or_default()
        } else {
            input.clone()
        };

        let mut x_start = 0;
        let cursor = state.cursor();

        // Calculate range of text to display
        if (area.width as usize) < text.len() {
            let width = (area.width / 2) as usize;
            // Display width characters left of cursor
            x_start = std::cmp::max(state.cursor(), width) - width;
            // Display width characters right of cursor
            let mut x_end = std::cmp::min(cursor + width, text.len());
            // Add more characters to the left, if right of cursor are not enough
            if (x_end - cursor) < (area.width as usize - width) {
                let remaining = (area.width as usize - width) - (x_end - cursor);
                x_start = std::cmp::max(x_start, remaining) - remaining;
            }
            // Add more characters to the right, if left of cursor are not enough
            if (cursor - x_start) < width {
                let remaining = width - (cursor - x_start);
                x_end = std::cmp::min(text.len(), x_end + remaining);
            }
            // Get displayable text area
            text = text[x_start..x_end].to_owned();
        }

        // Display text
        let input = Paragraph::new(Text::from(text).style(self.style.default));
        input.render(area, buf);
        if !state.disabled() {
            // Display cursor
            if state.focused() {
                buf[(area.x + (cursor - x_start) as u16, area.y)].set_style(self.style.cursor);
            }
        }
    }
}
