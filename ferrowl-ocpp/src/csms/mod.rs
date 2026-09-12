//! CSMS = Charging Station Management System (server role; accepts CS connections).

mod action_handler;
mod command;
mod config;
mod core;
mod registry;
mod tls_stream;

use std::marker::PhantomData;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ferrowl_util::backoff::{AttemptOutcome, BackoffPolicy, run_with_backoff};
use parking_lot::Mutex;
use tokio::net::TcpListener;
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode};

use self::command::ConnCommand;
use self::tls_stream::ServerStream;
use crate::action::Version;
use crate::error::{CallError, Error};
use crate::log::LogFn;
use crate::ocppj::CallErrorCode;
use crate::security::{BasicAuth, SelfSignedCache, build_server_config};

pub use action_handler::CsmsActionHandler;
pub use command::Command;
pub use config::Config;
pub use registry::{ConnectionId, ConnectionRegistry};

/// Capacity of the server command channel and each per-connection channel.
const COMMAND_CHANNEL_CAP: usize = 32;

/// Builds and binds a CSMS server for a specific OCPP [`Version`].
pub struct ServerBuilder<V: Version> {
    config: Config,
    cache: SelfSignedCache,
    _v: PhantomData<fn() -> V>,
}

impl<V: Version> ServerBuilder<V>
where
    V::Action: Clone,
{
    /// `cache` should be created once per module instance (e.g. in the owning backend's `new()`,
    /// via [`crate::new_self_signed_cache`]) and reused across every `spawn` for that instance —
    /// never a fresh cache per call — so a `self_signed` server certificate that stays
    /// self-signed across a rebind or reconfigure is not regenerated needlessly (OC-R-037).
    pub fn new(config: Config, cache: SelfSignedCache) -> Self {
        Self {
            config,
            cache,
            _v: PhantomData,
        }
    }

    /// Spawn the server task, which binds the listening socket automatically (OC-R-083).
    /// `handler` answers inbound Calls for every connection.
    ///
    /// A *bind* failure does not fail the start synchronously; it is retried from inside the
    /// task (OC-R-083, OC-R-108, OC-R-109). With `config.reconnect` set (the default), a
    /// failed bind is retried using the same backoff policy as the Modbus client (MB-R-051); with
    /// it unset, a failed bind ends the module task with that error, surfaced from
    /// [`Server::join`]/[`Server::terminate`].
    /// [`Server::local_addr`] is `None` until the first successful bind.
    ///
    /// A TLS-configuration *build* failure is a different kind of error: it is deterministic
    /// given the config (e.g. a `ServerTlsPolicy::Mutual` whose `verification` cannot be
    /// built, OC-R-039) and retrying it can never succeed, unlike a transient bind failure — so it is
    /// still checked once, synchronously, here, and fails `spawn` immediately rather than
    /// silently retrying forever.
    pub async fn spawn<H, L>(self, handler: H, log: L) -> Result<Server<V>, Error>
    where
        H: CsmsActionHandler<V>,
        L: LogFn + Clone,
    {
        // OC-R-040: checked once, synchronously — a permanent misconfiguration, not a transient
        // failure the retry loop below should ever see.
        let tls = build_server_config(&self.config.tls, &self.config.host, &self.cache)?;
        // OC-R-095/OC-R-096: log when no TLS material at all was configured and the listener
        // falls back to an ephemeral self-signed certificate.
        if let Some((_, used_fallback)) = &tls
            && *used_fallback
        {
            log.invoke(
                "No cert_file/key_file/self_signed configured for this TLS server; falling \
                 back to an ephemeral self-signed certificate."
                    .to_string(),
            )
            .await;
        }
        let tls = tls.map(|(config, _)| config);

        let handler = Arc::new(handler);
        let registry = ConnectionRegistry::<V>::new();
        let (cmd_tx, cmd_rx) = mpsc::channel(COMMAND_CHANNEL_CAP);
        let local_addr = Arc::new(Mutex::new(None));

        let handle = tokio::spawn(run_reconnect_loop::<V, H, L>(
            self.config,
            handler,
            registry.clone(),
            cmd_rx,
            log,
            local_addr.clone(),
            tls,
        ));

        Ok(Server {
            cmd_tx,
            registry,
            local_addr,
            handle: Some(handle),
            _v: PhantomData,
        })
    }
}

/// A handle to a running CSMS server task.
pub struct Server<V: Version> {
    cmd_tx: mpsc::Sender<Command<V>>,
    registry: Arc<ConnectionRegistry<V>>,
    local_addr: Arc<Mutex<Option<SocketAddr>>>,
    handle: Option<JoinHandle<Result<(), Error>>>,
    _v: PhantomData<fn() -> V>,
}

impl<V: Version> Server<V> {
    /// OC-R-124's task-alive signal, mirrors `cs::Client::is_running`.
    pub fn is_running(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// The bound local address (useful when the configured port was `0`) — `None` while the
    /// listener has never bound yet, or is currently backing off from a failed bind attempt
    /// (OC-R-083).
    pub fn local_addr(&self) -> Option<SocketAddr> {
        *self.local_addr.lock()
    }

    /// Access the connection registry to enumerate connections or look up identities.
    pub fn registry(&self) -> &Arc<ConnectionRegistry<V>> {
        &self.registry
    }

    /// Clone of the server command sender.
    pub fn sender(&self) -> mpsc::Sender<Command<V>> {
        self.cmd_tx.clone()
    }

    /// Send a raw command to the server task.
    pub async fn send(&self, command: Command<V>) -> Result<(), Error> {
        self.cmd_tx
            .send(command)
            .await
            .map_err(|_| Error::ChannelClosed)
    }

    /// Send a Call to one connection and await its typed reply.
    pub async fn call(&self, conn: ConnectionId, action: V::Action) -> Result<V::Response, Error> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send(Command::SendToConnectionAwait(conn, action, reply_tx))
            .await?;
        match reply_rx.await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(call_err)) => Err(Error::Call(call_err)),
            Err(_) => Err(Error::ChannelClosed),
        }
    }

    /// Terminate the server (and all connections) and wait for it to finish.
    pub async fn terminate(mut self) -> Result<(), Error> {
        let _ = self.cmd_tx.send(Command::Terminate).await;
        self.join().await
    }

    /// Wait for the server task to finish.
    pub async fn join(&mut self) -> Result<(), Error> {
        if let Some(handle) = self.handle.take() {
            handle.await.map_err(|_| Error::NotRunning)??;
        }
        Ok(())
    }
}

/// Derive the OCPP-J charge-point identity from the request URL path's last non-empty segment.
fn identity_from_path(path: &str) -> Option<String> {
    path.rsplit('/')
        .find(|seg| !seg.is_empty())
        .map(str::to_owned)
}

/// Drive the listener-bind retry loop: bind the configured address and run the accept loop,
/// retrying a failed bind per [`BackoffPolicy`] when `config.reconnect` is set (OC-R-083,
/// OC-R-108, OC-R-109); with it unset, a failed bind ends the loop with that error. `tls` is
/// already built (a build failure is checked once, synchronously, by [`ServerBuilder::spawn`] —
/// OC-R-040 — and never retried here). Fills `local_addr` once bound, clears it again if the
/// accept loop ever ends (own `Terminate`/channel close) so a caller can tell "never
/// bound"/"backing off" apart from "bound".
#[allow(clippy::too_many_arguments)]
async fn run_reconnect_loop<V, H, L>(
    config: Config,
    handler: Arc<H>,
    registry: Arc<ConnectionRegistry<V>>,
    receiver: mpsc::Receiver<Command<V>>,
    log: L,
    local_addr: Arc<Mutex<Option<SocketAddr>>>,
    tls: Option<Arc<rustls::ServerConfig>>,
) -> Result<(), Error>
where
    V: Version,
    V::Action: Clone,
    H: CsmsActionHandler<V>,
    L: LogFn + Clone,
{
    // Shared between `attempt` and `wait_abortable`, called strictly sequentially by
    // `run_with_backoff` and never concurrently — same technique as `ferrowl_ocpp::cs` and
    // `ferrowl_modbus::server_core`.
    let receiver = AsyncMutex::new(receiver);

    let attempt = || {
        let config = &config;
        let handler = handler.clone();
        let registry = registry.clone();
        let log = log.clone();
        let local_addr = local_addr.clone();
        let receiver = &receiver;
        let tls = tls.clone();
        async move {
            let mut receiver = receiver.lock().await;
            let mut parked: std::collections::VecDeque<Command<V>> =
                std::collections::VecDeque::new();
            let bind_result = crate::cs::race_terminate(
                TcpListener::bind((config.host.as_str(), config.port)),
                &mut receiver,
                |cmd: &Command<V>| matches!(cmd, Command::Terminate),
                &mut parked,
            )
            .await;
            match bind_result {
                None => {
                    // OC-R-176 — terminate (or channel close) arrived while the listener bind
                    // itself was in flight: the attempt is abandoned, never having bound.
                    AttemptOutcome::Done
                }
                Some(Err(e)) => {
                    // OC-E-097 — a command parked while this failed bind was in flight is
                    // dropped here, with the same log line as a command arriving while the
                    // listener is not bound and backing off: there is no accept loop left to
                    // hand it to.
                    for _ in 0..parked.len() {
                        log.invoke(
                            "Command dropped: CSMS listener is not bound yet and retrying."
                                .to_string(),
                        )
                        .await;
                    }
                    AttemptOutcome::Failed {
                        error: Error::from(e),
                        reconnect: config.reconnect,
                        reset: false,
                    }
                }
                Some(Ok(listener)) => {
                    let addr = match listener.local_addr() {
                        Ok(addr) => addr,
                        Err(e) => {
                            return AttemptOutcome::Failed {
                                error: Error::from(e),
                                reconnect: config.reconnect,
                                reset: false,
                            };
                        }
                    };
                    *local_addr.lock() = Some(addr);
                    let activity = Arc::new(AtomicBool::new(false));
                    let mut commands = crate::cs::Commands::new(&mut receiver, parked);
                    accept_loop::<V, H, L>(
                        listener,
                        handler.clone(),
                        registry.clone(),
                        &mut commands,
                        log.clone(),
                        config.timeout(),
                        config.basic_auth.clone(),
                        tls,
                        activity,
                    )
                    .await;
                    *local_addr.lock() = None;
                    // `accept_loop` only ever returns via its own `Terminate`/channel-close arm
                    // today (its per-connection `accept()` error arm logs and loops forever
                    // rather than ending the loop) — there is no error return from it to
                    // classify as `Failed`, so this is unconditionally `Done`.
                    AttemptOutcome::Done
                }
            }
        }
    };

    let wait_abortable = |backoff: std::time::Duration| {
        let receiver = &receiver;
        let log = log.clone();
        async move {
            let mut receiver = receiver.lock().await;
            // There is nothing to send a non-`Terminate` command to while the listener isn't
            // bound yet, so it is dropped with a log line rather than queued. OC-R-109.
            ferrowl_util::backoff::wait_backoff(
                &mut receiver,
                backoff,
                "Command dropped: CSMS listener is not bound yet and retrying.",
                |cmd: &Command<V>| matches!(cmd, Command::Terminate),
                |msg| log.invoke(msg),
            )
            .await
        }
    };

    run_with_backoff(BackoffPolicy::default(), attempt, wait_abortable).await
}

/// The accept loop: hand-shakes new sockets and routes server-level commands.
#[allow(clippy::too_many_arguments)]
async fn accept_loop<V, H, L>(
    listener: TcpListener,
    handler: Arc<H>,
    registry: Arc<ConnectionRegistry<V>>,
    commands: &mut crate::cs::Commands<'_, Command<V>>,
    log: L,
    timeout: std::time::Duration,
    basic_auth: Option<BasicAuth>,
    tls: Option<Arc<rustls::ServerConfig>>,
    activity: Arc<AtomicBool>,
) where
    V: Version,
    V::Action: Clone,
    H: CsmsActionHandler<V>,
    L: LogFn + Clone,
{
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    // OC-R-108: the listener-bind backoff resets once the listener has bound and
                    // accepted at least one connection — recorded here, before the per-connection
                    // handshake, since accepting the TCP connection itself is the "useful thing
                    // happened" signal, not whether the handshake or OCPP-J traffic that follows
                    // succeeds.
                    activity.store(true, Ordering::Relaxed);
                    let handler = handler.clone();
                    let registry = registry.clone();
                    let log = log.clone();
                    let basic_auth = basic_auth.clone();
                    let tls = tls.clone();
                    tokio::spawn(async move {
                        let stream = match tls {
                            Some(tls_config) => {
                                let acceptor = tokio_rustls::TlsAcceptor::from(tls_config);
                                match acceptor.accept(stream).await {
                                    Ok(tls_stream) => ServerStream::Tls(Box::new(tls_stream)),
                                    Err(e) => {
                                        log.invoke(format!("CSMS TLS handshake failed from {peer}: {e}")).await;
                                        return;
                                    }
                                }
                            }
                            None => ServerStream::Plain(stream),
                        };
                        let identity_cell = Arc::new(Mutex::new(None));
                        let cell = identity_cell.clone();
                        let callback = move |req: &Request, mut resp: Response| {
                            *cell.lock() = identity_from_path(req.uri().path());
                            if let Some(auth) = &basic_auth
                                && !auth.matches(req.headers().get("authorization"))
                            {
                                return Err(reject_unauthorized());
                            }
                            if !subprotocol_matches(req, V::subprotocol()) {
                                return Err(reject_subprotocol());
                            }
                            resp.headers_mut().append(
                                "Sec-WebSocket-Protocol",
                                HeaderValue::from_static(V::subprotocol()),
                            );
                            Ok(resp)
                        };
                        let ws = match accept_hdr_async(stream, callback).await {
                            Ok(ws) => ws,
                            Err(e) => {
                                log.invoke(format!("CSMS handshake failed from {peer}: {e}")).await;
                                return;
                            }
                        };
                        let conn = registry.next_id();
                        let identity = identity_cell.lock().clone();
                        let (conn_tx, conn_rx) = mpsc::channel(COMMAND_CHANNEL_CAP);
                        registry.insert(conn, conn_tx, identity);
                        core::run_connection::<V, H, _, _>(
                            ws, handler, conn, conn_rx, registry.clone(), log, timeout,
                        )
                        .await;
                    });
                }
                Err(e) => log.invoke(format!("CSMS accept error: {e}")).await,
            },
            cmd = commands.recv() => match cmd {
                None | Some(Command::Terminate) => {
                    for tx in registry.all_senders() {
                        let _ = tx.send(ConnCommand::Terminate).await;
                    }
                    break;
                }
                Some(Command::SendToConnection(id, action)) => match registry.sender(id) {
                    Some(tx) => { let _ = tx.send(ConnCommand::Fire(action)).await; }
                    None => log.invoke(format!("CSMS: no such connection {id}")).await,
                },
                Some(Command::SendToConnectionAwait(id, action, reply_tx)) => match registry.sender(id) {
                    Some(tx) => { let _ = tx.send(ConnCommand::Call(action, reply_tx)).await; }
                    None => {
                        let _ = reply_tx.send(Err(CallError::new(
                            CallErrorCode::InternalError,
                            format!("no such connection {id}"),
                        )));
                    }
                },
                Some(Command::Broadcast(action)) => {
                    for tx in registry.all_senders() {
                        let _ = tx.send(ConnCommand::Fire(action.clone())).await;
                    }
                }
                Some(Command::DisconnectConnection(id)) => {
                    if let Some(tx) = registry.sender(id) {
                        let _ = tx.send(ConnCommand::Terminate).await;
                    }
                }
            },
        }
    }
}

/// Whether the handshake request advertises the expected subprotocol token.
fn subprotocol_matches(req: &Request, expected: &str) -> bool {
    req.headers()
        .get_all("sec-websocket-protocol")
        .iter()
        .any(|value| {
            value
                .to_str()
                .is_ok_and(|s| s.split(',').any(|t| t.trim() == expected))
        })
}

/// Build the 400 response used to reject a mismatched subprotocol.
fn reject_subprotocol() -> ErrorResponse {
    let mut resp = ErrorResponse::new(Some("unsupported OCPP subprotocol".to_owned()));
    *resp.status_mut() = StatusCode::BAD_REQUEST;
    resp
}

/// Build the 401 response used to reject a missing or mismatched Basic Auth credential
/// (Security Profile 1). Never includes the expected credential in the response body.
fn reject_unauthorized() -> ErrorResponse {
    let mut resp = ErrorResponse::new(Some("authentication required".to_owned()));
    *resp.status_mut() = StatusCode::UNAUTHORIZED;
    resp
}

#[cfg(test)]
mod tests {
    use super::identity_from_path;

    #[test]
    /// OC-R-044 — the charge-point identity is the last non-empty path segment of the URL.
    fn ut_identity_is_last_non_empty_path_segment() {
        assert_eq!(identity_from_path("/ocpp/CS001").as_deref(), Some("CS001"));
        // A trailing slash is skipped to the previous non-empty segment.
        assert_eq!(identity_from_path("/ocpp/CS002/").as_deref(), Some("CS002"));
        // A single segment with no leading slash still works.
        assert_eq!(identity_from_path("CP42").as_deref(), Some("CP42"));
        // No non-empty segment yields no identity.
        assert_eq!(identity_from_path("/"), None);
        assert_eq!(identity_from_path(""), None);
    }

    #[cfg(feature = "v1_6")]
    #[tokio::test]
    /// OC-R-108 — the listener-bind backoff's reset hook fires once the listener has accepted at
    /// least one connection: `accept_loop` sets the shared activity flag before the per-connection
    /// handshake starts (a bare TCP `connect()`, no OCPP-J traffic, is enough to observe it).
    async fn ut_accept_loop_marks_activity_on_first_accept() {
        use std::sync::atomic::{AtomicBool, Ordering};

        use tokio::sync::mpsc;

        use crate::action::v1_6::V1_6;

        struct NoopHandler;
        impl super::CsmsActionHandler<V1_6> for NoopHandler {
            async fn handle_call(
                &self,
                _conn: super::ConnectionId,
                _action: crate::action::v1_6::Action,
            ) -> Result<crate::action::v1_6::Response, crate::CallError> {
                Err(crate::CallError::new(
                    crate::CallErrorCode::NotImplemented,
                    "unsupported",
                ))
            }
        }

        let listener = super::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handler = std::sync::Arc::new(NoopHandler);
        let registry = super::ConnectionRegistry::<V1_6>::new();
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<super::Command<V1_6>>(4);
        let mut commands = crate::cs::Commands::new(&mut cmd_rx, std::collections::VecDeque::new());
        let activity = std::sync::Arc::new(AtomicBool::new(false));

        let accept_fut = super::accept_loop::<V1_6, NoopHandler, _>(
            listener,
            handler,
            registry,
            &mut commands,
            |_s: String| async move {},
            std::time::Duration::from_secs(1),
            None,
            None,
            activity.clone(),
        );
        tokio::pin!(accept_fut);

        // A bare TCP connect is enough: the activity flag is set before the per-connection
        // handshake, not after it completes.
        let connect_fut = async {
            let _client = tokio::net::TcpStream::connect(addr)
                .await
                .expect("connect to accept_loop's listener");
            for _ in 0..50 {
                if activity.load(Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            let _ = cmd_tx.send(super::Command::Terminate).await;
        };

        tokio::select! {
            _ = &mut accept_fut => {}
            _ = connect_fut => {
                accept_fut.await;
            }
        }

        assert!(
            activity.load(Ordering::Relaxed),
            "accept_loop must mark activity once it accepts a connection"
        );
    }

    #[cfg(feature = "v1_6")]
    #[tokio::test]
    /// OC-R-176 — a terminate arriving while the listener bind is still in flight aborts the
    /// race immediately, well inside the bind's own timeout, matching the abort every
    /// `run_reconnect_loop` bind race performs.
    async fn ut_race_terminate_abandons_bind_on_terminate() {
        use crate::action::v1_6::V1_6;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<super::Command<V1_6>>(1);
        tx.send(super::Command::Terminate).await.unwrap();
        let mut parked = std::collections::VecDeque::new();

        let out = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            crate::cs::race_terminate(
                std::future::pending::<std::io::Result<super::TcpListener>>(),
                &mut rx,
                |cmd: &super::Command<V1_6>| matches!(cmd, super::Command::Terminate),
                &mut parked,
            ),
        )
        .await
        .expect("race_terminate did not return promptly");

        assert!(out.is_none(), "the pending bind is abandoned");
    }

    #[cfg(feature = "v1_6")]
    #[tokio::test]
    /// OC-R-124 — `is_running()` reports `true` while the server task is alive, and `false` once
    /// it has ended (here, via an explicit `Command::Terminate` sent without consuming `self`, so
    /// the handle stays available to poll afterward).
    async fn ut_server_is_running_false_before_spawn_true_after_until_terminate() {
        use crate::action::v1_6::V1_6;
        use crate::security::new_self_signed_cache;

        struct NoopHandler;
        impl super::CsmsActionHandler<V1_6> for NoopHandler {
            async fn handle_call(
                &self,
                _conn: super::ConnectionId,
                _action: crate::action::v1_6::Action,
            ) -> Result<crate::action::v1_6::Response, crate::CallError> {
                Err(crate::CallError::new(
                    crate::CallErrorCode::NotImplemented,
                    "unsupported",
                ))
            }
        }

        let config = super::Config {
            host: "127.0.0.1".to_string(),
            port: 0,
            timeout_ms: 1_000,
            reconnect: false,
            basic_auth: None,
            tls: Default::default(),
        };
        let server = super::ServerBuilder::<V1_6>::new(config, new_self_signed_cache())
            .spawn(NoopHandler, |_s: String| async move {})
            .await
            .unwrap();

        assert!(
            server.is_running(),
            "a freshly spawned server task must be running"
        );

        server.send(super::Command::Terminate).await.unwrap();

        let mut running = true;
        for _ in 0..50 {
            running = server.is_running();
            if !running {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            !running,
            "the server task must stop running after Terminate"
        );
    }
}
