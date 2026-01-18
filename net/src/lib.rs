pub mod rtu;
pub mod tcp;

use memory::Range;
use std::fmt::Debug;
use std::hash::Hash;
use tokio_modbus::{FunctionCode, SlaveId};

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

pub enum Error {
    TimedOut,
}

type Address = u16;
type Value = u16;
type Coil = bool;

pub enum Command {
    Terminate,
    WriteSingleCoil(SlaveId, Address, Coil),
    WriteMultipleCoils(SlaveId, Address, Vec<Coil>),
    WriteSingleRegister(SlaveId, Address, Value),
    WriteMultipleRegister(SlaveId, Address, Vec<Value>),
}
