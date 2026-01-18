pub mod enums;
pub mod format;
pub mod traits;
pub mod value;

use derive_getters::Getters;
use derive_setters::Setters;
use serde::{Deserialize, Serialize};
use tokio_modbus::SlaveId;

use crate::enums::{Access, Address, Kind};
use crate::format::{Alignment, Endian, Format};
use crate::traits::{IntoVec, ParseFromU8};
use crate::value::Value;

#[derive(Setters)]
pub struct Builder {
    #[setters(into)]
    slave_id: SlaveId,
    access: Access,
    kind: Kind,
    address: Address,
    format: Format,
}

impl Builder {
    pub fn new(format: Format) -> Self {
        Self {
            slave_id: 0,
            access: Access::ReadWrite,
            kind: Kind::InputRegisters,
            address: Address::Virtual,
            format,
        }
    }

    pub fn build(self) -> Register {
        Register {
            slave_id: self.slave_id,
            access: self.access,
            kind: self.kind,
            address: self.address,
            format: self.format,
        }
    }
}

#[derive(Getters, Setters, Serialize, Deserialize)]
pub struct Register {
    #[setters(borrow_self, into, prefix = "set_")]
    slave_id: SlaveId,
    access: Access,
    kind: Kind,
    address: Address,
    format: Format,
}

impl Register {
    pub fn decode(&self, bytes: &[u16]) -> anyhow::Result<Value> {
        let width = self.format.width();
        if bytes.len() < width {
            Err(anyhow::anyhow!(format!(
                "Too few bytes to parse {:?}",
                self.format
            )))
        } else {
            let bytes = bytes
                .iter()
                .take(width)
                .flat_map(|v| [(v >> 8) as u8, (v & 0xFF) as u8]);

            match &self.format {
                Format::U8(e) => Ok(Value::U8(match e {
                    Endian::Big => ParseFromU8::<u16>::parse(bytes) as u8,
                    Endian::Little => ParseFromU8::<u16>::parse(bytes.rev()) as u8,
                })),
                Format::U16(e) => Ok(Value::U16(match e {
                    Endian::Big => bytes.parse(),
                    Endian::Little => bytes.rev().parse(),
                })),
                Format::U32(e) => Ok(Value::U32(match e {
                    Endian::Big => bytes.parse(),
                    Endian::Little => bytes.rev().parse(),
                })),
                Format::U64(e) => Ok(Value::U64(match e {
                    Endian::Big => bytes.parse(),
                    Endian::Little => bytes.rev().parse(),
                })),
                Format::U128(e) => Ok(Value::U128(match e {
                    Endian::Big => bytes.parse(),
                    Endian::Little => bytes.rev().parse(),
                })),
                Format::I8(e) => Ok(Value::I8(match e {
                    Endian::Big => ParseFromU8::<u16>::parse(bytes) as i8,
                    Endian::Little => ParseFromU8::<u16>::parse(bytes.rev()) as i8,
                })),
                Format::I16(e) => Ok(Value::I16(match e {
                    Endian::Big => bytes.parse(),
                    Endian::Little => bytes.rev().parse(),
                })),
                Format::I32(e) => Ok(Value::I32(match e {
                    Endian::Big => bytes.parse(),
                    Endian::Little => bytes.rev().parse(),
                })),
                Format::I64(e) => Ok(Value::I64(match e {
                    Endian::Big => bytes.parse(),
                    Endian::Little => bytes.rev().parse(),
                })),
                Format::I128(e) => Ok(Value::I128(match e {
                    Endian::Big => bytes.parse(),
                    Endian::Little => bytes.rev().parse(),
                })),
                Format::F32(e) => {
                    let u: u32 = match e {
                        Endian::Big => bytes.parse(),
                        Endian::Little => bytes.rev().parse(),
                    };
                    Ok(Value::F32(f32::from_bits(u)))
                }
                Format::F64(e) => {
                    let u: u64 = match e {
                        Endian::Big => bytes.parse(),
                        Endian::Little => bytes.rev().parse(),
                    };
                    Ok(Value::F64(f64::from_bits(u)))
                }
                Format::Ascii(_) => Ok(Value::Ascii(
                    String::from_utf8(bytes.collect())
                        .map_err(|_| anyhow::anyhow!("Parse PackedAscii failed."))?,
                )),
            }
        }
    }

    pub fn encode(&self, s: &str) -> anyhow::Result<Vec<u16>> {
        match &self.format {
            Format::F32(e) => {
                let val: f32 = if let Some(s) = s.strip_prefix("0x") {
                    u32::from_str_radix(s, 16).map(f32::from_bits)?
                } else {
                    s.parse()?
                };
                Ok(match e {
                    Endian::Big => val.to_bits().to_be_bytes().iter().into_vec()?,
                    Endian::Little => val.to_bits().to_le_bytes().iter().into_vec()?,
                })
            }
            Format::F64(e) => {
                let val: f64 = if let Some(s) = s.strip_prefix("0x") {
                    u64::from_str_radix(s, 16).map(f64::from_bits)?
                } else {
                    s.parse()?
                };
                Ok(match e {
                    Endian::Big => val.to_bits().to_be_bytes().iter().into_vec()?,
                    Endian::Little => val.to_bits().to_le_bytes().iter().into_vec()?,
                })
            }
            Format::Ascii((a, w)) => {
                let length = 2 * w.0;

                let mut zeroes = itertools::repeat_n(0, 0);
                if s.len() < length {
                    zeroes = itertools::repeat_n(0u8, length - s.len());
                }

                match a {
                    Alignment::Left => Ok(s.bytes().chain(zeroes).take(length).into_vec()?),
                    Alignment::Right => Ok(zeroes.chain(s.bytes()).take(length).into_vec()?),
                }
            }
            Format::U8(e) => {
                let val: u8 = if let Some(s) = s.strip_prefix("0x") {
                    u8::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(match e {
                    Endian::Big => vec![val as u16],
                    Endian::Little => vec![(val as u16) << 8],
                })
            }
            Format::U16(e) => {
                let val: u16 = if let Some(s) = s.strip_prefix("0x") {
                    u16::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(match e {
                    Endian::Big => vec![val.to_be()],
                    Endian::Little => vec![val.to_le()],
                })
            }
            Format::U32(e) => {
                let val: u32 = if let Some(s) = s.strip_prefix("0x") {
                    u32::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(match e {
                    Endian::Big => val.to_be_bytes().iter().into_vec()?,
                    Endian::Little => val.to_le_bytes().iter().into_vec()?,
                })
            }
            Format::U64(e) => {
                let val: u64 = if let Some(s) = s.strip_prefix("0x") {
                    u64::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(match e {
                    Endian::Big => val.to_be_bytes().iter().into_vec()?,
                    Endian::Little => val.to_le_bytes().iter().into_vec()?,
                })
            }
            Format::U128(e) => {
                let val: u128 = if let Some(s) = s.strip_prefix("0x") {
                    u128::from_str_radix(s, 16)?
                } else {
                    s.parse()?
                };
                Ok(match e {
                    Endian::Big => val.to_be_bytes().iter().into_vec()?,
                    Endian::Little => val.to_le_bytes().iter().into_vec()?,
                })
            }
            Format::I8(e) => {
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
                Ok(match e {
                    Endian::Big => vec![val as u16],
                    Endian::Little => vec![(val as u16) << 8],
                })
            }
            Format::I16(e) => {
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
                Ok(match e {
                    Endian::Big => val.to_be_bytes().iter().into_vec()?,
                    Endian::Little => val.to_le_bytes().iter().into_vec()?,
                })
            }
            Format::I32(e) => {
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
                Ok(match e {
                    Endian::Big => val.to_be_bytes().iter().into_vec()?,
                    Endian::Little => val.to_le_bytes().iter().into_vec()?,
                })
            }
            Format::I64(e) => {
                let val: i64 = if let Some(s) = s.strip_prefix("-0x") {
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
                Ok(match e {
                    Endian::Big => val.to_be_bytes().iter().into_vec()?,
                    Endian::Little => val.to_le_bytes().iter().into_vec()?,
                })
            }
            Format::I128(e) => {
                let val: i128 = if let Some(s) = s.strip_prefix("-0x") {
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
                Ok(match e {
                    Endian::Big => val.to_be_bytes().iter().into_vec()?,
                    Endian::Little => val.to_le_bytes().iter().into_vec()?,
                })
            }
        }
    }
}
