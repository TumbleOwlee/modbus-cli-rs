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

    pub async fn spawn(
        &self,
        log: fn(String) -> (),
    ) -> Result<JoinHandle<Result<(), anyhow::Error>>, anyhow::Error> {
        match self.config.read() {
            Ok(guard) => Server::run(self.id.clone(), &guard, self.memory.clone(), log).await,
            Err(e) => Err(anyhow!("{}", e)),
        }
    }
}

pub struct Server<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    id: T,
    memory: Arc<RwLock<Memory<Key<T>>>>,
}

impl<T> Server<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    fn new(id: T, memory: Arc<RwLock<Memory<Key<T>>>>) -> Self {
        Self { id, memory }
    }

    async fn run(
        id: T,
        config: &Config,
        memory: Arc<RwLock<Memory<Key<T>>>>,
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
                let id = id.clone();
                Ok(tokio::task::spawn(async move {
                    let new_request_handler =
                        |_socket_addr| Ok(Some(Server::new(id.clone(), memory.clone())));
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

impl<T> tokio_modbus::server::Service for Server<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync,
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
            Request::ReadCoils(addr, cnt) => match self.memory.read() {
                Ok(guard) => {
                    match guard.read(key, &Type::Coil, &Range::new(addr as usize, cnt as usize)) {
                        Some(v) => future::ready(Ok(Response::ReadCoils(
                            v.into_iter().map(|b| b != 0).collect(),
                        ))),
                        None => future::ready(Err(ExceptionCode::IllegalFunction)),
                    }
                }
                _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
            },
            Request::ReadDiscreteInputs(addr, cnt) => match self.memory.read() {
                Ok(guard) => {
                    match guard.read(key, &Type::Coil, &Range::new(addr as usize, cnt as usize)) {
                        Some(v) => future::ready(Ok(Response::ReadDiscreteInputs(
                            v.into_iter().map(|b| b != 0).collect(),
                        ))),
                        None => future::ready(Err(ExceptionCode::IllegalFunction)),
                    }
                }
                _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
            },
            Request::ReadInputRegisters(addr, cnt) => match self.memory.read() {
                Ok(guard) => match guard.read(
                    key,
                    &Type::Register,
                    &Range::new(addr as usize, cnt as usize),
                ) {
                    Some(v) => future::ready(Ok(Response::ReadInputRegisters(v))),
                    None => future::ready(Err(ExceptionCode::IllegalFunction)),
                },
                _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
            },
            Request::ReadHoldingRegisters(addr, cnt) => match self.memory.read() {
                Ok(guard) => match guard.read(
                    key,
                    &Type::Register,
                    &Range::new(addr as usize, cnt as usize),
                ) {
                    Some(v) => future::ready(Ok(Response::ReadHoldingRegisters(v))),
                    None => future::ready(Err(ExceptionCode::IllegalFunction)),
                },
                _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
            },
            Request::WriteMultipleRegisters(addr, values) => match self.memory.write() {
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
            },
            Request::WriteSingleRegister(addr, value) => match self.memory.write() {
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
            },
            Request::WriteMultipleCoils(addr, values) => match self.memory.write() {
                Ok(mut guard) => {
                    let values: Vec<u16> = values.iter().map(|v| *v as u16).collect();
                    match guard.write(key, &Type::Coil, &Range::new(addr as usize, 1), &values) {
                        true => future::ready(Ok(Response::WriteMultipleCoils(
                            addr,
                            values.len() as u16,
                        ))),
                        false => future::ready(Err(ExceptionCode::IllegalFunction)),
                    }
                }
                _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
            },
            Request::WriteSingleCoil(addr, value) => match self.memory.write() {
                Ok(mut guard) => {
                    let val = value as u16;
                    match guard.write(key, &Type::Coil, &Range::new(addr as usize, 1), &[val]) {
                        true => future::ready(Ok(Response::WriteSingleCoil(addr, value))),
                        false => future::ready(Err(ExceptionCode::IllegalFunction)),
                    }
                }
                _ => future::ready(Err(Self::Exception::ServerDeviceFailure)),
            },
            _ => future::ready(Err(Self::Exception::IllegalDataAddress)),
        }
    }
}
