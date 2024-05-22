use crate::memory::{Memory, Range};
use crate::register::{Definition, Type};
use crate::tcp::TcpConfig;
use crate::util::{str, Expect};
use crate::types::LogMsg;
use crate::{Command, Status};

use itertools::Itertools;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_modbus::prelude::Reader;
use tokio_modbus::FunctionCode;

/// Run modbus client
pub async fn run(
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
