use crate::memory::{Memory, Range};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Serialize, Deserialize, Debug)]
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

#[derive(Serialize, Deserialize, Debug)]
pub struct Definition {
    address: Address,
    length: u16,
    r#type: Type,
}

impl Definition {
    #[allow(dead_code)]
    pub fn new(address: u16, length: u16, r#type: Type) -> Self {
        Self {
            address: Address::Decimal(address),
            length,
            r#type,
        }
    }

    pub fn get_range(&self) -> Range<u16> {
        Range::new(self.address.as_u16(), self.address.as_u16() + self.length)
    }

    pub fn get_address(&self) -> u16 {
        self.address.as_u16()
    }

    pub fn get_type(&self) -> &Type {
        &self.r#type
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Type {
    String,
    Uint8Le,
    Uint8Be,
}

pub struct Register {
    value: String,
    raw: Vec<u16>,
}

impl Register {
    pub fn new(length: usize) -> Self {
        Self {
            value: String::new(),
            raw: vec![0; length],
        }
    }
}

impl Type {
    pub fn from(&self, bytes: &[u16]) -> anyhow::Result<String> {
        match self {
            Type::String => Ok(String::from_utf8(
                bytes
                    .iter()
                    .flat_map(|v| vec![(*v >> 8) as u8, (*v & 0xFF) as u8])
                    .collect(),
            )
            .unwrap()),
            Type::Uint8Le => Ok((*(bytes.first().unwrap()) >> 8).to_string()),
            Type::Uint8Be => Ok((*(bytes.first().unwrap()) & 0xFF).to_string()),
        }
    }
}

pub struct RegisterHandler<'a, const SLICE_SIZE: usize> {
    definitions: &'a HashMap<String, Definition>,
    values: HashMap<String, Register>,
    memory: Arc<Mutex<Memory<SLICE_SIZE, u16>>>,
}

impl<'a, const SLICE_SIZE: usize> RegisterHandler<'a, SLICE_SIZE> {
    #[allow(dead_code)]
    pub fn new(
        definitions: &'a HashMap<String, Definition>,
        memory: Arc<Mutex<Memory<SLICE_SIZE, u16>>>,
    ) -> Self {
        Self {
            definitions,
            values: definitions
                .iter()
                .map(|(name, def)| (name.clone(), Register::new(def.get_range().length())))
                .collect(),
            memory,
        }
    }

    #[allow(dead_code)]
    pub fn update(&mut self) -> anyhow::Result<()> {
        let mut memory = self.memory.lock().unwrap();
        for (name, def) in self.definitions.iter() {
            if let Some(value) = self.values.get_mut(name) {
                let bytes: Vec<u16> = memory
                    .read(&def.get_range())?
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
