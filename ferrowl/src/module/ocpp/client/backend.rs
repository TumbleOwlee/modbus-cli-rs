//! Version-generic OCPP charging-station backend: wraps the `ferrowl-ocpp` `Client<V>` for any
//! OCPP version, tracks websocket online state, and records every request/response payload for the
//! view. Sending is fully generic — the caller builds a typed `V::Action` (e.g. from a UI action
//! button) and the backend records the request JSON, awaits the reply, and records the response.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde_json::Value;
use tokio::sync::RwLock;
use tokio::sync::mpsc;

use ferrowl_ocpp::cs::{Client, ClientBuilder, Command, Config, CsActionHandler};
use ferrowl_ocpp::{Error, Version};

use crate::module::ocpp::client::view::ClientVersion;
use crate::module::ocpp::config::device::OcppDeviceConfig;
use crate::module::ocpp::config::session::OcppSpec;
use crate::module::ocpp::scope::Scope;
use crate::module::ocpp::wire_log::{encode_action_or_log, encode_response_or_log};
use crate::module::view::SharedLog;

/// Number of `refresh` ticks per second (the UI ticks at ~100ms), used to convert second-based
/// cadences (heartbeat interval, MeterValues period) into tick counts.
pub const TICKS_PER_SEC: u32 = 10;

/// Fallback heartbeat cadence (seconds) used until the CSMS supplies one in its BootNotification
/// response (or when it sends `0`).
pub const DEFAULT_HEARTBEAT_SECS: u64 = 30;

/// Extract the heartbeat cadence from a BootNotification response. The CSMS dictates the interval;
/// an absent or zero value yields `None`, so the caller falls back to [`DEFAULT_HEARTBEAT_SECS`].
pub fn boot_interval(response: &Value) -> Option<u64> {
    response["interval"].as_u64().filter(|&i| i > 0)
}

/// Direction of a recorded OCPP message relative to this charging station.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Received from the CSMS (an inbound Call, or a reply to our Call).
    In,
    /// Sent to the CSMS (our Call, or our reply to an inbound Call).
    Out,
}

impl Dir {
    pub fn label(self) -> &'static str {
        match self {
            Dir::In => "Inbound",
            Dir::Out => "Outbound",
        }
    }
}

/// Largest number of messages kept in an in-memory message-log buffer. Older messages are evicted;
/// the full history survives only in the `:log <file>` sink (via the per-module `SharedLog`).
pub const MAX_MESSAGES: usize = 200;

/// Monotonic message sequence, so a view can tee only the messages it hasn't logged yet even as the
/// bounded buffer evicts older ones. Starts at 1 so a cursor initialised to 0 logs the first message.
static MSG_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_seq() -> u64 {
    MSG_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// One observed OCPP message, for the message-log table and the JSON view.
#[derive(Debug, Clone)]
pub struct OcppMessage {
    /// Monotonic id assigned at creation (see [`MSG_SEQ`]); used by the log tee cursor.
    pub seq: u64,
    pub ts: u64,
    pub direction: Dir,
    pub name: String,
    pub payload: Value,
    /// Outcome: `Some(true)` = success, `Some(false)` = error, `None` = neutral (e.g. a request).
    pub ok: Option<bool>,
    /// Extra context: a status string on success, an error message on failure.
    pub context: String,
    /// The connector/CS scope this message belongs to, for the client view's per-connector message
    /// filtering. CS-level (Boot/Heartbeat/etc.) is [`Scope::CS`]; connector traffic carries its
    /// connector scope. The server view ignores this (it routes by its own per-entry log).
    pub scope: Scope,
}

impl OcppMessage {
    /// Build a CS-level message, stamping it with the current time and the next sequence id.
    pub fn new(
        direction: Dir,
        name: impl Into<String>,
        payload: Value,
        ok: Option<bool>,
        context: impl Into<String>,
    ) -> Self {
        Self::new_scoped(Scope::CS, direction, name, payload, ok, context)
    }

    /// Build a message tagged with a connector/CS [`Scope`].
    pub fn new_scoped(
        scope: Scope,
        direction: Dir,
        name: impl Into<String>,
        payload: Value,
        ok: Option<bool>,
        context: impl Into<String>,
    ) -> Self {
        Self {
            seq: next_seq(),
            ts: ferrowl_util::time::now_unix_ms(),
            direction,
            name: name.into(),
            payload,
            ok,
            context: context.into(),
            scope,
        }
    }

    /// A one-line rendering for the persistent log: direction, name, outcome, context, payload.
    pub fn log_line(&self) -> String {
        let dir = match self.direction {
            Dir::In => "<-",
            Dir::Out => "->",
        };
        let status = match self.ok {
            Some(true) => " ok",
            Some(false) => " ERR",
            None => "",
        };
        let ctx = if self.context.is_empty() {
            String::new()
        } else {
            format!(" ({})", self.context)
        };
        let payload = serde_json::to_string(&self.payload).unwrap_or_default();
        format!("{dir} {}{status}{ctx} {payload}", self.name)
    }
}

pub type Messages = Arc<RwLock<Vec<OcppMessage>>>;

/// Push a message into a buffer, evicting the oldest once it exceeds [`MAX_MESSAGES`].
pub fn push_capped(buf: &mut Vec<OcppMessage>, msg: OcppMessage) {
    buf.push(msg);
    if buf.len() > MAX_MESSAGES {
        let overflow = buf.len() - MAX_MESSAGES;
        buf.drain(0..overflow);
    }
}

// --- Message table -----------------------------------------------------------
//
// Shared between the client and server views' message-log tables (identical row shape, styling,
// and construction from an `OcppMessage` in both), kept here next to `OcppMessage` itself.

#[derive(Clone, Debug, ferrowl_ui_derive::TableEntry)]
#[table_entry(header = MsgHeader, styles = msg_cell_styles)]
pub(crate) struct MsgRow {
    #[column(name = "Timestamp", min = 23, max = 23)]
    timestamp: String,
    #[column(name = "Direction", min = 8, max = 10)]
    direction: String,
    #[column(name = "Message", min = 14, max = 30)]
    name: String,
    #[column(name = "Status", min = 7, max = 8)]
    status: String,
    #[column(name = "Context", min = 6, max = 40)]
    context: String,
}

pub(crate) fn msg_cell_styles(row: &MsgRow) -> [Option<ratatui::style::Style>; 5] {
    let status_style = match row.status.as_str() {
        "Success" => Some(ratatui::style::Style::default().fg(ferrowl_ui::COLOR_SCHEME.success)),
        "Error" => Some(ratatui::style::Style::default().fg(ferrowl_ui::COLOR_SCHEME.error)),
        _ => None,
    };
    [None, None, None, status_style, None]
}

pub(crate) fn msg_row(m: &OcppMessage) -> MsgRow {
    let status = match m.ok {
        Some(true) => "Success",
        Some(false) => "Error",
        None => "",
    };
    MsgRow {
        timestamp: crate::view::log::format_timestamp(m.ts),
        direction: m.direction.label().to_string(),
        name: m.name.clone(),
        status: status.to_string(),
        context: m.context.clone(),
    }
}

/// Build the `ferrowl-ocpp` client `Config` from a runtime spec — a pure function so it can be
/// unit-tested without spinning up a task. `spec.reconnect` (OC-R-107, re-read from the shared
/// device config on every dial attempt via [`start`](OcppClient::start) rebuilding this on each
/// call) falls back to reconnect-enabled when unset, matching Modbus's own
/// `DEFAULT_RECONNECT`/`ModbusModule::resolve_timing`.
fn build_config(spec: &OcppSpec, device: &OcppDeviceConfig) -> Config {
    Config {
        url: spec.url(),
        timeout_ms: spec.timeout_ms.unwrap_or(30_000),
        basic_auth: spec.security.basic_auth(),
        tls: spec.security.cs_tls(),
        extra_headers: device.extra_headers.clone(),
        reconnect: spec.reconnect.unwrap_or(true),
    }
}

/// UI-R-314/UI-R-315 — the running client task's lifecycle, split so a stop request never has to
/// await the task ending: `Idle` (never started or fully stopped), `Running` (task alive, no stop
/// requested), `Stopping` (terminate sent, waiting for the task to end on its own — no abort
/// fallback, mirroring `Client<V>` itself, which has none). One enum rather than a flag plus a
/// dependent optional so a stopping-but-still-running client cannot be mistaken for `Idle` (UI-
/// E-147).
enum CsState<V: Version> {
    Idle,
    Running(Client<V>),
    Stopping(Client<V>),
}

/// The version-generic charging-station backend owned by a client view.
/// Deliberately holds no copy of the module spec: the connection config is built from the spec
/// the view passes into each [`start`](Self::start) call (see the CSMS backend for the rationale).
pub struct OcppClient<V: Version> {
    client: CsState<V>,
    online: Arc<AtomicBool>,
    messages: Messages,
    /// Cached self-signed mTLS client identity (OC-R-115), created once per backend instance and
    /// reused across every `start()` call so repeated `:restart`/reconnects don't regenerate it —
    /// never reinitialized inside `start()`.
    self_signed_cache: ferrowl_ocpp::SelfSignedCache,
    /// Live mirror of `client`'s command channel, shared with every [`OcppSender`] handed out by
    /// [`sender`](Self::sender) — including one captured **before** `start()` connects (an inbound
    /// Call handler built via `ClientVersion::handler(..)` holds exactly such a sender, since it is
    /// constructed before being passed into `start()`). Populated in `start()`, cleared in `stop()`,
    /// so a pre-connect sender becomes usable the moment the connection comes up instead of staying
    /// permanently stuck on the `None` snapshot it was built with.
    cmd_tx: Arc<parking_lot::RwLock<Option<mpsc::Sender<Command<V>>>>>,
}

impl<V: Version> OcppClient<V> {
    pub fn new() -> Self {
        Self {
            client: CsState::Idle,
            online: Arc::new(AtomicBool::new(false)),
            messages: Arc::new(RwLock::new(Vec::new())),
            self_signed_cache: ferrowl_ocpp::new_self_signed_cache(),
            cmd_tx: Arc::new(parking_lot::RwLock::new(None)),
        }
    }

    /// Shared online flag, for an inbound handler to flip on connect/disconnect.
    pub fn online_handle(&self) -> Arc<AtomicBool> {
        self.online.clone()
    }

    /// Shared message buffer, for an inbound handler to record into.
    pub fn messages_handle(&self) -> Messages {
        self.messages.clone()
    }

    pub fn is_online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }

    /// OC-R-123 — the tri-state connection status, same derivation as `Instance::connection_
    /// status` (`ferrowl_modbus`'s equivalent): not running → `Disconnected`;
    /// running and currently connected → `Connected`; running and not → `Reconnecting`.
    pub fn connection_status(&self) -> crate::view::status_bar::ConnStatus {
        use crate::view::status_bar::ConnStatus;
        match &self.client {
            CsState::Idle => ConnStatus::Disconnected,
            CsState::Running(c) | CsState::Stopping(c) if !c.is_running() => {
                ConnStatus::Disconnected
            }
            CsState::Running(_) | CsState::Stopping(_) => {
                if self.is_online() {
                    ConnStatus::Connected
                } else {
                    ConnStatus::Reconnecting
                }
            }
        }
    }

    pub async fn messages_snapshot(&self) -> Vec<OcppMessage> {
        self.messages.read().await.clone()
    }

    /// A detachable sender for off-thread Calls (or `None` cmd channel when not connected). The
    /// returned value owns clones of the command channel and message log, so the caller can
    /// `tokio::spawn` the round-trip and keep the UI responsive while the peer is slow/silent.
    pub fn sender(&self) -> OcppSender<V> {
        OcppSender {
            cmd_tx: self.cmd_tx.clone(),
            messages: self.messages.clone(),
        }
    }

    /// Dial the CSMS and spawn the client task, using the caller-supplied inbound handler.
    /// The connection config is built from the caller's `spec` on every call — the backend holds
    /// no copy, so an edited endpoint/security section always takes effect on the next start.
    pub async fn start<H: CsActionHandler<V>>(
        &mut self,
        spec: &OcppSpec,
        device: &OcppDeviceConfig,
        log: &SharedLog,
        handler: H,
    ) -> Result<(), Error> {
        if !matches!(self.client, CsState::Idle) {
            // Already connected: nothing to do. But if the websocket dropped without an explicit
            // `stop` the handle is stale (task dead, `online` already false) — tear it down so we
            // can redial instead of silently no-op'ing.
            if self.is_online() {
                return Ok(());
            }
            let _ = self.stop().await;
        }
        let config = build_config(spec, device);
        let wire_log = log_fn(self.messages.clone());
        // OC-R-120: connection-status lines (currently just "Client disconnected", emitted once
        // the client task ends regardless of why) go to the module log, not the message table —
        // the message table records only request/response pairs (§9). General diagnostic strings
        // (`wire_log`, e.g. "Command dropped...") are unaffected and keep going to the message
        // table as before.
        let status = status_fn(log.clone());
        let client = ClientBuilder::<V>::new(
            Arc::new(RwLock::new(config)),
            self.self_signed_cache.clone(),
        )
        .spawn(handler, wire_log, status)
        .await?;
        *self.cmd_tx.write() = Some(client.sender());
        self.client = CsState::Running(client);
        // `spawn` dials asynchronously (OC-R-048, OC-R-105): the handshake happens
        // inside the retried task, so `online` stays false here and is flipped by the handler's
        // on_connected/on_disconnected callbacks once a real handshake actually completes.
        Ok(())
    }

    /// UI-R-314 — sends the running client task a graceful terminate and returns immediately,
    /// without waiting for it to actually end (that's [`poll_stop`](Self::poll_stop)'s job, driven
    /// by the caller's own per-tick `refresh()`). `Err(NotRunning)` if the backend was already
    /// `Idle` or already `Stopping`.
    pub async fn request_stop(&mut self) -> Result<(), Error> {
        if !matches!(self.client, CsState::Running(_)) {
            return Err(Error::NotRunning);
        }
        self.online.store(false, Ordering::Relaxed);
        *self.cmd_tx.write() = None;
        let CsState::Running(client) = std::mem::replace(&mut self.client, CsState::Idle) else {
            unreachable!("matched Running just above");
        };
        let _ = client.send(Command::Terminate).await;
        self.client = CsState::Stopping(client);
        Ok(())
    }

    /// UI-R-315 — polls a stop requested via [`request_stop`](Self::request_stop): `None` while
    /// the task is still running; `Some(_)` once it has ended (no abort fallback — `Client<V>` has
    /// none, and with the connect/dial race landed the task ends promptly on its own).
    pub async fn poll_stop(&mut self) -> Option<Result<(), Error>> {
        let CsState::Stopping(client) = &self.client else {
            return None;
        };
        if client.is_running() {
            return None;
        }
        let CsState::Stopping(mut client) = std::mem::replace(&mut self.client, CsState::Idle)
        else {
            unreachable!("matched Stopping just above");
        };
        Some(client.join().await)
    }

    /// Terminate the client task, if running.
    pub async fn stop(&mut self) -> Result<(), Error> {
        if matches!(self.client, CsState::Idle) {
            return Ok(());
        }
        if matches!(self.client, CsState::Running(_)) {
            self.request_stop().await?;
        }
        loop {
            if let Some(res) = self.poll_stop().await {
                return res;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
    }
}

/// A self-contained Call sender, decoupled from the [`OcppClient`] borrow so the round-trip can be
/// `tokio::spawn`ed. Records the request and reply into the same shared message log the view reads.
/// `cmd_tx` is a live cell shared with the owning [`OcppClient`] (see its doc comment) rather than a
/// one-shot snapshot, so a sender captured before `start()` connects still works afterward.
pub struct OcppSender<V: Version> {
    cmd_tx: Arc<parking_lot::RwLock<Option<mpsc::Sender<Command<V>>>>>,
    messages: Messages,
}

// Manual impl (not `#[derive(Clone)]`): the derive would add a spurious `V: Clone` bound — neither
// field actually depends on `V` being `Clone`, only on `V: Version`.
impl<V: Version> Clone for OcppSender<V> {
    fn clone(&self) -> Self {
        Self {
            cmd_tx: self.cmd_tx.clone(),
            messages: self.messages.clone(),
        }
    }
}

impl<V: Version> OcppSender<V> {
    /// A sender with no live connection behind it (`send_scoped` always returns
    /// `Err(Error::NotRunning)`), sharing `messages` with whatever the caller records into
    /// elsewhere. For tests that need an `OcppSender` without spinning up an `OcppClient`.
    #[cfg(test)]
    pub(crate) fn detached(messages: Messages) -> Self {
        Self {
            cmd_tx: Arc::new(parking_lot::RwLock::new(None)),
            messages,
        }
    }

    /// Send a typed Call tagging the recorded request/reply with `scope`, so the client view can
    /// filter the message log per connector. Returns the response JSON on success. Awaiting this
    /// never blocks the UI loop because the caller spawns it.
    pub async fn send_scoped(self, action: V::Action, scope: Scope) -> Result<Value, Error> {
        let name = V::action_name(&action).to_string();
        let request = encode_action_or_log::<V>(&action);
        record(
            &self.messages,
            scope,
            Dir::Out,
            &name,
            request,
            None,
            String::new(),
        )
        .await;

        let cmd_tx = self.cmd_tx.read().clone();
        let result = match &cmd_tx {
            Some(cmd_tx) => Client::<V>::call_via(cmd_tx, action).await,
            None => Err(Error::NotRunning),
        };
        match result {
            Ok(response) => {
                let payload = encode_response_or_log::<V>(&response);
                record(
                    &self.messages,
                    scope,
                    Dir::In,
                    &name,
                    payload.clone(),
                    Some(true),
                    String::new(),
                )
                .await;
                Ok(payload)
            }
            Err(e) => {
                let msg = e.to_string();
                record(
                    &self.messages,
                    scope,
                    Dir::In,
                    &name,
                    Value::Null,
                    Some(false),
                    msg,
                )
                .await;
                Err(e)
            }
        }
    }
}

/// OC-R-122 — build and send a `StatusNotification` for `scope`'s current connector state. Called
/// after a transaction-start message succeeds, from both the RFID/operator path
/// (`ClientView::send_payload`) and an accepted remote-start (`spawn_remote_transaction_start`, OC-
/// R-070). Best-effort: `send_scoped` already records a failure into the message log; there is no
/// separate diagnostic log write here.
pub(crate) async fn send_status_notification<V: ClientVersion>(
    sender: OcppSender<V>,
    state: &Arc<parking_lot::RwLock<V::Cs>>,
    scope: Scope,
) -> Result<Value, Error> {
    let payload = {
        let s = state.read();
        V::state_payload(&s, "StatusNotification", scope)
    };
    let action = V::decode_call("StatusNotification", payload).map_err(Error::from)?;
    sender.send_scoped(action, scope).await
}

/// OC-R-070 — on an accepted remote-start, build the version's transaction-start message from
/// state exactly as the RFID/operator path does (`ClientVersion::has_tx_shortcuts()` picks
/// `StartTransaction` vs `TransactionEvent`), send it through the same `OcppSender`, apply the same
/// post-send/rollback state transition the RFID path uses, then (OC-R-122) send the coupled
/// `StatusNotification`. Fire-and-forget: the caller does not await this before building
/// `RemoteStartTransactionResponse`/`RequestStartTransactionResponse`.
pub(crate) fn spawn_remote_transaction_start<V: ClientVersion>(
    sender: OcppSender<V>,
    state: Arc<parking_lot::RwLock<V::Cs>>,
    scope: Scope,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let (name, payload) = if V::has_tx_shortcuts() {
            ("TransactionEvent".to_string(), {
                let mut s = state.write();
                V::start_event(&mut s, scope)
            })
        } else {
            ("StartTransaction".to_string(), {
                let s = state.read();
                V::state_payload(&s, "StartTransaction", scope)
            })
        };
        let started_tx = (name == "TransactionEvent")
            .then(|| {
                payload
                    .pointer("/transactionInfo/transactionId")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .flatten();
        let Ok(action) = V::decode_call(&name, payload) else {
            return;
        };
        match sender.clone().send_scoped(action, scope).await {
            Ok(response) => {
                {
                    let mut s = state.write();
                    V::apply_post_send(&mut s, &name, scope, started_tx.as_deref(), &response);
                }
                let _ = send_status_notification(sender, &state, scope).await;
            }
            Err(_) => {
                let mut s = state.write();
                V::rollback_tx(&mut s, scope, started_tx.as_deref());
            }
        }
    })
}

/// Push one message into a shared message log (bounded to [`MAX_MESSAGES`]).
async fn record(
    messages: &Messages,
    scope: Scope,
    dir: Dir,
    name: &str,
    payload: Value,
    ok: Option<bool>,
    context: String,
) {
    let mut guard = messages.write().await;
    push_capped(
        &mut guard,
        OcppMessage::new_scoped(scope, dir, name, payload, ok, context),
    );
}

/// A `LogFn` that records error/diagnostic strings into the message log.
fn log_fn(
    messages: Messages,
) -> impl Fn(String) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + Clone {
    move |s: String| {
        let messages = messages.clone();
        Box::pin(async move {
            let mut guard = messages.write().await;
            push_capped(
                &mut guard,
                OcppMessage::new(Dir::In, "log", Value::String(s), None, String::new()),
            );
        })
    }
}

/// A `LogFn` that records connection-status lines (currently just "Client disconnected",
/// emitted once the client task ends regardless of why) into the module log (OC-R-120), not the
/// message table — the message table records only request/response message pairs (§9).
fn status_fn(
    log: SharedLog,
) -> impl Fn(String) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + Clone {
    move |s: String| {
        let log = log.clone();
        Box::pin(async move {
            log.write().await.write(crate::app::Level::Info, &s);
        })
    }
}

/// Format a Unix-millisecond timestamp as an RFC3339 UTC string (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn rfc3339(ms: u64) -> String {
    let total_secs = ms / 1000;
    let h = (total_secs / 3600) % 24;
    let m = (total_secs / 60) % 60;
    let s = total_secs % 60;
    let (year, month, day) = civil_from_days((total_secs / 86400) as i64);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

/// RFC3339 UTC string for the current time.
pub fn rfc3339_now() -> String {
    rfc3339(ferrowl_util::time::now_unix_ms())
}

/// Days since the Unix epoch to a Gregorian (year, month, day) triple (Howard Hinnant).
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era: i64 = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::module::ocpp::config::device::OcppSecurityConfig;
    use crate::module::ocpp::config::session::OcppProtocol;
    use ferrowl_test_support::reserve_tcp_port;

    /// A handler that never receives a Call in these tests — `start()` never completes a
    /// handshake against a closed port.
    struct NoopCsHandler;
    impl CsActionHandler<ferrowl_ocpp::V1_6> for NoopCsHandler {
        async fn handle_call(
            &self,
            _action: ferrowl_ocpp::Action16,
        ) -> Result<ferrowl_ocpp::Response16, ferrowl_ocpp::CallError> {
            Err(ferrowl_ocpp::CallError::new(
                ferrowl_ocpp::CallErrorCode::NotImplemented,
                "unsupported",
            ))
        }
    }

    /// A fresh, empty module log for a test to hand to `start()` and inspect afterward.
    fn test_log() -> SharedLog {
        Arc::new(RwLock::new(crate::app::LogRing::init()))
    }

    fn spec_with_reconnect(reconnect: Option<bool>) -> OcppSpec {
        OcppSpec {
            name: "cs".to_owned(),
            version: Default::default(),
            role: Default::default(),
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".to_owned(),
            port: reserve_tcp_port().release(),
            path: "/ocpp/CS001".to_owned(),
            timeout_ms: Some(200),
            reconnect,
            security: OcppSecurityConfig::default(),
        }
    }

    #[test]
    /// OC-R-107 — the client `Config` built from a spec carries the spec's own `reconnect`
    /// setting, re-read fresh on every `start()` call, instead of a hardcoded `true`.
    fn ut_build_config_reads_reconnect_from_spec() {
        assert!(
            build_config(
                &spec_with_reconnect(Some(true)),
                &OcppDeviceConfig::default()
            )
            .reconnect
        );
        assert!(
            !build_config(
                &spec_with_reconnect(Some(false)),
                &OcppDeviceConfig::default()
            )
            .reconnect
        );
    }

    #[test]
    /// OC-R-048 — an unset `reconnect` (a device config predating the field, or one that never
    /// set it) falls back to reconnect-enabled, matching Modbus's own `DEFAULT_RECONNECT`.
    fn ut_build_config_defaults_reconnect_to_true_when_unset() {
        assert!(build_config(&spec_with_reconnect(None), &OcppDeviceConfig::default()).reconnect);
    }

    #[test]
    /// OC-R-117 — extra_headers flows from the device config into the dial Config.
    fn ut_build_config_carries_extra_headers() {
        let spec = spec_with_reconnect(Some(true));
        let device = OcppDeviceConfig {
            extra_headers: vec![ferrowl_ocpp::HeaderDef::new("X-Tenant", "acme-1").unwrap()],
            ..Default::default()
        };
        assert_eq!(
            build_config(&spec, &device).extra_headers,
            device.extra_headers
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    /// OC-R-048, OC-R-105-107 — `OcppClient::start()` against an unreachable CSMS returns `Ok(())`
    /// immediately: the dial happens inside the spawned retry task rather than during `start()`,
    /// so an unreachable CSMS surfaces as staying offline, not as a synchronous error.
    async fn it_cs_start_against_unreachable_csms_stays_running() {
        let spec = OcppSpec {
            name: "cs".to_owned(),
            version: Default::default(),
            role: Default::default(),
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".to_owned(),
            port: reserve_tcp_port().release(),
            path: "/ocpp/CS001".to_owned(),
            timeout_ms: Some(200),
            reconnect: None,
            security: OcppSecurityConfig::default(),
        };

        let mut backend = OcppClient::<ferrowl_ocpp::V1_6>::new();
        backend
            .start(
                &spec,
                &OcppDeviceConfig::default(),
                &test_log(),
                NoopCsHandler,
            )
            .await
            .expect("start must not fail synchronously against an unreachable CSMS");

        assert!(
            !backend.is_online(),
            "start must not assume connected before the handshake actually completes"
        );

        // The handle is still usable: stop() must not hang on a never-connected, backing-off client.
        tokio::time::timeout(std::time::Duration::from_secs(2), backend.stop())
            .await
            .expect("stop() must not hang while the client task is backing off")
            .expect("stop() must succeed");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    /// OC-R-123 — before `start()`, the tri-state status is `Disconnected` (no client task at
    /// all); against an unreachable CSMS with `reconnect` enabled (the default), it becomes
    /// `Reconnecting` once the task is running but never gets past the dial (never `Connected`,
    /// since no real handshake ever completes).
    async fn it_connection_status_disconnected_then_reconnecting_against_unreachable_csms() {
        use crate::view::status_bar::ConnStatus;

        let spec = OcppSpec {
            name: "cs".to_owned(),
            version: Default::default(),
            role: Default::default(),
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".to_owned(),
            port: reserve_tcp_port().release(),
            path: "/ocpp/CS001".to_owned(),
            timeout_ms: Some(200),
            reconnect: None, // defaults to true (OC-R-048)
            security: OcppSecurityConfig::default(),
        };

        let mut backend = OcppClient::<ferrowl_ocpp::V1_6>::new();
        assert_eq!(
            backend.connection_status(),
            ConnStatus::Disconnected,
            "no client task yet"
        );

        backend
            .start(
                &spec,
                &OcppDeviceConfig::default(),
                &test_log(),
                NoopCsHandler,
            )
            .await
            .expect("start must not fail synchronously against an unreachable CSMS");

        // The task is running but never completes a handshake against a closed port: it must
        // settle on Reconnecting, never Connected, and never fall back to Disconnected while the
        // task itself stays alive.
        for _ in 0..100 {
            if backend.connection_status() == ConnStatus::Reconnecting {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            backend.connection_status(),
            ConnStatus::Reconnecting,
            "a running task that never completes a handshake must report Reconnecting"
        );

        tokio::time::timeout(std::time::Duration::from_secs(2), backend.stop())
            .await
            .expect("stop() must not hang while the client task is backing off")
            .expect("stop() must succeed");
        assert_eq!(
            backend.connection_status(),
            ConnStatus::Disconnected,
            "after stop() the task is gone"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    /// UI-E-147, OC-E-096 (status half) — `request_stop()` on a client whose task never completes
    /// a handshake against a closed port must not flip the status to `Disconnected` until
    /// `poll_stop()` actually reaps the task.
    async fn ut_request_stop_keeps_status_reconnecting_until_the_task_ends() {
        use crate::view::status_bar::ConnStatus;

        let spec = OcppSpec {
            name: "cs".to_owned(),
            version: Default::default(),
            role: Default::default(),
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".to_owned(),
            port: reserve_tcp_port().release(),
            path: "/ocpp/CS001".to_owned(),
            timeout_ms: Some(200),
            reconnect: None, // defaults to true (OC-R-048)
            security: OcppSecurityConfig::default(),
        };

        let mut backend = OcppClient::<ferrowl_ocpp::V1_6>::new();
        backend
            .start(
                &spec,
                &OcppDeviceConfig::default(),
                &test_log(),
                NoopCsHandler,
            )
            .await
            .expect("start must not fail synchronously against an unreachable CSMS");

        for _ in 0..100 {
            if backend.connection_status() == ConnStatus::Reconnecting {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(backend.connection_status(), ConnStatus::Reconnecting);

        let before = std::time::Instant::now();
        backend.request_stop().await.expect("request_stop");
        assert_eq!(
            backend.connection_status(),
            ConnStatus::Reconnecting,
            "must not report Disconnected until poll_stop reaps the task"
        );

        loop {
            if backend.poll_stop().await.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(
            before.elapsed() < std::time::Duration::from_millis(500),
            "request_stop + poll_stop took {:?}",
            before.elapsed()
        );
        assert_eq!(backend.connection_status(), ConnStatus::Disconnected);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    /// OC-R-120 — a CS's connection-status line ("Client disconnected", emitted once the
    /// client task ends) lands in the module log, not the message table (the message table
    /// records only request/response pairs). `reconnect: false` against a refused dial makes
    /// the client task end almost immediately (one failed attempt, no retry), so the status
    /// line lands within the poll window below without needing a real CSMS.
    async fn it_disconnect_status_line_lands_in_module_log_not_message_table() {
        let spec = OcppSpec {
            name: "cs".to_owned(),
            version: Default::default(),
            role: Default::default(),
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".to_owned(),
            port: reserve_tcp_port().release(),
            path: "/ocpp/CS001".to_owned(),
            timeout_ms: Some(200),
            reconnect: Some(false),
            security: OcppSecurityConfig::default(),
        };

        let log = test_log();
        let mut backend = OcppClient::<ferrowl_ocpp::V1_6>::new();
        backend
            .start(&spec, &OcppDeviceConfig::default(), &log, NoopCsHandler)
            .await
            .expect("start must not fail synchronously");

        // Poll for the disconnect line rather than a fixed sleep: the client task's single
        // failed dial + status invocation should land well inside this window.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut found = false;
        while std::time::Instant::now() < deadline {
            let lines = log.write().await.peek_n(crate::app::LOG_SIZE);
            if lines.iter().any(|(_, _, line)| line.contains("disconnect")) {
                found = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            found,
            "the module log must receive a 'disconnect' status line once the client task ends"
        );

        let messages = backend.messages_snapshot().await;
        assert!(
            !messages
                .iter()
                .any(|m| m.payload.as_str().is_some_and(|s| s.contains("disconnect"))),
            "the disconnect status line must not be recorded in the message table"
        );

        let _ = backend.stop().await;
    }

    #[test]
    /// OC-R-060 — the CS heartbeat cadence uses the interval the CSMS returned in its BootNotification response.
    fn boot_interval_prefers_csms_value() {
        let resp =
            json!({ "currentTime": "2026-01-01T00:00:00Z", "interval": 120, "status": "Accepted" });
        assert_eq!(boot_interval(&resp), Some(120));
    }

    #[test]
    /// OC-R-060 — the heartbeat cadence falls back to 30 s when the CSMS interval is absent or zero.
    fn boot_interval_falls_back_on_zero_or_absent() {
        // Zero is treated as "unset" so the caller uses DEFAULT_HEARTBEAT_SECS.
        assert_eq!(boot_interval(&json!({ "interval": 0 })), None);
        // Missing field.
        assert_eq!(boot_interval(&json!({ "status": "Accepted" })), None);
    }

    /// No-op log sink for the loopback test below (mirrors `ferrowl-ocpp/tests/ws_loopback_v16.rs`).
    fn sink() -> impl ferrowl_ocpp::LogFn + Clone {
        |_s: String| async move {}
    }

    /// A minimal CSMS test double that default-accepts Heartbeat, for the live-sender test below.
    struct HeartbeatCsms;
    impl ferrowl_ocpp::csms::CsmsActionHandler<ferrowl_ocpp::V1_6> for HeartbeatCsms {
        async fn handle_call(
            &self,
            _conn: ferrowl_ocpp::csms::ConnectionId,
            action: ferrowl_ocpp::Action16,
        ) -> Result<ferrowl_ocpp::Response16, ferrowl_ocpp::CallError> {
            match action {
                ferrowl_ocpp::Action16::Heartbeat(_) => Ok(ferrowl_ocpp::Response16::Heartbeat(
                    serde_json::from_value(json!({ "currentTime": "2026-01-01T00:00:00Z" }))
                        .unwrap(),
                )),
                _ => Err(ferrowl_ocpp::CallError::new(
                    ferrowl_ocpp::CallErrorCode::NotImplemented,
                    "unsupported",
                )),
            }
        }
    }

    /// A CS-side handler that flips a shared flag once the handshake completes — `NoopCsHandler`'s
    /// `on_connected` is a no-op, so it never signals a real connect (see
    /// `it_cs_start_against_unreachable_csms_stays_running`, which only ever asserts `!is_online()`
    /// against a handler like it).
    struct ConnectedFlag(Arc<AtomicBool>);
    impl CsActionHandler<ferrowl_ocpp::V1_6> for ConnectedFlag {
        async fn handle_call(
            &self,
            _action: ferrowl_ocpp::Action16,
        ) -> Result<ferrowl_ocpp::Response16, ferrowl_ocpp::CallError> {
            Err(ferrowl_ocpp::CallError::new(
                ferrowl_ocpp::CallErrorCode::NotImplemented,
                "unsupported",
            ))
        }
        async fn on_connected(&self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    /// Poll until the CSMS listener has bound (`spawn` binds asynchronously).
    async fn bound_addr(server: &ferrowl_ocpp::csms::Server<ferrowl_ocpp::V1_6>) -> String {
        for _ in 0..50 {
            if let Some(addr) = server.local_addr() {
                return addr.to_string();
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("CSMS listener never bound");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    /// (infra, required for OC-R-070) — a sender captured via `OcppClient::sender()` **before**
    /// `start()` (exactly how `CsStateHandler` will hold one, since it is built and handed into
    /// `start()` before the connection exists) must become usable once the client actually
    /// connects, instead of permanently carrying the `cmd_tx: None` snapshot taken before the
    /// connection existed.
    async fn ut_sender_captured_before_start_becomes_live_after_connect() {
        let server = ferrowl_ocpp::csms::ServerBuilder::<ferrowl_ocpp::V1_6>::new(
            ferrowl_ocpp::csms::Config {
                host: "127.0.0.1".to_owned(),
                port: 0,
                timeout_ms: 2000,
                reconnect: true,
                basic_auth: None,
                tls: Default::default(),
            },
            ferrowl_ocpp::new_self_signed_cache(),
        )
        .spawn(HeartbeatCsms, sink())
        .await
        .expect("server failed to bind");
        let addr = bound_addr(&server).await;

        let spec = OcppSpec {
            name: "cs".to_owned(),
            version: Default::default(),
            role: Default::default(),
            protocol: OcppProtocol::Ws,
            ip: addr.split(':').next().unwrap().to_owned(),
            port: addr.rsplit(':').next().unwrap().parse().unwrap(),
            path: "/ocpp/CS001".to_owned(),
            timeout_ms: Some(2000),
            reconnect: Some(true),
            security: OcppSecurityConfig::default(),
        };

        let mut backend = OcppClient::<ferrowl_ocpp::V1_6>::new();
        // Captured before start() — this is the pre-connect snapshot that must stay usable.
        let sender = backend.sender();
        let connected = Arc::new(AtomicBool::new(false));
        backend
            .start(
                &spec,
                &OcppDeviceConfig::default(),
                &test_log(),
                ConnectedFlag(connected.clone()),
            )
            .await
            .expect("start must not fail synchronously");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !connected.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            connected.load(Ordering::Relaxed),
            "client never completed the handshake"
        );

        let action = ferrowl_ocpp::V1_6::default_action("Heartbeat").expect("Heartbeat is known");
        let resp = sender.send_scoped(action, Scope::CS).await;
        assert!(
            resp.is_ok(),
            "a sender captured before start() must still work once connected: {resp:?}"
        );

        let _ = backend.stop().await;
        let _ = server.terminate().await;
    }

    #[test]
    /// OC-R-083 — a client module does not connect automatically; a freshly built backend is offline until an explicit start.
    fn new_client_backend_is_offline_until_started() {
        let backend = OcppClient::<ferrowl_ocpp::V1_6>::new();
        assert!(!backend.is_online());
    }

    #[test]
    /// OC-R-088 — message sequence numbers are strictly increasing and unique, the cursor a file-log tee uses to write each message at most once.
    fn message_seqs_are_strictly_increasing() {
        let a = next_seq();
        let b = next_seq();
        let c = next_seq();
        assert!(a < b && b < c);
    }

    #[test]
    /// OC-R-087 — the in-memory message log is bounded to the most recent 200 messages, evicting oldest-first.
    fn push_capped_evicts_oldest_beyond_limit() {
        let mut buf = Vec::new();
        for _ in 0..(MAX_MESSAGES + 50) {
            push_capped(
                &mut buf,
                OcppMessage::new(Dir::Out, "Heartbeat", json!({}), None, String::new()),
            );
        }
        assert_eq!(buf.len(), MAX_MESSAGES);
        // The retained window is the newest MAX_MESSAGES, so seqs are strictly increasing and the
        // front is no longer seq 1.
        assert!(buf[0].seq < buf[buf.len() - 1].seq);
        assert!(buf.windows(2).all(|w| w[0].seq < w[1].seq));
    }

    #[test]
    /// OC-R-078 — each recorded message renders its direction, status, and payload for display and logging.
    fn log_line_renders_direction_status_and_payload() {
        let m = OcppMessage::new(
            Dir::In,
            "BootNotification",
            json!({"status":"Accepted"}),
            Some(true),
            "boot",
        );
        let line = m.log_line();
        assert!(line.starts_with("<- BootNotification ok (boot) "));
        assert!(line.contains("\"status\":\"Accepted\""));
    }

    #[test]
    fn ut_rfc3339_formats_epoch_and_known_instant() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        // 1_000_000_000 s since the epoch is 2001-09-09T01:46:40Z.
        assert_eq!(rfc3339(1_000_000_000_000), "2001-09-09T01:46:40Z");
    }

    #[test]
    fn ut_dir_label() {
        assert_eq!(Dir::In.label(), "Inbound");
        assert_eq!(Dir::Out.label(), "Outbound");
    }

    #[test]
    fn ut_new_scoped_tags_message_with_scope() {
        let m = OcppMessage::new_scoped(
            Scope::connector(3),
            Dir::Out,
            "StatusNotification",
            json!({}),
            None,
            "",
        );
        assert_eq!(m.scope, Scope::connector(3));
        assert_eq!(m.direction, Dir::Out);
    }

    #[test]
    /// OC-R-078 — a recorded message renders into the log table with its direction and status.
    fn ut_msg_row_and_cell_styles() {
        let ok = OcppMessage::new(Dir::In, "Boot", json!({}), Some(true), "ctx");
        let row = msg_row(&ok);
        assert_eq!(row.direction, "Inbound");
        assert_eq!(row.status, "Success");
        assert_eq!(row.context, "ctx");
        // The status cell (index 3) is styled for success/error, others are unstyled.
        let styles = msg_cell_styles(&row);
        assert!(styles[3].is_some());
        assert!(styles[0].is_none());
        let err = msg_row(&OcppMessage::new(
            Dir::Out,
            "Boot",
            json!({}),
            Some(false),
            "",
        ));
        assert_eq!(err.status, "Error");
        assert!(msg_cell_styles(&err)[3].is_some());
        let neutral = msg_row(&OcppMessage::new(Dir::Out, "Boot", json!({}), None, ""));
        assert_eq!(neutral.status, "");
        assert!(msg_cell_styles(&neutral)[3].is_none());
    }
}
