mod memory;
mod register;
mod tcp;
mod test;
mod tokio;
mod types;
mod ui;
mod util;

use crate::memory::{Memory, Range};
use crate::register::{Address, Definition, Handler};
use crate::tcp::client::run as run_client;
use crate::tcp::server::run as run_server;
use crate::tcp::TcpConfig;
use crate::types::{Command, LogMsg, Status};
use crate::ui::App;
use crate::util::{str, Expect};

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::spawn_detach;
use tokio::sync::mpsc::channel;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the JSON configuration file providing the register definitions.
    config: String,

    /// Switch on verbose output.
    #[arg(short, long, default_value_t = false)]
    verbose: bool,

    /// The interface to use for the service or the ip to connect to in client mode.
    #[arg(short, long, default_value_t = str!("127.0.0.1"))]
    ip: String,

    /// The port to use for the service or the port to connect to on target host.
    #[arg(short, long, default_value_t = 502)]
    port: u16,

    /// Start as client instead of service.
    #[arg(short, long, default_value_t = false)]
    client: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ContiguousMemory {
    read_code: u8,
    range: Range<Address>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Config {
    history_length: usize,
    interval_ms: u64,
    contiguous_memory: Vec<ContiguousMemory>,
    definitions: HashMap<String, Definition>,
}

impl Config {
    /// Read register configuration from file
    pub fn read(path: &str) -> anyhow::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        serde_json::from_reader(reader).map_err(|e| e.into())
    }
}

fn main() {
    let args = Args::parse();

    // Read register definitions
    let config =
        Config::read(&args.config).panic(|e| format!("Failed to read configuration file. [{}]", e));

    // Initialize memory storage for all registers
    let mut memory = Memory::<1024, u16>::new();
    memory.init(
        &config
            .definitions
            .values()
            .map(|d| d.get_range())
            .collect::<Vec<_>>(),
    );
    let memory = Arc::new(Mutex::new(memory));

    let (status_send, status_recv) = channel::<Status>(10);
    let (log_send, log_recv) = channel::<LogMsg>(10);
    let (command_send, command_recv) = channel::<Command>(10);

    // Initialize tokio runtime for modbus server
    let runtime = Runtime::new().panic(|e| format!("Failed to create runtime. [{}]", e));

    let tcp_config = TcpConfig {
        port: args.port,
        ip: args.ip,
        interval_ms: config.interval_ms,
    };
    if args.client {
        let memory = memory.clone();
        let definitions = config.definitions.clone();
        let contiguous_memory = config.contiguous_memory.clone();
        let status_send = status_send.clone();
        runtime.block_on(async move {
            spawn_detach(async move {
                run_client(
                    tcp_config,
                    memory,
                    contiguous_memory,
                    definitions,
                    status_send,
                    command_recv,
                    log_send,
                )
                .await
            })
            .await
        });
    } else {
        let status_send = status_send.clone();
        let memory = memory.clone();
        runtime.block_on(async move {
            spawn_detach(async move { run_server(tcp_config, memory, status_send, log_send).await })
                .await
        });
    };

    // Initialize register handler
    let register_handler = Handler::new(&config.definitions, memory.clone());

    // Run UI
    let app = App::new(register_handler, config.history_length);
    app.run(status_recv, log_recv, command_send)
        .panic(|e| format!("Run app failed [{}]", e));
    //runtime.block_on(async { tokio::join_all().await });
}
