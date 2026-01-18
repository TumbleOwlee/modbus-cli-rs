use crate::mem::memory::Memory;
use crate::mem::range::Range;
use crate::mem::{Request as MemRequest, Response as MemResponse};
use crate::net::rtu::Config;
use crate::sync::channel::DuplexChannel;
use crate::ui::{Request as UiRequest, Response as UiResponse};
use crate::util::str;
use crate::util::Chain;
use crate::LogMsg;

use std::{
    future,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::error::{SendError, TrySendError};
use tokio::sync::mpsc::Sender;
use tokio_modbus::prelude::{ExceptionCode, Request, Response, SlaveRequest};
use tokio_modbus::server::rtu::Server as RtuServer;
use tokio_modbus::SlaveId;
use tokio_serial::SerialStream;

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

pub struct Server {
    config: Config,
    memory: Option<DuplexChannel<MemRequest<SlaveId>, MemResponse>>,
    ui: Option<DuplexChannel<UiRequest, UiResponse>>,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            memory: None,
            ui: None,
        }
    }

    pub fn attach_ui(&mut self, channel: DuplexChannel<UiRequest, UiResponse>) -> () {
        self.ui = Some(channel);
    }

    pub fn attach_memory(
        &mut self,
        channel: DuplexChannel<MemRequest<SlaveId>, MemResponse>,
    ) -> () {
        self.memory = Some(channel);
    }

    pub async fn run(self) {
        let builder = tokio_serial::new(self.config.path.clone(), self.config.baud_rate);
        match SerialStream::open(&builder) {
            Ok(serial_stream) => {
                let ui_sender = self.ui.iter().map(|c| c.sender()).next();
                let server = RtuServer::new(serial_stream);
                let test = server.serve_forever(self).await;

                if let Err(e) = test {
                    if let Some(ref sender) = ui_sender {
                        sender
                            .send(UiRequest::Status(str!("Server not running.")))
                            .await;
                        sender
                            .send(UiRequest::LogError(format!(
                                "Server shut down unexpectedly ({})",
                                e
                            )))
                            .await;
                    };
                }
            }
            Err(e) => {
                if let Some(ref sender) = self.ui {
                    sender
                        .send(UiRequest::Status(str!("Server not running.")))
                        .await;
                    sender
                        .send(UiRequest::LogError(format!(
                            "Failed to open SerialStream ({})",
                            e
                        )))
                        .await;
                };
            }
        }
    }
}

impl tokio_modbus::server::Service for Server {
    type Request = SlaveRequest<'static>;
    type Exception = ExceptionCode;
    type Response = Response;
    type Future = future::Ready<Result<Response, ExceptionCode>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let SlaveRequest { slave, request } = req;
        match request {
            Request::ReadCoils(addr, cnt) => {
                if let Some(ref memory) = self.memory {
                    let range = Range::new(addr as usize, cnt as usize);
                    match memory.try_send(MemRequest::Read((slave as u8, range))) {
                        Ok(v) => {}
                        Err(e) => future::ready(Err(ExceptionCode::ServerDeviceFailure)),
                    }
                } else {
                    future::ready(Err(Self::Exception::IllegalDataAddress))
                }

                //self.memory
                //    .lock()
                //    .unwrap()
                //    .read(slave, &Range::new(addr, addr + cnt))
                //    .map_err(|e| {
                //        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                //            "Slave: {}, ReadCoils: [{:#06X}, {:#06X}) ({})",
                //            slave,
                //            addr,
                //            addr + cnt,
                //            e
                //        )));
                //        Self::Exception::IllegalDataAddress
                //    })
                //    .map(|v| {
                //        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                //            "Slave: {}, ReadCoils: [{:#06X}, {:#06X}) = {}",
                //            slave,
                //            addr,
                //            addr + cnt,
                //            refs_to_str(&v)
                //        )));
                //        Response::ReadCoils(v.into_iter().map(|b| *b != 0).collect())
                //    }),
            }
            Request::ReadDiscreteInputs(addr, cnt) => future::ready(
                self.memory
                    .lock()
                    .unwrap()
                    .read(slave, &Range::new(addr, addr + cnt))
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "Slave: {}, ReadDiscreteInputs: [{:#06X}, {:#06X}) ({})",
                            slave,
                            addr,
                            addr + cnt,
                            e
                        )));
                        Self::Exception::IllegalDataAddress
                    })
                    .map(|v| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "Slave: {}, ReadDiscreteInputs: [{:#06X}, {:#06X}) = {}",
                            slave,
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
                    .read(slave, &Range::new(addr, addr + cnt))
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "Slave: {}, ReadInputRegisters: [{:#06X}, {:#06X}) ({})",
                            slave,
                            addr,
                            addr + cnt,
                            e
                        )));
                        Self::Exception::IllegalDataAddress
                    })
                    .map(|v| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "Slave: {}, ReadInputRegisters: [{:#06X}, {:#06X}) = {}",
                            slave,
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
                    .read(slave, &Range::new(addr, addr + cnt))
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "Slave: {}, ReadHoldingRegisters: [{:#06X}, {:#06X}) ({})",
                            slave,
                            addr,
                            addr + cnt,
                            e
                        )));
                        Self::Exception::IllegalDataAddress
                    })
                    .map(|v| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "Slave: {}, ReadHoldingRegisters: [{:#06X}, {:#06X}) = {}",
                            slave,
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
                        slave,
                        Range::new(addr, addr + (values.len() as u16)),
                        &values,
                    )
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "Slave: {}, WriteMultipleRegisters: [{:#06X}, {:#06X}) ({})",
                            slave,
                            addr,
                            addr as usize + values.len(),
                            e
                        )));
                        Self::Exception::IllegalDataAddress
                    })
                    .map(|_| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "Slave: {}, WriteMultipleRegisters: [{:#06X}, {:#06X}) = {}",
                            slave,
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
                    .write(slave, Range::new(addr, addr + 1), &[value])
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "Slave: {}, WriteSingleRegister: [{:#06X}, {:#06X}) ({})",
                            slave,
                            addr,
                            addr + 1,
                            e
                        )));
                        Self::Exception::IllegalDataAddress
                    })
                    .map(|_| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "Slave: {}, WriteSingleRegister: [{:#06X}, {:#06X}) = {}",
                            slave,
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
                        .write(slave, Range::new(addr, addr + 1), &values)
                        .map_err(|e| {
                            let _ = self.log_sender.try_send(LogMsg::err(&format!(
                                "Slave: {}, WriteMultipleCoils: [{:#06X}, {:#06X}) ({})",
                                slave,
                                addr,
                                addr + 1,
                                e
                            )));
                            Self::Exception::IllegalDataAddress
                        })
                        .map(|_| {
                            let _ = self.log_sender.try_send(LogMsg::info(&format!(
                                "Slave: {}, WriteMultipleCoils: [{:#06X}, {:#06X}) = {}",
                                slave,
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
                        .write(slave, Range::new(addr, addr + 1), &[value])
                        .map_err(|e| {
                            let _ = self.log_sender.try_send(LogMsg::err(&format!(
                                "Slave: {}, WriteSingleCoil: [{:#06X}, {:#06X}) ({})",
                                slave,
                                addr,
                                addr + 1,
                                e
                            )));
                            Self::Exception::IllegalDataAddress
                        })
                        .map(|_| {
                            let _ = self.log_sender.try_send(LogMsg::info(&format!(
                                "Slave: {}, WriteSingleCoil: [{:#06X}, {:#06X}) = {}",
                                slave,
                                addr,
                                addr + 1,
                                value
                            )));
                            Response::WriteSingleCoil(addr, coil)
                        }),
                )
            }
            _ => future::ready(Err(Self::Exception::IllegalFunction)),
        }
    }
}
