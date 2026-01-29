pub mod builder;
pub mod config;
pub mod error;
pub mod handle;

use builder::Builder;
use config::{ClientConfig, ServerConfig};
use error::{Error, InstanceError};
use handle::Handle;

use std::fmt::Debug;
use std::hash::Hash;

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
            builder: Builder::TcpClient(net::tcp::ClientBuilder::new(
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
            builder: Builder::RtuClient(net::rtu::ClientBuilder::new(
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
            builder: Builder::TcpServer(net::tcp::ServerBuilder::new(
                config.id,
                config.config,
                config.memory,
            )),
            handle: None,
        }
    }

    pub fn with_rtu_server(config: ServerConfig<T, net::rtu::Config>) -> Self {
        Self {
            builder: Builder::RtuServer(net::rtu::ServerBuilder::new(
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
                        self.handle = Some(Handle::Client(handle::ClientHandle { handle, sender }));
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
                        self.handle = Some(Handle::Server(handle::ServerHandle { handle }));
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
                        self.handle = Some(Handle::Client(handle::ClientHandle { handle, sender }));
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
                        self.handle = Some(Handle::Server(handle::ServerHandle { handle }));
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

    pub async fn send_command(&self, command: net::Command) -> Result<(), Error> {
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
