use crate::config::FileType;
use crate::net::rtu::Config as RtuConfig;
use crate::net::tcp::Config as TcpConfig;
use crate::util::str;

use clap::{Parser, Subcommand};

#[derive(Parser)]
pub struct Format {
    /// The interface to use for the service or the ip to connect to in client mode.
    #[arg(value_enum)]
    file_type: FileType,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Use TCP connection
    Tcp(TcpConfig),

    /// Use RTU connection
    Rtu(RtuConfig),

    /// Convert configuration file to other type
    Convert(Format),
}

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct ArgParser {
    /// Path to the JSON configuration file providing the register definitions.
    #[arg(long)]
    pub config: Option<String>,

    /// Switch on verbose output.
    #[arg(short, long, default_value_t = false)]
    pub verbose: bool,

    /// Start as client instead of service.
    #[arg(long, default_value_t = false)]
    pub client: bool,

    #[command(subcommand)]
    pub command: Commands,
}
