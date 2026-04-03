use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::Color;
use ratatui::style::palette::tailwind;
use ratatui::text::Text;
use ratatui::widgets::Widget;
use ratatui::widgets::{Block, Paragraph, StatefulWidget};
use std::marker::PhantomData;

use crate::state::InputFieldState;
use crate::style::InputFieldStyle;
use crate::traits::AsConstraint;

pub trait Validate {
    fn validate(input: &str) -> Result<(), String>;
}

impl Validate for String {
    fn validate(_input: &str) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct InputField<ValueType>
where
    ValueType: Validate,
{
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
    #[getset(get = "pub")]
    #[builder(default = "0")]
    min_width: u16,
    #[getset(get = "pub")]
    #[builder(default = "false")]
    multiline: bool,
    #[builder(setter(skip))]
    #[builder(default = "PhantomData")]
    marker: PhantomData<ValueType>,
}

impl<ValueType> AsConstraint for InputField<ValueType>
where
    ValueType: Validate,
{
    fn horizontal(&self) -> Constraint {
        let width = if self.border { 2 } else { 0 };
        Constraint::Min(width + self.margins.horizontal + self.min_width)
    }

    fn vertical(&self) -> Constraint {
        let height = if self.border { 3 } else { 1 };
        Constraint::Length(height + self.margins.vertical)
    }
}

impl<ValueType> Widget for InputField<ValueType>
where
    ValueType: Validate,
{
    fn render(self, area: Rect, buf: &mut Buffer) {
        Widget::render(&self, area, buf);
    }
}

impl<ValueType> Widget for &InputField<ValueType>
where
    ValueType: Validate,
{
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut state = InputFieldState::default();
        StatefulWidget::render(self, area, buf, &mut state);
    }
}

impl<ValueType> StatefulWidget for InputField<ValueType>
where
    ValueType: Validate,
{
    type State = InputFieldState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

impl<ValueType> StatefulWidget for &InputField<ValueType>
where
    ValueType: Validate,
{
    type State = InputFieldState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        buf.set_style(area, self.style.default);

        let mut height = if self.border { 2 } else { 0 };
        if self.multiline {
            height += std::cmp::max(1, area.height);
        } else {
            height += 1;
        }

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

        let valid = if state.input().is_empty() {
            Ok(())
        } else {
            ValueType::validate(state.input())
        };

        // Create block if border is required
        if self.border {
            let style = if state.focused() && !state.disabled() {
                self.style.focused
            } else {
                self.style.default
            };
            let style = if valid.is_ok() {
                style
            } else {
                self.style.error
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
        let text_len = text.chars().count();

        // Calculate range of text to display
        if (area.width as usize) <= text_len {
            let total_len = area.width * area.height - 1;
            let width = (total_len / 2) as usize;
            // Display width characters left of cursor
            x_start = std::cmp::max(state.cursor(), width) - width;
            // Display width characters right of cursor
            let mut x_end = std::cmp::min(cursor + width, text_len);
            // Add more characters to the left, if right of cursor are not enough
            if (x_end - cursor) < (total_len as usize - width) {
                let remaining = (total_len as usize - width) - (x_end - cursor);
                x_start = std::cmp::max(x_start, remaining) - remaining;
            }
            // Add more characters to the right, if left of cursor are not enough
            if (cursor - x_start) < width {
                let remaining = width - (cursor - x_start);
                x_end = std::cmp::min(text_len, x_end + remaining);
            }
            // Get displayable text area
            text = text.chars().enumerate().fold(
                String::with_capacity(x_end - x_start),
                |mut s, (i, c)| {
                    if i >= x_start && i < x_end {
                        s.push(c);
                    }
                    s
                },
            );
        }

        let text_style = if valid.is_ok() {
            self.style.default
        } else {
            self.style.error
        };

        let mut text_area = area.clone();
        let (len, remain) = text
            .chars()
            .fold((0, String::new()), |(mut len, mut line), c| {
                line.push(c);
                len += 1;
                if len >= area.width as usize {
                    let input = Paragraph::new(Text::from(line).style(text_style));
                    input.render(text_area, buf);
                    text_area.y += 1;
                    (0, String::new())
                } else {
                    (len, line)
                }
            });
        if len > 0 {
            let input = Paragraph::new(Text::from(remain).style(text_style));
            input.render(text_area, buf);
        }

        if !state.disabled() {
            // Display cursor
            if state.focused() {
                let pos = (cursor - x_start) as u16;
                let pos_x = pos % area.width;
                let pos_y = pos / area.width;
                buf[(area.x + pos_x, area.y + pos_y)].set_style(self.style.cursor);
            }
        }
    }
}
