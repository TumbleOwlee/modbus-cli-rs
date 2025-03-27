use crate::mem::memory::{Memory, Range};
use crate::tcp::TcpConfig;
use crate::util::str;
use crate::util::Expect;
use crate::LogMsg;
use crate::Status;

use std::net::SocketAddr;
use std::{
    future,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;
use tokio::sync::mpsc::Sender;
use tokio_modbus::prelude::{Exception, Request, Response, Slave};
use tokio_modbus::server::tcp::{accept_tcp_connection, Server as TcpServer};

struct Service {
    memory: Arc<Mutex<Memory>>,
    log_sender: Sender<LogMsg>,
}

fn refs_to_str(values: &[&u16]) -> String {
    let mut s = str!("[ ");
    for i in 0..values.len() {
        if i == values.len() - 1 {
            s += &format!("{:#06X} ", values[i]);
        } else {
            s += &format!("{:#06X}, ", values[i]);
        }
    }
    s + "]"
}

fn to_str(values: &[u16]) -> String {
    let mut s = str!("[ ");
    for i in 0..values.len() {
        if i == values.len() - 1 {
            s += &format!("{:#06X} ", values[i]);
        } else {
            s += &format!("{:#06X}, ", values[i]);
        }
    }
    s + "]"
}

impl tokio_modbus::server::Service for Service {
    type Request = Request<'static>;
    type Future = future::Ready<Result<Response, Exception>>;

    fn call(&self, slave: Slave, req: Self::Request) -> Self::Future {
        match req {
            Request::ReadCoils(addr, cnt) => future::ready(
                self.memory
                    .lock()
                    .unwrap()
                    .read(slave.0, &Range::new(addr, addr + cnt))
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "Slave: {}, ReadCoils: [{:#06X}, {:#06X}) ({})",
                            slave.0,
                            addr,
                            addr + cnt,
                            e
                        )));
                        Exception::IllegalDataAddress
                    })
                    .map(|v| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "Slave: {}, ReadCoils: [{:#06X}, {:#06X}) = {}",
                            slave.0,
                            addr,
                            addr + cnt,
                            refs_to_str(&v)
                        )));
                        Response::ReadCoils(v.into_iter().map(|b| *b != 0).collect())
                    }),
            ),
            Request::ReadDiscreteInputs(addr, cnt) => future::ready(
                self.memory
                    .lock()
                    .unwrap()
                    .read(slave.0, &Range::new(addr, addr + cnt))
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "Slave: {}, ReadDiscreteInputs: [{:#06X}, {:#06X}) ({})",
                            slave.0,
                            addr,
                            addr + cnt,
                            e
                        )));
                        Exception::IllegalDataAddress
                    })
                    .map(|v| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "Slave: {}, ReadDiscreteInputs: [{:#06X}, {:#06X}) = {}",
                            slave.0,
                            addr,
                            addr + cnt,
                            refs_to_str(&v)
                        )));
                        Response::ReadDiscreteInputs(v.into_iter().map(|b| *b != 0).collect())
                    }),
            ),
            Request::ReadInputRegisters(addr, cnt) => future::ready(
                self.memory
                    .lock()
                    .unwrap()
                    .read(slave.0, &Range::new(addr, addr + cnt))
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "Slave: {}, ReadInputRegisters: [{:#06X}, {:#06X}) ({})",
                            slave.0,
                            addr,
                            addr + cnt,
                            e
                        )));
                        Exception::IllegalDataAddress
                    })
                    .map(|v| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "Slave: {}, ReadInputRegisters: [{:#06X}, {:#06X}) = {}",
                            slave.0,
                            addr,
                            addr + cnt,
                            refs_to_str(&v)
                        )));
                        Response::ReadInputRegisters(v.into_iter().copied().collect())
                    }),
            ),
            Request::ReadHoldingRegisters(addr, cnt) => future::ready(
                self.memory
                    .lock()
                    .unwrap()
                    .read(slave.0, &Range::new(addr, addr + cnt))
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "Slave: {}, ReadHoldingRegisters: [{:#06X}, {:#06X}) ({})",
                            slave.0,
                            addr,
                            addr + cnt,
                            e
                        )));
                        Exception::IllegalDataAddress
                    })
                    .map(|v| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "Slave: {}, ReadHoldingRegisters: [{:#06X}, {:#06X}) = {}",
                            slave.0,
                            addr,
                            addr + cnt,
                            refs_to_str(&v)
                        )));
                        Response::ReadHoldingRegisters(v.into_iter().copied().collect())
                    }),
            ),
            Request::WriteMultipleRegisters(addr, values) => future::ready(
                self.memory
                    .lock()
                    .unwrap()
                    .write(
                        slave.0,
                        Range::new(addr, addr + (values.len() as u16)),
                        &values,
                    )
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "Slave: {}, WriteMultipleRegisters: [{:#06X}, {:#06X}) ({})",
                            slave.0,
                            addr,
                            addr as usize + values.len(),
                            e
                        )));
                        Exception::IllegalDataAddress
                    })
                    .map(|_| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "Slave: {}, WriteMultipleRegisters: [{:#06X}, {:#06X}) = {}",
                            slave.0,
                            addr,
                            addr as usize + values.len(),
                            to_str(&values)
                        )));
                        Response::WriteMultipleRegisters(addr, values.len() as u16)
                    }),
            ),
            Request::WriteSingleRegister(addr, value) => future::ready(
                self.memory
                    .lock()
                    .unwrap()
                    .write(slave.0, Range::new(addr, addr + 1), &[value])
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "Slave: {}, WriteSingleRegister: [{:#06X}, {:#06X}) ({})",
                            slave.0,
                            addr,
                            addr + 1,
                            e
                        )));
                        Exception::IllegalDataAddress
                    })
                    .map(|_| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "Slave: {}, WriteSingleRegister: [{:#06X}, {:#06X}) = {}",
                            slave.0,
                            addr,
                            addr + 1,
                            value
                        )));
                        Response::WriteSingleRegister(addr, value)
                    }),
            ),
            Request::WriteMultipleCoils(addr, coils) => {
                let values: Vec<u16> = coils.iter().map(|v| if *v { 1 } else { 0 }).collect();
                future::ready(
                    self.memory
                        .lock()
                        .unwrap()
                        .write(slave.0, Range::new(addr, addr + 1), &values)
                        .map_err(|e| {
                            let _ = self.log_sender.try_send(LogMsg::err(&format!(
                                "Slave: {}, WriteMultipleCoils: [{:#06X}, {:#06X}) ({})",
                                slave.0,
                                addr,
                                addr + 1,
                                e
                            )));
                            Exception::IllegalDataAddress
                        })
                        .map(|_| {
                            let _ = self.log_sender.try_send(LogMsg::info(&format!(
                                "Slave: {}, WriteMultipleCoils: [{:#06X}, {:#06X}) = {}",
                                slave.0,
                                addr,
                                addr + 1,
                                to_str(&values)
                            )));
                            Response::WriteMultipleCoils(addr, values.len() as u16)
                        }),
                )
            }
            Request::WriteSingleCoil(addr, coil) => {
                let value = if coil { 1 } else { 0 };
                future::ready(
                    self.memory
                        .lock()
                        .unwrap()
                        .write(slave.0, Range::new(addr, addr + 1), &[value])
                        .map_err(|e| {
                            let _ = self.log_sender.try_send(LogMsg::err(&format!(
                                "Slave: {}, WriteSingleCoil: [{:#06X}, {:#06X}) ({})",
                                slave.0,
                                addr,
                                addr + 1,
                                e
                            )));
                            Exception::IllegalDataAddress
                        })
                        .map(|_| {
                            let _ = self.log_sender.try_send(LogMsg::info(&format!(
                                "Slave: {}, WriteSingleCoil: [{:#06X}, {:#06X}) = {}",
                                slave.0,
                                addr,
                                addr + 1,
                                value
                            )));
                            Response::WriteSingleCoil(addr, coil)
                        }),
                )
            }
            _ => {
                let _ = self.log_sender.try_send(LogMsg::err(&format!(
                    "Slave: {} (Illegal Function)",
                    slave.0,
                )));
                future::ready(Err(Exception::IllegalFunction))
            }
        }
    }
}

impl Service {
    pub fn new(memory: Arc<Mutex<Memory>>, log_sender: Sender<LogMsg>) -> Self {
        Self { memory, log_sender }
    }
}

pub struct Server {
    config: TcpConfig,
    memory: Arc<Mutex<Memory>>,
    status_sender: Sender<Status>,
    log_sender: Sender<LogMsg>,
}

impl Server {
    pub fn new(
        config: TcpConfig,
        memory: Arc<Mutex<Memory>>,
        status_sender: Sender<Status>,
        log_sender: Sender<LogMsg>,
    ) -> Self {
        Self {
            config,
            memory,
            status_sender,
            log_sender,
        }
    }

    pub async fn run(&self) {
        let addr: SocketAddr = format!("{}:{}", self.config.ip, self.config.port)
            .parse()
            .panic(|e| format!("Failed to create SocketAddr ({e})"));
        if let Ok(listener) = TcpListener::bind(addr).await {
            let server = TcpServer::new(listener);
            let new_request_handler = |_socket_addr| {
                Ok(Some(Service::new(
                    self.memory.clone(),
                    self.log_sender.clone(),
                )))
            };
            let on_connected = |stream, socket_addr| async move {
                accept_tcp_connection(stream, socket_addr, new_request_handler)
            };
            let on_process_log = self.log_sender.clone();
            let on_process_error = move |err| {
                let _ = on_process_log
                    .try_send(LogMsg::err(&format!("Server processing failed. [{}]", err)));
            };
            server
                .serve(&on_connected, on_process_error)
                .await
                .panic(|e| format!("Serve server failed [{}]", e));
        } else {
            let _ = self
                .status_sender
                .send(Status::String(str!("Server not running.")))
                .await;
            let _ = self
                .log_sender
                .send(LogMsg::err(&format!(
                    "Failed to bind to address {}:{}. Please restart.",
                    self.config.ip, self.config.port
                )))
                .await;
        }
    }
}
