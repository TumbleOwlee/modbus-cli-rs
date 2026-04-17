use crate::dialog::edit::{Alignment, Endian, Format, ValueType};
use derive_builder::Builder;
use modbus_derive::{Focus, focusable};
use modbus_reg::format::{
    Alignment as TextAlignment, Endian as RegisterEndian, Format as RegisterFormat, Resolution,
};
use modbus_ui::{
    state::{InputFieldState, InputFieldStateBuilder, SelectionState, SelectionStateBuilder},
    style::{InputFieldStyle, SelectionStyle, TextStyle},
    types::Border,
    widgets::{
        GetValue, InputField, InputFieldBuilder, Selection, SelectionBuilder, Text, TextBuilder,
        Validate, Widget,
    },
};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, HorizontalAlignment, Layout, Margin, Rect},
    style::palette::tailwind,
    widgets::{Block, StatefulWidget, Widget as UiWidget},
};
use std::fmt::Debug;

#[focusable]
#[derive(Builder, Debug, Focus)]
pub struct EditInputDialog {
    // Label for the register
    #[focus]
    pub label: Widget<InputFieldState, InputField<String>>,
    // Description for the register
    #[focus]
    pub description: Widget<InputFieldState, InputField<String>>,
    // Address of the start register
    #[focus]
    pub address: Widget<InputFieldState, InputField<u16>>,
    // Type selection
    #[focus]
    pub value_type: Widget<SelectionState<ValueType>, Selection<ValueType>>,
    // Number format selection
    #[focus(when = {self.value_type.get_value() == ValueType::Number})]
    pub number_format: Widget<SelectionState<Format>, Selection<Format>>,
    // Number endianess selection
    #[focus(when = {self.value_type.get_value() == ValueType::Number})]
    pub number_endian: Widget<SelectionState<Endian>, Selection<Endian>>,
    // Number resolution input
    #[focus(when = {self.value_type.get_value() == ValueType::Number})]
    pub number_resolution: Widget<InputFieldState, InputField<f64>>,
    // Text alignment selection
    #[focus(when = {self.value_type.get_value() == ValueType::Text})]
    pub text_alignment: Widget<SelectionState<Alignment>, Selection<Alignment>>,
    // Text length input
    #[focus(when = {self.value_type.get_value() == ValueType::Text})]
    pub text_width: Widget<InputFieldState, InputField<usize>>,
    // Value input
    #[focus]
    pub value: Widget<InputFieldState, InputField<String>>,
    // Error display field
    pub error: Widget<String, Text>,
    // Success display field
    pub success: Widget<String, Text>,
    // Keybinds display field
    pub keybinds: Widget<String, Text>,
}

impl EditInputDialog {
    fn validate(&self) -> Result<(), String> {
        if let Err(e) = String::validate(self.label.state.input()) {
            return Err(format!("Label: {e}"));
        } else if let Err(e) = u16::validate(self.address.state.input()) {
            return Err(format!("Address: {e}"));
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
        match self.validate() {
            Ok(_) => {
                self.error.state.clear();
            }
            Err(e) => {
                self.error.state = e;
            }
        }

        let horizontal_layout: [Rect; 3] =
            Layout::horizontal([Constraint::Min(1), Constraint::Max(70), Constraint::Min(1)])
                .areas(area);

        let vertical_layout: [Rect; 3] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(25),
            Constraint::Min(1),
        ])
        .areas(horizontal_layout[1]);

        let block = Block::bordered()
            .style(ratatui::prelude::Style::default().fg(tailwind::INDIGO.c400))
            .title_alignment(HorizontalAlignment::Center)
            .title("Edit");
        let area = block.inner(vertical_layout[1]).inner(Margin::new(2, 1));
        block.render(vertical_layout[1], buf);

        let mut vertical_index = 0;
        let vertical_layout: [Rect; 8] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(6),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);

        StatefulWidget::render(
            &self.label.widget,
            vertical_layout[vertical_index],
            buf,
            &mut self.label.state,
        );
        vertical_index += 1;

        StatefulWidget::render(
            &self.description.widget,
            vertical_layout[vertical_index],
            buf,
            &mut self.description.state,
        );
        vertical_index += 1;

        let horizontal_layout: [Rect; 2] =
            Layout::horizontal([Constraint::Min(1), Constraint::Min(1)])
                .areas(vertical_layout[vertical_index]);
        vertical_index += 1;

        StatefulWidget::render(
            &self.address.widget,
            horizontal_layout[0],
            buf,
            &mut self.address.state,
        );

        StatefulWidget::render(
            &self.value_type.widget,
            horizontal_layout[1],
            buf,
            &mut self.value_type.state,
        );

        match self.value_type.state.values()[self.value_type.state.selection()] {
            ValueType::Number => {
                let horizontal_layout: [Rect; 3] = Layout::horizontal([
                    Constraint::Min(1),
                    Constraint::Min(1),
                    Constraint::Min(1),
                ])
                .areas(vertical_layout[vertical_index]);

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
                let horizontal_layout: [Rect; 2] =
                    Layout::horizontal([Constraint::Min(1), Constraint::Min(1)])
                        .areas(vertical_layout[vertical_index]);

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
        vertical_index += 1;

        StatefulWidget::render(
            &self.value.widget,
            vertical_layout[vertical_index],
            buf,
            &mut self.value.state,
        );
        vertical_index += 1;

        if !self.error.state.is_empty() {
            StatefulWidget::render(
                &self.error.widget,
                vertical_layout[vertical_index],
                buf,
                &mut self.error.state,
            );
        } else {
            StatefulWidget::render(
                &self.success.widget,
                vertical_layout[vertical_index],
                buf,
                &mut self.success.state,
            );
        }
        vertical_index += 1;
        vertical_index += 1;

        StatefulWidget::render(
            &self.keybinds.widget,
            vertical_layout[vertical_index],
            buf,
            &mut self.keybinds.state,
        );
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
        let error_style = TextStyle {
            general: ratatui::prelude::Style::default().fg(tailwind::RED.c500),
        };
        let success_style = TextStyle {
            general: ratatui::prelude::Style::default().fg(tailwind::GREEN.c500),
        };
        let text_style = TextStyle::default();

        EditInputDialogBuilder::default()
            .label(Widget {
                state: InputFieldStateBuilder::default()
                    .focused(true)
                    .disabled(false)
                    .placeholder(Some("Custom label...".to_string()))
                    .build()
                    .unwrap(),
                widget: InputFieldBuilder::default()
                    .border(Border::Full(Margin::new(1, 0)))
                    .title(Some("Label".into()))
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(input_style.clone())
                    .build()
                    .unwrap(),
            })
            .description(Widget {
                state: InputFieldStateBuilder::default()
                    .focused(false)
                    .disabled(false)
                    .placeholder(Some("Some description...".to_string()))
                    .build()
                    .unwrap(),
                widget: InputFieldBuilder::default()
                    .border(Border::Full(Margin::new(1, 0)))
                    .title(Some("Description".into()))
                    .multiline(true)
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(input_style.clone())
                    .build()
                    .unwrap(),
            })
            .address(Widget {
                state: InputFieldStateBuilder::default()
                    .focused(false)
                    .disabled(false)
                    .placeholder(Some("e.g. 100".to_string()))
                    .build()
                    .unwrap(),
                widget: InputFieldBuilder::default()
                    .border(Border::Full(Margin::new(1, 0)))
                    .title(Some("Address".into()))
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
                    .title(Some(("Type", HorizontalAlignment::Right).into()))
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
                    .title(Some(("Format", HorizontalAlignment::Left).into()))
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
                    .title(Some(("Endian", HorizontalAlignment::Center).into()))
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
                    .title(Some(("Reolution", HorizontalAlignment::Right).into()))
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
                    .title(Some("Alignment".into()))
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
                    .title(Some(("Width", HorizontalAlignment::Right).into()))
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
                    .title(Some("Value".into()))
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(input_style.clone())
                    .build()
                    .unwrap(),
            })
            .error(Widget {
                state: "".to_string(),
                widget: TextBuilder::default()
                    .title(Some("Error".into()))
                    .border(Border::Full(Margin::new(1, 0)))
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(error_style.clone())
                    .build()
                    .unwrap(),
            })
            .success(Widget {
                state: "Everything is fine.".to_string(),
                widget: TextBuilder::default()
                    .title(Some("Success".into()))
                    .border(Border::Full(Margin::new(1, 0)))
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .style(success_style.clone())
                    .build()
                    .unwrap(),
            })
            .keybinds(Widget {
                state: "<E>: Edit | <ENTER>: Confirm".to_string(),
                widget: TextBuilder::default()
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .horizontal_alignment(HorizontalAlignment::Center)
                    .style(text_style.clone())
                    .build()
                    .unwrap(),
            })
            .focus(EditInputDialogFocus::Label)
            .build()
            .unwrap()
    }
}
