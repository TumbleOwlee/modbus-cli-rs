use crate::util::str;
use crate::widgets::{InputField, InputFieldAction, InputStyle};

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::{Alignment, Color, Constraint, Layout, Margin, Style, Stylize};
use ratatui::style::palette::tailwind;
use ratatui::widgets::{Block, Clear, Widget, WidgetRef};

pub enum FieldType {
    Name,
    Register,
    ValueType,
    Value,
}

pub struct EditDialog {
    bg_color: Color,
    name: InputField,
    register: InputField,
    value_type: InputField,
    value: InputField,
}

impl EditDialog {
    pub fn new(focus_color: Color, bg_color: Color) -> Self {
        Self {
            bg_color,
            name: InputField::new()
                .title(str!("Name"))
                .bordered(true)
                .margins(Margin {
                    vertical: 0,
                    horizontal: 1,
                })
                .disabled(),
            register: InputField::new()
                .title(str!("Register"))
                .bordered(true)
                .margins(Margin {
                    vertical: 0,
                    horizontal: 1,
                })
                .disabled(),
            value_type: InputField::new()
                .title(str!("Type"))
                .bordered(true)
                .margins(Margin {
                    vertical: 0,
                    horizontal: 1,
                })
                .disabled(),
            value: InputField::new()
                .title(str!("Value"))
                .bordered(true)
                .margins(Margin {
                    vertical: 0,
                    horizontal: 1,
                })
                .style(InputStyle {
                    focused: Style::default().fg(focus_color).bg(bg_color),
                    cursor: Style::default().bg(focus_color).fg(bg_color),
                    ..InputStyle::default()
                }),
        }
    }

    pub fn set_highlight_color(&mut self, hi: Color) {
        self.value.set_style(InputStyle {
            focused: Style::default().fg(hi).bg(tailwind::SLATE.c950),
            cursor: Style::default().bg(hi).fg(tailwind::SLATE.c950),
            ..InputStyle::default()
        })
    }

    pub fn set(&mut self, ty: FieldType, input: Option<String>, placeholder: Option<String>) {
        let field = match ty {
            FieldType::Name => &mut self.name,
            FieldType::Register => &mut self.register,
            FieldType::ValueType => &mut self.value_type,
            FieldType::Value => &mut self.value,
        };
        if let Some(v) = input {
            field.set_input(v);
        } else {
            field.clear_input();
        }
        if let Some(v) = placeholder {
            field.set_placeholder(v);
        } else {
            field.clear_placeholder();
        }
    }

    pub fn focus(&mut self) {
        self.value.focus();
    }

    pub fn get_input(&self, ty: FieldType) -> Option<String> {
        match ty {
            FieldType::Name => &self.name,
            FieldType::Register => &self.register,
            FieldType::ValueType => &self.value_type,
            FieldType::Value => &self.value,
        }
        .get_input()
    }

    pub fn handle_events(
        &mut self,
        modifiers: KeyModifiers,
        code: KeyCode,
    ) -> Option<InputFieldAction> {
        self.value.handle_events(modifiers, code)
    }
}

impl WidgetRef for EditDialog {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        // Center dialog
        let width = std::cmp::min(area.width, 50);
        let height = std::cmp::min(area.height, 19);
        let layout = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(width),
            Constraint::Min(1),
        ])
        .split(area);
        let area = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(height),
            Constraint::Min(1),
        ])
        .split(layout[1])[1];

        // Clear area
        Clear.render(area, buf);

        // Render boxed dialog
        let block = Block::bordered()
            .title("Edit Register")
            .title_alignment(Alignment::Center)
            .bg(self.bg_color);
        let inner = block.inner(area);
        block.render(area, buf);

        let area = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .split(inner.inner(&Margin {
            vertical: 1,
            horizontal: 1,
        }));

        self.name.render_ref(area[0], buf);
        self.register.render_ref(area[2], buf);
        self.value_type.render_ref(area[4], buf);
        self.value.render_ref(area[6], buf);
    }
}
