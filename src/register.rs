use crate::memory::{Memory, Range};
use crate::util::Expect;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio_modbus::FunctionCode;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Type {
    PackedString,
    LooseString,
    U8,
    U16,
    U32,
    U64,
}

impl Type {
    pub fn from(&self, bytes: &[u16]) -> anyhow::Result<String> {
        match self {
            Type::PackedString => Ok(String::from_utf8(
                bytes
                    .iter()
                    .flat_map(|v| vec![(*v >> 8) as u8, (*v & 0xFF) as u8])
                    .collect(),
            )
            .unwrap()),
            Type::LooseString => {
                Ok(String::from_utf8(bytes.iter().map(|v| (*v & 0xFF) as u8).collect()).unwrap())
            }
            Type::U8 => Ok((*(bytes.first().unwrap()) & 0xFF).to_string()),
            Type::U16 => Ok(bytes.first().unwrap().to_string()),
            Type::U32 => Ok(((((*bytes.first().unwrap()) as u32) << 16) + ((*bytes.get(1).unwrap()) as u32)).to_string()),
            Type::U64 => Ok(
                ((((*bytes.first().unwrap()) as u64) << 48)
                    + (((*bytes.get(1).unwrap()) as u64) << 32)
                    + (((*bytes.get(2).unwrap()) as u64) << 16)
                    + (*bytes.get(3).unwrap()) as u64) .to_string()),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Address {
    Hex(String),
    Decimal(u16),
}

impl Address {
    pub fn as_u16(&self) -> u16 {
        match self {
            Address::Decimal(v) => *v,
            Address::Hex(v) if v.starts_with("0x") => {
                if let Ok(v) = u16::from_str_radix(&v[2..], 16) {
                    v
                } else {
                    panic!("Failed to parse HEX address.")
                }
            }
            _ => panic!("Invalid HEX address specified."),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Definition {
    address: Address,
    length: u16,
    r#type: Type,
    read_code: u8,
}

impl Definition {
    #[allow(dead_code)]
    pub fn new(address: u16, length: u16, r#type: Type, read_code: u8) -> Self {
        Self {
            address: Address::Decimal(address),
            length,
            r#type,
            read_code,
        }
    }

    pub fn get_range(&self) -> Range<u16> {
        Range::new(self.address.as_u16(), self.address.as_u16() + self.length)
    }

    pub fn get_address(&self) -> u16 {
        self.address.as_u16()
    }

    pub fn get_type(&self) -> Type {
        self.r#type.clone()
    }

    pub fn read_code(&self) -> u8 {
        self.read_code.clone()
    }
}

pub struct Register {
    address: u16,
    value: String,
    raw: Vec<u16>,
    r#type: Type,
    read_code: FunctionCode,
}

impl Register {
    pub fn new(definition: &Definition) -> Self {
        let read_code = FunctionCode::new(definition.read_code);
        match read_code {
            FunctionCode::WriteSingleCoil
            | FunctionCode::WriteMultipleCoils
            | FunctionCode::WriteSingleRegister
            | FunctionCode::WriteMultipleRegisters
            | FunctionCode::ReadWriteMultipleRegisters
            | FunctionCode::Custom(_) => {
                panic!(
                    "Invalid read function code for register {:?}",
                    definition.address
                )
            }
            _ => {}
        };
        Self {
            address: definition.address.as_u16(),
            value: String::new(),
            raw: vec![0; definition.get_range().length()],
            r#type: definition.get_type().clone(),
            read_code,
        }
    }

    pub fn address(&self) -> u16 {
        self.address
    }

    pub fn value(&self) -> &String {
        &self.value
    }

    pub fn raw(&self) -> &Vec<u16> {
        &self.raw
    }

    pub fn r#type(&self) -> Type {
        self.r#type.clone()
    }
}

pub struct Handler<'a, const SLICE_SIZE: usize> {
    definitions: &'a HashMap<String, Definition>,
    values: HashMap<String, Register>,
    memory: Arc<Mutex<Memory<SLICE_SIZE, u16>>>,
}

impl<'a, const SLICE_SIZE: usize> Handler<'a, SLICE_SIZE> {
    #[allow(dead_code)]
    pub fn new(
        definitions: &'a HashMap<String, Definition>,
        memory: Arc<Mutex<Memory<SLICE_SIZE, u16>>>,
    ) -> Self {
        Self {
            definitions,
            values: definitions
                .iter()
                .map(|(name, def)| (name.clone(), Register::new(def)))
                .collect(),
            memory,
        }
    }

    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    pub fn values(&self) -> &HashMap<String, Register> {
        &self.values
    }

    pub fn update(&mut self) -> anyhow::Result<()> {
        let mut memory = self.memory.lock().unwrap();
        for (name, def) in self.definitions.iter() {
            if let Some(value) = self.values.get_mut(name) {
                let bytes: Vec<u16> = memory
                    .read(&def.get_range())
                    .panic(|e| format!("{}", e))
                    .into_iter()
                    .copied()
                    .collect();
                value.value = def.get_type().from(&bytes)?;
                value.raw.copy_from_slice(&bytes);
            } else {
                panic!("Name not found in value map in `update()`.");
            }
        }
        Ok(())
    }
}
