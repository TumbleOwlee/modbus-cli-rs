use std::marker::PhantomData;

use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Margin, Rect},
    widgets::{Block, StatefulWidget, Widget},
};

use crate::{
    state::{TableState, TableStateBuilder, ToRow},
    style::TableStyle,
    traits::AsConstraint,
};

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct Table<ValueType, const N: usize>
where
    ValueType: ToRow<N> + Clone,
{
    #[getset(get = "pub")]
    #[builder(default = "None")]
    title: Option<String>,
    #[getset(get = "pub")]
    header: [String; N],
    #[getset(get_copy = "pub")]
    #[builder(default = "false")]
    border: bool,
    #[getset(get = "pub")]
    #[builder(default = "TableStyle::default()")]
    style: TableStyle<N>,
    #[getset(get = "pub")]
    #[builder(default = "Margin::default()")]
    margin: Margin,
    #[getset(get = "pub")]
    #[builder(default = "Margin::default()")]
    row_margin: Margin,
    #[builder(setter(skip))]
    #[builder(default = "PhantomData")]
    marker: PhantomData<ValueType>,
}

impl<ValueType, const N: usize> AsConstraint for Table<ValueType, N>
where
    ValueType: ToRow<N> + Clone,
{
    type State = TableState<ValueType, N>;

    fn horizontal(&self, _state: &Self::State, _height: Option<u16>) -> Constraint {
        let width = if self.border { 7 } else { 5 };
        Constraint::Min(width + self.margin.horizontal + self.row_margin.horizontal)
    }

    fn vertical(&self, _state: &Self::State, _width: Option<u16>) -> Constraint {
        let height = if self.border { 3 } else { 1 };
        Constraint::Min(height + self.margin.vertical + self.row_margin.vertical)
    }
}

impl<ValueType, const N: usize> Widget for Table<ValueType, N>
where
    ValueType: ToRow<N> + Clone,
{
    fn render(self, area: Rect, buf: &mut Buffer) {
        Widget::render(&self, area, buf);
    }
}

impl<ValueType, const N: usize> Widget for &Table<ValueType, N>
where
    ValueType: ToRow<N> + Clone,
{
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut state = TableStateBuilder::default().build().unwrap();
        StatefulWidget::render(self, area, buf, &mut state);
    }
}

impl<ValueType, const N: usize> StatefulWidget for Table<ValueType, N>
where
    ValueType: ToRow<N> + Clone,
{
    type State = TableState<ValueType, N>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

impl<ValueType, const N: usize> StatefulWidget for &Table<ValueType, N>
where
    ValueType: ToRow<N> + Clone,
{
    type State = TableState<ValueType, N>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let area = Layout::vertical([
            Constraint::Length(self.margin.vertical),
            Constraint::Min(2),
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
        if self.border {
            let style = if state.focused() {
                self.style.border
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

        let column_margin = 2;
        let rows: Vec<_> = state.values().iter().map(|v| v.to_row()).collect();

        let column_widths = {
            let mut widths = [0usize; N];
            // Get widths necessary for each heading
            for i in 0..N {
                widths[i] = self.header[i].chars().count() + column_margin;
            }
            // Get max widths for each column
            rows.iter().fold(widths, |mut v, row| {
                for i in 0..N {
                    v[i] = std::cmp::max(v[i], row[i].chars().count() + column_margin);
                }
                v
            })
        };

        unimplemented!("Render of table");
    }
}
