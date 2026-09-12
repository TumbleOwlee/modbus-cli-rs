use crate::server_core::{
    BoundAddr, ResetOn, ServeEnd, Server, drive_serve, wait_reconnect_backoff,
};
use crate::udp::Config;
use crate::{Error, Key, KeyParams, LogFn, ServerCommand, TcpError};

use ferrowl_store::Memory;
use ferrowl_util::backoff::{AttemptOutcome, BackoffPolicy, run_with_backoff};

use parking_lot::RwLock as MemLock;
use rust_modbus::Server as ModbusServer;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::UdpSocket;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::RwLock;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;

/// Builds and spawns a Modbus UDP server task answering requests from the
/// shared `memory`.
pub struct ServerBuilder<T: KeyParams> {
    config: Arc<RwLock<Config>>,
    memory: Arc<MemLock<Memory<Key<T>>>>,
}

impl<T: KeyParams> ServerBuilder<T> {
    pub fn new(config: Arc<RwLock<Config>>, memory: Arc<MemLock<Memory<Key<T>>>>) -> Self {
        Self { config, memory }
    }

    /// Spawns the serve loop as a tokio task and always returns `Ok` (MB-R-130): the UDP bind
    /// moves inside the retried task itself, so a bind failure does not fail `spawn()`
    /// synchronously — it surfaces from the joined `JoinHandle`, after exhausting
    /// retries (`reconnect: false`, MB-R-134) or never, if `reconnect` stays true and the
    /// caller eventually sends `ServerCommand::Terminate` (MB-R-133). `log` receives log
    /// lines, `status` receives lifecycle status lines.
    ///
    /// The returned `BoundAddr` is `None` until the socket actually binds and clears again
    /// once its serve loop ends — a caller that needs to know the socket is up (rather than
    /// merely that the task was scheduled) polls it instead of racing `spawn()`'s return with a
    /// fixed sleep (see [`BoundAddr`]).
    pub async fn spawn<L, St>(
        &self,
        receiver: Receiver<ServerCommand>,
        log: L,
        status: St,
    ) -> Result<(JoinHandle<Result<(), Error>>, BoundAddr), Error>
    where
        L: LogFn + Clone,
        St: LogFn + Clone,
    {
        let config = self.config.clone();
        let memory = self.memory.clone();
        let bound_addr: BoundAddr = Arc::new(parking_lot::Mutex::new(None));
        let handle = tokio::task::spawn(run(
            config,
            memory,
            receiver,
            log,
            status,
            bound_addr.clone(),
        ));
        Ok((handle, bound_addr))
    }
}

/// Every production server logs per-request outcomes (MB-R-067); UDP is no exception.
const VERBOSE: bool = true;

/// MB-R-128 — this transport is never physical Rtu/Ascii serial (RtuOverTcp/AsciiOverTcp ride
/// TCP; Tcp and Udp have no serial concept at all): an unmapped slave id keeps the ordinary
/// exception, same as MB-R-065/MB-R-060.
const PHYSICAL_SERIAL: bool = false;

/// Bind the configured UDP address and serve it, retrying the bind with the shared backoff
/// policy on failure (MB-R-120 revised, MB-R-130–134); each datagram answers from the shared
/// `memory` via a [`Server`] (verbose logging on, MB-R-067, mirroring the TCP server). No
/// accept loop, no TLS branch (MB-R-116 — `udp::Config` carries no `tls` field at all).
/// `ResetOn::Request`: `serve_udp` never calls `on_connect` at all (no connection concept for
/// UDP), so reading a datagram is the only "did something useful" signal.
async fn run<T, L, St>(
    config: Arc<RwLock<Config>>,
    memory: Arc<MemLock<Memory<Key<T>>>>,
    receiver: Receiver<ServerCommand>,
    log: L,
    status: St,
    bound_addr: BoundAddr,
) -> Result<(), Error>
where
    T: KeyParams,
    L: LogFn + Clone,
    St: LogFn + Clone,
{
    let receiver = AsyncMutex::new(receiver);
    let activity = Arc::new(AtomicBool::new(false));

    let attempt = || {
        let config = config.clone();
        let memory = memory.clone();
        let log = log.clone();
        let activity = activity.clone();
        let receiver = &receiver;
        let bound_addr = bound_addr.clone();
        async move {
            activity.store(false, Ordering::Relaxed);
            let guard = config.read().await;
            let reconnect = guard.reconnect;
            let addr: SocketAddr = match format!("{}:{}", guard.ip, guard.port).parse() {
                Ok(addr) => addr,
                Err(e) => {
                    return AttemptOutcome::Failed {
                        error: Error::Tcp(TcpError::Address(e)),
                        reconnect: false, // config errors never retry
                        reset: false,
                    };
                }
            };
            drop(guard);
            let mut receiver = receiver.lock().await;
            let mut parked: std::collections::VecDeque<ServerCommand> =
                std::collections::VecDeque::new();
            let bind_result = crate::common::race_terminate(
                UdpSocket::bind(addr),
                &mut receiver,
                |_: &ServerCommand| true,
                &mut parked,
            )
            .await;
            debug_assert!(
                parked.is_empty(),
                "ServerCommand has only one variant; nothing is ever parked"
            );
            match bind_result {
                None => AttemptOutcome::Done,
                Some(Err(e)) => AttemptOutcome::Failed {
                    error: Error::Server(e.into()),
                    reconnect,
                    reset: false,
                },
                Some(Ok(socket)) => {
                    let bound = match socket.local_addr() {
                        Ok(addr) => addr,
                        Err(e) => {
                            return AttemptOutcome::Failed {
                                error: Error::Server(e.into()),
                                reconnect,
                                reset: false,
                            };
                        }
                    };
                    *bound_addr.lock() = Some(bound);
                    let server = ModbusServer::new(
                        Server::new(memory.clone(), log.clone(), VERBOSE, PHYSICAL_SERIAL)
                            .with_reset_on(activity.clone(), ResetOn::Request),
                    );
                    let handle = server.handle();
                    let end = drive_serve(server.serve_udp(socket), handle, &mut receiver).await;
                    *bound_addr.lock() = None;
                    match end {
                        ServeEnd::Terminated => AttemptOutcome::Done,
                        ServeEnd::Failed(e) => AttemptOutcome::Failed {
                            error: Error::Server(e),
                            reconnect,
                            reset: activity.load(Ordering::Relaxed),
                        },
                    }
                }
            }
        }
    };

    let wait_abortable = |backoff: std::time::Duration| {
        let receiver = &receiver;
        async move {
            let mut receiver = receiver.lock().await;
            wait_reconnect_backoff(&mut receiver, backoff).await
        }
    };

    let result = run_with_backoff(BackoffPolicy::default(), attempt, wait_abortable).await;
    status.invoke("Server stopped".to_string()).await;
    result
}

#[cfg(test)]
mod tests {
    use super::{PHYSICAL_SERIAL, ServerBuilder, VERBOSE};
    use crate::udp::Config;
    use crate::{Key, ServerCommand, SlaveKey};

    /// MB-R-067 — the UDP server logs per-request outcomes exactly like every other transport.
    #[test]
    fn ut_udp_server_is_verbose() {
        const { assert!(VERBOSE) };
    }

    /// MB-R-128 — Udp is never physical-serial: an unmapped slave id keeps the ordinary
    /// exception.
    #[test]
    fn ut_udp_server_is_not_physical_serial() {
        const { assert!(!PHYSICAL_SERIAL) };
    }

    fn sink() -> impl crate::LogFn + Clone {
        |_s: String| async move {}
    }

    /// MB-R-120 revised (bound_addr companion) — same lifecycle as `tcp::server`'s own test:
    /// `None` before the first successful bind, `Some(<real addr>)` once bound, `None` again
    /// once `ServerCommand::Terminate` ends the serve loop.
    #[tokio::test]
    async fn ut_bound_addr_reflects_bind_lifecycle() {
        let config = Config {
            ip: "127.0.0.1".to_string(),
            port: 0,
            timeout_ms: 1000,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        };
        let memory = std::sync::Arc::new(parking_lot::RwLock::new(ferrowl_store::Memory::<
            Key<SlaveKey>,
        >::default()));
        let (tx, rx) = tokio::sync::mpsc::channel::<ServerCommand>(1);
        let (handle, bound_addr) = ServerBuilder::new(
            std::sync::Arc::new(tokio::sync::RwLock::new(config)),
            memory,
        )
        .spawn(rx, sink(), sink())
        .await
        .expect("spawn always returns Ok");

        let mut addr = None;
        for _ in 0..50 {
            addr = *bound_addr.lock();
            if addr.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let addr = addr.expect("socket must have bound within 1s");
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert_ne!(addr.port(), 0, "the OS must have assigned a real port");

        tx.send(ServerCommand::Terminate).await.unwrap();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
        assert!(
            bound_addr.lock().is_none(),
            "bound_addr must clear once the serve loop ends"
        );
    }
}
