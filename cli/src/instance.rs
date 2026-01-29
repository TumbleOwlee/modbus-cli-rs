use memory::Memory;
use net::*;
use tokio::sync::mpsc::error::SendError;

use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

pub enum InstanceError {
    AlreadyActive,
    NotRunning,
    CancelFailed,
    SendError(SendError<Command>),
    InvalidOperation,
}

impl Display for InstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstanceError::AlreadyActive => write!(f, "Instance is already active"),
            InstanceError::NotRunning => write!(f, "Instance is not running"),
            InstanceError::CancelFailed => write!(f, "Failed to cancel instance"),
            InstanceError::SendError(e) => {
                write!(f, "Failed to send command to instance: {}", e)
            }
            InstanceError::InvalidOperation => write!(f, "Invalid operation specified"),
        }
    }
}

pub enum Error {
    Net(net::Error),
    Instance(InstanceError),
}

impl From<InstanceError> for Error {
    fn from(e: InstanceError) -> Self {
        Error::Instance(e)
    }
}

impl From<net::Error> for Error {
    fn from(e: net::Error) -> Self {
        Error::Net(e)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Net(e) => write!(f, "Network error: {}", e),
            Error::Instance(s) => write!(f, "Instance error: {}", s),
        }
    }
}

#[derive(Clone)]
pub struct ClientConfig<T, Config>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    pub id: T,
    pub config: Arc<RwLock<Config>>,
    pub operations: Arc<RwLock<Vec<Operation>>>,
    pub memory: Arc<RwLock<Memory<Key<T>>>>,
}

#[derive(Clone)]
pub struct ServerConfig<T, Config>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    pub id: T,
    pub config: Arc<RwLock<Config>>,
    pub memory: Arc<RwLock<Memory<Key<T>>>>,
}

#[derive(Clone)]
enum Type<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    TcpClient(ClientConfig<T, net::tcp::Config>),
    RtuClient(ClientConfig<T, net::rtu::Config>),
    TcpServer(ServerConfig<T, net::tcp::Config>),
    RtuServer(ServerConfig<T, net::rtu::Config>),
}

enum Builder<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    TcpClient(tcp::ClientBuilder<T>),
    TcpServer(tcp::ServerBuilder<T>),
    RtuClient(rtu::ClientBuilder<T>),
    RtuServer(rtu::ServerBuilder<T>),
}

enum Client {
    Tcp(tcp::Client),
    Rtu(rtu::Client),
}

enum Server<T, L>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
    L: AsyncFn(String) -> () + Clone + Send + Sync + 'static,
    for<'a> L::CallRefFuture<'a>: Send,
{
    Tcp(tcp::Server<T, L>),
    Rtu(rtu::Server<T, L>),
}

struct ClientHandle {
    handle: JoinHandle<Result<(), net::Error>>,
    sender: Sender<Command>,
}

struct ServerHandle {
    handle: JoinHandle<Result<(), net::Error>>,
}

pub enum Handle {
    Server(ServerHandle),
    Client(ClientHandle),
}

pub struct Instance<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    builder: Builder<T>,
    handle: Option<Handle>,
}

impl<T> Instance<T>
where
    T: Hash + Debug + PartialEq + Eq + Clone + Default + Send + Sync + 'static,
{
    pub fn with_tcp_client(config: ClientConfig<T, net::tcp::Config>) -> Self {
        Self {
            builder: Builder::TcpClient(tcp::ClientBuilder::new(
                config.id,
                config.config,
                config.operations,
                config.memory,
            )),
            handle: None,
        }
    }

    pub fn with_rtu_client(config: ClientConfig<T, net::rtu::Config>) -> Self {
        Self {
            builder: Builder::RtuClient(rtu::ClientBuilder::new(
                config.id,
                config.config,
                config.operations,
                config.memory,
            )),
            handle: None,
        }
    }

    pub fn with_tcp_server(config: ServerConfig<T, net::tcp::Config>) -> Self {
        Self {
            builder: Builder::TcpServer(tcp::ServerBuilder::new(
                config.id,
                config.config,
                config.memory,
            )),
            handle: None,
        }
    }

    pub fn with_rtu_server(config: ServerConfig<T, net::rtu::Config>) -> Self {
        Self {
            builder: Builder::RtuServer(rtu::ServerBuilder::new(
                config.id,
                config.config,
                config.memory,
            )),
            handle: None,
        }
    }

    pub async fn start<L, S>(&mut self, log: L, status: S) -> Result<(), Error>
    where
        L: AsyncFn(String) -> () + Clone + Send + Sync + 'static,
        S: AsyncFn(String) -> () + Clone + Send + Sync + 'static,
        for<'a> L::CallRefFuture<'a>: Send,
        for<'a> S::CallRefFuture<'a>: Send,
    {
        if self.handle.is_some() {
            return Err(InstanceError::AlreadyActive.into());
        }

        match &self.builder {
            Builder::TcpClient(builder) => {
                let (sender, receiver) = tokio::sync::mpsc::channel(10);
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok(handle) => {
                        self.handle = Some(Handle::Client(ClientHandle { handle, sender }));
                    }
                }
            }
            Builder::TcpServer(builder) => {
                let res = builder.spawn(log).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok(handle) => {
                        self.handle = Some(Handle::Server(ServerHandle { handle }));
                    }
                }
            }
            Builder::RtuClient(builder) => {
                let (sender, receiver) = tokio::sync::mpsc::channel(10);
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok(handle) => {
                        self.handle = Some(Handle::Client(ClientHandle { handle, sender }));
                    }
                }
            }
            Builder::RtuServer(builder) => {
                let res = builder.spawn(log).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok(handle) => {
                        self.handle = Some(Handle::Server(ServerHandle { handle }));
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), Error> {
        if self.handle.is_none() {
            return Err(InstanceError::NotRunning.into());
        }

        let handle = self.handle.take();

        let res = match handle {
            Some(Handle::Client(h)) => {
                if h.handle.is_finished() {
                    Ok(Ok(()))
                } else {
                    h.handle.abort();
                    h.handle.await
                }
            }
            Some(Handle::Server(h)) => {
                if h.handle.is_finished() {
                    Ok(Ok(()))
                } else {
                    h.handle.abort();
                    h.handle.await
                }
            }
            _ => {
                unreachable!("case is unreachable");
            }
        };

        match res {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e.into()),
            Err(e) => {
                if e.is_cancelled() {
                    Ok(())
                } else {
                    Err(InstanceError::CancelFailed.into())
                }
            }
        }
    }

    pub async fn send_command(&self, command: Command) -> Result<(), Error> {
        if self.handle.is_none() {
            return Err(InstanceError::NotRunning.into());
        }
        match &self.handle {
            Some(Handle::Client(handle)) => handle
                .sender
                .send(command)
                .await
                .map_err(|e| InstanceError::SendError(e).into()),
            _ => Err(InstanceError::InvalidOperation.into()),
        }
    }
}
