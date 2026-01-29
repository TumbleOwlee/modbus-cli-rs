#![feature(async_fn_traits)]

pub mod rtu;
pub mod tcp;

use memory::Range;
use std::fmt::Debug;
use std::hash::Hash;
pub use tokio_modbus::{FunctionCode, SlaveId};

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
    id: T,
    slave_id: SlaveId,
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

#[derive(Debug)]
pub enum Error {
    TimedOut,
}

unsafe impl Send for Error {}

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
