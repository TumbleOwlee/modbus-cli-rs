use std::fmt::Display;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Kind {
    Coil,
    DiscreteInput,
    HoldingRegister,
    InputRegister,
}

impl Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Kind::Coil => write!(f, "Coil"),
            Kind::DiscreteInput => write!(f, "Discrete Input"),
            Kind::HoldingRegister => write!(f, "Holding Register"),
            Kind::InputRegister => write!(f, "Input Register"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Address {
    Fixed(u16),
    Virtual,
}

impl Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Address::Fixed(v) => write!(f, "{}", v),
            Address::Virtual => write!(f, "virtual"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Access {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

impl Display for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Access::ReadOnly => write!(f, "ReadOnly"),
            Access::WriteOnly => write!(f, "WriteOnly"),
            Access::ReadWrite => write!(f, "ReadWrite"),
        }
    }
}
