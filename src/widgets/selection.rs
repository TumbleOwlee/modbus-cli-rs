use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Style, Stylize},
    text::Text,
    widgets::{Block, Widget, WidgetRef},
};

use crate::{
    mem::register::Values,
    ui::{TableColors, PALETTES},
    widgets::{InputFieldAction, InputStyle},
};

pub struct Selection {
    selection: usize,
    items: Vec<Values>,

    title: Option<String>,
    bordered: bool,
    style: InputStyle,
    margins: Margin,

    colors: TableColors,
}

impl Selection {
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

    pub fn set_values(&mut self, values: Vec<Values>) {
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

    pub fn handle_events(
        &mut self,
        modifiers: KeyModifiers,
        code: KeyCode,
    ) -> Option<InputFieldAction> {
        match (modifiers, code) {
            (_, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                self.next_row();
                Some(InputFieldAction::InputTaken)
            }
            (_, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                self.previous_row();
                Some(InputFieldAction::InputTaken)
            }
            _ => None,
        }
    }

    pub fn get_selection(&self) -> Option<String> {
        let f = |o: &Values| -> String {
            match o {
                Values::Value(s) => s.to_string(),
                Values::ValueDef(d) => d.value.to_string(),
            }
        };
        if self.items.len() > self.selection {
            self.items.get(self.selection).map(f)
        } else {
            self.items.first().map(f)
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

impl WidgetRef for Selection {
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

        let items = self
            .items
            .iter()
            .map(|v| match v {
                Values::Value(s) => format!(" {}", s),
                Values::ValueDef(d) => format!(" {}", d.name),
            })
            .collect::<Vec<_>>();

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
            let mut bg = self.colors.row_color.bg.get(i % 2);
            let mut fg = self.colors.row_color.fg;
            let mut style = Style::new().fg(fg).bg(bg);
            if i == selection {
                style = self.style.focused.reversed();
            }
            let t = Text::from(v).style(style);
            t.render_ref(area[n], buf);
        }
    }
}
