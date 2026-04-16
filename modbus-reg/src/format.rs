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
pub struct Resolution(pub f64);

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Format {
    Ascii((Alignment, Width)),
    U8((Endian, Resolution)),
    U16((Endian, Resolution)),
    U32((Endian, Resolution)),
    U64((Endian, Resolution)),
    U128((Endian, Resolution)),
    I8((Endian, Resolution)),
    I16((Endian, Resolution)),
    I32((Endian, Resolution)),
    I64((Endian, Resolution)),
    I128((Endian, Resolution)),
    F32((Endian, Resolution)),
    F64((Endian, Resolution)),
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
}
