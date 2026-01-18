pub mod client;
pub mod server;

use clap::Args;

#[derive(Clone, Debug, Default, Args)]
pub struct Config {
    /// The device path to use for communication.
    pub path: String,

    /// The baud rate to use for the serial connection.
    #[arg(short, long, default_value_t = 115200)]
    pub baud_rate: u32,

    /// The Modbus slave id to use.
    #[arg(short, long, default_value_t = 1)]
    pub slave: u8,

    /// The Modbus parity bit [values: even, odd, none]
    #[arg(short, long)]
    pub parity: Option<String>,

    /// The Modbus data bits [values: 5, 6, 7, 8]
    #[arg(short, long)]
    pub data_bits: Option<u8>,

    /// The Modbus stop bits [values: 1, 2]
    #[arg(short, long)]
    pub stop_bits: Option<u8>,

    /// The timeout in milliseconds for each Modbus operation
    #[arg(id = "timeout", short, long, default_value_t = 3000)]
    pub timeout_ms: usize,

    /// The delay in milliseconds of first operation after connect
    #[arg(id = "delay", short, long, default_value_t = 0)]
    pub delay_ms: usize,

    /// The interval in milliseconds between successive operations
    #[arg(id = "interval", short, long, default_value_t = 0)]
    pub interval_ms: usize,
}
