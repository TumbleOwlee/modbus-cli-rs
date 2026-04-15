use std::marker::PhantomData;

use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Margin, Rect},
    text::{Line, Text},
    widgets::{
        Block, Cell, HighlightSpacing, Row, Scrollbar, ScrollbarOrientation, StatefulWidget,
        Table as UiTable, Widget,
    },
};

use crate::{
    state::{TableState, TableStateBuilder},
    style::TableStyle,
    types::Border,
};

pub trait Header<const N: usize> {
    fn header() -> [String; N];
    fn widths() -> [u16; N];
}

pub trait TableEntry<const N: usize> {
    fn values(&self) -> [String; N];
    fn height(&self) -> u16;
}

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct Table<V, H, const N: usize>
where
    V: TableEntry<N>,
    H: Header<N>,
{
    #[getset(get = "pub")]
    #[builder(default = "None")]
    title: Option<String>,
    #[getset(get = "pub")]
    #[builder(default = "Border::None")]
    border: Border,
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
    marker: PhantomData<V>,
    #[builder(setter(skip))]
    #[builder(default = "PhantomData")]
    header: PhantomData<H>,
}

impl<V, H, const N: usize> Widget for Table<V, H, N>
where
    V: TableEntry<N> + Clone,
    H: Header<N>,
{
    fn render(self, area: Rect, buf: &mut Buffer) {
        Widget::render(&self, area, buf);
    }
}

impl<V, H, const N: usize> Widget for &Table<V, H, N>
where
    V: TableEntry<N> + Clone,
    H: Header<N>,
{
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut state = TableStateBuilder::default().build().unwrap();
        StatefulWidget::render(self, area, buf, &mut state);
    }
}

impl<V, H, const N: usize> StatefulWidget for Table<V, H, N>
where
    V: TableEntry<N>,
    H: Header<N>,
{
    type State = TableState<V, N>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

impl<V, H, const N: usize> StatefulWidget for &Table<V, H, N>
where
    V: TableEntry<N>,
    H: Header<N>,
{
    type State = TableState<V, N>;

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
        if let Border::Full(margin) = &self.border {
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
            area = inner.inner(margin.clone());
        }

        let header = H::header();
        let header = header
            .iter()
            .map(|c| Cell::from(c.as_str()))
            .collect::<Row>()
            .style(self.style.header.clone())
            .height(1);
        let table_width = H::widths()
            .iter()
            .fold(0u16, |acc, w| acc + w + self.row_margin.vertical * 2);
        state.set_total_width(std::cmp::max(table_width, area.width));

        let selected_style = &self.style.focused;
        let bar_style = &self.style.focused;
        let mut bar_height = 0;

        let rows = state.values().iter().enumerate().map(|(i, item)| {
            let color = self.style.rows.get(i % 2).unwrap();
            let spacing =
                itertools::repeat_n('\n', self.row_margin.vertical as usize).collect::<String>();
            let mut max_line_cnt = 0;
            let row = item
                .values()
                .iter()
                .zip(H::widths())
                .map(|(content, width)| {
                    let mut line_cnt = 0;
                    let mut line = String::with_capacity(width as usize);
                    let mut output =
                        String::with_capacity(content.len() + (content.len() / width as usize) + 1);
                    for s in content.split_whitespace() {
                        if line.len() + s.len() < width as usize {
                            if !line.is_empty() {
                                line += " ";
                            }
                            line += s;
                        } else {
                            if output.is_empty() {
                                output += &format!("{line}");
                            } else {
                                output += &format!("\n{line}");
                            }
                            line_cnt += 1;
                            line.clear();
                            line += s;
                        }
                    }
                    if !line.is_empty() {
                        if output.is_empty() {
                            output += &format!("{line}");
                        } else {
                            output += &format!("\n{line}");
                        }
                        line_cnt += 1;
                    }
                    max_line_cnt = std::cmp::max(line_cnt, max_line_cnt);
                    Cell::from(Text::from(format!("{spacing}{output}{spacing}")))
                })
                .collect::<Row>()
                .style(*color)
                .height(self.row_margin.vertical * 2 + max_line_cnt);
            if state
                .table_state()
                .selected()
                .map(|i| state.values().get(i).map(|v| v.height()).unwrap_or(0))
                .unwrap_or(0) as usize
                == i
            {
                bar_height = max_line_cnt;
            }
            row
        });

        let constraints = H::widths()
            .iter()
            .map(|w| Constraint::Min(*w))
            .collect::<Vec<_>>();

        let bar = " █ ";
        let t = UiTable::new(rows, constraints)
            .header(header)
            .row_highlight_style(selected_style.clone())
            .highlight_symbol({
                Text::from({
                    let mut text = vec!["".into()];
                    text.append(
                        &mut itertools::repeat_n(bar.into(), bar_height as usize)
                            .collect::<Vec<Line>>(),
                    );
                    text.append(&mut vec!["".into()]);
                    text
                })
                .style(bar_style.clone())
            })
            .highlight_spacing(HighlightSpacing::Always);

        state.set_visible_width(area.width);
        if state.total_width() <= area.width {
            StatefulWidget::render(t, area, buf, &mut state.table_state());
        } else {
            let rect = Rect {
                x: 0,
                y: 0,
                width: state.total_width(),
                height: area.height,
            };

            let mut buffer = Buffer::empty(rect);
            ratatui::widgets::StatefulWidget::render(
                t,
                rect,
                &mut buffer,
                &mut state.table_state(),
            );
            let offset = std::cmp::min(state.total_width() - area.width, state.horizontal_scroll());

            for (x, y) in itertools::iproduct!(offset..(offset + area.width), 0..(rect.height)) {
                buf[(x - offset + area.x, y + area.y)].clone_from(&buffer[(x, y)]);
            }
        }
        ratatui::widgets::StatefulWidget::render(
            Scrollbar::default()
                .orientation(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            buf,
            &mut state.vertical_scroll(),
        );
    }
}
