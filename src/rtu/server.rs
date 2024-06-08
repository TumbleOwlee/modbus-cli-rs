use crate::mem::memory::{Memory, Range};
use crate::rtu::RtuConfig;
use crate::util::str;
use crate::util::Expect;
use crate::LogMsg;
use crate::Status;

use std::{
    future,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::Sender;
use tokio_modbus::prelude::{Exception, Request, Response};
use tokio_modbus::server::rtu::Server as RtuServer;
use tokio_serial::SerialStream;

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

impl Service {
    pub fn new(memory: Arc<Mutex<Memory>>, log_sender: Sender<LogMsg>) -> Self {
        Self { memory, log_sender }
    }
}

pub struct Server {
    config: RtuConfig,
    memory: Arc<Mutex<Memory>>,
    status_sender: Sender<Status>,
    log_sender: Sender<LogMsg>,
}

impl Server {
    pub fn new(
        config: RtuConfig,
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
        let builder = tokio_serial::new(self.config.path.clone(), self.config.baud_rate);
        match SerialStream::open(&builder) {
            Ok(serial_stream) => {
                let server = RtuServer::new(serial_stream);
                let service = Service::new(self.memory.clone(), self.log_sender.clone());

                if let Err(e) = server.serve_forever(service).await {
                    let _ = self
                        .status_sender
                        .send(Status::String(str!("Server not running.")))
                        .await;
                    let _ = self
                        .log_sender
                        .send(LogMsg::err(&format!("Server shut down unexpectedly ({e})")))
                        .await;
                }
            }
            Err(e) => {
                let _ = self
                    .status_sender
                    .send(Status::String(str!("Server not running.")))
                    .await;
                let _ = self
                    .log_sender
                    .send(LogMsg::err(&format!("Failed to open SerialStream ({e})")))
                    .await;
            }
        }
    }
}
