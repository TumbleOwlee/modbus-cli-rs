use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Value {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(u128),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    I128(i128),
    F32(f32),
    F64(f64),
    Ascii(String),
}

impl Value {
    pub fn as_str(&self) -> String {
        match self {
            Self::U8(v) => format!("{v}"),
            Self::U16(v) => format!("{v}"),
            Self::U32(v) => format!("{v}"),
            Self::U64(v) => format!("{v}"),
            Self::U128(v) => format!("{v}"),
            Self::I8(v) => format!("{v}"),
            Self::I16(v) => format!("{v}"),
            Self::I32(v) => format!("{v}"),
            Self::I64(v) => format!("{v}"),
            Self::I128(v) => format!("{v}"),
            Self::F32(v) => format!("{v}"),
            Self::F64(v) => format!("{v}"),
            Self::Ascii(v) => v.to_string(),
        }
    }

    pub fn as_hex_str(&self) -> String {
        match self {
            Self::U8(v) => format!("0x{:01$X}", v, 2),
            Self::U16(v) => format!("0x{:01$X}", v, 4),
            Self::U32(v) => format!("0x{:01$X}", v, 8),
            Self::U64(v) => format!("0x{:01$X}", v, 16),
            Self::U128(v) => format!("0x{:01$X}", v, 32),
            Self::I8(v) => format!("0x{:01$X}", v, 2),
            Self::I16(v) => format!("0x{:01$X}", v, 4),
            Self::I32(v) => format!("0x{:01$X}", v, 8),
            Self::I64(v) => format!("0x{:01$X}", v, 16),
            Self::I128(v) => format!("0x{:01$X}", v, 32),
            Self::F32(v) => format!("0x{:01$X}", v.to_bits(), 8),
            Self::F64(v) => format!("0x{:01$X}", v.to_bits(), 16),
            Self::Ascii(v) => {
                let bytes = v.as_bytes();
                let mut str = "0x".to_string();
                for b in bytes.iter() {
                    str += &format!("{:01$X}", b, 2);
                }
                str
            }
        }
    }
}
