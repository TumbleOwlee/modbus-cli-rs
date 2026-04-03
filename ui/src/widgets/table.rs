use std::marker::PhantomData;

use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};
use ratatui::{
    buffer::Buffer,
    layout::{Margin, Rect},
    widgets::{StatefulWidget, Widget},
};

use crate::{
    state::{TableState, TableStateBuilder, ToRow},
    style::TableStyle,
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
    bordered: bool,
    #[getset(get = "pub")]
    #[builder(default = "TableStyle::default()")]
    style: TableStyle,
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
        unimplemented!("Render of table");
    }
}
