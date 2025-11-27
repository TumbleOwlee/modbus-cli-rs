pub mod client;
pub mod server;

use std::fmt::Display;

use crate::util::str;

use clap::{Args, ValueEnum};

#[derive(Clone, Debug, ValueEnum)]
pub enum FlowControl {
    None,
    Software,
    Hardware,
}

impl Display for FlowControl {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlowControl::None => fmt.write_str("NONE"),
            FlowControl::Software => fmt.write_str("SOFTWARE"),
            FlowControl::Hardware => fmt.write_str("HARDWARE"),
        }
    }
}

#[derive(Clone, Debug, Default, Args)]
pub struct RtuConfig {
    /// The device path to use for communication.
    pub path: String,

    /// The baud rate to use for the serial connection.
    #[arg(short, long, default_value_t = 115200)]
    pub baud_rate: u32,

    /// The Modbus slave id to use.
    #[arg(short, long, default_value_t = 1)]
    pub client_id: u8,

    /// The Modbus parity bit [values: even, odd, none]
    #[arg(short, long)]
    pub parity: Option<String>,

    /// The Modbus data bits [values: 5, 6, 7, 8]
    #[arg(short, long)]
    pub data_bits: Option<u8>,

    /// The Modbus stop bits [values: 1, 2]
    #[arg(short, long)]
    pub stop_bits: Option<u8>,

    /// The Modbus flow control
    #[arg(short, long)]
    pub flow_control: Option<FlowControl>,
}
