// Crate
use crate::tcp::Config;
use crate::{Error, Key, TcpError};

// Workspace
use memory::{Memory, Range, Type};

// External
use std::fmt::Debug;
use std::future;
use std::hash::Hash;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
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

    pub async fn spawn<L>(&self, log: L) -> Result<JoinHandle<Result<(), Error>>, Error>
    where
        L: AsyncFn(String) -> () + Clone + Send + Sync + 'static,
        for<'a> L::CallRefFuture<'a>: Send,
    {
        let guard = self.config.read().await;
        Server::run(self.id.clone(), &guard, self.memory.clone(), log).await
    }
}

pub struct Server<T, L>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
    L: AsyncFn(String) -> () + Clone + Send + Sync + 'static,
    for<'a> L::CallRefFuture<'a>: Send,
{
    id: T,
    memory: Arc<RwLock<Memory<Key<T>>>>,
    log: L,
}

impl<T, L> Server<T, L>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
    L: AsyncFn(String) -> () + Clone + Send + Sync + 'static,
    for<'a> L::CallRefFuture<'a>: Send,
{
    fn new(id: T, memory: Arc<RwLock<Memory<Key<T>>>>, log: L) -> Self {
        Self { id, memory, log }
    }

    async fn run(
        id: T,
        config: &Config,
        memory: Arc<RwLock<Memory<Key<T>>>>,
        log: L,
    ) -> Result<JoinHandle<Result<(), Error>>, Error> {
        let addr: SocketAddr = format!("{}:{}", config.ip, config.port)
            .parse()
            .map_err(|e| Error::Tcp(TcpError::Address(e)))?;
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
                        tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(async {
                                on_process_log(format!("Server processing failed. [{}]", err))
                                    .await;
                            })
                        })
                    };
                    server
                        .serve(&on_connected, on_process_error)
                        .await
                        .map_err(|e| Error::Server(e))
                }))
            }
            Err(e) => Err(Error::Server(e).into()),
        }
    }
}

impl<T, L> tokio_modbus::server::Service for Server<T, L>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync,
    L: AsyncFn(String) -> () + Clone + Send + Sync + 'static,
    for<'a> L::CallRefFuture<'a>: Send,
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

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                match request {
                    Request::ReadCoils(addr, cnt) => {
                        (self.log)(format!(
                                "ReadCoils request received for slave ID {} and range [{}, {}).",
                                slave,
                                addr,
                                addr + cnt
                        )).await;
                        let guard = self.memory.read().await;
                        match guard.read(key, &Type::Coil, &Range::new(addr as usize, cnt as usize)) {
                            Some(v) =>
                            {
                                (self.log)(format!(
                                        "ReadCoils request for slave ID {} and range [{}, {}) successful.",
                                        slave,
                                        addr,
                                        addr + cnt
                                )).await;
                                future::ready(Ok(Response::ReadCoils(
                                            v.into_iter().map(|b| b != 0).collect(),
                                )))
                            },
                            None => {
                                (self.log)(format!(
                                        "ReadCoils request for slave ID {} and range [{}, {}) failed.",
                                        slave,
                                        addr,
                                        addr + cnt
                                )).await;
                                future::ready(Err(ExceptionCode::IllegalFunction))
                            },
                        }
                    }
                    Request::ReadDiscreteInputs(addr, cnt) => {
                        (self.log)(format!(
                                "ReadDiscreteInputs request received for slave ID {} and range [{}, {}).",
                                slave,
                                addr,
                                addr + cnt
                        )).await;
                        let guard = self.memory.read().await;
                        match guard.read(key, &Type::Coil, &Range::new(addr as usize, cnt as usize)) {
                            Some(v) => {
                                (self.log)(format!(
                                        "ReadDiscreteInputs request for slave ID {} and range [{}, {}) successful.",
                                        slave,
                                        addr,
                                        addr + cnt
                                )).await;
                                future::ready(Ok(Response::ReadDiscreteInputs(
                                            v.into_iter().map(|b| b != 0).collect(),
                                )))},
                            None => {
                                (self.log)(format!(
                                        "ReadDiscreteInputs request for slave ID {} and range [{}, {}) failed.",
                                        slave,
                                        addr,
                                        addr + cnt
                                )).await;
                                future::ready(Err(ExceptionCode::IllegalFunction))
                            },
                        }
                    }
                    Request::ReadInputRegisters(addr, cnt) => {
                        (self.log)(format!(
                                "ReadInputRegisters request received for slave ID {} and range [{}, {}).",
                                slave,
                                addr,
                                addr + cnt
                        )).await;
                        let guard = self.memory.read().await;
                        match guard.read(
                            key,
                            &Type::Register,
                            &Range::new(addr as usize, cnt as usize),
                        ) {
                            Some(v) => {
                                (self.log)(format!(
                                        "ReadInputRegisters request for slave ID {} and range [{}, {}) successful.",
                                        slave,
                                        addr,
                                        addr + cnt
                                )).await;
                                future::ready(Ok(Response::ReadInputRegisters(v)))
                            },
                            None => {
                                (self.log)(format!(
                                        "ReadInputRegisters request for slave ID {} and range [{}, {}) failed.",
                                        slave,
                                        addr,
                                        addr + cnt
                                )).await;
                                future::ready(Err(ExceptionCode::IllegalFunction))
                            },
                        }
                    }
                    Request::ReadHoldingRegisters(addr, cnt) => {
                        (self.log)(format!(
                                "ReadHoldingRegisters request received for slave ID {} and range [{}, {}).",
                                slave,
                                addr,
                                addr + cnt
                        )).await;
                        let guard = self.memory.read().await;
                        match guard.read(
                            key,
                            &Type::Register,
                            &Range::new(addr as usize, cnt as usize),
                        ) {
                            Some(v) => {
                                (self.log)(format!(
                                        "ReadHoldingRegisters request for slave ID {} and range [{}, {}) successful.",
                                        slave,
                                        addr,
                                        addr + cnt
                                )).await;
                                future::ready(Ok(Response::ReadHoldingRegisters(v)))
                            },
                            None => {
                                (self.log)(format!(
                                        "ReadHoldingRegisters request for slave ID {} and range [{}, {}) failed.",
                                        slave,
                                        addr,
                                        addr + cnt
                                )).await;
                                future::ready(Err(ExceptionCode::IllegalFunction))
                            },
                        }
                    }
                    Request::WriteMultipleRegisters(addr, values) => {
                        (self.log)(format!(
                                "WriteMultipleRegisters request received for slave ID {}, range [{}, {}), and values {:?}.",
                                slave,
                                addr,
                                addr as usize + values.len(),
                                values
                        )).await;
                        let mut guard = self.memory.write().await;
                        match guard.write(
                            key,
                            &Type::Register,
                            &Range::new(addr as usize, values.len()),
                            &values,
                        ) {
                            true => {
                                (self.log)(format!(
                                        "WriteMultipleRegisters request for slave ID {}, range [{}, {}), and values {:?} successful.",
                                        slave,
                                        addr,
                                        addr as usize + values.len(),
                                        values
                                )).await;
                                future::ready(Ok(Response::WriteMultipleRegisters(
                                            addr,
                                            values.len() as u16,
                                )))
                            },
                            false => {
                                (self.log)(format!(
                                        "WriteMultipleRegisters request for slave ID {}, range [{}, {}), and values {:?} failed.",
                                        slave,
                                        addr,
                                        addr as usize + values.len(),
                                        values
                                )).await;
                                future::ready(Err(ExceptionCode::IllegalFunction))
                            },

                        }
                    }
                    Request::WriteSingleRegister(addr, value) => {
                        (self.log)(format!(
                                "WriteSingleRegister request received for slave ID {}, address {}, and value {}.",
                                slave, addr, value
                        )).await;
                        let mut guard = self.memory.write().await;
                        match guard.write(
                            key,
                            &Type::Register,
                            &Range::new(addr as usize, 1),
                            &[value],
                        ) {
                            true => {
                                (self.log)(format!(
                                        "WriteSingleRegister request for slave ID {}, address {}, and value {} successful.",
                                        slave, addr, value
                                )).await;
                                future::ready(Ok(Response::WriteSingleRegister(addr, value)))
                            },
                            false => {
                                (self.log)(format!(
                                        "WriteSingleRegister request for slave ID {}, address {}, and value {} failed.",
                                        slave, addr, value
                                )).await;
                                future::ready(Err(ExceptionCode::IllegalFunction))
                            },
                        }
                    }
                    Request::WriteMultipleCoils(addr, values) => {
                        (self.log)(format!(
                                "WriteMultipleCoils request received for slave ID {}, range [{}, {}), and values {:?}.",
                                slave,
                                addr,
                                addr as usize + values.len(), values
                        )).await;
                        let mut guard = self.memory.write().await;
                        let values: Vec<u16> = values.iter().map(|v| *v as u16).collect();
                        match guard.write(key, &Type::Coil, &Range::new(addr as usize, 1), &values) {
                            true => {
                                (self.log)(format!(
                                        "WriteMultipleCoils request for slave ID {}, range [{}, {}), and values {:?} successful.",
                                        slave,
                                        addr,
                                        addr as usize + values.len(), values
                                )).await;
                                future::ready(Ok(Response::WriteMultipleCoils(addr, values.len() as u16)))
                            }
                            false => {
                                (self.log)(format!(
                                        "WriteMultipleCoils request for slave ID {}, range [{}, {}), and values {:?} failed.",
                                        slave,
                                        addr,
                                        addr as usize + values.len(), values
                                )).await;
                                future::ready(Err(ExceptionCode::IllegalFunction))
                            }
                        }
                    }
                    Request::WriteSingleCoil(addr, value) => {
                        (self.log)(format!(
                                "WriteSingleCoil request received for slave ID {}, address {}, and value {}.",
                                slave, addr, value
                        )).await;
                        let mut guard = self.memory.write().await;
                        let val = value as u16;
                        match guard.write(key, &Type::Coil, &Range::new(addr as usize, 1), &[val]) {
                            true => {
                                (self.log)(format!(
                                        "WriteSingleCoil request for slave ID {}, address {}, and value {} successful.",
                                        slave, addr, value
                                )).await;
                                future::ready(Ok(Response::WriteSingleCoil(addr, value)))
                            },
                            false => {
                                (self.log)(format!(
                                        "WriteSingleCoil request for slave ID {}, address {}, and value {} failed.",
                                        slave, addr, value
                                )).await;
                                future::ready(Err(ExceptionCode::IllegalFunction))
                            }
                        }
                    }
                    Request::ReportServerId => {
                        (self.log)(format!(
                                "ReportServerId request received for slave ID {}. Unsupported function.",
                                slave,
                        )).await;
                        future::ready(Err(ExceptionCode::IllegalFunction))
                    }
                    Request::MaskWriteRegister(_, _, _) => {
                        (self.log)(format!(
                                "MaskWriteRegister request received for slave ID {}. Unsupported function.",
                                slave,
                        )).await;
                        future::ready(Err(ExceptionCode::IllegalFunction))
                    }
                    Request::ReadWriteMultipleRegisters(read_addr, cnt, write_addr, values) => {
                        (self.log)(format!(
                                "ReadWriteMultipleRegisrters request received for slave ID {}, read address {}, count {}, write address {}, and values {:?}.",
                                slave, read_addr, cnt, write_addr, values
                        )).await;
                        let mut guard = self.memory.write().await;
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
                            (self.log)(format!(
                                    "ReadWriteMultipleRegisrters request for slave ID {}, read address {}, count {}, write address {}, and values {:?} failed.",
                                    slave, read_addr, cnt, write_addr, values
                            )).await;
                            return future::ready(Err(ExceptionCode::IllegalDataAddress));
                        }
                        let v = match guard.read(
                            key.clone(),
                            &Type::Register,
                            &Range::new(read_addr as usize, cnt as usize),
                        ) {
                            Some(v) => v,
                            None => {
                                (self.log)(format!(
                                        "ReadWriteMultipleRegisrters request for slave ID {}, read address {}, count {}, write address {}, and values {:?} failed.",
                                        slave, read_addr, cnt, write_addr, values
                                )).await;
                                return future::ready(Err(ExceptionCode::IllegalFunction))
                            },
                        };
                        if !guard.write(
                            key,
                            &Type::Register,
                            &Range::new(write_addr as usize, values.len()),
                            &values,
                        ) {
                            (self.log)(format!(
                                    "ReadWriteMultipleRegisrters request for slave ID {}, read address {}, count {}, write address {}, and values {:?} failed.",
                                    slave, read_addr, cnt, write_addr, values
                            )).await;
                            return future::ready(Err(ExceptionCode::IllegalFunction));
                        }
                        (self.log)(format!(
                                "ReadWriteMultipleRegisrters request for slave ID {}, read address {}, count {}, write address {}, and values {:?} successful.",
                                slave, read_addr, cnt, write_addr, values
                        )).await;
                        future::ready(Ok(Response::ReadWriteMultipleRegisters(v)))
                    }
                    Request::ReadDeviceIdentification(_, _) => {
                        (self.log)(format!(
                                "ReadDeviceIdentification request received for slave ID {}. Unsupported function.",
                                slave,
                        )).await;
                        future::ready(Err(ExceptionCode::IllegalFunction))
                    }
                    Request::Custom(func, _) => {
                        (self.log)(format!(
                                "Custom function {} request received for slave ID {}. Unsupported function.",
                                func, slave,
                        )).await;
                        future::ready(Err(ExceptionCode::IllegalFunction))
                    }
                }
            })
        })
    }
}
