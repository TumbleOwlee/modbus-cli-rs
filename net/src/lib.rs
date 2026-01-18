use tokio_modbus::SlaveId;

pub mod rtu;
pub mod tcp;

#[derive(Hash, Debug, PartialEq, Eq, Clone, Default)]
pub struct Key {
    slave_id: SlaveId,
    fn_code: u8,
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
