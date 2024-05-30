use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ValueType {
    PackedString,
    LooseString,
    U8,
    U16,
    U32,
    U64,
    U128,
    I8,
    I16,
    I32,
    I64,
    I128,
    U8le,
    U16le,
    U32le,
    U64le,
    U128le,
    I8le,
    I16le,
    I32le,
    I64le,
    I128le,
}

impl ValueType {
    pub fn as_str(&self, bytes: &[u16]) -> anyhow::Result<String> {
        match self {
            ValueType::PackedString => Ok(String::from_utf8(
                bytes
                    .iter()
                    .flat_map(|v| vec![(*v >> 8) as u8, (*v & 0xFF) as u8])
                    .collect(),
            )
            .unwrap()),
            ValueType::LooseString => {
                Ok(String::from_utf8(bytes.iter().map(|v| (*v & 0xFF) as u8).collect()).unwrap())
            }
            ValueType::U8 => {
                let val: u8 = (*(bytes.first().unwrap()) & 0xFF) as u8;
                Ok(format!("{:#04X} ({})", val, val))
            }
            ValueType::U16 => {
                let val: u16 = *bytes.first().unwrap();
                Ok(format!("{:#06X} ({})", val, val))
            }
            ValueType::U32 => {
                let val: u32 =
                    (((*bytes.first().unwrap()) as u32) << 16) + ((*bytes.get(1).unwrap()) as u32);
                Ok(format!("0x{:02$X} ({})", val, val, 8))
            }
            ValueType::U64 => {
                let val: u64 = (((*bytes.first().unwrap()) as u64) << 48)
                    + (((*bytes.get(1).unwrap()) as u64) << 32)
                    + (((*bytes.get(2).unwrap()) as u64) << 16)
                    + (*bytes.get(3).unwrap()) as u64;
                Ok(format!("0x{:02$X} ({})", val, val, 16))
            }
            ValueType::U128 => {
                let val: u128 = (((*bytes.first().unwrap()) as u128) << 112)
                    + (((*bytes.get(1).unwrap()) as u128) << 96)
                    + (((*bytes.get(2).unwrap()) as u128) << 80)
                    + (((*bytes.get(3).unwrap()) as u128) << 64)
                    + (((*bytes.get(4).unwrap()) as u128) << 48)
                    + (((*bytes.get(5).unwrap()) as u128) << 32)
                    + (((*bytes.get(6).unwrap()) as u128) << 16)
                    + (*bytes.get(7).unwrap()) as u128;
                Ok(format!("0x{:02$X} ({})", val, val, 32))
            }
            ValueType::I8 => {
                let val: i8 = (*(bytes.first().unwrap()) & 0xFF) as i8;
                Ok(format!("{:#04X} ({})", val, val))
            }
            ValueType::I16 => {
                let val: i16 = *bytes.first().unwrap() as i16;
                Ok(format!("{:#06X} ({})", val, val))
            }
            ValueType::I32 => {
                let val: i32 =
                    (((*bytes.first().unwrap()) as i32) << 16) + ((*bytes.get(1).unwrap()) as i32);
                Ok(format!("0x{:02$X} ({})", val, val, 8))
            }
            ValueType::I64 => {
                let val: i64 = (((*bytes.first().unwrap()) as i64) << 48)
                    + (((*bytes.get(1).unwrap()) as i64) << 32)
                    + (((*bytes.get(2).unwrap()) as i64) << 16)
                    + (*bytes.get(3).unwrap()) as i64;
                Ok(format!("0x{:02$X} ({})", val, val, 16))
            }
            ValueType::I128 => {
                let val: i128 = (((*bytes.first().unwrap()) as i128) << 112)
                    + (((*bytes.get(1).unwrap()) as i128) << 96)
                    + (((*bytes.get(2).unwrap()) as i128) << 80)
                    + (((*bytes.get(3).unwrap()) as i128) << 64)
                    + (((*bytes.get(4).unwrap()) as i128) << 48)
                    + (((*bytes.get(5).unwrap()) as i128) << 32)
                    + (((*bytes.get(6).unwrap()) as i128) << 16)
                    + (*bytes.get(7).unwrap()) as i128;
                Ok(format!("0x{:02$X} ({})", val, val, 32))
            }
            ValueType::U8le => {
                let val: u8 = (*(bytes.first().unwrap()) & 0xFF) as u8;
                Ok(format!("{:#04X} ({})", val, val))
            }
            ValueType::U16le => {
                let val: u16 = *bytes.first().unwrap();
                Ok(format!("{:#06X} ({})", val, val))
            }
            ValueType::U32le => {
                let val: u32 =
                    (((*bytes.get(1).unwrap()) as u32) << 16) + ((*bytes.first().unwrap()) as u32);
                Ok(format!("0x{:02$X} ({})", val, val, 8))
            }
            ValueType::U64le => {
                let val: u64 = (((*bytes.get(3).unwrap()) as u64) << 48)
                    + (((*bytes.get(2).unwrap()) as u64) << 32)
                    + (((*bytes.get(1).unwrap()) as u64) << 16)
                    + (*bytes.first().unwrap()) as u64;
                Ok(format!("0x{:02$X} ({})", val, val, 16))
            }
            ValueType::U128le => {
                let val: u128 = (((*bytes.get(7).unwrap()) as u128) << 112)
                    + (((*bytes.get(6).unwrap()) as u128) << 96)
                    + (((*bytes.get(5).unwrap()) as u128) << 80)
                    + (((*bytes.get(4).unwrap()) as u128) << 64)
                    + (((*bytes.get(3).unwrap()) as u128) << 48)
                    + (((*bytes.get(2).unwrap()) as u128) << 32)
                    + (((*bytes.get(1).unwrap()) as u128) << 16)
                    + (*bytes.first().unwrap()) as u128;
                Ok(format!("0x{:02$X} ({})", val, val, 32))
            }
            ValueType::I8le => {
                let val: i8 = (*(bytes.first().unwrap()) & 0xFF) as i8;
                Ok(format!("{:#04X} ({})", val, val))
            }
            ValueType::I16le => {
                let val: i16 = *bytes.first().unwrap() as i16;
                Ok(format!("{:#06X} ({})", val, val))
            }
            ValueType::I32le => {
                let val: i32 =
                    (((*bytes.first().unwrap()) as i32) << 16) + ((*bytes.get(1).unwrap()) as i32);
                Ok(format!("0x{:02$X} ({})", val, val, 8))
            }
            ValueType::I64le => {
                let val: i64 = (((*bytes.get(3).unwrap()) as i64) << 48)
                    + (((*bytes.get(2).unwrap()) as i64) << 32)
                    + (((*bytes.get(1).unwrap()) as i64) << 16)
                    + (*bytes.first().unwrap()) as i64;
                Ok(format!("0x{:02$X} ({})", val, val, 16))
            }
            ValueType::I128le => {
                let val: i128 = (((*bytes.get(7).unwrap()) as i128) << 112)
                    + (((*bytes.get(6).unwrap()) as i128) << 96)
                    + (((*bytes.get(5).unwrap()) as i128) << 80)
                    + (((*bytes.get(4).unwrap()) as i128) << 64)
                    + (((*bytes.get(3).unwrap()) as i128) << 48)
                    + (((*bytes.get(2).unwrap()) as i128) << 32)
                    + (((*bytes.get(1).unwrap()) as i128) << 16)
                    + (*bytes.first().unwrap()) as i128;
                Ok(format!("0x{:02$X} ({})", val, val, 32))
            }
        }
    }

    pub fn from_str(&self, s: &str) -> anyhow::Result<Vec<u16>> {
        match self {
            ValueType::PackedString => {
                let mut v = Vec::with_capacity(s.len() / 2 + 1);
                let bytes = s.as_bytes();
                let mut i = 0usize;
                loop {
                    let mut value: u16 = 0;
                    if i < s.len() {
                        value += (bytes[i] as u16) << 8;
                    }
                    if (i + 1) < s.len() {
                        value += bytes[i + 1] as u16;
                    }
                    v.push(value);
                    i += 2;

                    if i >= s.len() {
                        break;
                    }
                }
                Ok(v)
            }
            ValueType::LooseString => Ok(s.chars().map(|c| c as u16).collect()),
            ValueType::U8 | ValueType::U8le => {
                let val: u8 = if let Some(s) = s.strip_prefix("0x") {
                    u8::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![val as u16])
            }
            ValueType::U16 | ValueType::U16le => {
                let val: u16 = if let Some(s) = s.strip_prefix("0x") {
                    u16::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![val])
            }
            ValueType::U32 => {
                let val: u32 = if let Some(s) = s.strip_prefix("0x") {
                    u32::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![(val >> 16) as u16, (val & 0xFFFF) as u16])
            }
            ValueType::U64 => {
                let val: u64 = if let Some(s) = s.strip_prefix("0x") {
                    u64::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![
                    ((val >> 48) & 0xFFFF) as u16,
                    ((val >> 32) & 0xFFFF) as u16,
                    ((val >> 16) & 0xFFFF) as u16,
                    (val & 0xFFFF) as u16,
                ])
            }
            ValueType::U128 => {
                let val: u128 = if let Some(s) = s.strip_prefix("0x") {
                    u128::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![
                    ((val >> 112) & 0xFFFF) as u16,
                    ((val >> 96) & 0xFFFF) as u16,
                    ((val >> 80) & 0xFFFF) as u16,
                    ((val >> 64) & 0xFFFF) as u16,
                    ((val >> 48) & 0xFFFF) as u16,
                    ((val >> 32) & 0xFFFF) as u16,
                    ((val >> 16) & 0xFFFF) as u16,
                    (val & 0xFFFF) as u16,
                ])
            }
            ValueType::I8 | ValueType::I8le => {
                let val: i8 = if let Some(s) = s.strip_prefix("-0x") {
                    -i8::from_str_radix(s, 16)?
                } else if let Some(s) = s.strip_prefix("0x") {
                    let v = u8::from_str_radix(s, 16)?;
                    if v > (i8::MAX as u8) {
                        v as i8
                    } else {
                        return Err(anyhow::anyhow!("Value too large for i8."));
                    }
                } else {
                    s.parse()?
                };
                Ok(vec![val as u16])
            }
            ValueType::I16 | ValueType::I16le => {
                let val: i16 = if let Some(s) = s.strip_prefix("-0x") {
                    -i16::from_str_radix(s, 16)?
                } else if let Some(s) = s.strip_prefix("0x") {
                    let v = u16::from_str_radix(s, 16)?;
                    if v > (i16::MAX as u16) {
                        v as i16
                    } else {
                        return Err(anyhow::anyhow!("Value too large for i16."));
                    }
                } else {
                    s.parse()?
                };
                Ok(vec![val as u16])
            }
            ValueType::I32 => {
                let val: i32 = if let Some(s) = s.strip_prefix("-0x") {
                    -i32::from_str_radix(s, 16)?
                } else if let Some(s) = s.strip_prefix("0x") {
                    let v = u32::from_str_radix(s, 16)?;
                    if v > (i32::MAX as u32) {
                        v as i32
                    } else {
                        return Err(anyhow::anyhow!("Value too large for i32."));
                    }
                } else {
                    s.parse()?
                };
                Ok(vec![(val >> 16) as u16, (val & 0xFFFF) as u16])
            }
            ValueType::I64 => {
                let val: i64 = if let Some(s) = s.strip_prefix("0x") {
                    -i64::from_str_radix(s, 16)?
                } else if let Some(s) = s.strip_prefix("0x") {
                    let v = u64::from_str_radix(s, 16)?;
                    if v > (i64::MAX as u64) {
                        v as i64
                    } else {
                        return Err(anyhow::anyhow!("Value too large for i64."));
                    }
                } else {
                    s.parse()?
                };
                Ok(vec![
                    ((val >> 48) & 0xFFFF) as u16,
                    ((val >> 32) & 0xFFFF) as u16,
                    ((val >> 16) & 0xFFFF) as u16,
                    (val & 0xFFFF) as u16,
                ])
            }
            ValueType::I128 => {
                let val: i128 = if let Some(s) = s.strip_prefix("0x") {
                    -i128::from_str_radix(s, 16)?
                } else if let Some(s) = s.strip_prefix("0x") {
                    let v = u128::from_str_radix(s, 16)?;
                    if v > (i128::MAX as u128) {
                        v as i128
                    } else {
                        return Err(anyhow::anyhow!("Value too large for i128."));
                    }
                } else {
                    s.parse()?
                };
                Ok(vec![
                    ((val >> 112) & 0xFFFF) as u16,
                    ((val >> 96) & 0xFFFF) as u16,
                    ((val >> 80) & 0xFFFF) as u16,
                    ((val >> 64) & 0xFFFF) as u16,
                    ((val >> 48) & 0xFFFF) as u16,
                    ((val >> 32) & 0xFFFF) as u16,
                    ((val >> 16) & 0xFFFF) as u16,
                    (val & 0xFFFF) as u16,
                ])
            }
            ValueType::U32le => {
                let val: u32 = if let Some(s) = s.strip_prefix("0x") {
                    u32::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![(val & 0xFFFF) as u16, (val >> 16) as u16])
            }
            ValueType::U64le => {
                let val: u64 = if let Some(s) = s.strip_prefix("0x") {
                    u64::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![
                    (val & 0xFFFF) as u16,
                    ((val >> 16) & 0xFFFF) as u16,
                    ((val >> 32) & 0xFFFF) as u16,
                    ((val >> 48) & 0xFFFF) as u16,
                ])
            }
            ValueType::U128le => {
                let val: u128 = if let Some(s) = s.strip_prefix("0x") {
                    u128::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![
                    (val & 0xFFFF) as u16,
                    ((val >> 16) & 0xFFFF) as u16,
                    ((val >> 32) & 0xFFFF) as u16,
                    ((val >> 48) & 0xFFFF) as u16,
                    ((val >> 64) & 0xFFFF) as u16,
                    ((val >> 80) & 0xFFFF) as u16,
                    ((val >> 96) & 0xFFFF) as u16,
                    ((val >> 112) & 0xFFFF) as u16,
                ])
            }
            ValueType::I32le => {
                let val: i32 = if let Some(s) = s.strip_prefix("-0x") {
                    -i32::from_str_radix(s, 16)?
                } else if let Some(s) = s.strip_prefix("0x") {
                    let v = u32::from_str_radix(s, 16)?;
                    if v > (i32::MAX as u32) {
                        v as i32
                    } else {
                        return Err(anyhow::anyhow!("Value too large for i32."));
                    }
                } else {
                    s.parse()?
                };
                Ok(vec![(val & 0xFFFF) as u16, (val >> 16) as u16])
            }
            ValueType::I64le => {
                let val: i64 = if let Some(s) = s.strip_prefix("0x") {
                    -i64::from_str_radix(s, 16)?
                } else if let Some(s) = s.strip_prefix("0x") {
                    let v = u64::from_str_radix(s, 16)?;
                    if v > (i64::MAX as u64) {
                        v as i64
                    } else {
                        return Err(anyhow::anyhow!("Value too large for i64."));
                    }
                } else {
                    s.parse()?
                };
                Ok(vec![
                    (val & 0xFFFF) as u16,
                    ((val >> 16) & 0xFFFF) as u16,
                    ((val >> 32) & 0xFFFF) as u16,
                    ((val >> 48) & 0xFFFF) as u16,
                ])
            }
            ValueType::I128le => {
                let val: i128 = if let Some(s) = s.strip_prefix("0x") {
                    -i128::from_str_radix(s, 16)?
                } else if let Some(s) = s.strip_prefix("0x") {
                    let v = u128::from_str_radix(s, 16)?;
                    if v > (i128::MAX as u128) {
                        v as i128
                    } else {
                        return Err(anyhow::anyhow!("Value too large for i128."));
                    }
                } else {
                    s.parse()?
                };
                Ok(vec![
                    (val & 0xFFFF) as u16,
                    ((val >> 16) & 0xFFFF) as u16,
                    ((val >> 32) & 0xFFFF) as u16,
                    ((val >> 48) & 0xFFFF) as u16,
                    ((val >> 64) & 0xFFFF) as u16,
                    ((val >> 80) & 0xFFFF) as u16,
                    ((val >> 96) & 0xFFFF) as u16,
                    ((val >> 112) & 0xFFFF) as u16,
                ])
            }
        }
    }
}
