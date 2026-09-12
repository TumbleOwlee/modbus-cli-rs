//! CS = Charging Station (client role; dials out to a CSMS).

mod action_handler;
mod command;
mod config;
mod core;

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use ferrowl_util::backoff::{AttemptOutcome, BackoffPolicy, run_with_backoff};
use tokio::net::TcpStream;
use tokio::sync::{Mutex as AsyncMutex, RwLock, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async_tls_with_config;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::action::Version;
use crate::error::{Error, WsError};
use crate::log::LogFn;
use crate::security::{SelfSignedCache, build_connector};

pub use action_handler::CsActionHandler;
pub use command::Command;
pub use config::Config;

/// Capacity of the command channel between a [`Client`] handle and its task.
const COMMAND_CHANNEL_CAP: usize = 32;

/// The concrete websocket stream type a successful dial produces.
type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Dial the configured CSMS (advertising `V::subprotocol()`). Extracted out of `spawn`'s old
/// synchronous connect so it can be retried from inside the reconnect loop (OC-R-048/OC-R-105).
async fn dial<V: Version>(config: &Config, cache: &SelfSignedCache) -> Result<Ws, Error> {
    let mut request = config
        .url
        .as_str()
        .into_client_request()
        .map_err(WsError::from)?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_static(V::subprotocol()),
    );
    if let Some(auth) = &config.basic_auth {
        request
            .headers_mut()
            .insert("Authorization", auth.header_value());
    }
    for header in &config.extra_headers {
        request.headers_mut().insert(
            HeaderName::from_bytes(header.name.as_bytes())
                .expect("HeaderDef::new already validated the name is a valid token"),
            HeaderValue::from_str(&header.value)
                .expect("HeaderDef::new already validated the value is printable ASCII"),
        );
    }
    // OC-R-097: the scheme decides the transport, so a `ws://` endpoint's `tls` must be
    // neither validated nor loaded.
    let connector = if request.uri().scheme_str() == Some("wss") {
        Some(build_connector(&config.tls, cache)?)
    } else {
        None
    };
    let (ws, _response) = connect_async_tls_with_config(request, None, false, connector)
        .await
        .map_err(WsError::from)?;
    Ok(ws)
}

/// What happened during one connection attempt, as fed to [`classify_attempt`].
enum AttemptResult {
    /// The dial itself failed; no handshake ever completed.
    DialFailed(Error),
    /// `core::run` ended via `Command::Terminate` (or the command channel closing).
    Terminated,
    /// `core::run` ended because the connection dropped, after a completed handshake.
    Disconnected,
}

/// Classifies one connection attempt's result into [`AttemptOutcome`] (OC-R-048, OC-R-105). Pure
/// and free of I/O so it is directly unit-testable without waiting out a real backoff — in
/// particular, OC-R-105's "reset unconditionally after any completed handshake, regardless of
/// whether any OCPP message was subsequently exchanged" is exactly the difference between the
/// `DialFailed` and `Disconnected` arms below.
fn classify_attempt(result: AttemptResult, reconnect: bool) -> AttemptOutcome<Error> {
    match result {
        AttemptResult::DialFailed(error) => AttemptOutcome::Failed {
            error,
            reconnect,
            // `reset` is about *this* run having completed a handshake, not about the failed
            // dial that never got one.
            reset: false,
        },
        AttemptResult::Terminated => AttemptOutcome::Done,
        AttemptResult::Disconnected => AttemptOutcome::Failed {
            error: Error::Disconnected,
            reconnect,
            // The handshake that started this `core::run` call did complete (we only reach
            // `Disconnected` after a successful `dial`), so the reset condition is met
            // unconditionally here, regardless of whether any OCPP message was exchanged.
            reset: true,
        },
    }
}

/// A command channel with a queue in front of it: whatever a dial/bind race parked while the
/// attempt was running is handed out before anything newer off the channel, so a command sent
/// concurrently with a dial is delivered once connected instead of being lost.
pub(crate) struct Commands<'a, C> {
    receiver: &'a mut mpsc::Receiver<C>,
    queued: std::collections::VecDeque<C>,
}

impl<'a, C> Commands<'a, C> {
    /// `queued` is what a preceding `race_terminate` parked; empty when nothing was parked.
    pub(crate) fn new(
        receiver: &'a mut mpsc::Receiver<C>,
        queued: std::collections::VecDeque<C>,
    ) -> Self {
        Self { receiver, queued }
    }

    /// Cancel-safe, so it can sit in a `tokio::select!` arm exactly where `receiver.recv()` did:
    /// the queue pop is synchronous and returns without awaiting, and `Receiver::recv` is itself
    /// cancel-safe.
    pub(crate) async fn recv(&mut self) -> Option<C> {
        match self.queued.pop_front() {
            Some(c) => Some(c),
            None => self.receiver.recv().await,
        }
    }
}

/// Awaits `fut` while watching `receiver`, so an in-flight dial or bind is abandoned the moment a
/// terminate-matching command arrives instead of running to its own success or failure (OC-R-175,
/// OC-R-176). Returns `Some(output)` when `fut` finished first, `None` when terminate arrived or
/// the channel closed — the caller drops `fut`, cancelling the attempt. Any other command is
/// moved into `parked` for the caller to hand to the connection loop it is about to start (or to
/// dispose of if the attempt failed); nothing is dropped here.
pub(crate) async fn race_terminate<T, C, F>(
    fut: F,
    receiver: &mut mpsc::Receiver<C>,
    is_terminate: impl Fn(&C) -> bool,
    parked: &mut std::collections::VecDeque<C>,
) -> Option<T>
where
    F: std::future::Future<Output = T>,
{
    tokio::pin!(fut);
    loop {
        tokio::select! {
            out = &mut fut => return Some(out),
            cmd = receiver.recv() => match cmd {
                None => return None,
                Some(c) if is_terminate(&c) => return None,
                Some(c) => parked.push_back(c),
            },
        }
    }
}

/// Drive the retry loop: dial the configured CSMS and run the connection, retrying a failed
/// dial or a dropped connection per [`BackoffPolicy`] when `config.reconnect` is set (OC-R-048,
/// OC-R-105–107). `status` receives a "Client disconnected" line once the task ends, regardless
/// of why (mirrors `ferrowl_modbus`'s server tasks logging "Server stopped" the same way).
async fn run_reconnect_loop<V, H, L, St>(
    config: Arc<RwLock<Config>>,
    cache: SelfSignedCache,
    handler: Arc<H>,
    receiver: mpsc::Receiver<Command<V>>,
    log: L,
    status: St,
) -> Result<(), Error>
where
    V: Version,
    H: CsActionHandler<V>,
    L: LogFn + Clone,
    St: LogFn + Clone,
{
    // Shared between `attempt` and `wait_abortable`, called strictly sequentially by
    // `run_with_backoff` and never concurrently — same technique as
    // `ferrowl_modbus::server_core`'s TCP server.
    let receiver = AsyncMutex::new(receiver);

    let attempt = || {
        let config = config.clone();
        let cache = cache.clone();
        let handler = handler.clone();
        let log = log.clone();
        let receiver = &receiver;
        async move {
            let guard = config.read().await;
            let reconnect = guard.reconnect;
            let timeout = guard.timeout();
            let mut receiver = receiver.lock().await;
            let mut parked: std::collections::VecDeque<Command<V>> =
                std::collections::VecDeque::new();
            let dial_result = race_terminate(
                dial::<V>(&guard, &cache),
                &mut receiver,
                |cmd: &Command<V>| matches!(cmd, Command::Terminate),
                &mut parked,
            )
            .await;
            drop(guard);
            let dial_result = match dial_result {
                Some(dial_result) => dial_result,
                None => {
                    // OC-R-175 — terminate (or channel close) arrived while the dial itself was
                    // in flight: the attempt is abandoned, never having reached a connected
                    // state. `status.invoke` already runs once `run_with_backoff` returns below.
                    return AttemptOutcome::Done;
                }
            };
            match dial_result {
                Err(e) => {
                    // OC-E-097 — a command parked while this failed dial was in flight is
                    // dropped here, with the same log line as a command arriving while backing
                    // off: there is no connection loop left to hand it to.
                    for _ in 0..parked.len() {
                        log.invoke(
                            "Command dropped: client is disconnected and reconnecting.".to_string(),
                        )
                        .await;
                    }
                    log.invoke(format!("{e}")).await;
                    classify_attempt(AttemptResult::DialFailed(e), reconnect)
                }
                Ok(ws) => {
                    let mut commands = Commands::new(&mut receiver, parked);
                    let run_end = core::run::<V, H, _, _>(
                        ws,
                        handler.clone(),
                        &mut commands,
                        log.clone(),
                        timeout,
                    )
                    .await;
                    let attempt_result = match run_end {
                        core::RunEnd::Terminated => AttemptResult::Terminated,
                        core::RunEnd::Disconnected => {
                            // `core::RunEnd::Disconnected` carries no underlying error — the
                            // reader task already logs the specific reason ("websocket error:
                            // {e}", "OCPP-J framing error: {e}") for most drops, but a clean
                            // peer-initiated close carries none, so OC-R-114's "log the failure
                            // reason" is satisfied here with a fixed reason string covering every
                            // path uniformly.
                            log.invoke("Connection dropped.".to_string()).await;
                            AttemptResult::Disconnected
                        }
                    };
                    classify_attempt(attempt_result, reconnect)
                }
            }
        }
    };

    let wait_abortable = |backoff: Duration| {
        let receiver = &receiver;
        let log = log.clone();
        async move {
            log.invoke(format!("Reconnecting in {}s.", backoff.as_secs()))
                .await;
            let mut receiver = receiver.lock().await;
            // Any command other than `Terminate` received while disconnected is dropped with a
            // log line rather than queued for after reconnect — a `SendActionAwait`'s
            // `oneshot::Sender` is simply dropped, which naturally surfaces `Error::ChannelClosed`
            // to whichever caller was awaiting the reply (the same failure mode a closed channel
            // already produces elsewhere). OC-R-106.
            ferrowl_util::backoff::wait_backoff(
                &mut receiver,
                backoff,
                "Command dropped: client is disconnected and reconnecting.",
                |cmd: &Command<V>| matches!(cmd, Command::Terminate),
                |msg| log.invoke(msg),
            )
            .await
        }
    };

    let result = run_with_backoff(BackoffPolicy::default(), attempt, wait_abortable).await;
    status.invoke("Client disconnected".to_string()).await;
    result
}

/// Builds and connects a CS client for a specific OCPP [`Version`].
pub struct ClientBuilder<V: Version> {
    config: Arc<RwLock<Config>>,
    cache: SelfSignedCache,
    _v: PhantomData<fn() -> V>,
}

impl<V: Version> ClientBuilder<V> {
    /// `cache` should be created once per module instance (e.g. in the owning backend's `new()`,
    /// via [`crate::new_self_signed_cache`]) and reused across every `spawn` for that instance —
    /// never a fresh cache per call — so a `client_self_signed` identity that stays self-signed
    /// across a reconnect or reconfigure is not regenerated needlessly (OC-R-115).
    pub fn new(config: Arc<RwLock<Config>>, cache: SelfSignedCache) -> Self {
        Self {
            config,
            cache,
            _v: PhantomData,
        }
    }

    /// Spawn the client task, which dials the configured CSMS (advertising `V::subprotocol()`).
    ///
    /// `handler` answers CSMS-initiated Calls. For the low-level API pass a [`CsActionHandler`];
    /// for the semantic API pass [`SemanticAdapter::new(your_cs_handler)`](SemanticAdapter).
    ///
    /// `spawn` itself always returns `Ok` — a dial or mid-connection failure does not fail the
    /// start synchronously; it surfaces from [`Client::join`] (OC-R-048/OC-R-105). With
    /// `config.reconnect` set (the default), a failed dial or a dropped connection does not end
    /// the task: it logs, waits an exponential backoff (capped, reset after a connection whose
    /// handshake completed), and retries. `status` receives a "Client disconnected" line once
    /// the task ends, regardless of why.
    pub async fn spawn<H, L, St>(self, handler: H, log: L, status: St) -> Result<Client<V>, Error>
    where
        H: CsActionHandler<V>,
        L: LogFn + Clone,
        St: LogFn + Clone,
    {
        let (cmd_tx, cmd_rx) = mpsc::channel(COMMAND_CHANNEL_CAP);
        let handle = tokio::spawn(run_reconnect_loop::<V, H, _, _>(
            self.config,
            self.cache,
            Arc::new(handler),
            cmd_rx,
            log,
            status,
        ));

        Ok(Client {
            cmd_tx,
            handle: Some(handle),
            _v: PhantomData,
        })
    }
}

/// A handle to a running CS client task. Send typed [`Command`]s, or use [`Client::call`] /
/// [`Client::notify`] to drive actions directly.
pub struct Client<V: Version> {
    cmd_tx: mpsc::Sender<Command<V>>,
    handle: Option<JoinHandle<Result<(), Error>>>,
    _v: PhantomData<fn() -> V>,
}

impl<V: Version> Client<V> {
    /// OC-R-123's task-alive signal, mirrors `ferrowl_modbus::instance::Handle::is_finished`.
    pub fn is_running(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }

    /// Clone of the command sender, for drivers that want to hold their own.
    pub fn sender(&self) -> mpsc::Sender<Command<V>> {
        self.cmd_tx.clone()
    }

    /// Send a Call and await its reply over a cloned command sender, without borrowing the
    /// [`Client`]. Lets a caller spawn the round-trip off-thread so a non-responding peer never
    /// blocks the caller. Same semantics as [`Client::call`].
    pub async fn call_via(
        cmd_tx: &mpsc::Sender<Command<V>>,
        action: V::Action,
    ) -> Result<V::Response, Error> {
        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(Command::SendActionAwait(action, reply_tx))
            .await
            .map_err(|_| Error::ChannelClosed)?;
        match reply_rx.await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(call_err)) => Err(Error::Call(call_err)),
            Err(_) => Err(Error::ChannelClosed),
        }
    }

    /// Send a raw command to the client task.
    pub async fn send(&self, command: Command<V>) -> Result<(), Error> {
        self.cmd_tx
            .send(command)
            .await
            .map_err(|_| Error::ChannelClosed)
    }

    /// Send a Call and await its typed reply. A peer rejection is surfaced as [`Error::Call`].
    pub async fn call(&self, action: V::Action) -> Result<V::Response, Error> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.send(Command::SendActionAwait(action, reply_tx))
            .await?;
        match reply_rx.await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(call_err)) => Err(Error::Call(call_err)),
            Err(_) => Err(Error::ChannelClosed),
        }
    }

    /// Send a Call without awaiting its reply.
    pub async fn notify(&self, action: V::Action) -> Result<(), Error> {
        self.send(Command::SendAction(action)).await
    }

    /// Terminate the client task and wait for it to finish.
    pub async fn terminate(mut self) -> Result<(), Error> {
        let _ = self.cmd_tx.send(Command::Terminate).await;
        self.join().await
    }

    /// Wait for the client task to finish.
    pub async fn join(&mut self) -> Result<(), Error> {
        match self.handle.take() {
            Some(handle) => handle.await.map_err(|_| Error::NotRunning)?,
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CsActionHandler;
    use super::{AttemptOutcome, AttemptResult, ClientBuilder, Config, classify_attempt};
    use crate::error::{CallError, Error};
    use crate::log::LogFn;
    use crate::security::new_self_signed_cache;
    use crate::{Action16, CallErrorCode, Response16, V1_6};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::RwLock;

    /// OC-R-105 — a failed dial never resets the backoff (no handshake ever completed), but the
    /// caller's freshly-read `reconnect` flag still passes through unchanged.
    #[test]
    fn ut_classify_attempt_dial_failed_never_resets() {
        match classify_attempt(AttemptResult::DialFailed(Error::ChannelClosed), true) {
            AttemptOutcome::Failed {
                reconnect, reset, ..
            } => {
                assert!(reconnect);
                assert!(!reset);
            }
            AttemptOutcome::Done => panic!("expected Failed"),
        }
    }

    /// OC-R-106 — an explicit terminate (or the command channel closing) ends the retry loop
    /// gracefully, regardless of `reconnect`.
    #[test]
    fn ut_classify_attempt_terminated_is_done() {
        assert!(matches!(
            classify_attempt(AttemptResult::Terminated, true),
            AttemptOutcome::Done
        ));
        assert!(matches!(
            classify_attempt(AttemptResult::Terminated, false),
            AttemptOutcome::Done
        ));
    }

    /// OC-R-105 — a connection that completed its handshake and then dropped always resets the
    /// backoff, regardless of whether any OCPP message was subsequently exchanged (this pure
    /// function cannot see whether one was — that's exactly the point).
    #[test]
    fn ut_classify_attempt_disconnected_always_resets() {
        match classify_attempt(AttemptResult::Disconnected, true) {
            AttemptOutcome::Failed {
                reconnect, reset, ..
            } => {
                assert!(reconnect);
                assert!(reset);
            }
            AttemptOutcome::Done => panic!("expected Failed"),
        }
    }

    /// OC-R-048, OC-R-107 — the freshly-read `reconnect` flag passes through unchanged on a
    /// dropped connection too.
    #[test]
    fn ut_classify_attempt_disconnected_honors_reconnect_false() {
        match classify_attempt(AttemptResult::Disconnected, false) {
            AttemptOutcome::Failed { reconnect, .. } => assert!(!reconnect),
            AttemptOutcome::Done => panic!("expected Failed"),
        }
    }

    /// A `LogFn` that records every line into a shared buffer for assertions. Mirrors
    /// `ferrowl_modbus::client_core`'s own `recording_log()` fixture.
    fn recording_log() -> (impl LogFn + Clone, Arc<parking_lot::Mutex<Vec<String>>>) {
        let lines = Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
        let sink = lines.clone();
        let log = move |s: String| {
            let sink = sink.clone();
            async move {
                sink.lock().push(s);
            }
        };
        (log, lines)
    }

    /// CS handler; these tests never receive a server-initiated Call.
    struct TestCs;

    impl CsActionHandler<V1_6> for TestCs {
        async fn handle_call(&self, _action: Action16) -> Result<Response16, CallError> {
            Err(CallError::new(CallErrorCode::NotImplemented, "unsupported"))
        }
    }

    fn config(reconnect: bool) -> Arc<RwLock<Config>> {
        Arc::new(RwLock::new(Config {
            url: format!(
                "ws://127.0.0.1:{}/CS001",
                ferrowl_test_support::reserve_tcp_port().release()
            ),
            timeout_ms: 1_000,
            basic_auth: None,
            tls: Default::default(),
            extra_headers: Vec::new(),
            reconnect,
        }))
    }

    /// OC-R-114 — a failed dial (`reconnect: false`, so the task ends after the single attempt)
    /// logs the dial error's `Display` text before the task returns.
    #[tokio::test]
    async fn ut_dial_failure_logs_reason() {
        let (log, lines) = recording_log();
        let (status, _status_lines) = recording_log();
        let mut client = ClientBuilder::<V1_6>::new(config(false), new_self_signed_cache())
            .spawn(TestCs, log, status)
            .await
            .unwrap();

        let result = client.join().await;

        let err =
            result.expect_err("a refused dial with reconnect: false ends the task with an error");
        let err_text = err.to_string();
        assert!(
            lines.lock().iter().any(|l| l.contains(&err_text)),
            "expected the dial failure reason ({err_text:?}) to be logged, got: {:?}",
            lines.lock()
        );
    }

    /// OC-R-114 — with `reconnect: true` against a dead port, the first backoff wait (MB-R-051's
    /// shared 1s-start policy, OC-R-106) is logged before the task waits it out.
    #[tokio::test]
    async fn ut_backoff_wait_logs_duration() {
        let (log, lines) = recording_log();
        let (status, _status_lines) = recording_log();
        let client = ClientBuilder::<V1_6>::new(config(true), new_self_signed_cache())
            .spawn(TestCs, log, status)
            .await
            .unwrap();

        // Give the reconnect loop time to fail its first dial and log the backoff wait, then
        // terminate before it retries.
        tokio::time::sleep(Duration::from_millis(300)).await;
        client.terminate().await.unwrap();

        assert!(
            lines
                .lock()
                .iter()
                .any(|l| l.contains("Reconnecting in 1s.")),
            "expected a logged backoff-wait duration line, got: {:?}",
            lines.lock()
        );
    }

    #[derive(Debug, PartialEq)]
    enum TestCmd {
        Terminate,
        Other(u32),
    }

    #[tokio::test]
    /// OC-R-175, OC-R-176 — a terminate-matching command arriving while the attempt is still
    /// pending abandons it at once instead of waiting for it to resolve.
    async fn ut_race_terminate_abandons_dial_on_terminate() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TestCmd>(1);
        tx.send(TestCmd::Terminate).await.unwrap();
        let mut parked = std::collections::VecDeque::new();

        let out = tokio::time::timeout(
            Duration::from_millis(250),
            super::race_terminate(
                std::future::pending::<()>(),
                &mut rx,
                |cmd: &TestCmd| matches!(cmd, TestCmd::Terminate),
                &mut parked,
            ),
        )
        .await
        .expect("race_terminate did not return promptly");

        assert_eq!(out, None);
    }

    #[tokio::test]
    /// `OC-E-097` — a non-terminate command arriving while the attempt is still in flight is
    /// parked, not dropped: the race continues, the future still wins, and the command survives
    /// for the next `Commands::recv()` to hand out.
    async fn ut_race_terminate_queues_non_terminate_command() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TestCmd>(1);
        tx.send(TestCmd::Other(7)).await.unwrap();
        let mut parked = std::collections::VecDeque::new();

        let out = super::race_terminate(
            async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                "connected"
            },
            &mut rx,
            |cmd: &TestCmd| matches!(cmd, TestCmd::Terminate),
            &mut parked,
        )
        .await;

        assert_eq!(out, Some("connected"));
        assert_eq!(
            parked.into_iter().collect::<Vec<_>>(),
            vec![TestCmd::Other(7)]
        );

        let (_tx2, mut rx2) = tokio::sync::mpsc::channel::<TestCmd>(1);
        let mut queued = std::collections::VecDeque::new();
        queued.push_back(TestCmd::Other(7));
        let mut commands = super::Commands::new(&mut rx2, queued);
        assert_eq!(commands.recv().await, Some(TestCmd::Other(7)));
    }
}
