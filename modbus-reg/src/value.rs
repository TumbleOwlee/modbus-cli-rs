use serde::{Deserialize, Serialize};

use crate::format::Resolution;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Value {
    U8((u8, Resolution)),
    U16((u16, Resolution)),
    U32((u32, Resolution)),
    U64((u64, Resolution)),
    U128((u128, Resolution)),
    I8((i8, Resolution)),
    I16((i16, Resolution)),
    I32((i32, Resolution)),
    I64((i64, Resolution)),
    I128((i128, Resolution)),
    F32((f32, Resolution)),
    F64((f64, Resolution)),
    Ascii(String),
}

impl Value {
    pub fn as_str(&self) -> String {
        match self {
            Self::U8((v, r)) => {
                let v = *v as f64 * r.0;
                format!("{v}")
            }
            Self::U16((v, r)) => {
                let v = *v as f64 * r.0;
                format!("{v}")
            }
            Self::U32((v, r)) => {
                let v = *v as f64 * r.0;
                format!("{v}")
            }
            Self::U64((v, r)) => {
                let v = *v as f64 * r.0;
                format!("{v}")
            }
            Self::U128((v, r)) => {
                let v = *v as f64 * r.0;
                format!("{v}")
            }
            Self::I8((v, r)) => {
                let v = *v as f64 * r.0;
                format!("{v}")
            }
            Self::I16((v, r)) => {
                let v = *v as f64 * r.0;
                format!("{v}")
            }
            Self::I32((v, r)) => {
                let v = *v as f64 * r.0;
                format!("{v}")
            }
            Self::I64((v, r)) => {
                let v = *v as f64 * r.0;
                format!("{v}")
            }
            Self::I128((v, r)) => {
                let v = *v as f64 * r.0;
                format!("{v}")
            }
            Self::F32((v, r)) => {
                let v = *v as f64 * r.0;
                format!("{v}")
            }
            Self::F64((v, r)) => {
                let v = *v as f64 * r.0;
                format!("{v}")
            }
            Self::Ascii(v) => v.to_string(),
        }
    }

    pub fn as_hex_str(&self) -> String {
        match self {
            Self::U8((v, _)) => format!("0x{:01$X}", v, 2),
            Self::U16((v, _)) => format!("0x{:01$X}", v, 4),
            Self::U32((v, _)) => format!("0x{:01$X}", v, 8),
            Self::U64((v, _)) => format!("0x{:01$X}", v, 16),
            Self::U128((v, _)) => format!("0x{:01$X}", v, 32),
            Self::I8((v, _)) => format!("0x{:01$X}", v, 2),
            Self::I16((v, _)) => format!("0x{:01$X}", v, 4),
            Self::I32((v, _)) => format!("0x{:01$X}", v, 8),
            Self::I64((v, _)) => format!("0x{:01$X}", v, 16),
            Self::I128((v, _)) => format!("0x{:01$X}", v, 32),
            Self::F32((v, _)) => format!("0x{:01$X}", v.to_bits(), 8),
            Self::F64((v, _)) => format!("0x{:01$X}", v.to_bits(), 16),
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
