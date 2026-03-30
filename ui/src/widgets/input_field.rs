use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::text::Text;
use ratatui::widgets::Widget;
use ratatui::widgets::{Block, Paragraph, StatefulWidget};

use crate::state::InputFieldState;
use crate::style::InputFieldStyle;
use crate::traits::AsConstraint;

#[derive(Clone)]
pub struct InputField {
    placeholder: Option<String>,
    bordered: bool,
    style: InputFieldStyle,
    title: Option<String>,
    margins: Margin,
}

impl AsConstraint for InputField {
    fn horizontal(&self) -> Constraint {
        let width = if self.bordered { 2 } else { 0 };
        Constraint::Min(width + self.placeholder.as_ref().map(|s| s.len()).unwrap_or(0) as u16)
    }

    fn vertical(&self) -> Constraint {
        let height = if self.bordered { 3 } else { 1 };
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

        let height = if self.bordered { 3 } else { 1 };

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
        if self.bordered {
            let style = if state.in_focus() && !state.is_disabled() {
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

        let mut text = state
            .get_input()
            .unwrap_or(self.placeholder.clone().unwrap_or_default());

        let mut x_start = 0;
        let cursor = state.get_cursor();

        // Calculate range of text to display
        if (area.width as usize) < text.len() {
            let width = (area.width / 2) as usize;
            // Display width characters left of cursor
            x_start = std::cmp::max(state.get_cursor(), width) - width;
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
        if !state.is_disabled() {
            // Display cursor
            if state.in_focus() {
                buf[(area.x + (cursor - x_start) as u16, area.y)].set_style(self.style.cursor);
            }
        }
    }
}

impl InputField {
    pub fn new() -> Self {
        Self {
            placeholder: None,
            bordered: false,
            style: InputFieldStyle::default(),
            title: None,
            margins: Margin {
                vertical: 0,
                horizontal: 0,
            },
        }
    }

    pub fn title(self, title: String) -> Self {
        Self {
            title: Some(title),
            ..self
        }
    }

    pub fn bordered(self, bordered: bool) -> Self {
        Self { bordered, ..self }
    }

    pub fn style(self, style: InputFieldStyle) -> Self {
        Self { style, ..self }
    }

    pub fn margins(self, margins: Margin) -> Self {
        Self { margins, ..self }
    }

    pub fn set_placeholder(&mut self, input: String) {
        self.placeholder = Some(input);
    }

    pub fn clear_placeholder(&mut self) {
        self.placeholder = None;
    }

    pub fn set_style(&mut self, style: InputFieldStyle) {
        self.style = style;
    }
}
