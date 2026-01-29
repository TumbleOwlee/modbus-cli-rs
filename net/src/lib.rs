#![feature(async_fn_traits)]

pub mod rtu;
pub mod tcp;

use memory::Range;
use std::fmt::{Debug, Display};
use std::hash::Hash;
use tokio_modbus::ExceptionCode;
pub use tokio_modbus::{FunctionCode, SlaveId};

#[derive(Debug, Clone)]
pub enum Config {
    Tcp(tcp::Config),
    Rtu(rtu::Config),
}

#[derive(Debug, Clone)]
pub struct Operation {
    pub slave_id: SlaveId,
    pub fn_code: FunctionCode,
    pub range: Range,
}

#[derive(Hash, Debug, PartialEq, Eq, Clone, Default)]
pub struct Key<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync,
{
    pub id: T,
    pub slave_id: SlaveId,
}

impl<T> Key<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync,
{
    pub fn from(id: T, slave_id: SlaveId) -> Self {
        Self { id, slave_id }
    }

    pub fn create(slave_id: SlaveId) -> Self {
        Self {
            id: T::default(),
            slave_id,
        }
    }
}

pub enum ModbusError {
    Exception(ExceptionCode),
    Error(tokio_modbus::Error),
    Timeout(tokio::time::error::Elapsed),
}

impl Display for ModbusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModbusError::Exception(code) => write!(f, "Modbus exception: {:?}", code),
            ModbusError::Error(e) => write!(f, "Modbus error: {}", e),
            ModbusError::Timeout(e) => write!(f, "Modbus timeout: {}", e),
        }
    }
}

pub enum SerialError {
    Error(tokio_serial::Error),
    Configuration(String),
}

impl Display for SerialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SerialError::Error(e) => write!(f, "Serial error: {}", e),
            SerialError::Configuration(e) => write!(f, "Serial configuration error: {}", e),
        }
    }
}

pub enum TcpError {
    Address(std::net::AddrParseError),
    Configuration(String),
    Error(tokio::io::Error),
    Timeout(tokio::time::error::Elapsed),
}

impl Display for TcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TcpError::Address(e) => write!(f, "TCP address error: {}", e),
            TcpError::Configuration(e) => write!(f, "TCP configuration error: {}", e),
            TcpError::Error(e) => write!(f, "TCP error: {}", e),
            TcpError::Timeout(e) => write!(f, "TCP timeout: {}", e),
        }
    }
}

pub enum Error {
    Modbus(ModbusError),
    Serial(SerialError),
    Tcp(TcpError),
    Server(std::io::Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Modbus(e) => write!(f, "{}", e),
            Error::Serial(e) => write!(f, "{}", e),
            Error::Tcp(e) => write!(f, "{}", e),
            Error::Server(e) => write!(f, "Server error: {}", e),
        }
    }
}

impl From<TcpError> for Error {
    fn from(e: TcpError) -> Self {
        Error::Tcp(e)
    }
}

impl From<SerialError> for Error {
    fn from(e: SerialError) -> Self {
        Error::Serial(e)
    }
}

impl From<ModbusError> for Error {
    fn from(e: ModbusError) -> Self {
        Error::Modbus(e)
    }
}

pub type Address = u16;
pub type Value = u16;
pub type Coil = bool;

pub enum Command {
    Terminate,
    WriteSingleCoil(SlaveId, Address, Coil),
    WriteMultipleCoils(SlaveId, Address, Vec<Coil>),
    WriteSingleRegister(SlaveId, Address, Value),
    WriteMultipleRegister(SlaveId, Address, Vec<Value>),
}
