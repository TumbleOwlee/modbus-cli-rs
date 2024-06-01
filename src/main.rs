mod mem;
mod msg;
mod tcp;
mod test;
mod ui;
mod util;
mod widgets;

use crate::mem::memory::{Memory, Range};
use crate::msg::{Command, LogMsg, Status};
use crate::mem::register::{Address, Definition, Handler};
use crate::tcp::client::run as run_client;
use crate::tcp::server::run as run_server;
use crate::tcp::TcpConfig;
use crate::ui::App;
use crate::util::tokio::spawn_detach;
use crate::util::{str, Expect};

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::default::Default;
use std::fs::File;
use std::io::BufReader;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::channel;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the JSON configuration file providing the register definitions.
    config: Option<String>,

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

impl Default for Config {
    fn default() -> Self {
        Self {
            history_length: 50,
            interval_ms: 500,
            contiguous_memory: Vec::new(),
            definitions: HashMap::new(),
        }
    }
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
    let config = if let Some(config) = &args.config {
        Config::read(config).panic(|e| format!("Failed to read configuration file. [{}]", e))
    } else {
        Config::default()
    };

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

    // Initialize tokio runtime for modbus server
    let runtime = Runtime::new().panic(|e| format!("Failed to create runtime. [{}]", e));

    let tcp_config = TcpConfig {
        port: args.port,
        ip: args.ip,
        interval_ms: config.interval_ms,
    };

    let mut command_send = None;

    if args.client {
        let (cmd_send, cmd_recv) = channel::<Command>(10);
        command_send = Some(cmd_send);
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
                    cmd_recv,
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

    let config = Arc::new(Mutex::new(config));

    // Initialize register handler
    let register_handler = Handler::new(config.clone(), memory.clone());

    // Run UI
    let app = App::new(register_handler, config);
    app.run(status_recv, log_recv, command_send)
        .panic(|e| format!("Run app failed [{}]", e));
    //runtime.block_on(async { tokio::join_all().await });
}
