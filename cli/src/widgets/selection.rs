use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Style, Stylize},
    text::Text,
    widgets::{Block, Widget, WidgetRef},
};

use super::Action;
use super::Style as InputStyle;
use super::color::{PALETTES, TableColors};

pub trait ToLabel {
    fn to_label(&self) -> String;
}

pub struct Selection<ValueType>
where
    ValueType: ToLabel,
{
    selection: usize,
    items: Vec<ValueType>,
    title: Option<String>,
    bordered: bool,
    style: InputStyle,
    margins: Margin,
    colors: TableColors,
}

impl<ValueType> Selection<ValueType>
where
    ValueType: ToLabel,
{
    pub fn new() -> Self {
        Self {
            selection: 0,
            items: vec![],
            bordered: false,
            style: InputStyle::default(),
            title: None,
            margins: Margin {
                vertical: 0,
                horizontal: 0,
            },
            colors: TableColors::new(&PALETTES[0]),
        }
    }

    pub fn set_values(&mut self, values: Vec<ValueType>) {
        self.selection = 0;
        self.items = values;
    }

    pub fn next_row(&mut self) {
        self.selection = if self.selection >= self.items.len() - 1 {
            0
        } else {
            self.selection + 1
        };
    }

    pub fn previous_row(&mut self) {
        self.selection = if self.selection == 0 {
            self.items.len() - 1
        } else {
            self.selection - 1
        };
    }

    pub fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> Option<Action> {
        match (modifiers, code) {
            (_, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                self.next_row();
                Some(Action::InputTaken)
            }
            (_, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                self.previous_row();
                Some(Action::InputTaken)
            }
            _ => None,
        }
    }

    pub fn get_selection(&self) -> Option<String> {
        if self.items.len() > self.selection {
            self.items.get(self.selection).map(ToLabel::to_label)
        } else {
            self.items.first().map(ToLabel::to_label)
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

impl<ValueType> WidgetRef for Selection<ValueType>
where
    ValueType: ToLabel,
{
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
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

        let items = self.items.iter().map(ToLabel::to_label).collect::<Vec<_>>();

        let constraints = vec![Constraint::Length(1); count];
        let area = Layout::vertical(constraints).split(area);

        let selection = self.selection;
        let start = if selection >= split && items.len() - split >= selection {
            selection - split
        } else if selection < split || items.len() < count {
            0
        } else {
            items.len() - count
        };

        for (n, (i, v)) in items
            .into_iter()
            .enumerate()
            .filter(|(i, _)| *i >= start && *i < (start + count))
            .enumerate()
        {
            let bg = self.colors.row_color.bg.get(i % 2);
            let fg = self.colors.row_color.fg;
            let mut style = Style::new().fg(fg).bg(bg);
            if i == selection {
                style = self.style.focused.reversed();
            }
            let t = Text::from(v).style(style);
            t.render(area[n], buf);
        }
    }
}
