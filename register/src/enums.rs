use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Kind {
    Coil,
    DiscreteInput,
    HoldingRegister,
    InputRegisters,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Address {
    Fixed(u16),
    Virtual,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Access {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}
