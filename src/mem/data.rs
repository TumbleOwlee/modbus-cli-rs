use anyhow::anyhow;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Format {
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DataType {
    #[serde(rename = "type")]
    format: Format,
    #[serde(default)]
    reverse: bool,
}

impl Default for DataType {
    fn default() -> Self {
        Self {
            format: Format::U8,
            reverse: false,
        }
    }
}

impl DataType {
    pub fn label(&self) -> String {
        if self.reverse {
            format!("{:?} (reversed)", self.format)
        } else {
            format!("{:?}", self.format)
        }
    }

    fn apply_order(&self, v: u16) -> u16 {
        if self.reverse {
            let v1 = (v & 0xFF00) >> 8;
            let v2 = (v & 0x00FF) << 8;
            v1 + v2
        } else {
            v
        }
    }

    pub fn as_str(&self, bytes: &[u16]) -> anyhow::Result<String> {
        match self.format {
            Format::F32 => {
                if let (Some(b1), Some(b2)) = (
                    bytes.first().map(|v| self.apply_order(*v)),
                    bytes.get(1).map(|v| self.apply_order(*v)),
                ) {
                    let uval: u32 = ((b1 as u32) << 16) + (b2 as u32);
                    let val = f32::from_bits(uval);
                    Ok(format!("0x{:02$X} ({})", uval, val, 8))
                } else {
                    Err(anyhow!("Not enough bytes"))
                }
            }
            Format::F32le => {
                if let (Some(b1), Some(b2)) = (
                    bytes.first().map(|v| self.apply_order(*v)),
                    bytes.get(1).map(|v| self.apply_order(*v)),
                ) {
                    let uval: u32 = ((b2 as u32) << 16) + (b1 as u32);
                    let val = f32::from_bits(uval);
                    Ok(format!("0x{:02$X} ({})", uval, val, 8))
                } else {
                    Err(anyhow!("Not enough bytes"))
                }
            }
            Format::F64 => {
                if let (Some(b1), Some(b2), Some(b3), Some(b4)) = (
                    bytes.first().map(|v| self.apply_order(*v)),
                    bytes.get(1).map(|v| self.apply_order(*v)),
                    bytes.get(2).map(|v| self.apply_order(*v)),
                    bytes.get(3).map(|v| self.apply_order(*v)),
                ) {
                    let uval: u64 = ((b1 as u64) << 48)
                        + ((b2 as u64) << 32)
                        + ((b3 as u64) << 16)
                        + (b4 as u64);
                    let val = f64::from_bits(uval);
                    Ok(format!("0x{:02$X} ({})", uval, val, 16))
                } else {
                    Err(anyhow!("Not enough bytes"))
                }
            }
            Format::F64le => {
                if let (Some(b1), Some(b2), Some(b3), Some(b4)) = (
                    bytes.first().map(|v| self.apply_order(*v)),
                    bytes.get(1).map(|v| self.apply_order(*v)),
                    bytes.get(2).map(|v| self.apply_order(*v)),
                    bytes.get(3).map(|v| self.apply_order(*v)),
                ) {
                    let uval: u64 = ((b4 as u64) << 48)
                        + ((b3 as u64) << 32)
                        + ((b2 as u64) << 16)
                        + (b1 as u64);
                    let val = f64::from_bits(uval);
                    Ok(format!("0x{:02$X} ({})", uval, val, 16))
                } else {
                    Err(anyhow!("Not enough bytes"))
                }
            }
            Format::PackedAscii => String::from_utf8(
                bytes
                    .iter()
                    .map(|v| self.apply_order(*v))
                    .flat_map(|v| vec![((v & 0xFF00) >> 8) as u8, (v & 0xFF) as u8])
                    .collect(),
            )
            .map_err(|e| e.into()),
            Format::LooseAscii => String::from_utf8(
                bytes
                    .iter()
                    .map(|v| self.apply_order(*v))
                    .map(|v| (v & 0xFF) as u8)
                    .collect(),
            )
            .map_err(|e| e.into()),
            Format::PackedUtf8 => String::from_utf8(
                bytes
                    .iter()
                    .map(|v| self.apply_order(*v))
                    .flat_map(|v| vec![((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8])
                    .collect(),
            )
            .map_err(|e| e.into()),
            Format::LooseUtf8 => bytes
                .iter()
                .map(|v| self.apply_order(*v))
                .map(|v| {
                    if (v & 0xFF00) != 0x0000 {
                        let s = String::from_utf8(vec![((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8]);
                        match s {
                            Ok(s) if s.len() == 1 => Ok(s),
                            Ok(_) => Err(anyhow!("Invalid data")),
                            Err(e) => Err(e.into()),
                        }
                    } else {
                        String::from_utf8(vec![(v & 0xFF) as u8]).map_err(|e| e.into())
                    }
                })
                .try_fold(String::new(), |mut s, r| match r {
                    Ok(r) => {
                        s.push_str(&r);
                        Ok(s)
                    }
                    Err(_) => Err(anyhow!("Invalid data")),
                }),
            Format::U8 => {
                let val: u8 = ((self.apply_order(*bytes.first().unwrap())) & 0xFF) as u8;
                Ok(format!("{:#04X} ({})", val, val))
            }
            Format::U16 => {
                let val: u16 = self.apply_order(*bytes.first().unwrap());
                Ok(format!("{:#06X} ({})", val, val))
            }
            Format::U32 => {
                let val: u32 = ((self.apply_order(*bytes.first().unwrap()) as u32) << 16)
                    + (self.apply_order(*bytes.get(1).unwrap()) as u32);
                Ok(format!("0x{:02$X} ({})", val, val, 8))
            }
            Format::U64 => {
                let val: u64 = ((self.apply_order(*bytes.first().unwrap()) as u64) << 48)
                    + ((self.apply_order(*bytes.get(1).unwrap()) as u64) << 32)
                    + ((self.apply_order(*bytes.get(2).unwrap()) as u64) << 16)
                    + self.apply_order(*bytes.get(3).unwrap()) as u64;
                Ok(format!("0x{:02$X} ({})", val, val, 16))
            }
            Format::U128 => {
                let val: u128 = ((self.apply_order(*bytes.first().unwrap()) as u128) << 112)
                    + ((self.apply_order(*bytes.get(1).unwrap()) as u128) << 96)
                    + ((self.apply_order(*bytes.get(2).unwrap()) as u128) << 80)
                    + ((self.apply_order(*bytes.get(3).unwrap()) as u128) << 64)
                    + ((self.apply_order(*bytes.get(4).unwrap()) as u128) << 48)
                    + ((self.apply_order(*bytes.get(5).unwrap()) as u128) << 32)
                    + ((self.apply_order(*bytes.get(6).unwrap()) as u128) << 16)
                    + self.apply_order(*bytes.get(7).unwrap()) as u128;
                Ok(format!("0x{:02$X} ({})", val, val, 32))
            }
            Format::I8 => {
                let val: i8 = (self.apply_order(*bytes.first().unwrap()) & 0xFF) as i8;
                Ok(format!("{:#04X} ({})", val, val))
            }
            Format::I16 => {
                let val: i16 = self.apply_order(*bytes.first().unwrap()) as i16;
                Ok(format!("{:#06X} ({})", val, val))
            }
            Format::I32 => {
                let val: i32 = ((self.apply_order(*bytes.first().unwrap()) as i32) << 16)
                    + (self.apply_order(*bytes.get(1).unwrap()) as i32);
                Ok(format!("0x{:02$X} ({})", val, val, 8))
            }
            Format::I64 => {
                let val: i64 = ((self.apply_order(*bytes.first().unwrap()) as i64) << 48)
                    + ((self.apply_order(*bytes.get(1).unwrap()) as i64) << 32)
                    + ((self.apply_order(*bytes.get(2).unwrap()) as i64) << 16)
                    + self.apply_order(*bytes.get(3).unwrap()) as i64;
                Ok(format!("0x{:02$X} ({})", val, val, 16))
            }
            Format::I128 => {
                let val: i128 = ((self.apply_order(*bytes.first().unwrap()) as i128) << 112)
                    + ((self.apply_order(*bytes.get(1).unwrap()) as i128) << 96)
                    + ((self.apply_order(*bytes.get(2).unwrap()) as i128) << 80)
                    + ((self.apply_order(*bytes.get(3).unwrap()) as i128) << 64)
                    + ((self.apply_order(*bytes.get(4).unwrap()) as i128) << 48)
                    + ((self.apply_order(*bytes.get(5).unwrap()) as i128) << 32)
                    + ((self.apply_order(*bytes.get(6).unwrap()) as i128) << 16)
                    + self.apply_order(*bytes.get(7).unwrap()) as i128;
                Ok(format!("0x{:02$X} ({})", val, val, 32))
            }
            Format::U16le => {
                let val: u16 = self.apply_order(*bytes.first().unwrap());
                Ok(format!("{:#06X} ({})", val, val))
            }
            Format::U32le => {
                let val: u32 = ((self.apply_order(*bytes.get(1).unwrap()) as u32) << 16)
                    + (self.apply_order(*bytes.first().unwrap()) as u32);
                Ok(format!("0x{:02$X} ({})", val, val, 8))
            }
            Format::U64le => {
                let val: u64 = ((self.apply_order(*bytes.get(3).unwrap()) as u64) << 48)
                    + ((self.apply_order(*bytes.get(2).unwrap()) as u64) << 32)
                    + ((self.apply_order(*bytes.get(1).unwrap()) as u64) << 16)
                    + self.apply_order(*bytes.first().unwrap()) as u64;
                Ok(format!("0x{:02$X} ({})", val, val, 16))
            }
            Format::U128le => {
                let val: u128 = ((self.apply_order(*bytes.get(7).unwrap()) as u128) << 112)
                    + ((self.apply_order(*bytes.get(6).unwrap()) as u128) << 96)
                    + ((self.apply_order(*bytes.get(5).unwrap()) as u128) << 80)
                    + ((self.apply_order(*bytes.get(4).unwrap()) as u128) << 64)
                    + ((self.apply_order(*bytes.get(3).unwrap()) as u128) << 48)
                    + ((self.apply_order(*bytes.get(2).unwrap()) as u128) << 32)
                    + ((self.apply_order(*bytes.get(1).unwrap()) as u128) << 16)
                    + self.apply_order(*bytes.first().unwrap()) as u128;
                Ok(format!("0x{:02$X} ({})", val, val, 32))
            }
            Format::I8le => {
                let val: i8 = (self.apply_order(*bytes.first().unwrap()) & 0xFF) as i8;
                Ok(format!("{:#04X} ({})", val, val))
            }
            Format::I16le => {
                let val: i16 = self.apply_order(*bytes.first().unwrap()) as i16;
                Ok(format!("{:#06X} ({})", val, val))
            }
            Format::I32le => {
                let val: i32 = ((self.apply_order(*bytes.first().unwrap()) as i32) << 16)
                    + (self.apply_order(*bytes.get(1).unwrap()) as i32);
                Ok(format!("0x{:02$X} ({})", val, val, 8))
            }
            Format::I64le => {
                let val: i64 = ((self.apply_order(*bytes.get(3).unwrap()) as i64) << 48)
                    + ((self.apply_order(*bytes.get(2).unwrap()) as i64) << 32)
                    + ((self.apply_order(*bytes.get(1).unwrap()) as i64) << 16)
                    + self.apply_order(*bytes.first().unwrap()) as i64;
                Ok(format!("0x{:02$X} ({})", val, val, 16))
            }
            Format::I128le => {
                let val: i128 = ((self.apply_order(*bytes.get(7).unwrap()) as i128) << 112)
                    + ((self.apply_order(*bytes.get(6).unwrap()) as i128) << 96)
                    + ((self.apply_order(*bytes.get(5).unwrap()) as i128) << 80)
                    + ((self.apply_order(*bytes.get(4).unwrap()) as i128) << 64)
                    + ((self.apply_order(*bytes.get(3).unwrap()) as i128) << 48)
                    + ((self.apply_order(*bytes.get(2).unwrap()) as i128) << 32)
                    + ((self.apply_order(*bytes.get(1).unwrap()) as i128) << 16)
                    + self.apply_order(*bytes.first().unwrap()) as i128;
                Ok(format!("0x{:02$X} ({})", val, val, 32))
            }
        }
    }

    pub fn encode(&self, s: &str) -> anyhow::Result<Vec<u16>> {
        match self.format {
            Format::F32 => {
                let val: f32 = if let Some(s) = s.strip_prefix("0x") {
                    u32::from_str_radix(s, 16).map(f32::from_bits)?
                } else {
                    s.parse()?
                };
                let val = val.to_bits();
                Ok(vec![
                    self.apply_order(((val & 0xFFFF0000) >> 16) as u16),
                    self.apply_order((val & 0x0000FFFF) as u16),
                ])
            }
            Format::F32le => {
                let val: f32 = if let Some(s) = s.strip_prefix("0x") {
                    u32::from_str_radix(s, 16).map(f32::from_bits)?
                } else {
                    s.parse()?
                };
                let val = val.to_bits();
                Ok(vec![
                    self.apply_order((val & 0x0000FFFF) as u16),
                    self.apply_order(((val & 0xFFFF0000) >> 16) as u16),
                ])
            }
            Format::F64 => {
                let val: f64 = if let Some(s) = s.strip_prefix("0x") {
                    u64::from_str_radix(s, 16).map(f64::from_bits)?
                } else {
                    s.parse()?
                };
                let val = val.to_bits();
                Ok(vec![
                    self.apply_order(((val & 0xFFFF000000000000) >> 48) as u16),
                    self.apply_order(((val & 0x0000FFFF00000000) >> 32) as u16),
                    self.apply_order(((val & 0x00000000FFFF0000) >> 16) as u16),
                    self.apply_order((val & 0x000000000000FFFF) as u16),
                ])
            }
            Format::F64le => {
                let val: f64 = if let Some(s) = s.strip_prefix("0x") {
                    u64::from_str_radix(s, 16).map(f64::from_bits)?
                } else {
                    s.parse()?
                };
                let val = val.to_bits();
                Ok(vec![
                    self.apply_order((val & 0x000000000000FFFF) as u16),
                    self.apply_order(((val & 0x00000000FFFF0000) >> 16) as u16),
                    self.apply_order(((val & 0x0000FFFF00000000) >> 32) as u16),
                    self.apply_order(((val & 0xFFFF000000000000) >> 48) as u16),
                ])
            }
            Format::PackedAscii => {
                let mut v = Vec::with_capacity(s.len() / 2 + 1);
                let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
                let mut i = 0usize;
                loop {
                    let mut value: u16 = 0;
                    if i < s.len() {
                        if self.reverse {
                            value += bytes[i] as u16;
                        } else {
                            value += (bytes[i] as u16) << 8;
                        }
                    }
                    if (i + 1) < s.len() {
                        if self.reverse {
                            value += (bytes[i + 1] as u16) << 8;
                        } else {
                            value += bytes[i + 1] as u16;
                        }
                    }
                    v.push(value);
                    i += 2;

                    if i >= s.len() {
                        break;
                    }
                }
                Ok(v)
            }
            Format::LooseAscii => Ok(s.chars().map(|c| self.apply_order(c as u16)).collect()),
            Format::LooseUtf8 => Ok(s.encode_utf16().map(|c| self.apply_order(c)).collect()),
            Format::PackedUtf8 => {
                let mut v = Vec::with_capacity(s.len() / 2 + 1);
                let bytes = s.as_bytes();
                let mut i = 0usize;
                loop {
                    let mut value: u16 = 0;
                    if i < s.len() {
                        if self.reverse {
                            value += bytes[i] as u16;
                        } else {
                            value += (bytes[i] as u16) << 8;
                        }
                    }
                    if (i + 1) < s.len() {
                        if self.reverse {
                            value += (bytes[i + 1] as u16) << 8;
                        } else {
                            value += bytes[i + 1] as u16;
                        }
                    }
                    v.push(value);
                    i += 2;

                    if i >= s.len() {
                        break;
                    }
                }
                Ok(v)
            }
            Format::U8 => {
                let val: u8 = if let Some(s) = s.strip_prefix("0x") {
                    u8::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![self.apply_order(val as u16)])
            }
            Format::U16 | Format::U16le => {
                let val: u16 = if let Some(s) = s.strip_prefix("0x") {
                    u16::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![self.apply_order(val)])
            }
            Format::U32 => {
                let val: u32 = if let Some(s) = s.strip_prefix("0x") {
                    u32::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![
                    self.apply_order((val >> 16) as u16),
                    self.apply_order((val & 0xFFFF) as u16),
                ])
            }
            Format::U64 => {
                let val: u64 = if let Some(s) = s.strip_prefix("0x") {
                    u64::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![
                    self.apply_order(((val >> 48) & 0xFFFF) as u16),
                    self.apply_order(((val >> 32) & 0xFFFF) as u16),
                    self.apply_order(((val >> 16) & 0xFFFF) as u16),
                    self.apply_order((val & 0xFFFF) as u16),
                ])
            }
            Format::U128 => {
                let val: u128 = if let Some(s) = s.strip_prefix("0x") {
                    u128::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![
                    self.apply_order(((val >> 112) & 0xFFFF) as u16),
                    self.apply_order(((val >> 96) & 0xFFFF) as u16),
                    self.apply_order(((val >> 80) & 0xFFFF) as u16),
                    self.apply_order(((val >> 64) & 0xFFFF) as u16),
                    self.apply_order(((val >> 48) & 0xFFFF) as u16),
                    self.apply_order(((val >> 32) & 0xFFFF) as u16),
                    self.apply_order(((val >> 16) & 0xFFFF) as u16),
                    self.apply_order((val & 0xFFFF) as u16),
                ])
            }
            Format::I8 | Format::I8le => {
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
                Ok(vec![self.apply_order(val as u16)])
            }
            Format::I16 | Format::I16le => {
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
                Ok(vec![self.apply_order(val as u16)])
            }
            Format::I32 => {
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
                Ok(vec![
                    self.apply_order((val >> 16) as u16),
                    self.apply_order((val & 0xFFFF) as u16),
                ])
            }
            Format::I64 => {
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
                    self.apply_order(((val >> 48) & 0xFFFF) as u16),
                    self.apply_order(((val >> 32) & 0xFFFF) as u16),
                    self.apply_order(((val >> 16) & 0xFFFF) as u16),
                    self.apply_order((val & 0xFFFF) as u16),
                ])
            }
            Format::I128 => {
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
                    self.apply_order(((val >> 112) & 0xFFFF) as u16),
                    self.apply_order(((val >> 96) & 0xFFFF) as u16),
                    self.apply_order(((val >> 80) & 0xFFFF) as u16),
                    self.apply_order(((val >> 64) & 0xFFFF) as u16),
                    self.apply_order(((val >> 48) & 0xFFFF) as u16),
                    self.apply_order(((val >> 32) & 0xFFFF) as u16),
                    self.apply_order(((val >> 16) & 0xFFFF) as u16),
                    self.apply_order((val & 0xFFFF) as u16),
                ])
            }
            Format::U32le => {
                let val: u32 = if let Some(s) = s.strip_prefix("0x") {
                    u32::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![
                    self.apply_order((val & 0xFFFF) as u16),
                    self.apply_order((val >> 16) as u16),
                ])
            }
            Format::U64le => {
                let val: u64 = if let Some(s) = s.strip_prefix("0x") {
                    u64::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![
                    self.apply_order((val & 0xFFFF) as u16),
                    self.apply_order(((val >> 16) & 0xFFFF) as u16),
                    self.apply_order(((val >> 32) & 0xFFFF) as u16),
                    self.apply_order(((val >> 48) & 0xFFFF) as u16),
                ])
            }
            Format::U128le => {
                let val: u128 = if let Some(s) = s.strip_prefix("0x") {
                    u128::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(vec![
                    self.apply_order((val & 0xFFFF) as u16),
                    self.apply_order(((val >> 16) & 0xFFFF) as u16),
                    self.apply_order(((val >> 32) & 0xFFFF) as u16),
                    self.apply_order(((val >> 48) & 0xFFFF) as u16),
                    self.apply_order(((val >> 64) & 0xFFFF) as u16),
                    self.apply_order(((val >> 80) & 0xFFFF) as u16),
                    self.apply_order(((val >> 96) & 0xFFFF) as u16),
                    self.apply_order(((val >> 112) & 0xFFFF) as u16),
                ])
            }
            Format::I32le => {
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
                Ok(vec![
                    self.apply_order((val & 0xFFFF) as u16),
                    self.apply_order((val >> 16) as u16),
                ])
            }
            Format::I64le => {
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
                    self.apply_order((val & 0xFFFF) as u16),
                    self.apply_order(((val >> 16) & 0xFFFF) as u16),
                    self.apply_order(((val >> 32) & 0xFFFF) as u16),
                    self.apply_order(((val >> 48) & 0xFFFF) as u16),
                ])
            }
            Format::I128le => {
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
                    self.apply_order((val & 0xFFFF) as u16),
                    self.apply_order(((val >> 16) & 0xFFFF) as u16),
                    self.apply_order(((val >> 32) & 0xFFFF) as u16),
                    self.apply_order(((val >> 48) & 0xFFFF) as u16),
                    self.apply_order(((val >> 64) & 0xFFFF) as u16),
                    self.apply_order(((val >> 80) & 0xFFFF) as u16),
                    self.apply_order(((val >> 96) & 0xFFFF) as u16),
                    self.apply_order(((val >> 112) & 0xFFFF) as u16),
                ])
            }
        }
    }
}
