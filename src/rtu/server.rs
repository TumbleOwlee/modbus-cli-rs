use crate::mem::memory::{Memory, Range};
use crate::rtu::RtuConfig;
use crate::util::str;
use crate::LogMsg;
use crate::Status;

use std::{
    future,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::Sender;
use tokio_modbus::prelude::{ExceptionCode, Request, Response, SlaveRequest};
use tokio_modbus::server::rtu::Server as RtuServer;
use tokio_serial::{DataBits, Parity, SerialPortBuilder, SerialStream, StopBits};

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
    type Request = SlaveRequest<'static>;
    type Exception = ExceptionCode;
    type Response = Response;
    type Future = future::Ready<Result<Response, ExceptionCode>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let SlaveRequest { slave, request } = req;
        match request {
            Request::ReadCoils(addr, cnt) => future::ready(
                self.memory
                    .lock()
                    .expect("Unable to lock memory")
                    .read(slave, &Range::new(addr, addr + cnt))
                    .map_err(|e| {
                        let _ = self.log_sender.try_send(LogMsg::err(&format!(
                            "Slave: {}, ReadCoils: [{:#06X}, {:#06X}) ({})",
                            slave,
                            addr,
                            addr + cnt,
                            e
                        )));
                        Self::Exception::IllegalDataAddress
                    })
                    .map(|v| {
                        let _ = self.log_sender.try_send(LogMsg::info(&format!(
                            "Slave: {}, ReadCoils: [{:#06X}, {:#06X}) = {}",
                            slave,
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
                    .expect("Unable to lock memory")
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
                    .expect("Unable to lock memory")
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
                    .expect("Unable to lock memory")
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
                    .expect("Unable to lock memory")
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
                    .expect("Unable to lock memory")
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
                        .expect("Unable to lock memory")
                        .write(slave, Range::new(addr, addr + values.len() as u16), &values)
                        .map_err(|e| {
                            let _ = self.log_sender.try_send(LogMsg::err(&format!(
                                "Slave: {}, WriteMultipleCoils: [{:#06X}, {:#06X}) ({})",
                                slave,
                                addr,
                                addr + values.len() as u16,
                                e
                            )));
                            Self::Exception::IllegalDataAddress
                        })
                        .map(|_| {
                            let _ = self.log_sender.try_send(LogMsg::info(&format!(
                                "Slave: {}, WriteMultipleCoils: [{:#06X}, {:#06X}) = {}",
                                slave,
                                addr,
                                addr + values.len() as u16,
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
                        .expect("Unable to lock memory")
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

    async fn create_serial_builder(&self) -> SerialPortBuilder {
        let mut builder = tokio_serial::new(self.config.path.clone(), self.config.baud_rate);
        let data_bits = self.config.data_bits.unwrap_or(8);
        let stop_bits = self.config.stop_bits.unwrap_or(1);
        let parity = self
            .config
            .parity
            .as_ref()
            .unwrap_or(&"NONE".to_string())
            .to_uppercase();
        let flow_control = self
            .config
            .flow_control
            .as_ref()
            .unwrap_or(&crate::rtu::FlowControl::None);

        builder = builder.data_bits(match data_bits {
            5 => DataBits::Five,
            6 => DataBits::Six,
            7 => DataBits::Seven,
            8 => DataBits::Eight,
            _ => panic!("Invalid data bits specified."),
        });

        builder = builder.stop_bits(match stop_bits {
            1 => StopBits::One,
            2 => StopBits::Two,
            _ => panic!("Invalid stop bits specified"),
        });

        if parity == "ODD" {
            builder = builder.parity(Parity::Odd);
        } else if parity == "EVEN" {
            builder = builder.parity(Parity::Even);
        } else if parity == "NONE" {
            builder = builder.parity(Parity::None);
        } else {
            panic!("Invalid parity specified");
        }

        builder = builder.flow_control(match flow_control {
            crate::rtu::FlowControl::None => tokio_serial::FlowControl::None,
            crate::rtu::FlowControl::Software => tokio_serial::FlowControl::Software,
            crate::rtu::FlowControl::Hardware => tokio_serial::FlowControl::Hardware,
        });

        builder
    }

    fn config_as_str(&self) -> String {
        let path = &self.config.path;
        let baud_rate = self.config.baud_rate;
        let data_bits = self.config.data_bits.unwrap_or(8);
        let stop_bits = self.config.stop_bits.unwrap_or(1);
        let parity = self
            .config
            .parity
            .as_ref()
            .unwrap_or(&"NONE".to_string())
            .to_uppercase();
        let flow_control = self
            .config
            .flow_control
            .as_ref()
            .unwrap_or(&crate::rtu::FlowControl::None);
        format!(
            "{}, baud rate: {}, data bits: {}, parity: {}, stop bits: {}, flow control: {}",
            path, baud_rate, data_bits, parity, stop_bits, flow_control
        )
    }

    pub async fn run(&self) {
        let builder = self.create_serial_builder().await;

        match SerialStream::open(&builder) {
            Ok(serial_stream) => {
                let server = RtuServer::new(serial_stream);
                let service = Service::new(self.memory.clone(), self.log_sender.clone());

                let _ = self
                    .log_sender
                    .send(LogMsg::ok(&format!(
                        "Successfully attached to serial port {}.",
                        self.config_as_str()
                    )))
                    .await;

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
                    .send(LogMsg::err(&format!(
                        "Failed to open SerialStream {} ({})",
                        self.config_as_str(),
                        e
                    )))
                    .await;
            }
        }
    }
}
