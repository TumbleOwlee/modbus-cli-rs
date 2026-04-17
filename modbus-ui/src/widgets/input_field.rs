use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::text::Text;
use ratatui::widgets::Widget;
use ratatui::widgets::{Block, Paragraph, StatefulWidget};
use std::marker::PhantomData;

use crate::state::InputFieldState;
use crate::style::InputFieldStyle;
use crate::traits::Margins;
use crate::types::Border;
use crate::widgets::Title;

pub trait Validate {
    fn validate(input: &str) -> Result<(), String>;
}

impl Validate for String {
    fn validate(_input: &str) -> Result<(), String> {
        Ok(())
    }
}

macro_rules! generate_validate {
    ($v:ty) => {
        impl Validate for $v {
            fn validate(input: &str) -> Result<(), String> {
                let result = input.parse::<$v>();
                match result {
                    Ok(_) => Ok(()),
                    Err(e) => Err(format!("{}", e)),
                }
            }
        }
    };
}

generate_validate!(usize);
generate_validate!(u8);
generate_validate!(u16);
generate_validate!(u32);
generate_validate!(u64);
generate_validate!(u128);
generate_validate!(i8);
generate_validate!(i16);
generate_validate!(i32);
generate_validate!(i64);
generate_validate!(i128);
generate_validate!(f32);
generate_validate!(f64);

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct InputField<ValueType>
where
    ValueType: Validate,
{
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
    #[builder(default = "false")]
    multiline: bool,
    #[builder(setter(skip))]
    #[builder(default = "PhantomData")]
    marker: PhantomData<ValueType>,
}

impl<ValueType> Margins for InputField<ValueType>
where
    ValueType: Validate,
{
    fn margins(&self) -> Margin {
        let horizontal = if let Border::Full(margin) = &self.border {
            4 + margin.horizontal * 2
        } else {
            0
        } + 2 * self.margin.horizontal
            + 1;
        let vertical = if let Border::Full(margin) = &self.border {
            2 + margin.vertical * 2
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
        buf.set_style(area, self.style.general);

        let mut height = if let Border::Full(margin) = &self.border {
            2 + margin.vertical * 2
        } else {
            0
        };
        if self.multiline {
            height += std::cmp::max(1, area.height);
        } else {
            height += 1;
        }

        let area = Layout::vertical([
            Constraint::Length(self.margin.vertical),
            Constraint::Length(height),
            Constraint::Length(self.margin.vertical),
        ])
        .split(area)[1];

        let mut area = Layout::horizontal([
            Constraint::Length(self.margin.horizontal),
            Constraint::Min(1),
            Constraint::Length(self.margin.horizontal),
        ])
        .split(area)[1];

        let valid = if state.input().is_empty() {
            Ok(())
        } else {
            ValueType::validate(state.input())
        };

        // Create block if border is required
        if let Border::Full(margin) = &self.border {
            let style = if state.focused() && !state.disabled() {
                self.style.focused
            } else {
                self.style.general
            };
            let style = if valid.is_ok() {
                style
            } else {
                self.style.error
            };
            let mut block = Block::bordered().style(style);
            if let Some(title) = self.title.as_ref() {
                block = block
                    .title(title.name.as_str())
                    .title_alignment(title.alignment);
            }
            let inner = block.inner(area);
            block.render(area, buf);
            area = inner.inner(margin.clone());
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
        if area.width > 0 && area.height > 0 && (area.width as usize) <= text_len {
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

        let text_style = if state.input().is_empty() {
            self.style.placeholder
        } else if valid.is_ok() {
            self.style.general
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
