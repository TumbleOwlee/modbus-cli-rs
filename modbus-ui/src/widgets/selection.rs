use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Margin, Rect},
    text::Text,
    widgets::{Block, StatefulWidget, Widget},
};
use std::marker::PhantomData;

use crate::traits::ToLabel;
use crate::{
    state::{SelectionState, SelectionStateBuilder},
    traits::Margins,
    types::Border,
};
use crate::{style::SelectionStyle, widgets::Title};

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct Selection<ValueType>
where
    ValueType: ToLabel + Clone,
{
    #[getset(get = "pub")]
    #[builder(default = "None")]
    title: Option<Title>,
    #[getset(get = "pub")]
    #[builder(default = "Border::None")]
    border: Border,
    #[getset(get = "pub")]
    #[builder(default = "SelectionStyle::default()")]
    style: SelectionStyle,
    #[getset(get = "pub")]
    #[builder(default = "Margin::default()")]
    margin: Margin,
    #[builder(setter(skip))]
    #[builder(default = "PhantomData")]
    marker: PhantomData<ValueType>,
}

impl<ValueType> Margins for Selection<ValueType>
where
    ValueType: ToLabel + Clone,
{
    fn margins(&self) -> Margin {
        let horizontal = if let Border::Full(margin) = &self.border {
            4 + margin.horizontal * 2
        } else {
            0
        } + 2 * self.margin.horizontal;
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

impl<ValueType> Widget for Selection<ValueType>
where
    ValueType: ToLabel + Clone,
{
    fn render(self, area: Rect, buf: &mut Buffer) {
        Widget::render(&self, area, buf);
    }
}

impl<ValueType> Widget for &Selection<ValueType>
where
    ValueType: ToLabel + Clone,
{
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut state = SelectionStateBuilder::default().build().unwrap();
        StatefulWidget::render(self, area, buf, &mut state);
    }
}

impl<ValueType> StatefulWidget for Selection<ValueType>
where
    ValueType: ToLabel + Clone,
{
    type State = SelectionState<ValueType>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

impl<ValueType: ToLabel + Clone> StatefulWidget for &Selection<ValueType> {
    type State = SelectionState<ValueType>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let border_lines = if let Border::Full(margin) = &self.border {
            2 + 2 * margin.vertical as i32
        } else {
            0
        };
        let max_lines = area.height as i32 - border_lines;
        let lines = state.values().len();
        let lines = std::cmp::min(lines as i32, max_lines);
        let height = if let Border::Full(margin) = &self.border {
            lines + 2 + 2 * margin.vertical as i32
        } else {
            lines
        };

        let area = Layout::vertical([
            Constraint::Length(self.margin.vertical),
            Constraint::Length(height as u16),
            Constraint::Length(self.margin.vertical),
        ])
        .split(area)[1];

        let mut area = Layout::horizontal([
            Constraint::Length(self.margin.horizontal),
            Constraint::Min(1),
            Constraint::Length(self.margin.horizontal),
        ])
        .split(area)[1];

        // Create block if border is required
        if let Border::Full(margin) = &self.border {
            let style = if state.focused() {
                self.style.border
            } else {
                self.style.general
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

        let values = state
            .values()
            .iter()
            .map(ToLabel::to_label)
            .collect::<Vec<_>>();

        let constraints = vec![Constraint::Length(1); lines as usize];
        let area = Layout::vertical(constraints).split(area);

        let selection = state.selection();
        let offset = lines / 2;

        let mut start = std::cmp::max(0, selection as i32 - offset as i32);
        let end = std::cmp::min(state.values().len() as i32, start + max_lines as i32);
        if end == state.values().len() as i32 {
            start = std::cmp::max(end - max_lines as i32, 0);
        }

        for (n, (i, v)) in values
            .into_iter()
            .enumerate()
            .filter(|(i, _)| (*i as i32) >= start && (*i as i32) < end)
            .enumerate()
        {
            let t = if i == selection {
                let text = format!("{}", v);
                let text_len = text.chars().count();
                let width = area[n].width as usize;
                let mut offset = state.horizontal_offset();

                if text_len > width {
                    offset = std::cmp::min(offset, text_len - width);
                    state.set_horizontal_offset(offset);
                }

                let text = if width < text_len {
                    text.chars().skip(offset).collect()
                } else {
                    text
                };

                if state.focused() {
                    Text::from(text).style(self.style.focused.clone())
                } else {
                    Text::from(text).style(self.style.general.clone())
                }
            } else {
                Text::from(v).style(self.style.rows[i % 2])
            };
            t.render(area[n], buf);
        }
    }
}
