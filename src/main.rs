mod memory;
mod modbus;
mod register;
mod test;
mod tokio;
mod ui;
mod util;

use crate::memory::{Memory, Range};
use crate::modbus::Server;
use crate::register::{Definition, Handler};
use crate::ui::App;
use crate::util::{str, Expect};

use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::spawn_detach;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio_modbus::prelude::{Reader, Writer};
use tokio_modbus::server::tcp::{accept_tcp_connection, Server as TcpServer};

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

pub enum Status {
    String(String),
}

pub enum Command {
    Connect,
    Disconnect,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Config {
    interval_ms: u64,
    definitions: HashMap<String, Definition>,
}

fn main() {
    let args = Args::parse();

    // Read register definitions
    let config =
        read_config(&args.config).panic(|e| format!("Failed to read configuration file. [{}]", e));
    let definitions = config.definitions;

    // Initialize memory storage for all registers
    let mut memory = Memory::<1024, u16>::new();
    memory.init(
        &definitions
            .values()
            .map(|d| d.get_range())
            .collect::<Vec<_>>(),
    );
    let memory = Arc::new(Mutex::new(memory));

    let (status_send, status_recv) = channel::<Status>(10);
    let (command_send, command_recv) = channel::<Command>(10);

    // Initialize tokio runtime for modbus server
    let runtime = Runtime::new().panic(|e| format!("Failed to create runtime. [{}]", e));
    if args.client {
        let memory = memory.clone();
        let definitions = definitions.clone();
        let status_send = status_send.clone();
        runtime.block_on(async move {
            spawn_detach(async move {
                run_client(
                    args.ip,
                    args.port,
                    config.interval_ms,
                    memory,
                    definitions,
                    status_send,
                    command_recv,
                )
                .await
            })
            .await
        });
    } else {
        let status_send = status_send.clone();
        let memory = memory.clone();
        runtime.block_on(async move {
            spawn_detach(async move {
                run_server(args.ip, args.port, memory, status_send, command_recv).await
            })
            .await
        });
    };

    // Initialize register handler
    let register_handler = Handler::new(&definitions, memory.clone());

    // Run UI
    let app = App::new(register_handler);
    app.run(status_recv, command_send)
        .panic(|e| format!("Run app failed [{}]", e));
    //runtime.block_on(async { tokio::join_all().await });
}

/// Read register configuration from file
fn read_config(path: &str) -> anyhow::Result<Config> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader).map_err(|e| e.into())
}

/// Run modbus client
async fn run_client(
    ip: String,
    port: u16,
    interval_ms: u64,
    memory: Arc<Mutex<Memory<1024, u16>>>,
    definitions: HashMap<String, Definition>,
    status_send: Sender<Status>,
    mut command_recv: Receiver<Command>,
) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", ip, port).parse()?;
    let bounds = definitions.iter().fold((0xFFFFu16, 0x0000u16), |acc, def| {
        let addr = def.1.get_address();
        (std::cmp::min(acc.0, addr), std::cmp::max(acc.1, addr))
    });
    let mut lower_bound = bounds.0;
    let mut connection = tokio_modbus::client::tcp::connect(addr).await.ok();
    if connection.is_some() {
        let _ = status_send
            .send(Status::String(str!("Modbus TCP connected.")))
            .await;
    } else {
        let _ = status_send
            .send(Status::String(str!("Modbus TCP disconnected.")))
            .await;
    };

    let mut time_last_read = SystemTime::now()
        .checked_sub(Duration::from_millis(interval_ms + 1))
        .unwrap();
    loop {
        if let Some(ref mut context) = connection {
            let now = SystemTime::now();
            let res = now.duration_since(time_last_read);
            if res.is_ok_and(|d| d.as_millis() > interval_ms as u128) {
                time_last_read = now;
                if let Ok(vec) = context
                    .read_holding_registers(
                        lower_bound,
                        std::cmp::min(lower_bound + 127, bounds.1) - lower_bound,
                    )
                    .await
                {
                    let mut memory = memory.lock().unwrap();
                    memory
                        .write(
                            Range::new(lower_bound, lower_bound + vec.len() as u16),
                            &vec,
                        )
                        .panic(|e| format!("Failed to write to memory ({})", e));
                    drop(memory);
                    lower_bound = std::cmp::min(lower_bound + 127, bounds.1);
                    if lower_bound == bounds.1 {
                        lower_bound = bounds.0;
                    }
                } else {
                    let _ = status_send
                        .send(Status::String(str!("Modbus TCP disconnected.")))
                        .await;
                    connection = None;
                }
            }
            if let Ok(cmd) = command_recv.try_recv() {
                match cmd {
                    Command::Disconnect => {
                        let _ = status_send
                            .send(Status::String(str!("Modbus TCP disconnected.")))
                            .await;
                        connection = None;
                    }
                    Command::Connect => {}
                }
            }
        } else if let Ok(cmd) = command_recv.try_recv() {
            match cmd {
                Command::Connect => {
                    connection = tokio_modbus::client::tcp::connect(addr).await.ok();
                    if connection.is_some() {
                        let _ = status_send
                            .send(Status::String(str!("Modbus TCP connected.")))
                            .await;
                    }
                    lower_bound = bounds.0;
                }
                Command::Disconnect => {}
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    #[allow(unreachable_code)]
    Ok(())
}

/// Run modbus server to provide read and write operations
async fn run_server(
    ip: String,
    port: u16,
    memory: Arc<Mutex<Memory<1024, u16>>>,
    status_send: Sender<Status>,
    command_recv: Receiver<Command>,
) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", ip, port).parse()?;
    let listener = TcpListener::bind(addr)
        .await
        .panic(|e| format!("Failed to bind to address {}:{} [{}]", ip, port, e));
    let server = TcpServer::new(listener);
    let new_service = |_socket_addr| Ok(Some(Server::new(memory.clone())));
    let on_connected = |stream, socket_addr| async move {
        accept_tcp_connection(stream, socket_addr, new_service)
    };
    let on_process_error = |err| {
        eprintln!("{err}");
    };
    server
        .serve(&on_connected, on_process_error)
        .await
        .panic(|e| format!("Serve server failed [{}]", e));
    Ok(())
}
