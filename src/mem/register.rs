use crate::mem::data::DataType;
use crate::mem::memory::{Memory, Range};
use crate::util::str;
use crate::util::Expect;
use crate::AppConfig;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio_modbus::prelude::SlaveId;
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
#[serde(untagged)]
pub enum Value {
    Str(String),
    Num(i64),
    Float(f64),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Definition {
    slave_id: Option<SlaveId>,
    address: Address,
    length: u16,
    #[serde(flatten)]
    r#type: DataType,
    read_code: u8,
    access: AccessType,
    default: Option<Value>,
}

impl Definition {
    #[allow(dead_code)]
    pub fn new(
        slave_id: Option<SlaveId>,
        address: u16,
        length: u16,
        r#type: DataType,
        read_code: u8,
        access: AccessType,
        default: Option<Value>,
    ) -> Self {
        Self {
            slave_id,
            address: Address::Decimal(address),
            length,
            r#type,
            read_code,
            access,
            default,
        }
    }

    pub fn get_default(&self) -> &Option<Value> {
        &self.default
    }

    pub fn get_slave_id(&self) -> &Option<SlaveId> {
        &self.slave_id
    }

    pub fn get_range(&self) -> Range<u16> {
        Range::new(self.address.as_u16(), self.address.as_u16() + self.length)
    }

    pub fn get_address(&self) -> u16 {
        self.address.as_u16()
    }

    pub fn get_type(&self) -> DataType {
        self.r#type.clone()
    }

    pub fn read_code(&self) -> u8 {
        self.read_code
    }

    pub fn length(&self) -> u16 {
        self.length
    }

    pub fn access_type(&self) -> AccessType {
        self.access.clone()
    }
}

#[derive(Clone)]
pub struct Register {
    slave: SlaveId,
    address: u16,
    value: String,
    length: u16,
    function_code: FunctionCode,
    raw: Vec<u16>,
    r#type: DataType,
    access: AccessType,
}

impl Register {
    pub fn new(definition: &Definition, memory: &Arc<Mutex<Memory>>) -> Self {
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

        let bytes: Vec<u16> = memory
            .lock()
            .unwrap()
            .read(
                definition.get_slave_id().unwrap_or(0),
                &definition.get_range(),
            )
            .panic(|e| format!("{}", e))
            .into_iter()
            .copied()
            .collect();
        let value = definition
            .get_type()
            .as_str(&bytes)
            .unwrap_or(str!("Invalid data"));

        Self {
            slave: definition.get_slave_id().unwrap_or(0),
            address: definition.address.as_u16(),
            value,
            function_code: read_code,
            length: definition.length(),
            raw: bytes,
            r#type: definition.get_type().clone(),
            access: definition.access_type(),
        }
    }

    pub fn slave_id(&self) -> SlaveId {
        self.slave
    }

    pub fn address(&self) -> u16 {
        self.address
    }

    pub fn value(&self) -> &String {
        &self.value
    }

    pub fn length(&self) -> u16 {
        self.length
    }

    pub fn raw(&self) -> &Vec<u16> {
        &self.raw
    }

    pub fn r#type(&self) -> &DataType {
        &self.r#type
    }

    pub fn function_code(&self) -> FunctionCode {
        self.function_code
    }

    pub fn access_type(&self) -> AccessType {
        self.access.clone()
    }
}

pub struct Handler {
    config: Arc<Mutex<AppConfig>>,
    memory: Arc<Mutex<Memory>>,
}

impl Handler {
    #[allow(dead_code)]
    pub fn new(config: Arc<Mutex<AppConfig>>, memory: Arc<Mutex<Memory>>) -> Self {
        Self { config, memory }
    }

    pub fn len(&self) -> usize {
        self.config.lock().unwrap().definitions.len()
    }

    pub fn values(&self) -> HashMap<String, Register> {
        self.config
            .lock()
            .unwrap()
            .definitions
            .iter()
            .map(|(name, def)| (name.clone(), Register::new(def, &self.memory)))
            .collect()
    }

    pub fn set_values(&mut self, slave: SlaveId, addr: u16, values: &[u16]) -> anyhow::Result<()> {
        let mut memory = self.memory.lock().unwrap();
        memory
            .write(slave, Range::new(addr, addr + values.len() as u16), values)
            .map(|_| ())
    }
}
