mod input;
mod selection;

pub use input::*;
pub use selection::*;

use modbus_reg::format::{
    Alignment as TextAlignment, Endian as RegisterEndian, Format as RegisterFormat,
};
use modbus_ui::traits::ToLabel;
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
