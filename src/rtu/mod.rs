pub mod client;
pub mod server;

use crate::util::str;

use clap::Args;

#[derive(Clone, Debug, Default, Args)]
pub struct RtuConfig {
    /// The device path to use for communication.
    pub path: String,

    /// The baud rate to use for the serial connection.
    #[arg(short, long, default_value_t = 115200)]
    pub baud_rate: u32,

    /// The Modbus slave id to use.
    #[arg(short, long, default_value_t = 1)]
    pub slave: u8,

    /// The Modbus parity bit
    #[arg(short, long)]
    pub parity: Option<String>,

    /// The Modbus data bits
    #[arg(short, long)]
    pub data_bits: Option<u8>,

    /// The Modbus stop bits
    #[arg(short, long)]
    pub stop_bits: Option<u8>,
}
