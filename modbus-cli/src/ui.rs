use crate::dialog::edit::{Alignment, Endian, Format, ValueType};
use derive_builder::Builder;
use modbus_derive::{Focus, focusable};
use modbus_mem::{Kind, Memory, Range};
use modbus_net::{Address, SlaveId};
use modbus_reg::{
    Register,
    enums::Access,
    format::{
        Alignment as TextAlignment, Endian as RegisterEndian, Format as RegisterFormat, Resolution,
    },
};
use modbus_ui::{
    state::{
        InputFieldState, InputFieldStateBuilder, SelectionState, SelectionStateBuilder, TableState,
    },
    style::{InputFieldStyle, SelectionStyle, TextStyle},
    types::Border,
    widgets::{
        GetValue, Header, InputField, InputFieldBuilder, Selection, SelectionBuilder, Table,
        TableEntry, Text, TextBuilder, Validate, Widget,
    },
};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, HorizontalAlignment, Layout, Margin, Rect},
    style::palette::tailwind,
    widgets::{Block, StatefulWidget, Widget as UiWidget},
};
use std::{fmt::Debug, sync::Arc};

const COLUMN_COUNT: usize = 11;

#[derive(Clone, Debug)]
struct TableHeader {}

impl Header<COLUMN_COUNT> for TableHeader {
    fn header() -> [String; COLUMN_COUNT] {
        [
            "Name".into(),
            "Comment".into(),
            "Slave ID".into(),
            "Address".into(),
            "Access".into(),
            "Kind".into(),
            "Format".into(),
            "Length".into(),
            "Resolution".into(),
            "Value".into(),
            "Raw Value".into(),
        ]
    }

    fn widths() -> [u16; COLUMN_COUNT] {
        let mut widths = [0; COLUMN_COUNT];
        for (i, h) in Self::header().iter().enumerate() {
            widths[i] = h.len() as u16;
        }
        widths
    }
}

struct Definition {
    name: String,
    comment: String,
    register: Register,
    memory: Arc<Memory<SlaveId>>,
}

impl TableEntry<COLUMN_COUNT> for Definition {
    fn values(&self) -> [String; COLUMN_COUNT] {
        let resolution = if let Some(v) = self.register.format().resolution() {
            format!("{}", v)
        } else {
            "None".into()
        };
        let range = Range::new(
            *self.register.address() as usize,
            self.register.format().width(),
        );
        let bytes = self.memory.read(
            *self.register.slave_id(),
            self.register.kind().get_type(),
            &range,
        );

        [
            self.name.clone(),
            self.comment.clone(),
            format!("{}", self.register.slave_id()),
            format!("{}", self.register.address()),
            format!("{}", self.register.access()),
            format!("{}", self.register.kind()),
            format!("{}", self.register.format()),
            format!("{}", self.register.format().width()),
            resolution,
            format!("{}",),
            format!("{}", "1"),
        ]
    }
    fn height(&self) -> u16 {
        return 3;
    }
}

#[focusable]
#[derive(Builder, Focus)]
pub struct TableView {
    // Label for the register
    #[focus]
    pub table:
        Widget<TableState<Register, COLUMN_COUNT>, Table<Register, TableHeader, COLUMN_COUNT>>,
}
