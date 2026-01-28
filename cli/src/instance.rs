use memory::Memory;
use net::*;

use anyhow::anyhow;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use tokio::task::JoinHandle;

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
    L: Fn(String) -> () + Clone + Send + Sync + 'static,
{
    Tcp(tcp::Server<T, L>),
    Rtu(rtu::Server<T, L>),
}

struct ClientHandle {
    handle: JoinHandle<Result<(), anyhow::Error>>,
    sender: Sender<Command>,
}

struct ServerHandle {
    handle: JoinHandle<Result<(), anyhow::Error>>,
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

    pub async fn start<L, S>(&mut self, log: L, status: S) -> Result<(), anyhow::Error>
    where
        L: Fn(String) -> () + Clone + Send + Sync + 'static,
        S: Fn(String) -> () + Clone + Send + Sync + 'static,
    {
        if self.handle.is_some() {
            return Err(anyhow!("instance already active"));
        }

        match &self.builder {
            Builder::TcpClient(builder) => {
                let (sender, receiver) = std::sync::mpsc::channel();
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(anyhow!("{}", e));
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
                        return Err(anyhow!("{}", e));
                    }
                    Ok(handle) => {
                        self.handle = Some(Handle::Server(ServerHandle { handle }));
                    }
                }
            }
            Builder::RtuClient(builder) => {
                let (sender, receiver) = std::sync::mpsc::channel();
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(anyhow!("{}", e));
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
                        return Err(anyhow!("{}", e));
                    }
                    Ok(handle) => {
                        self.handle = Some(Handle::Server(ServerHandle { handle }));
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), anyhow::Error> {
        if self.handle.is_none() {
            return Err(anyhow!("instance not running"));
        }

        let handle = self.handle.take();

        let res = match handle {
            Some(Handle::Client(h)) => {
                h.handle.abort();
                h.handle.await
            }
            Some(Handle::Server(h)) => {
                h.handle.abort();
                h.handle.await
            }
            _ => {
                unreachable!("case is unreachable");
            }
        };

        match res {
            Ok(r) => r,
            Err(e) => {
                if e.is_cancelled() {
                    Ok(())
                } else {
                    Err(anyhow!("{}", e))
                }
            }
        }
    }
}
