use crate::mem::register::Values;
use crate::util::str;
use crate::widgets::selection::Selection;
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
    DataType,
    Value,
}

pub struct EditDialog {
    bg_color: Color,
    name: InputField,
    register: InputField,
    value_type: InputField,
    input: InputField,
    selection: Selection,
    values: Vec<Values>,
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
            input: InputField::new()
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
            values: vec![],
            selection: Selection::new()
                .title(str!("Selection"))
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
        self.input.set_style(InputStyle {
            focused: Style::default().fg(hi).bg(tailwind::SLATE.c950),
            cursor: Style::default().bg(hi).fg(tailwind::SLATE.c950),
            ..InputStyle::default()
        })
    }

    pub fn set(&mut self, ty: FieldType, input: Option<String>, placeholder: Option<String>) {
        let field = match ty {
            FieldType::Name => &mut self.name,
            FieldType::Register => &mut self.register,
            FieldType::DataType => &mut self.value_type,
            FieldType::Value => &mut self.input,
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

    pub fn limit_to(&mut self, values: Vec<Values>) {
        self.selection.set_values(values.clone());
        self.values = values;
    }

    pub fn focus(&mut self) {
        self.input.focus();
    }

    pub fn get_input(&self, ty: FieldType) -> Option<String> {
        match ty {
            FieldType::Name => self.name.get_input(),
            FieldType::Register => self.register.get_input(),
            FieldType::DataType => self.value_type.get_input(),
            FieldType::Value => {
                if self.values.is_empty() {
                    self.input.get_input()
                } else {
                    self.selection.get_selection()
                }
            }
        }
    }

    pub fn handle_events(
        &mut self,
        modifiers: KeyModifiers,
        code: KeyCode,
    ) -> Option<InputFieldAction> {
        if self.values.is_empty() {
            self.input.handle_events(modifiers, code)
        } else {
            self.selection.handle_events(modifiers, code)
        }
    }
}

impl WidgetRef for EditDialog {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        let min_height = if self.values.is_empty() { 19 } else { 27 };
        // Center dialog
        let width = std::cmp::min(area.width, 50);
        let height = std::cmp::min(area.height, min_height);
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

        let input_length = if self.values.is_empty() {
            Constraint::Length(3)
        } else {
            Constraint::Length(11)
        };

        let area = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            input_length,
        ])
        .split(inner.inner(Margin {
            vertical: 1,
            horizontal: 1,
        }));

        self.name.render_ref(area[0], buf);
        self.register.render_ref(area[2], buf);
        self.value_type.render_ref(area[4], buf);
        if self.values.is_empty() {
            self.input.render_ref(area[6], buf);
        } else {
            self.selection.render_ref(area[6], buf);
        }
    }
}
