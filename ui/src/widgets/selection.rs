use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Margin, Rect},
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
    max_lines: usize,
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
            max_lines: 5,
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

    pub fn max_lines(self, height: usize) -> Self {
        Self {
            max_lines: height,
            ..self
        }
    }

    pub fn set_style(&mut self, style: InputStyle) {
        self.style = style;
    }

    pub fn set_max_lines(&mut self, height: usize) {
        self.max_lines = height;
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
        let lines = state.get_values().len();
        let lines = std::cmp::min(lines, self.max_lines);
        let height = if self.bordered { lines + 2 } else { lines };

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
            let style = if state.in_focus() {
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

        let values = state
            .get_values()
            .iter()
            .map(ToLabel::to_label)
            .collect::<Vec<_>>();

        let constraints = vec![Constraint::Length(1); lines];
        let area = Layout::vertical(constraints).split(area);

        let selection = state.get_selection_index();
        let offset = lines / 2;

        let mut start = std::cmp::max(0, selection as i32 - offset as i32);
        let end = std::cmp::min(
            state.get_values().len() as i32,
            start + self.max_lines as i32,
        );
        if end == state.get_values().len() as i32 {
            start = std::cmp::max(end - self.max_lines as i32, 0);
        }

        for (n, (i, v)) in values
            .into_iter()
            .enumerate()
            .filter(|(i, _)| (*i as i32) >= start && (*i as i32) < end)
            .enumerate()
        {
            let t = if i == selection {
                if state.in_focus() {
                    Text::from(v).style(self.style.focused.clone().reversed())
                } else {
                    Text::from(v).style(self.style.default.clone())
                }
            } else {
                Text::from(v).style(self.style.default.clone())
            };
            t.render(area[n], buf);
        }
    }
}
