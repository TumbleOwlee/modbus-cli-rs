// Crate
use crate::Key;
use crate::tcp::Config;

// Workspace
use memory::Range;
use memory::memory::Memory;

// External
use anyhow::anyhow;
use std::net::SocketAddr;
use std::sync::Arc;
use std::{future, sync::RwLock};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_modbus::Request;
use tokio_modbus::prelude::{ExceptionCode, Response, SlaveRequest};
use tokio_modbus::server::tcp::{Server as TcpServer, accept_tcp_connection};

pub struct ServerBuilder {
    config: Arc<RwLock<Config>>,
    memory: Arc<RwLock<Memory<Key>>>,
}

impl ServerBuilder {
    pub fn new(config: Arc<RwLock<Config>>, memory: Arc<RwLock<Memory<Key>>>) -> Self {
        Self { config, memory }
    }

    pub async fn spawn(
        &self,
        log: fn(String) -> (),
    ) -> Result<JoinHandle<Result<(), anyhow::Error>>, anyhow::Error> {
        match self.config.read() {
            Ok(guard) => Server::run(&guard, self.memory.clone(), log).await,
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
        log: fn(String) -> (),
    ) -> Result<JoinHandle<Result<(), anyhow::Error>>, anyhow::Error> {
        let addr: SocketAddr = format!("{}:{}", config.ip, config.port)
            .parse()
            .map_err(|e| anyhow!("{e}"))?;
        match TcpListener::bind(addr).await {
            Ok(listener) => {
                let server = TcpServer::new(listener);
                let memory = memory.clone();
                let log = log.clone();
                Ok(tokio::task::spawn(async move {
                    let new_request_handler = |_socket_addr| Ok(Some(Server::new(memory.clone())));
                    let on_connected = |stream, socket_addr| async move {
                        accept_tcp_connection(stream, socket_addr, new_request_handler)
                    };
                    let on_process_log = log;
                    let on_process_error = move |err| {
                        on_process_log(format!("Server processing failed. [{}]", err));
                    };
                    server
                        .serve(&on_connected, on_process_error)
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
