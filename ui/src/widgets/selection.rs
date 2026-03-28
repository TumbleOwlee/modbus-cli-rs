use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Style, Stylize},
    text::Text,
    widgets::{Block, StatefulWidget, Widget},
};
use std::marker::PhantomData;

use super::super::state::SelectionState;
use super::super::traits::ToLabel;
use crate::Style as InputStyle;

pub struct Selection<ValueType>
where
    ValueType: ToLabel + Clone,
{
    title: Option<String>,
    bordered: bool,
    style: InputStyle,
    margins: Margin,
    marker: PhantomData<ValueType>,
    //colors: TableColors,
}

impl<ValueType> Selection<ValueType>
where
    ValueType: ToLabel + Clone,
{
    pub fn new() -> Self {
        Self {
            bordered: false,
            style: InputStyle::default(),
            title: None,
            margins: Margin {
                vertical: 0,
                horizontal: 0,
            },
            marker: PhantomData,
            //colors: TableColors::new(&PALETTES[0]),
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

    pub fn style(self, style: InputStyle) -> Self {
        Self { style, ..self }
    }

    pub fn margins(self, margins: Margin) -> Self {
        Self { margins, ..self }
    }

    pub fn set_style(&mut self, style: InputStyle) {
        self.style = style;
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
        let mut state = SelectionState::default();
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
        let split = 4usize;
        let count = split * 2 + 1;
        let height = if self.bordered { count + 2 } else { count };

        let area = Layout::vertical([
            Constraint::Length(self.margins.vertical),
            Constraint::Length(height as u16),
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
            let style = self.style.focused;
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

        let values = state
            .get_values()
            .iter()
            .map(ToLabel::to_label)
            .collect::<Vec<_>>();

        let constraints = vec![Constraint::Length(1); count];
        let area = Layout::vertical(constraints).split(area);

        let selection = state.get_selection_index();
        let start = if selection >= split && values.len() - split >= selection {
            selection - split
        } else if selection < split || values.len() < count {
            0
        } else {
            values.len() - count
        };

        for (n, (i, v)) in values
            .into_iter()
            .enumerate()
            .filter(|(i, _)| *i >= start && *i < (start + count))
            .enumerate()
        {
            //let bg = self.colors.row_color.bg.get(i % 2);
            //let fg = self.colors.row_color.fg;
            //let mut style = Style::new().fg(fg).bg(bg);
            //if i == selection {
            //    style = self.style.focused.reversed();
            //}
            let t = Text::from(v); //.style(style);
            t.render(area[n], buf);
        }
    }
}
