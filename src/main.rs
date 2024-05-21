mod memory;
mod modbus;
mod register;
mod test;
mod tokio;
mod ui;
mod util;

use crate::memory::{Memory, Range};
use crate::modbus::Server;
use crate::register::{Definition, Handler, Type};
use crate::ui::App;
use crate::util::{str, Expect};

use chrono::Local;
use clap::Parser;
use itertools::Itertools;
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
use tokio_modbus::prelude::Reader;
use tokio_modbus::server::tcp::{accept_tcp_connection, Server as TcpServer};
use tokio_modbus::FunctionCode;

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

#[derive(Clone, Debug)]
pub struct TcpConfig {
    pub port: u16,
    pub ip: String,
    pub interval_ms: u64,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub timestamp: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub enum LogMsg {
    Err(Message),
    Ok(Message),
    Info(Message),
}

impl LogMsg {
    pub fn info(msg: &str) -> LogMsg {
        Self::Info(Message {
            timestamp: format!("{}", Local::now().format("[ %d:%m:%Y | %H:%M:%S ]")),
            message: str!(msg),
        })
    }

    pub fn err(msg: &str) -> LogMsg {
        Self::Err(Message {
            timestamp: format!("{}", Local::now().format("[ %d:%m:%Y | %H:%M:%S ]")),
            message: str!(msg),
        })
    }

    pub fn ok(msg: &str) -> LogMsg {
        Self::Ok(Message {
            timestamp: format!("{}", Local::now().format("[ %d:%m:%Y | %H:%M:%S ]")),
            message: str!(msg),
        })
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Config {
    history_length: usize,
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
    let (log_send, log_recv) = channel::<LogMsg>(10);
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
                    TcpConfig {
                        port: args.port,
                        ip: args.ip,
                        interval_ms: config.interval_ms,
                    },
                    memory,
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
            spawn_detach(async move {
                run_server(args.ip, args.port, memory, status_send, log_send).await
            })
            .await
        });
    };

    // Initialize register handler
    let register_handler = Handler::new(&definitions, memory.clone());

    // Run UI
    let app = App::new(register_handler, config.history_length);
    app.run(status_recv, log_recv, command_send)
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
    tcp_config: TcpConfig,
    memory: Arc<Mutex<Memory<1024, u16>>>,
    definitions: HashMap<String, Definition>,
    status_send: Sender<Status>,
    mut command_recv: Receiver<Command>,
    log_send: Sender<LogMsg>,
) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", tcp_config.ip, tcp_config.port).parse()?;
    let mut connection = tokio_modbus::client::tcp::connect(addr).await.ok();
    if connection.is_some() {
        let _ = status_send
            .send(Status::String(str!("Modbus TCP connected.")))
            .await;
        let _ = log_send
            .send(LogMsg::ok(&format!(
                "Modbus TCP connected to {}:{}",
                tcp_config.ip, tcp_config.port
            )))
            .await;
    } else {
        let _ = status_send
            .send(Status::String(str!("Modbus TCP disconnected.")))
            .await;
        let _ = log_send
            .send(LogMsg::err(&format!(
                "Modbus TCP failed to connect to {}:{}",
                tcp_config.ip, tcp_config.port
            )))
            .await;
    };

    let mut sorted_defs = definitions
        .iter()
        .sorted_by(|a, b| {
            a.1.read_code()
                .cmp(&b.1.read_code())
                .then(a.1.get_address().cmp(&b.1.get_address()))
        })
        .collect::<Vec<_>>();
    let marker = (str!(""), Definition::new(0, 0, Type::U8, 0));
    sorted_defs.push((&marker.0, &marker.1));
    let mut fc = 0;
    let mut operations = Vec::new();
    let mut range: Option<Range<u16>> = None;
    for (name, def) in sorted_defs.into_iter() {
        if range.is_some()
            && (fc != def.read_code() || def.get_address() != range.as_ref().unwrap().to() as u16)
        {
            let fc = FunctionCode::new(fc);
            match fc {
                FunctionCode::ReadCoils => {
                    unimplemented!("Read Coils")
                }
                FunctionCode::ReadDiscreteInputs => {
                    unimplemented!("Read Discrete Inputs")
                }
                FunctionCode::ReadInputRegisters | FunctionCode::ReadHoldingRegisters => {
                    if let Some(r) = range {
                        let len = r.length();
                        if len > 0 {
                            let mut addr = r.from();
                            loop {
                                operations.push((
                                    fc,
                                    Range::new(addr, std::cmp::min(addr + 127, r.to())),
                                ));
                                addr = std::cmp::min(addr + 127, r.to());
                                if addr == r.to() {
                                    break;
                                }
                            }
                        }
                    }
                }
                _ => panic!("Invalid read function code for register {}", name),
            };
            range = Some(Range::new(def.get_address(), def.get_address()));
        }
        fc = def.read_code();
        range = match range {
            None => Some(def.get_range()),
            Some(v) => Some(Range::new(v.from() as u16, def.get_range().to() as u16)),
        };
    }

    let mut time_last_read = SystemTime::now()
        .checked_sub(Duration::from_millis(tcp_config.interval_ms + 1))
        .unwrap();
    let mut op_idx = 0;
    loop {
        if let Some(ref mut context) = connection {
            let now = SystemTime::now();
            let res = now.duration_since(time_last_read);
            if res.is_ok_and(|d| d.as_millis() > tcp_config.interval_ms as u128) {
                time_last_read = now;
                let (fc, op) = operations.get(op_idx).unwrap();
                let modbus_result = match fc {
                    FunctionCode::ReadInputRegisters => {
                        context
                            .read_input_registers(op.from() as u16, (op.to() - op.from()) as u16)
                            .await
                    }
                    FunctionCode::ReadHoldingRegisters => {
                        context
                            .read_holding_registers(op.from() as u16, (op.to() - op.from()) as u16)
                            .await
                    }
                    _ => panic!("Invalid function code in operation."),
                };
                if let Ok(vec) = modbus_result {
                    let _ = log_send
                        .send(LogMsg::info(&format!(
                            "Read address space [ {:#06X} ({}), {:#06X} ({}) ) successful.",
                            op.from(),
                            op.from(),
                            op.to(),
                            op.to()
                        )))
                        .await;
                    let mut memory = memory.lock().unwrap();
                    memory
                        .write(Range::new(op.from(), op.from() + vec.len()), &vec)
                        .panic(|e| format!("Failed to write to memory ({})", e));
                    drop(memory);
                    op_idx = if op_idx + 1 == operations.len() {
                        0
                    } else {
                        op_idx + 1
                    };
                } else {
                    let _ = log_send
                        .send(LogMsg::err(&format!(
                            "Read address space [ {:#06X} ({}), {:#06X} ({}) ) failed.",
                            op.from(),
                            op.from(),
                            op.to(),
                            op.to()
                        )))
                        .await;
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
                        let _ = log_send
                            .send(LogMsg::ok(&format!(
                                "Modbus TCP disconnected from {}:{}",
                                tcp_config.ip, tcp_config.port
                            )))
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
                        let _ = log_send
                            .send(LogMsg::ok(&format!(
                                "Modbus TCP connected successfully to {}:{}",
                                tcp_config.ip, tcp_config.port
                            )))
                            .await;
                    } else {
                        let _ = log_send
                            .send(LogMsg::err(&format!(
                                "Modbus TCP failed to connect to {}:{}",
                                tcp_config.ip, tcp_config.port
                            )))
                            .await;
                    }
                    op_idx = 0;
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
    log_send: Sender<LogMsg>,
) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", ip, port).parse()?;
    if let Ok(listener) = TcpListener::bind(addr).await {
        let server = TcpServer::new(listener);
        let new_service = |_socket_addr| Ok(Some(Server::new(memory.clone(), log_send.clone())));
        let on_connected = |stream, socket_addr| async move {
            accept_tcp_connection(stream, socket_addr, new_service)
        };
        let on_process_log = log_send.clone();
        let on_process_error = move |err| {
            let _ = on_process_log
                .try_send(LogMsg::err(&format!("Server processing failed. [{}]", err)));
        };
        server
            .serve(&on_connected, on_process_error)
            .await
            .panic(|e| format!("Serve server failed [{}]", e));
    } else {
        let _ = status_send
            .send(Status::String(str!("Server not running.")))
            .await;
        let _ = log_send
            .send(LogMsg::err(&format!(
                "Failed to bind to address {}:{}. Please restart.",
                ip, port
            )))
            .await;
    }
    Ok(())
}
