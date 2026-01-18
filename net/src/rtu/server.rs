// Crate
use crate::Key;
use crate::rtu::Config;

// Workspace
use memory::Range;
use memory::memory::Memory;

// External
use anyhow::anyhow;
use std::sync::Arc;
use std::{future, sync::RwLock};
use tokio::task::JoinHandle;
use tokio_modbus::Request;
use tokio_modbus::prelude::{ExceptionCode, Response, SlaveRequest};
use tokio_modbus::server::rtu::Server as RtuServer;
use tokio_serial::{DataBits, Parity, SerialStream, StopBits};

pub struct ServerBuilder {
    config: Arc<RwLock<Config>>,
    memory: Arc<RwLock<Memory<Key>>>,
}

impl ServerBuilder {
    pub fn new(config: Arc<RwLock<Config>>, memory: Arc<RwLock<Memory<Key>>>) -> Self {
        Self { config, memory }
    }

    pub async fn spawn(&self) -> Result<JoinHandle<Result<(), anyhow::Error>>, anyhow::Error> {
        match self.config.read() {
            Ok(guard) => Server::run(&guard, self.memory.clone()).await,
            Err(e) => Err(anyhow!("{}", e)),
        }
    }
}

pub struct Server {
    memory: Arc<RwLock<Memory<Key>>>,
}

impl Server {
    fn new(memory: Arc<RwLock<Memory<Key>>>) -> Self {
        Self { memory }
    }

    async fn run(
        config: &Config,
        memory: Arc<RwLock<Memory<Key>>>,
    ) -> Result<JoinHandle<Result<(), anyhow::Error>>, anyhow::Error> {
        let mut builder = tokio_serial::new(&config.path, config.baud_rate);
        if let Some(v) = config.data_bits {
            builder = builder.data_bits(match v {
                5 => DataBits::Five,
                6 => DataBits::Six,
                7 => DataBits::Seven,
                8 => DataBits::Eight,
                _ => panic!("Invalid data bits specified"),
            });
        }
        if let Some(v) = config.stop_bits {
            builder = builder.stop_bits(match v {
                1 => StopBits::One,
                2 => StopBits::Two,
                _ => panic!("Invalid stop bits specified"),
            });
        }
        if let Some(ref v) = config.parity {
            let v = v.to_lowercase();
            if v == "odd" {
                builder = builder.parity(Parity::Odd);
            } else if v == "even" {
                builder = builder.parity(Parity::Even);
            } else if v == "none" {
                builder = builder.parity(Parity::None);
            } else {
                panic!("Invalid parity specified");
            }
        }

        match SerialStream::open(&builder) {
            Ok(serial_stream) => {
                let rtu_server = RtuServer::new(serial_stream);
                let server = Server::new(memory);
                Ok(tokio::task::spawn(async {
                    rtu_server
                        .serve_forever(server)
                        .await
                        .map_err(|e| anyhow!("{}", e))
                }))
            }
            Err(e) => Err(anyhow!("{}", e)),
        }
    }
}

impl tokio_modbus::server::Service for Server {
    type Request = SlaveRequest<'static>;
    type Exception = ExceptionCode;
    type Response = Response;
    type Future = future::Ready<Result<Response, ExceptionCode>>;

    fn call(&self, request: Self::Request) -> Self::Future {
        let SlaveRequest { slave, request } = request;
        match request {
            Request::ReadCoils(addr, cnt) => {
                let key = Key {
                    slave_id: slave,
                    fn_code: tokio_modbus::FunctionCode::ReadCoils.value(),
                };
                match self.memory.read() {
                    Ok(guard) => match guard.read(key, &Range::new(addr as usize, cnt as usize)) {
                        Some(v) => future::ready(Ok(Response::ReadCoils(
                            v.into_iter().map(|b| b != 0).collect(),
                        ))),
                        None => future::ready(Err(ExceptionCode::IllegalFunction)),
                    },
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::ReadDiscreteInputs(addr, cnt) => {
                let key = Key {
                    slave_id: slave,
                    fn_code: tokio_modbus::FunctionCode::ReadDiscreteInputs.value(),
                };
                match self.memory.read() {
                    Ok(guard) => match guard.read(key, &Range::new(addr as usize, cnt as usize)) {
                        Some(v) => future::ready(Ok(Response::ReadDiscreteInputs(
                            v.into_iter().map(|b| b != 0).collect(),
                        ))),
                        None => future::ready(Err(ExceptionCode::IllegalFunction)),
                    },
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::ReadInputRegisters(addr, cnt) => {
                let key = Key {
                    slave_id: slave,
                    fn_code: tokio_modbus::FunctionCode::ReadInputRegisters.value(),
                };
                match self.memory.read() {
                    Ok(guard) => match guard.read(key, &Range::new(addr as usize, cnt as usize)) {
                        Some(v) => future::ready(Ok(Response::ReadInputRegisters(v))),
                        None => future::ready(Err(ExceptionCode::IllegalFunction)),
                    },
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::ReadHoldingRegisters(addr, cnt) => {
                let key = Key {
                    slave_id: slave,
                    fn_code: tokio_modbus::FunctionCode::ReadHoldingRegisters.value(),
                };
                match self.memory.read() {
                    Ok(guard) => match guard.read(key, &Range::new(addr as usize, cnt as usize)) {
                        Some(v) => future::ready(Ok(Response::ReadHoldingRegisters(v))),
                        None => future::ready(Err(ExceptionCode::IllegalFunction)),
                    },
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::WriteMultipleRegisters(addr, values) => {
                let key = Key {
                    slave_id: slave,
                    fn_code: tokio_modbus::FunctionCode::WriteMultipleRegisters.value(),
                };
                match self.memory.write() {
                    Ok(mut guard) => {
                        match guard.write(key, &Range::new(addr as usize, values.len()), &values) {
                            true => future::ready(Ok(Response::WriteMultipleRegisters(
                                addr,
                                values.len() as u16,
                            ))),
                            false => future::ready(Err(ExceptionCode::IllegalFunction)),
                        }
                    }
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::WriteSingleRegister(addr, value) => {
                let key = Key {
                    slave_id: slave,
                    fn_code: tokio_modbus::FunctionCode::WriteSingleRegister.value(),
                };
                match self.memory.write() {
                    Ok(mut guard) => {
                        match guard.write(key, &Range::new(addr as usize, 1), &[value]) {
                            true => future::ready(Ok(Response::WriteSingleRegister(addr, value))),
                            false => future::ready(Err(ExceptionCode::IllegalFunction)),
                        }
                    }
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::WriteMultipleCoils(addr, values) => {
                let key = Key {
                    slave_id: slave,
                    fn_code: tokio_modbus::FunctionCode::WriteMultipleCoils.value(),
                };
                match self.memory.write() {
                    Ok(mut guard) => {
                        let values: Vec<u16> = values.iter().map(|v| *v as u16).collect();
                        match guard.write(key, &Range::new(addr as usize, 1), &values) {
                            true => future::ready(Ok(Response::WriteMultipleCoils(
                                addr,
                                values.len() as u16,
                            ))),
                            false => future::ready(Err(ExceptionCode::IllegalFunction)),
                        }
                    }
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::WriteSingleCoil(addr, value) => {
                let key = Key {
                    slave_id: slave,
                    fn_code: tokio_modbus::FunctionCode::WriteSingleCoil.value(),
                };
                match self.memory.write() {
                    Ok(mut guard) => {
                        let val = value as u16;
                        match guard.write(key, &Range::new(addr as usize, 1), &[val]) {
                            true => future::ready(Ok(Response::WriteSingleCoil(addr, value))),
                            false => future::ready(Err(ExceptionCode::IllegalFunction)),
                        }
                    }
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            _ => future::ready(Err(Self::Exception::IllegalDataAddress)),
        }
    }
}
