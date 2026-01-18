use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Alignment {
    Left,
    Right,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Width(pub usize);

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Endian {
    Little,
    Big,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Format {
    Ascii((Alignment, Width)),
    U8(Endian),
    U16(Endian),
    U32(Endian),
    U64(Endian),
    U128(Endian),
    I8(Endian),
    I16(Endian),
    I32(Endian),
    I64(Endian),
    I128(Endian),
    F32(Endian),
    F64(Endian),
}

impl Format {
    // The width in Modbus registers (u16) of the format
    pub fn width(&self) -> usize {
        match self {
            Self::Ascii((_, w)) => w.0,
            Self::U8(_) | Self::U16(_) | Self::I8(_) | Self::I16(_) => 1,
            Self::U32(_) | Self::I32(_) | Self::F32(_) => 2,
            Self::U64(_) | Self::I64(_) | Self::F64(_) => 4,
            Self::U128(_) | Self::I128(_) => 8,
        }
    }

    /// The length in bytes (u8) of the format
    pub fn length(&self) -> usize {
        self.width() * 2
    }

    pub fn as_u8(&self) -> Option<&Endian> {
        if let Self::U8(v) = self {
            Some(v)
        } else {
            None
        }
    }
}
