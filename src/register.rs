use crate::memory::{Memory, Range};
use crate::util::Expect;
use crate::value::ValueType;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio_modbus::FunctionCode;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Address {
    Hex(String),
    Decimal(u16),
}

impl From<Address> for usize {
    fn from(address: Address) -> usize {
        address.as_u16() as usize
    }
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
pub enum AccessType {
    ReadWrite,
    ReadOnly,
    WriteOnly,
}

impl std::fmt::Display for AccessType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self {
            AccessType::ReadOnly => f.write_str("ReadOnly"),
            AccessType::WriteOnly => f.write_str("WriteOnly"),
            AccessType::ReadWrite => f.write_str("ReadWrite"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Definition {
    address: Address,
    length: u16,
    r#type: ValueType,
    read_code: u8,
    access: AccessType,
}

impl Definition {
    #[allow(dead_code)]
    pub fn new(
        address: u16,
        length: u16,
        r#type: ValueType,
        read_code: u8,
        access: AccessType,
    ) -> Self {
        Self {
            address: Address::Decimal(address),
            length,
            r#type,
            read_code,
            access,
        }
    }

    pub fn get_range(&self) -> Range<u16> {
        Range::new(self.address.as_u16(), self.address.as_u16() + self.length)
    }

    pub fn get_address(&self) -> u16 {
        self.address.as_u16()
    }

    pub fn get_type(&self) -> ValueType {
        self.r#type.clone()
    }

    pub fn read_code(&self) -> u8 {
        self.read_code
    }

    pub fn access_type(&self) -> AccessType {
        self.access.clone()
    }
}

#[derive(Clone)]
pub struct Register {
    address: u16,
    value: String,
    raw: Vec<u16>,
    r#type: ValueType,
    access: AccessType,
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
            access: definition.access_type(),
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

    pub fn r#type(&self) -> ValueType {
        self.r#type.clone()
    }

    pub fn access_type(&self) -> AccessType {
        self.access.clone()
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

    pub fn set_values(&mut self, addr: u16, values: &[u16]) -> anyhow::Result<()> {
        let mut memory = self.memory.lock().unwrap();
        memory
            .write(Range::new(addr, addr + values.len() as u16), values)
            .map(|_| ())
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
                value.value = def.get_type().as_str(&bytes)?;
                value.raw.copy_from_slice(&bytes);
            } else {
                panic!("Name not found in value map in `update()`.");
            }
        }
        Ok(())
    }
}
