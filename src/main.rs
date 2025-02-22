mod mem;
mod msg;
mod rtu;
mod tcp;
mod test;
mod ui;
mod util;
mod widgets;

use crate::mem::memory::{Memory, Range};
use crate::mem::register::{Address, Definition, Handler, Value};
use crate::msg::{Command, LogMsg, Status};
use crate::rtu::client::Client as RtuClient;
use crate::rtu::server::Server as RtuServer;
use crate::rtu::RtuConfig;
use crate::tcp::client::Client as TcpClient;
use crate::tcp::server::Server as TcpServer;
use crate::tcp::TcpConfig;
use crate::ui::App;
use crate::util::tokio::spawn_detach;
use crate::util::{async_cloned, str, Expect};

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::default::Default;
use std::fs::File;
use std::io::BufReader;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::channel;
use tokio_modbus::prelude::SlaveId;

#[derive(Subcommand)]
enum Commands {
    /// Use TCP connection
    Tcp(TcpConfig),

    /// Use RTU connection
    Rtu(RtuConfig),
}

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to the JSON configuration file providing the register definitions.
    #[arg(long)]
    config: Option<String>,

    /// Switch on verbose output.
    #[arg(short, long, default_value_t = false)]
    verbose: bool,

    /// Start as client instead of service.
    #[arg(long, default_value_t = false)]
    client: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ContiguousMemory {
    slave_id: Option<SlaveId>,
    read_code: u8,
    range: Range<Address>,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct AppConfig {
    history_length: usize,
    interval_ms: u64,
    contiguous_memory: Vec<ContiguousMemory>,
    definitions: HashMap<String, Definition>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            history_length: 50,
            interval_ms: 500,
            contiguous_memory: Vec::new(),
            definitions: HashMap::new(),
        }
    }
}

impl AppConfig {
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
    let app_config = args
        .config
        .map(|p| {
            AppConfig::read(&p).panic(|e| format!("Failed to read configuration file. [{}]", e))
        })
        .unwrap_or(AppConfig::default());
    let interval_ms = app_config.interval_ms;

    // Initialize memory storage for all registers
    let mut memory = Memory::new();
    let map = app_config.definitions.values().fold(
        HashMap::new(),
        |mut f: HashMap<SlaveId, Vec<Range<_>>>, d| {
            f.entry(d.get_slave_id().unwrap_or(0))
                .or_default()
                .push(d.get_range());
            f
        },
    );
    for (slave, ranges) in map.into_iter() {
        memory.init(slave, &ranges);
    }
    let memory = Arc::new(Mutex::new(memory));
    let app_config = Arc::new(Mutex::new(app_config));

    let (status_sender, status_receiver) = channel::<Status>(10);
    let (log_sender, log_receiver) = channel::<LogMsg>(10);
    let (cmd_sender, cmd_receiver) = channel::<Command>(10);

    // Initialize tokio runtime for modbus server
    let runtime = Runtime::new().panic(|e| format!("Failed to create runtime. [{}]", e));

    if args.client {
        match args.command {
            Commands::Tcp(config) => {
                runtime.block_on(async_cloned!(interval_ms, app_config, memory; {
                    spawn_detach(async move {
                        let mut client = TcpClient::new(app_config, config, memory, status_sender, cmd_receiver, log_sender);
                        client.run(interval_ms).await
                    })
                    .await
                }));
            }
            Commands::Rtu(config) => {
                runtime.block_on(async_cloned!(interval_ms, app_config, memory; {
                    spawn_detach(async move {
                        let mut client = RtuClient::new(app_config, config, memory, status_sender, cmd_receiver, log_sender);
                        client.run(interval_ms).await
                    })
                    .await
                }));
            }
        }
    } else {
        match args.command {
            Commands::Tcp(config) => {
                runtime.block_on(async_cloned!(memory; {
                    spawn_detach(async move {
                        let server = TcpServer::new(config, memory, status_sender, log_sender);
                        server.run().await
                    })
                    .await
                }));
            }
            Commands::Rtu(config) => {
                runtime.block_on(async_cloned!(memory; {
                    spawn_detach(async move {
                        let server = RtuServer::new(config, memory, status_sender, log_sender);
                        server.run().await
                    })
                    .await
                }));
            }
        }
    };

    // Initialize register handler
    let register_handler = Handler::new(app_config.clone(), memory.clone());
    for def in app_config.lock().unwrap().definitions.values() {
        if let Some(value) = def.get_default() {
            let s: String = match value {
                Value::Str(v) => v.to_string(),
                Value::Num(v) => format!("{}", v),
            };
            if let Ok(v) = def.get_type().encode(&s) {
                if memory
                    .lock()
                    .unwrap()
                    .write(def.get_slave_id().unwrap_or(0), def.get_range(), &v)
                    .is_err()
                {}
            }
        }
    }

    // Run UI
    let app = App::new(register_handler, app_config);
    let cmd_sender = if args.client { Some(cmd_sender) } else { None };
    app.run(status_receiver, log_receiver, cmd_sender)
        .panic(|e| format!("Run app failed [{}]", e));
    //runtime.block_on(async { crate::util::tokio::join_all().await });
}
