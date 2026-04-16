use derive_builder::Builder;
use modbus_derive::{Focus, focusable};
use modbus_reg::format::{
    Alignment as TextAlignment, Endian as RegisterEndian, Format as RegisterFormat, Resolution,
};
use modbus_ui::{
    state::{InputFieldState, InputFieldStateBuilder, SelectionState, SelectionStateBuilder},
    style::{InputFieldStyle, SelectionStyle},
    traits::{SetFocus, ToLabel},
    types::Border,
    widgets::{InputField, InputFieldBuilder, Selection, SelectionBuilder, Validate, Widget},
};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, HorizontalAlignment, Layout, Margin, Rect},
    style::{Style, palette::tailwind},
    widgets::{Block, StatefulWidget, Widget as UiWidget},
};
use std::fmt::Debug;

#[derive(Debug, Clone)]
pub struct Format(RegisterFormat);

impl ToLabel for Format {
    fn to_label(&self) -> String {
        match self.0 {
            RegisterFormat::U8(_) => "U8",
            RegisterFormat::U16(_) => "U16",
            RegisterFormat::U32(_) => "U32",
            RegisterFormat::U64(_) => "U64",
            RegisterFormat::U128(_) => "U128",
            RegisterFormat::I8(_) => "I8",
            RegisterFormat::I16(_) => "I16",
            RegisterFormat::I32(_) => "I32",
            RegisterFormat::I64(_) => "I64",
            RegisterFormat::I128(_) => "I128",
            RegisterFormat::F32(_) => "F32",
            RegisterFormat::F64(_) => "F64",
            RegisterFormat::Ascii(_) => "ASCII",
        }
        .to_string()
    }
}

#[derive(Debug, Clone)]
pub struct Endian(RegisterEndian);

impl ToLabel for Endian {
    fn to_label(&self) -> String {
        match self.0 {
            RegisterEndian::Big => "Big",
            RegisterEndian::Little => "Little",
        }
        .to_string()
    }
}

#[derive(Debug, Clone)]
pub enum ValueType {
    Number,
    Text,
}

impl ToLabel for ValueType {
    fn to_label(&self) -> String {
        match self {
            ValueType::Number => "Number",
            ValueType::Text => "Text",
        }
        .to_string()
    }
}

#[derive(Debug, Clone)]
pub struct Alignment(TextAlignment);

impl ToLabel for Alignment {
    fn to_label(&self) -> String {
        match self.0 {
            TextAlignment::Right => "Right",
            TextAlignment::Left => "Left",
        }
        .to_string()
    }
}

#[focusable]
#[derive(Builder, Debug, Focus)]
pub struct EditDialog {
    // Label for the register
    #[focus]
    pub label: Widget<InputFieldState, InputField<String>>,
    // Type selection
    #[focus]
    pub value_type: Widget<SelectionState<ValueType>, Selection<ValueType>>,
    // Number format selection
    #[focus]
    pub number_format: Widget<SelectionState<Format>, Selection<Format>>,
    // Number endianess selection
    #[focus]
    pub number_endian: Widget<SelectionState<Endian>, Selection<Endian>>,
    // Number resolution input
    #[focus]
    pub number_resolution: Widget<InputFieldState, InputField<f64>>,
    // Text alignment selection
    #[focus]
    pub text_alignment: Widget<SelectionState<Alignment>, Selection<Alignment>>,
    // Text length input
    #[focus]
    pub text_width: Widget<InputFieldState, InputField<usize>>,
    // Value input
    #[focus]
    pub value: Widget<InputFieldState, InputField<String>>,
    // Error display field
    pub error: Widget<InputFieldState, InputField<String>>,
}

impl EditDialog {
    fn result(&self) -> Result<(), String> {
        if let Err(e) = String::validate(self.label.state.input()) {
            return Err(format!("Label: {e}"));
        }

        match self.value_type.state.values()[self.value_type.state.selection()] {
            ValueType::Number => {
                if let Err(e) = f64::validate(self.number_resolution.state.input()) {
                    return Err(format!("Resolution: {e}"));
                }
            }
            ValueType::Text => {
                if let Err(e) = usize::validate(self.text_width.state.input()) {
                    return Err(format!("Width: {e}"));
                }
            }
        }
        Ok(())
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        // Show error
        match self.result() {
            Ok(_) => {
                self.error.state.set_input(String::new());
            }
            Err(e) => {
                self.error.state.set_input(e);
            }
        }

        let horizontal_layout: [Rect; 3] =
            Layout::horizontal([Constraint::Min(1), Constraint::Max(70), Constraint::Min(1)])
                .areas(area);

        let vertical_layout: [Rect; 3] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(
                9 + 2 + 2 + {
                    if self.error.state.input().is_empty() {
                        0
                    } else {
                        1
                    }
                },
            ),
            Constraint::Min(1),
        ])
        .areas(horizontal_layout[1]);

        let block = Block::bordered()
            .style(ratatui::prelude::Style::default().fg(tailwind::INDIGO.c400))
            .title_alignment(HorizontalAlignment::Center)
            .title("Edit");
        let area = block.inner(vertical_layout[1]).inner(Margin::new(2, 1));
        block.render(vertical_layout[1], buf);

        let vertical_layout: [Rect; 4] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length({
                if self.error.state.input().is_empty() {
                    0
                } else {
                    1
                }
            }),
        ])
        .areas(area);

        let horizontal_layout: [Rect; 2] =
            Layout::horizontal([Constraint::Min(1), Constraint::Min(1)]).areas(vertical_layout[0]);

        StatefulWidget::render(
            &self.label.widget,
            horizontal_layout[0],
            buf,
            &mut self.label.state,
        );

        StatefulWidget::render(
            &self.value_type.widget,
            horizontal_layout[1],
            buf,
            &mut self.value_type.state,
        );

        match self.value_type.state.values()[self.value_type.state.selection()] {
            ValueType::Number => {
                if self.text_alignment.is_focused() {
                    self.focus_next();
                    self.focus_next();
                } else if self.text_width.is_focused() {
                    self.focus_previous();
                    self.focus_previous();
                }

                let horizontal_layout: [Rect; 3] = Layout::horizontal([
                    Constraint::Min(1),
                    Constraint::Min(1),
                    Constraint::Min(1),
                ])
                .areas(vertical_layout[1]);

                StatefulWidget::render(
                    &self.number_format.widget,
                    horizontal_layout[0],
                    buf,
                    &mut self.number_format.state,
                );

                StatefulWidget::render(
                    &self.number_endian.widget,
                    horizontal_layout[1],
                    buf,
                    &mut self.number_endian.state,
                );

                StatefulWidget::render(
                    &self.number_resolution.widget,
                    horizontal_layout[2],
                    buf,
                    &mut self.number_resolution.state,
                );
            }
            ValueType::Text => {
                if self.number_format.is_focused() {
                    self.focus_next();
                    self.focus_next();
                    self.focus_next();
                } else if self.number_resolution.is_focused() {
                    self.focus_previous();
                    self.focus_previous();
                    self.focus_previous();
                }

                let horizontal_layout: [Rect; 2] =
                    Layout::horizontal([Constraint::Min(1), Constraint::Min(1)])
                        .areas(vertical_layout[1]);

                StatefulWidget::render(
                    &self.text_alignment.widget,
                    horizontal_layout[0],
                    buf,
                    &mut self.text_alignment.state,
                );

                StatefulWidget::render(
                    &self.text_width.widget,
                    horizontal_layout[1],
                    buf,
                    &mut self.text_width.state,
                );
            }
        }

        StatefulWidget::render(
            &self.value.widget,
            vertical_layout[2],
            buf,
            &mut self.value.state,
        );

        if !self.error.state.input().is_empty() {
            StatefulWidget::render(
                &self.error.widget,
                vertical_layout[3],
                buf,
                &mut self.error.state,
            );
        }
    }

    pub fn new() -> Self {
        let selection_style = SelectionStyle {
            focused: ratatui::prelude::Style::default()
                .bg(tailwind::INDIGO.c400)
                .fg(tailwind::BLACK),
            border: ratatui::prelude::Style::default().fg(tailwind::INDIGO.c400),
            ..SelectionStyle::default()
        };
        let input_style = InputFieldStyle {
            focused: ratatui::prelude::Style::default().fg(tailwind::INDIGO.c400),
            cursor: ratatui::prelude::Style::default()
                .bg(tailwind::INDIGO.c400)
                .fg(tailwind::WHITE),
            ..InputFieldStyle::default()
        };
        let error_style = InputFieldStyle {
            focused: ratatui::prelude::Style::default().fg(tailwind::RED.c500),
            cursor: ratatui::prelude::Style::default(),
            general: ratatui::prelude::Style::default().fg(tailwind::RED.c500),
            ..InputFieldStyle::default()
        };

        EditDialogBuilder::default()
            .label(Widget {
                state: InputFieldStateBuilder::default()
                    .focused(true)
                    .disabled(false)
                    .placeholder(Some("Custom label...".to_string()))
                    .build()
                    .unwrap(),
                widget: InputFieldBuilder::default()
                    .border(Border::Full(Margin::new(1, 0)))
                    .title(Some("Label".to_string()))
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(input_style.clone())
                    .build()
                    .unwrap(),
            })
            .value_type(Widget {
                state: SelectionStateBuilder::default()
                    .focused(false)
                    .values(vec![ValueType::Number, ValueType::Text])
                    .build()
                    .unwrap(),
                widget: SelectionBuilder::default()
                    .border(Border::Full(Margin::new(1, 0)))
                    .title(Some("Type".to_string()))
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(selection_style.clone())
                    .build()
                    .unwrap(),
            })
            .number_format(Widget {
                state: SelectionStateBuilder::default()
                    .focused(false)
                    .values(vec![
                        Format(RegisterFormat::U8((RegisterEndian::Big, Resolution(1.0)))),
                        Format(RegisterFormat::U16((RegisterEndian::Big, Resolution(1.0)))),
                        Format(RegisterFormat::U32((RegisterEndian::Big, Resolution(1.0)))),
                        Format(RegisterFormat::U64((RegisterEndian::Big, Resolution(1.0)))),
                        Format(RegisterFormat::U128((RegisterEndian::Big, Resolution(1.0)))),
                        Format(RegisterFormat::I8((RegisterEndian::Big, Resolution(1.0)))),
                        Format(RegisterFormat::I16((RegisterEndian::Big, Resolution(1.0)))),
                        Format(RegisterFormat::I32((RegisterEndian::Big, Resolution(1.0)))),
                        Format(RegisterFormat::I64((RegisterEndian::Big, Resolution(1.0)))),
                        Format(RegisterFormat::I128((RegisterEndian::Big, Resolution(1.0)))),
                        Format(RegisterFormat::F32((RegisterEndian::Big, Resolution(1.0)))),
                        Format(RegisterFormat::F64((RegisterEndian::Big, Resolution(1.0)))),
                    ])
                    .build()
                    .unwrap(),
                widget: SelectionBuilder::default()
                    .border(Border::Full(Margin::new(1, 0)))
                    .title(Some("Format".to_string()))
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(selection_style.clone())
                    .build()
                    .unwrap(),
            })
            .number_endian(Widget {
                state: SelectionStateBuilder::default()
                    .focused(false)
                    .values(vec![
                        Endian(RegisterEndian::Big),
                        Endian(RegisterEndian::Little),
                    ])
                    .build()
                    .unwrap(),
                widget: SelectionBuilder::default()
                    .border(Border::Full(Margin::new(1, 0)))
                    .title(Some("Endian".to_string()))
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(selection_style.clone())
                    .build()
                    .unwrap(),
            })
            .number_resolution(Widget {
                state: InputFieldStateBuilder::default()
                    .focused(false)
                    .input("1.0".to_string())
                    .cursor(3)
                    .disabled(false)
                    .build()
                    .unwrap(),
                widget: InputFieldBuilder::default()
                    .border(Border::Full(Margin::new(1, 0)))
                    .title(Some("Reolution".to_string()))
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(input_style.clone())
                    .build()
                    .unwrap(),
            })
            .text_alignment(Widget {
                state: SelectionStateBuilder::default()
                    .focused(false)
                    .values(vec![
                        Alignment(TextAlignment::Right),
                        Alignment(TextAlignment::Left),
                    ])
                    .build()
                    .unwrap(),
                widget: SelectionBuilder::default()
                    .border(Border::Full(Margin::new(1, 0)))
                    .title(Some("Alignment".to_string()))
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(selection_style.clone())
                    .build()
                    .unwrap(),
            })
            .text_width(Widget {
                state: InputFieldStateBuilder::default()
                    .focused(false)
                    .disabled(false)
                    .placeholder(Some("1".to_string()))
                    .build()
                    .unwrap(),
                widget: InputFieldBuilder::default()
                    .border(Border::Full(Margin::new(1, 0)))
                    .title(Some("Width".to_string()))
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(input_style.clone())
                    .build()
                    .unwrap(),
            })
            .value(Widget {
                state: InputFieldStateBuilder::default()
                    .focused(false)
                    .disabled(false)
                    .placeholder(Some("Enter value...".to_string()))
                    .build()
                    .unwrap(),
                widget: InputFieldBuilder::default()
                    .border(Border::Full(Margin::new(1, 0)))
                    .title(Some("Value".to_string()))
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(input_style.clone())
                    .build()
                    .unwrap(),
            })
            .error(Widget {
                state: InputFieldStateBuilder::default()
                    .focused(true)
                    .disabled(false)
                    .build()
                    .unwrap(),
                widget: InputFieldBuilder::default()
                    .title(None)
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(error_style.clone())
                    .build()
                    .unwrap(),
            })
            .focus(0)
            .build()
            .unwrap()
    }
}
