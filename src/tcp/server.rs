use crate::memory::{Memory, Range};
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
use tokio_modbus::prelude::{Exception, Request, Response};
use tokio_modbus::server::tcp::{accept_tcp_connection, Server as TcpServer};

struct Server<const SLICE_SIZE: usize> {
    memory: Arc<Mutex<Memory<SLICE_SIZE, u16>>>,
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

impl<const SLICE_SIZE: usize> tokio_modbus::server::Service for Server<SLICE_SIZE> {
    type Request = Request<'static>;
    type Future = future::Ready<Result<Response, Exception>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        match req {
            Request::ReadInputRegisters(addr, cnt) => future::ready(
                self.memory
                    .lock()
                    .unwrap()
                    .read(&Range::new(addr, addr + cnt))
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "ReadInputRegisters: [{:#06X}, {:#06X}) ({})",
                            addr,
                            addr + cnt,
                            e
                        )));
                        Exception::IllegalDataAddress
                    })
                    .map(|v| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "ReadInputRegisters: [{:#06X}, {:#06X}) = {}",
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
                    .read(&Range::new(addr, addr + cnt))
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "ReadHoldingRegisters: [{:#06X}, {:#06X}) ({})",
                            addr,
                            addr + cnt,
                            e
                        )));
                        Exception::IllegalDataAddress
                    })
                    .map(|v| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "ReadHoldingRegisters: [{:#06X}, {:#06X}) = {}",
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
                    .write(Range::new(addr, addr + (values.len() as u16)), &values)
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "WriteMultipleRegisters: [{:#06X}, {:#06X}) ({})",
                            addr,
                            addr as usize + values.len(),
                            e
                        )));
                        Exception::IllegalDataAddress
                    })
                    .map(|_| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "WriteMultipleRegisters: [{:#06X}, {:#06X}) = {}",
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
                    .write(Range::new(addr, addr + 1), &[value])
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "WriteSingleRegister: [{:#06X}, {:#06X}) ({})",
                            addr,
                            addr + 1,
                            e
                        )));
                        Exception::IllegalDataAddress
                    })
                    .map(|_| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "WriteSingleRegister: [{:#06X}, {:#06X}) = {}",
                            addr,
                            addr + 1,
                            value
                        )));
                        Response::WriteSingleRegister(addr, value)
                    }),
            ),
            _ => future::ready(Err(Exception::IllegalFunction)),
        }
    }
}

impl<const SLICE_SIZE: usize> Server<SLICE_SIZE> {
    pub fn new(memory: Arc<Mutex<Memory<SLICE_SIZE, u16>>>, log_sender: Sender<LogMsg>) -> Self {
        Self { memory, log_sender }
    }
}

/// Run modbus server to provide read and write operations
pub async fn run(
    config: TcpConfig,
    memory: Arc<Mutex<Memory<1024, u16>>>,
    status_send: Sender<Status>,
    log_send: Sender<LogMsg>,
) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.ip, config.port).parse()?;
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
                config.ip, config.port
            )))
            .await;
    }
    Ok(())
}
