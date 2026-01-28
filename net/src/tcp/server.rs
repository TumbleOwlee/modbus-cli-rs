// Crate
use crate::Key;
use crate::tcp::Config;

// Workspace
use memory::{Memory, Range, Type};

// External
use anyhow::anyhow;
use std::fmt::Debug;
use std::hash::Hash;
use std::net::SocketAddr;
use std::sync::Arc;
use std::{future, sync::RwLock};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_modbus::Request;
use tokio_modbus::prelude::{ExceptionCode, Response, SlaveRequest};
use tokio_modbus::server::tcp::{Server as TcpServer, accept_tcp_connection};

pub struct ServerBuilder<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    id: T,
    config: Arc<RwLock<Config>>,
    memory: Arc<RwLock<Memory<Key<T>>>>,
}

impl<T> ServerBuilder<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    pub fn new(id: T, config: Arc<RwLock<Config>>, memory: Arc<RwLock<Memory<Key<T>>>>) -> Self {
        Self { id, config, memory }
    }

    pub async fn spawn<L>(
        &self,
        log: L,
    ) -> Result<JoinHandle<Result<(), anyhow::Error>>, anyhow::Error>
    where
        L: Fn(String) -> () + Clone + Send + Sync + 'static,
    {
        match self.config.read() {
            Ok(guard) => Server::run(self.id.clone(), &guard, self.memory.clone(), log).await,
            Err(e) => Err(anyhow!("{}", e)),
        }
    }
}

pub struct Server<T, L>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
    L: Fn(String) -> () + Clone + Send + Sync + 'static,
{
    id: T,
    memory: Arc<RwLock<Memory<Key<T>>>>,
    log: L,
}

impl<T, L> Server<T, L>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
    L: Fn(String) -> () + Clone + Send + Sync + 'static,
{
    fn new(id: T, memory: Arc<RwLock<Memory<Key<T>>>>, log: L) -> Self {
        Self { id, memory, log }
    }

    async fn run(
        id: T,
        config: &Config,
        memory: Arc<RwLock<Memory<Key<T>>>>,
        log: L,
    ) -> Result<JoinHandle<Result<(), anyhow::Error>>, anyhow::Error> {
        let addr: SocketAddr = format!("{}:{}", config.ip, config.port)
            .parse()
            .map_err(|e| anyhow!("{e}"))?;
        match TcpListener::bind(addr).await {
            Ok(listener) => {
                let server = TcpServer::new(listener);
                let memory = memory.clone();
                let log = log.clone();
                let id = id.clone();
                Ok(tokio::task::spawn(async move {
                    let new_request_handler = |_socket_addr| {
                        Ok(Some(Server::new(id.clone(), memory.clone(), log.clone())))
                    };
                    let on_connected = |stream, socket_addr| async move {
                        accept_tcp_connection(stream, socket_addr, new_request_handler)
                    };
                    let on_process_log = log.clone();
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

impl<T, L> tokio_modbus::server::Service for Server<T, L>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync,
    L: Fn(String) -> () + Clone + Send + Sync + 'static,
{
    type Request = SlaveRequest<'static>;
    type Exception = ExceptionCode;
    type Response = Response;
    type Future = future::Ready<Result<Response, ExceptionCode>>;

    fn call(&self, request: Self::Request) -> Self::Future {
        let SlaveRequest { slave, request } = request;
        let key = Key::<T> {
            id: self.id.clone(),
            slave_id: slave,
        };
        match request {
            Request::ReadCoils(addr, cnt) => {
                (self.log)(format!(
                    "ReadCoils request received for slave ID {} and range [{}, {})",
                    slave,
                    addr,
                    addr + cnt
                ));
                match self.memory.read() {
                    Ok(guard) => {
                        match guard.read(key, &Type::Coil, &Range::new(addr as usize, cnt as usize))
                        {
                            Some(v) => future::ready(Ok(Response::ReadCoils(
                                v.into_iter().map(|b| b != 0).collect(),
                            ))),
                            None => future::ready(Err(ExceptionCode::IllegalFunction)),
                        }
                    }
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::ReadDiscreteInputs(addr, cnt) => {
                (self.log)(format!(
                    "ReadDiscreteInputs request received for slave ID {} and range [{}, {})",
                    slave,
                    addr,
                    addr + cnt
                ));
                match self.memory.read() {
                    Ok(guard) => {
                        match guard.read(key, &Type::Coil, &Range::new(addr as usize, cnt as usize))
                        {
                            Some(v) => future::ready(Ok(Response::ReadDiscreteInputs(
                                v.into_iter().map(|b| b != 0).collect(),
                            ))),
                            None => future::ready(Err(ExceptionCode::IllegalFunction)),
                        }
                    }
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::ReadInputRegisters(addr, cnt) => {
                (self.log)(format!(
                    "ReadInputRegisters request received for slave ID {} and range [{}, {})",
                    slave,
                    addr,
                    addr + cnt
                ));
                match self.memory.read() {
                    Ok(guard) => match guard.read(
                        key,
                        &Type::Register,
                        &Range::new(addr as usize, cnt as usize),
                    ) {
                        Some(v) => future::ready(Ok(Response::ReadInputRegisters(v))),
                        None => future::ready(Err(ExceptionCode::IllegalFunction)),
                    },
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::ReadHoldingRegisters(addr, cnt) => {
                (self.log)(format!(
                    "ReadHoldingRegisters request received for slave ID {} and range [{}, {})",
                    slave,
                    addr,
                    addr + cnt
                ));
                match self.memory.read() {
                    Ok(guard) => match guard.read(
                        key,
                        &Type::Register,
                        &Range::new(addr as usize, cnt as usize),
                    ) {
                        Some(v) => future::ready(Ok(Response::ReadHoldingRegisters(v))),
                        None => future::ready(Err(ExceptionCode::IllegalFunction)),
                    },
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::WriteMultipleRegisters(addr, values) => {
                (self.log)(format!(
                    "WriteMultipleRegisters request received for slave ID {} and range [{}, {})",
                    slave,
                    addr,
                    addr as usize + values.len()
                ));
                match self.memory.write() {
                    Ok(mut guard) => {
                        match guard.write(
                            key,
                            &Type::Register,
                            &Range::new(addr as usize, values.len()),
                            &values,
                        ) {
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
                (self.log)(format!(
                    "WriteSingleRegister request received for slave ID {} and address {}.",
                    slave, addr,
                ));
                match self.memory.write() {
                    Ok(mut guard) => match guard.write(
                        key,
                        &Type::Register,
                        &Range::new(addr as usize, 1),
                        &[value],
                    ) {
                        true => future::ready(Ok(Response::WriteSingleRegister(addr, value))),
                        false => future::ready(Err(ExceptionCode::IllegalFunction)),
                    },
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::WriteMultipleCoils(addr, values) => {
                (self.log)(format!(
                    "WriteMultipleCoils request received for slave ID {} and range [{}, {}).",
                    slave,
                    addr,
                    addr as usize + values.len()
                ));
                match self.memory.write() {
                    Ok(mut guard) => {
                        let values: Vec<u16> = values.iter().map(|v| *v as u16).collect();
                        match guard.write(key, &Type::Coil, &Range::new(addr as usize, 1), &values)
                        {
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
                (self.log)(format!(
                    "WriteSingleCoil request received for slave ID {} and address {}.",
                    slave, addr,
                ));
                match self.memory.write() {
                    Ok(mut guard) => {
                        let val = value as u16;
                        match guard.write(key, &Type::Coil, &Range::new(addr as usize, 1), &[val]) {
                            true => future::ready(Ok(Response::WriteSingleCoil(addr, value))),
                            false => future::ready(Err(ExceptionCode::IllegalFunction)),
                        }
                    }
                    _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::ReportServerId => {
                (self.log)(format!(
                    "ReportServerId request received for slave ID {}. Unsupported function.",
                    slave,
                ));
                future::ready(Err(ExceptionCode::IllegalFunction))
            }
            Request::MaskWriteRegister(_, _, _) => {
                (self.log)(format!(
                    "MaskWriteRegister request received for slave ID {}. Unsupported function.",
                    slave,
                ));
                future::ready(Err(ExceptionCode::IllegalFunction))
            }
            Request::ReadWriteMultipleRegisters(read_addr, cnt, write_addr, values) => {
                (self.log)(format!(
                    "ReadWriteMultipleRegisrters request received for slave ID {}. Unsupported function.",
                    slave,
                ));
                match self.memory.write() {
                    Ok(mut guard) => {
                        let readable = guard.readable(
                            &key,
                            &Type::Register,
                            &Range::new(read_addr as usize, cnt as usize),
                        );
                        let writable = guard.writable(
                            &key,
                            &Type::Register,
                            &Range::new(write_addr as usize, values.len()),
                        );
                        if !readable || !writable {
                            return future::ready(Err(ExceptionCode::IllegalDataAddress));
                        }
                        let v = match guard.read(
                            key.clone(),
                            &Type::Register,
                            &Range::new(read_addr as usize, cnt as usize),
                        ) {
                            Some(v) => v,
                            None => return future::ready(Err(ExceptionCode::IllegalFunction)),
                        };
                        if !guard.write(
                            key,
                            &Type::Register,
                            &Range::new(write_addr as usize, values.len()),
                            &values,
                        ) {
                            return future::ready(Err(ExceptionCode::IllegalFunction));
                        }
                        future::ready(Ok(Response::ReadWriteMultipleRegisters(v)))
                    }
                    _ => return future::ready(Err(Self::Exception::ServerDeviceFailure)),
                }
            }
            Request::ReadDeviceIdentification(_, _) => {
                (self.log)(format!(
                    "ReadDeviceIdentification request received for slave ID {}. Unsupported function.",
                    slave,
                ));
                future::ready(Err(ExceptionCode::IllegalFunction))
            }
            Request::Custom(func, _) => {
                (self.log)(format!(
                    "Custom function {} request received for slave ID {}. Unsupported function.",
                    func, slave,
                ));
                future::ready(Err(ExceptionCode::IllegalFunction))
            }
        }
    }
}
