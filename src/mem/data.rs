use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum DataType {
    PackedAscii,
    LooseAscii,
    PackedUtf8,
    LooseUtf8,
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
    U16le,
    U32le,
    U64le,
    U128le,
    I8le,
    I16le,
    I32le,
    I64le,
    I128le,
    F32,
    F32le,
    F64,
    F64le,
}

impl DataType {
    pub fn as_str(&self, bytes: &[u16]) -> anyhow::Result<String> {
        match self {
            DataType::F32 => {
                if let (Some(b1), Some(b2)) = (bytes.first(), bytes.get(1)) {
                    let uval: u32 = ((*b1 as u32) << 16) + (*b2 as u32);
                    let val = f32::from_bits(uval);
                    Ok(format!("0x{:02$X} ({})", uval, val, 8))
                } else {
                    Err(anyhow!("Not enough bytes"))
                }
            }
            DataType::F32le => {
                if let (Some(b1), Some(b2)) = (bytes.first(), bytes.get(1)) {
                    let uval: u32 = ((*b2 as u32) << 16) + (*b1 as u32);
                    let val = f32::from_bits(uval);
                    Ok(format!("0x{:02$X} ({})", uval, val, 8))
                } else {
                    Err(anyhow!("Not enough bytes"))
                }
            }
            DataType::F64 => {
                if let (Some(b1), Some(b2), Some(b3), Some(b4)) =
                    (bytes.first(), bytes.get(1), bytes.get(2), bytes.get(3))
                {
                    let uval: u64 = ((*b1 as u64) << 48)
                        + ((*b2 as u64) << 32)
                        + ((*b3 as u64) << 16)
                        + (*b4 as u64);
                    let val = f64::from_bits(uval);
                    Ok(format!("0x{:02$X} ({})", uval, val, 16))
                } else {
                    Err(anyhow!("Not enough bytes"))
                }
            }
            DataType::F64le => {
                if let (Some(b1), Some(b2), Some(b3), Some(b4)) =
                    (bytes.first(), bytes.get(1), bytes.get(2), bytes.get(3))
                {
                    let uval: u64 = ((*b4 as u64) << 48)
                        + ((*b3 as u64) << 32)
                        + ((*b2 as u64) << 16)
                        + (*b1 as u64);
                    let val = f64::from_bits(uval);
                    Ok(format!("0x{:02$X} ({})", uval, val, 16))
                } else {
                    Err(anyhow!("Not enough bytes"))
                }
            }
            DataType::PackedAscii => String::from_utf8(
                bytes
                    .iter()
                    .flat_map(|v| vec![(*v >> 8) as u8, (*v & 0xFF) as u8])
                    .collect(),
            )
            .map_err(|e| e.into()),
            DataType::LooseAscii => {
                String::from_utf8(bytes.iter().map(|v| (*v & 0xFF) as u8).collect())
                    .map_err(|e| e.into())
            }
            DataType::PackedUtf8 => String::from_utf8(
                bytes
                    .iter()
                    .flat_map(|v| vec![((*v >> 8) & 0xFF) as u8, (*v & 0xFF) as u8])
                    .collect(),
            )
            .map_err(|e| e.into()),
            DataType::LooseUtf8 => bytes
                .iter()
                .map(|v| {
                    if (*v & 0xFF00) != 0x0000 {
                        let s =
                            String::from_utf8(vec![((*v >> 8) & 0xFF) as u8, (*v & 0xFF) as u8]);
                        match s {
                            Ok(s) if s.len() == 1 => Ok(s),
                            Ok(_) => Err(anyhow!("Invalid data")),
                            Err(e) => Err(e.into()),
                        }
                    } else {
                        String::from_utf8(vec![(*v & 0xFF) as u8]).map_err(|e| e.into())
                    }
                })
                .try_fold(String::new(), |mut s, r| match r {
                    Ok(r) => {
                        s.push_str(&r);
                        Ok(s)
                    }
                    Err(_) => Err(anyhow!("Invalid data")),
                }),
            DataType::U8 => {
                let val: u8 = (*(bytes.first().unwrap()) & 0xFF) as u8;
                Ok(format!("{:#04X} ({})", val, val))
            }
            DataType::U16 => {
                let val: u16 = *bytes.first().unwrap();
                Ok(format!("{:#06X} ({})", val, val))
            }
            DataType::U32 => {
                let val: u32 =
                    (((*bytes.first().unwrap()) as u32) << 16) + ((*bytes.get(1).unwrap()) as u32);
                Ok(format!("0x{:02$X} ({})", val, val, 8))
            }
            DataType::U64 => {
                let val: u64 = (((*bytes.first().unwrap()) as u64) << 48)
                    + (((*bytes.get(1).unwrap()) as u64) << 32)
                    + (((*bytes.get(2).unwrap()) as u64) << 16)
                    + (*bytes.get(3).unwrap()) as u64;
                Ok(format!("0x{:02$X} ({})", val, val, 16))
            }
            DataType::U128 => {
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
            DataType::I8 => {
                let val: i8 = (*(bytes.first().unwrap()) & 0xFF) as i8;
                Ok(format!("{:#04X} ({})", val, val))
            }
            DataType::I16 => {
                let val: i16 = *bytes.first().unwrap() as i16;
                Ok(format!("{:#06X} ({})", val, val))
            }
            DataType::I32 => {
                let val: i32 =
                    (((*bytes.first().unwrap()) as i32) << 16) + ((*bytes.get(1).unwrap()) as i32);
                Ok(format!("0x{:02$X} ({})", val, val, 8))
            }
            DataType::I64 => {
                let val: i64 = (((*bytes.first().unwrap()) as i64) << 48)
                    + (((*bytes.get(1).unwrap()) as i64) << 32)
                    + (((*bytes.get(2).unwrap()) as i64) << 16)
                    + (*bytes.get(3).unwrap()) as i64;
                Ok(format!("0x{:02$X} ({})", val, val, 16))
            }
            DataType::I128 => {
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
            DataType::U16le => {
                let val: u16 = *bytes.first().unwrap();
                Ok(format!("{:#06X} ({})", val, val))
            }
            DataType::U32le => {
                let val: u32 =
                    (((*bytes.get(1).unwrap()) as u32) << 16) + ((*bytes.first().unwrap()) as u32);
                Ok(format!("0x{:02$X} ({})", val, val, 8))
            }
            DataType::U64le => {
                let val: u64 = (((*bytes.get(3).unwrap()) as u64) << 48)
                    + (((*bytes.get(2).unwrap()) as u64) << 32)
                    + (((*bytes.get(1).unwrap()) as u64) << 16)
                    + (*bytes.first().unwrap()) as u64;
                Ok(format!("0x{:02$X} ({})", val, val, 16))
            }
            DataType::U128le => {
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
            DataType::I8le => {
                let val: i8 = (*(bytes.first().unwrap()) & 0xFF) as i8;
                Ok(format!("{:#04X} ({})", val, val))
            }
            DataType::I16le => {
                let val: i16 = *bytes.first().unwrap() as i16;
                Ok(format!("{:#06X} ({})", val, val))
            }
            DataType::I32le => {
                let val: i32 =
                    (((*bytes.first().unwrap()) as i32) << 16) + ((*bytes.get(1).unwrap()) as i32);
                Ok(format!("0x{:02$X} ({})", val, val, 8))
            }
            DataType::I64le => {
                let val: i64 = (((*bytes.get(3).unwrap()) as i64) << 48)
                    + (((*bytes.get(2).unwrap()) as i64) << 32)
                    + (((*bytes.get(1).unwrap()) as i64) << 16)
                    + (*bytes.first().unwrap()) as i64;
                Ok(format!("0x{:02$X} ({})", val, val, 16))
            }
            DataType::I128le => {
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

    pub fn encode(&self, s: &str) -> anyhow::Result<Vec<u16>> {
        match self {
            DataType::F32 => {
                let val: f32 = if let Some(s) = s.strip_prefix("0x") {
                    u32::from_str_radix(s, 16).map(f32::from_bits)?
                } else {
                    s.parse()?
                };
                let val = val.to_bits();
                Ok(vec![
                    ((val & 0xFFFF0000) >> 16) as u16,
                    (val & 0x0000FFFF) as u16,
                ])
            }
            DataType::F32le => {
                let val: f32 = if let Some(s) = s.strip_prefix("0x") {
                    u32::from_str_radix(s, 16).map(f32::from_bits)?
                } else {
                    s.parse()?
                };
                let val = val.to_bits();
                Ok(vec![
                    (val & 0x0000FFFF) as u16,
                    ((val & 0xFFFF0000) >> 16) as u16,
                ])
            }
            DataType::F64 => {
                let val: f64 = if let Some(s) = s.strip_prefix("0x") {
                    u64::from_str_radix(s, 16).map(f64::from_bits)?
                } else {
                    s.parse()?
                };
                let val = val.to_bits();
                Ok(vec![
                    ((val & 0xFFFF000000000000) >> 48) as u16,
                    ((val & 0x0000FFFF00000000) >> 32) as u16,
                    ((val & 0x00000000FFFF0000) >> 16) as u16,
                    (val & 0x000000000000FFFF) as u16,
                ])
            }
            DataType::F64le => {
                let val: f64 = if let Some(s) = s.strip_prefix("0x") {
                    u64::from_str_radix(s, 16).map(f64::from_bits)?
                } else {
                    s.parse()?
                };
                let val = val.to_bits();
                Ok(vec![
                    (val & 0x000000000000FFFF) as u16,
                    ((val & 0x00000000FFFF0000) >> 16) as u16,
                    ((val & 0x0000FFFF00000000) >> 32) as u16,
                    ((val & 0xFFFF000000000000) >> 48) as u16,
                ])
            }
            DataType::PackedAscii => {
                let mut v = Vec::with_capacity(s.len() / 2 + 1);
                let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
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
            DataType::LooseAscii => Ok(s.chars().map(|c| c as u16).collect()),
            DataType::LooseUtf8 => Ok(s.encode_utf16().collect()),
            DataType::PackedUtf8 => {
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
            DataType::U8 => {
                let val: u8 = if let Some(s) = s.strip_prefix("0x") {
                    u8::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![val as u16])
            }
            DataType::U16 | DataType::U16le => {
                let val: u16 = if let Some(s) = s.strip_prefix("0x") {
                    u16::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![val])
            }
            DataType::U32 => {
                let val: u32 = if let Some(s) = s.strip_prefix("0x") {
                    u32::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![(val >> 16) as u16, (val & 0xFFFF) as u16])
            }
            DataType::U64 => {
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
            DataType::U128 => {
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
            DataType::I8 | DataType::I8le => {
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
            DataType::I16 | DataType::I16le => {
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
            DataType::I32 => {
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
            DataType::I64 => {
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
            DataType::I128 => {
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
            DataType::U32le => {
                let val: u32 = if let Some(s) = s.strip_prefix("0x") {
                    u32::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![(val & 0xFFFF) as u16, (val >> 16) as u16])
            }
            DataType::U64le => {
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
            DataType::U128le => {
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
            DataType::I32le => {
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
            DataType::I64le => {
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
            DataType::I128le => {
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
