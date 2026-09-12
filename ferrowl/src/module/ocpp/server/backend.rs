//! Version-generic OCPP CSMS (server) backend: wraps the `ferrowl-ocpp` `Server<V>`, binds/unbinds
//! the listening socket, and funnels every connection lifecycle change and every inbound/outbound
//! OCPP message to the view through a single event channel. Unlike the client backend there is no
//! single shared message log — the view keeps a separate log per connected entry (CS / connector),
//! so all the backend does is deliver [`ServerEvent`]s the view sorts into the right entry.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

use ferrowl_ocpp::csms::{Command, Config, ConnectionId, CsmsActionHandler, Server, ServerBuilder};
use ferrowl_ocpp::{Error, Version};

use crate::module::ocpp::client::backend::{Dir, OcppMessage};
use crate::module::ocpp::config::session::OcppSpec;
use crate::module::ocpp::lock::{with_state, with_state_mut};
pub use crate::module::ocpp::scope::Scope;
use crate::module::ocpp::wire_log::encode_response_or_log;

/// A lifecycle/message event delivered from the CSMS server tasks to the view. Version-agnostic:
/// action payloads are carried as JSON so the (version-specific) view extracts connector ids and
/// state from them.
#[derive(Debug, Clone)]
pub enum ServerEvent {
    /// A charging station completed its WebSocket handshake. The identity is resolved by the view
    /// from the registry (see [`OcppServer::identity`]).
    Connected { conn: ConnectionId },
    /// A charging station's connection ended.
    Disconnected { conn: ConnectionId },
    /// An inbound CS→CSMS Call and the CSMS's reply to it.
    Inbound {
        conn: ConnectionId,
        name: String,
        request: Value,
        response: Value,
    },
    /// An outbound CSMS→CS Call (initiated by the view) and the CS's reply, or an error string.
    Outbound {
        conn: ConnectionId,
        /// The entry scope this Call was sent from (CS-level/connector/EVSE), for log routing.
        scope: Scope,
        name: String,
        request: Value,
        response: Value,
        ok: bool,
        context: String,
    },
}

pub type EventTx = mpsc::UnboundedSender<ServerEvent>;
pub type EventRx = mpsc::UnboundedReceiver<ServerEvent>;

/// CSMS RFID accept-lists, split by level: a charge-point-wide (CS) list plus per-connector/EVSE
/// lists keyed by [`Scope`]. A connector inherits the CS list (its effective set is the connector
/// list unioned with the CS list). An *empty effective set accepts every tag* (open mode); once any
/// tag is listed in the effective set, only listed tags pass.
#[derive(Debug, Clone, Default)]
pub struct RfidStore {
    /// Charge-point-wide tags, inherited by every connector.
    pub cs: Vec<String>,
    /// Per-connector/EVSE tags, keyed by the connector's [`Scope`].
    pub by_scope: HashMap<Scope, Vec<String>>,
}

impl RfidStore {
    /// The connector list for `scope` (empty if none recorded).
    pub fn scope_list(&self, scope: Scope) -> &[String] {
        self.by_scope.get(&scope).map_or(&[], Vec::as_slice)
    }

    /// Add `tag` to a level (deduplicated); returns whether it was newly inserted. `scope`
    /// [`Scope::CS`] targets the charge-point-wide list, otherwise the connector list.
    pub fn add(&mut self, scope: Scope, tag: String) -> bool {
        let list = if scope == Scope::CS {
            &mut self.cs
        } else {
            self.by_scope.entry(scope).or_default()
        };
        if list.contains(&tag) {
            false
        } else {
            list.push(tag);
            true
        }
    }

    /// Remove `tag` from a level; returns whether it was present.
    pub fn remove(&mut self, scope: Scope, tag: &str) -> bool {
        let list = if scope == Scope::CS {
            Some(&mut self.cs)
        } else {
            self.by_scope.get_mut(&scope)
        };
        match list {
            Some(list) => {
                let before = list.len();
                list.retain(|t| t != tag);
                list.len() < before
            }
            None => false,
        }
    }
}

/// Shared CSMS RFID accept-lists, edited live by the view (detail dialogs / `:rfid`) and read by the
/// inbound handler to gate Authorize / transaction starts.
pub type RfidLists = Arc<RwLock<RfidStore>>;

/// Run `f` with a read guard on `store`, dropped before returning.
pub fn with_rfids<R>(store: &RfidLists, f: impl FnOnce(&RfidStore) -> R) -> R {
    with_state(store, f)
}

/// Run `f` with a write guard on `store`, dropped before returning.
pub fn with_rfids_mut<R>(store: &RfidLists, f: impl FnOnce(&mut RfidStore) -> R) -> R {
    with_state_mut(store, f)
}

/// Whether a tag passes a CS-wide check (Authorize, which carries no connector): accepted if the
/// effective set — the CS list unioned with *every* connector list — is empty or contains the tag.
pub fn cs_authorized(store: &RfidLists, id_tag: &str) -> bool {
    with_rfids(store, |s| {
        let mut empty = s.cs.is_empty();
        if s.cs.iter().any(|t| t == id_tag) {
            return true;
        }
        for list in s.by_scope.values() {
            if !list.is_empty() {
                empty = false;
                if list.iter().any(|t| t == id_tag) {
                    return true;
                }
            }
        }
        empty
    })
}

/// Whether a tag passes a connector-scoped check (a transaction start that names a connector):
/// accepted if the effective set — that connector's list unioned with the inherited CS list — is
/// empty or contains the tag.
pub fn scope_authorized(store: &RfidLists, scope: Scope, id_tag: &str) -> bool {
    with_rfids(store, |s| {
        let conn = s.scope_list(scope);
        let effective_empty = s.cs.is_empty() && conn.is_empty();
        effective_empty || s.cs.iter().chain(conn).any(|t| t == id_tag)
    })
}

/// Which TLS mode a [`OcppServer::start`] actually bound with — returned so the caller's log
/// line reports the listener's real state instead of re-deriving (and possibly mispredicting)
/// it from the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsBinding {
    Plain,
    SelfSigned,
    Certificates,
}

/// UI-R-314/UI-R-315 — the running server task's lifecycle, split so a stop request never has to
/// await the task ending; mirrors `CsState` (`client/backend.rs`) for the CSMS side. No abort
/// fallback: `Server<V>` has none, and the task ends promptly on its own once terminated.
enum CsmsState<V: Version> {
    Idle,
    Running(Server<V>),
    Stopping(Server<V>),
}

/// The version-generic CSMS backend owned by a server view.
///
/// Deliberately holds no copy of the module spec: the listener config is built from the spec the
/// view passes into each [`start`](Self::start) call, so an edited endpoint/security section can
/// never drift from what the listener actually binds with.
pub struct OcppServer<V: Version> {
    server: CsmsState<V>,
    /// Cached self-signed server certificate (OC-R-037), created once per backend instance and
    /// reused across every `start()` call so repeated `:restart`/rebind attempts don't
    /// regenerate it — never reinitialized inside `start()`.
    self_signed_cache: ferrowl_ocpp::SelfSignedCache,
}

impl<V: Version> OcppServer<V>
where
    V::Action: Clone,
{
    pub fn new() -> Self {
        Self {
            server: CsmsState::Idle,
            self_signed_cache: ferrowl_ocpp::new_self_signed_cache(),
        }
    }

    /// The running (or stopping) server, if any.
    fn server(&self) -> Option<&Server<V>> {
        match &self.server {
            CsmsState::Idle => None,
            CsmsState::Running(s) | CsmsState::Stopping(s) => Some(s),
        }
    }

    /// OC-R-124 — the tri-state connection status: not running → `Disconnected`; running and
    /// actually bound → `Connected`; running but backing off from a failed bind → `Reconnecting`.
    /// Supersedes the old `online` flag, which was set once `true` in `start()` and never
    /// updated again, so it went stale the moment a live listener's bind later dropped and began
    /// backing off.
    pub fn connection_status(&self) -> crate::view::status_bar::ConnStatus {
        use crate::view::status_bar::ConnStatus;
        match self.server() {
            None => ConnStatus::Disconnected,
            Some(s) if !s.is_running() => ConnStatus::Disconnected,
            Some(_) => {
                if self.bound_addr().is_some() {
                    ConnStatus::Connected
                } else {
                    ConnStatus::Reconnecting
                }
            }
        }
    }

    /// Thin wrapper over `connection_status()`, kept for `view/backend.rs`'s auto-bind guard
    /// (`want_running && !is_online()`) — harmless to call `start()` again while reconnecting,
    /// since `start()` itself no-ops once `self.server` is already `Some`.
    pub fn is_online(&self) -> bool {
        self.connection_status() == crate::view::status_bar::ConnStatus::Connected
    }

    /// Bind the listening socket and spawn the accept loop with the caller-supplied inbound handler.
    /// Idempotent: a no-op if already bound.
    pub async fn start<H: CsmsActionHandler<V>>(
        &mut self,
        spec: &OcppSpec,
        handler: H,
    ) -> Result<TlsBinding, Error> {
        // A wss endpoint without configured TLS material falls back to an ephemeral
        // self-signed certificate instead of silently binding plain TCP.
        let tls = spec.effective_csms_tls();
        let binding = match &tls {
            ferrowl_util::tls::ServerTlsPolicy::None {} => TlsBinding::Plain,
            ferrowl_util::tls::ServerTlsPolicy::Tls { identity }
            | ferrowl_util::tls::ServerTlsPolicy::Mutual { identity, .. } => match identity {
                ferrowl_util::tls::CertSource::SelfSigned {}
                | ferrowl_util::tls::CertSource::Ephemeral {} => TlsBinding::SelfSigned,
                ferrowl_util::tls::CertSource::Files { .. } => TlsBinding::Certificates,
            },
        };
        if !matches!(self.server, CsmsState::Idle) {
            return Ok(binding);
        }
        let config = Config {
            host: spec.ip.clone(),
            port: spec.port,
            timeout_ms: spec.timeout_ms.unwrap_or(30_000),
            reconnect: spec.reconnect.unwrap_or(true),
            basic_auth: spec.security.basic_auth(),
            tls,
        };
        let server = ServerBuilder::<V>::new(config, self.self_signed_cache.clone())
            .spawn(handler, |_s: String| async {})
            .await?;
        self.server = CsmsState::Running(server);
        Ok(binding)
    }

    /// UI-R-314 — sends the running server task a graceful terminate and returns immediately,
    /// without waiting for it (and every connection) to actually end (that's
    /// [`poll_stop`](Self::poll_stop)'s job, driven by the caller's own per-tick `refresh()`).
    /// `Err(NotRunning)` if the backend was already `Idle` or already `Stopping`.
    pub async fn request_stop(&mut self) -> Result<(), Error> {
        if !matches!(self.server, CsmsState::Running(_)) {
            return Err(Error::NotRunning);
        }
        let CsmsState::Running(server) = std::mem::replace(&mut self.server, CsmsState::Idle)
        else {
            unreachable!("matched Running just above");
        };
        let _ = server.send(Command::Terminate).await;
        self.server = CsmsState::Stopping(server);
        Ok(())
    }

    /// UI-R-315 — polls a stop requested via [`request_stop`](Self::request_stop): `None` while
    /// the task is still running; `Some(_)` once it (and every connection) has ended.
    pub async fn poll_stop(&mut self) -> Option<Result<(), Error>> {
        let CsmsState::Stopping(server) = &self.server else {
            return None;
        };
        if server.is_running() {
            return None;
        }
        let CsmsState::Stopping(mut server) = std::mem::replace(&mut self.server, CsmsState::Idle)
        else {
            unreachable!("matched Stopping just above");
        };
        Some(server.join().await)
    }

    /// Terminate the server task and every connection, if running.
    pub async fn stop(&mut self) -> Result<(), Error> {
        if matches!(self.server, CsmsState::Idle) {
            return Ok(());
        }
        if matches!(self.server, CsmsState::Running(_)) {
            self.request_stop().await?;
        }
        loop {
            if let Some(res) = self.poll_stop().await {
                return res;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
    }

    /// The bound local address (`host:port`) when running, for the status line. `None` both
    /// while never bound and while backing off from a failed bind (OC-R-083).
    pub fn bound_addr(&self) -> Option<String> {
        self.server()
            .and_then(ferrowl_ocpp::csms::Server::local_addr)
            .map(|a| a.to_string())
    }

    /// The charge-point identity for a connection (URL-path segment), if known.
    pub fn identity(&self, conn: ConnectionId) -> Option<String> {
        self.server().and_then(|s| s.registry().identity(conn))
    }

    /// A detachable sender for off-thread Calls to a specific connection, decoupled from the
    /// `OcppServer` borrow so the round-trip can be `tokio::spawn`ed. `None` when not bound.
    pub fn sender(&self) -> Option<OcppServerSender<V>> {
        self.server()
            .map(|s| OcppServerSender { cmd_tx: s.sender() })
    }
}

/// A self-contained Call sender to one connection, decoupled from the [`OcppServer`] borrow.
pub struct OcppServerSender<V: Version> {
    cmd_tx: mpsc::Sender<Command<V>>,
}

impl<V: Version> OcppServerSender<V> {
    /// Send a typed Call to `conn` and await its reply. Mirrors `Server::call` but over a cloned
    /// command channel so it can run in a spawned task.
    pub async fn call(self, conn: ConnectionId, action: V::Action) -> Result<Value, Error> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.cmd_tx
            .send(Command::SendToConnectionAwait(conn, action, reply_tx))
            .await
            .map_err(|_| Error::ChannelClosed)?;
        match reply_rx.await {
            Ok(Ok(response)) => Ok(encode_response_or_log::<V>(&response)),
            Ok(Err(call_err)) => Err(Error::Call(call_err)),
            Err(_) => Err(Error::ChannelClosed),
        }
    }
}

/// Build a message-log entry for a request (inbound) / its reply (outbound).
pub fn inbound_messages(name: &str, request: Value, response: Value) -> [OcppMessage; 2] {
    [
        OcppMessage::new(Dir::In, name, request, None, String::new()),
        OcppMessage::new(Dir::Out, name, response, Some(true), String::new()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::module::ocpp::config::device::OcppSecurityConfig;
    use crate::module::ocpp::config::session::OcppProtocol;

    /// A handler that never receives a Call in this test — `start()` binds an occupied port, so
    /// no connection is ever accepted.
    struct NoopCsmsHandler;
    impl CsmsActionHandler<ferrowl_ocpp::V1_6> for NoopCsmsHandler {
        async fn handle_call(
            &self,
            _conn: ConnectionId,
            _action: ferrowl_ocpp::Action16,
        ) -> Result<ferrowl_ocpp::Response16, ferrowl_ocpp::CallError> {
            Err(ferrowl_ocpp::CallError::new(
                ferrowl_ocpp::CallErrorCode::NotImplemented,
                "unsupported",
            ))
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    /// NF-R-047, NF-R-059, OC-R-139, OC-R-108-109 — `OcppServer::start()` against an occupied port still returns
    /// `Ok(TlsBinding)`; `bound_addr()`
    /// stays `None` while backing off and becomes `Some(_)` once the port frees up.
    async fn it_csms_start_against_occupied_port_stays_running() {
        let occupier = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("occupier bind failed");
        let occupied_port = occupier.local_addr().expect("occupier addr").port();

        let spec = OcppSpec {
            name: "csms".to_owned(),
            version: Default::default(),
            role: Default::default(),
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".to_owned(),
            port: occupied_port,
            path: "/ocpp/CS001".to_owned(),
            timeout_ms: Some(1000),
            reconnect: None,
            security: OcppSecurityConfig::default(),
        };

        let mut backend = OcppServer::<ferrowl_ocpp::V1_6>::new();
        backend
            .start(&spec, NoopCsmsHandler)
            .await
            .expect("start must not fail synchronously on an occupied port");

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(
            backend.bound_addr().is_none(),
            "bound_addr must stay None while the port is occupied and the bind is retrying"
        );

        drop(occupier);
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        assert!(
            backend.bound_addr().is_some(),
            "bound_addr must become Some once the occupied port is freed"
        );

        backend.stop().await.expect("stop() must succeed");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    /// UI-R-315 — `poll_stop()` reports the same outcome `stop()` would have, and `bound_addr()`
    /// stays readable (still bound) while the stop is in flight (UI-E-147).
    async fn ut_csms_request_stop_then_poll_stop_reports_outcome() {
        let spec = OcppSpec {
            name: "csms".to_owned(),
            version: Default::default(),
            role: Default::default(),
            protocol: OcppProtocol::Ws,
            ip: "127.0.0.1".to_owned(),
            port: 0,
            path: "/ocpp/CS001".to_owned(),
            timeout_ms: Some(1000),
            reconnect: None,
            security: OcppSecurityConfig::default(),
        };

        let mut backend = OcppServer::<ferrowl_ocpp::V1_6>::new();
        backend
            .start(&spec, NoopCsmsHandler)
            .await
            .expect("start must succeed");

        for _ in 0..50 {
            if backend.bound_addr().is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(backend.bound_addr().is_some(), "listener must have bound");

        backend.request_stop().await.expect("request_stop");
        assert!(
            backend.bound_addr().is_some(),
            "bound_addr must stay readable while the stop is in flight"
        );

        let result = loop {
            if let Some(res) = backend.poll_stop().await {
                break res;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        };
        assert!(result.is_ok());
        assert!(backend.bound_addr().is_none());
    }

    fn store(cs: &[&str]) -> RfidLists {
        Arc::new(RwLock::new(RfidStore {
            cs: cs.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }))
    }

    #[test]
    /// OC-R-074 — the CSMS maintains charge-point-wide and per-connector RFID accept-lists, deduplicating entries.
    fn ut_add_remove_dedup() {
        let mut s = RfidStore::default();
        assert!(s.add(Scope::CS, "A".into()));
        assert!(!s.add(Scope::CS, "A".into())); // duplicate
        assert!(s.add(Scope::connector(1), "B".into()));
        assert_eq!(s.scope_list(Scope::connector(1)), ["B"]);
        assert!(s.remove(Scope::connector(1), "B"));
        assert!(!s.remove(Scope::connector(1), "B")); // already gone
        assert!(s.scope_list(Scope::connector(1)).is_empty());
    }

    #[test]
    /// OC-R-075 — an empty effective accept-set (nothing listed anywhere) accepts every tag.
    fn ut_empty_everywhere_accepts_all() {
        let s = store(&[]);
        assert!(cs_authorized(&s, "ANY"));
        assert!(scope_authorized(&s, Scope::connector(1), "ANY"));
    }

    #[test]
    /// OC-R-076 — a charge-point-wide authorization is checked against the cp-wide list unioned with every connector list.
    fn ut_cs_authorize_unions_all_connectors() {
        let s = store(&["CS"]);
        with_rfids_mut(&s, |s| s.add(Scope::connector(2), "CONN2".into()));
        // The CS list and any connector list both authorize at the CS (connector-less) level.
        assert!(cs_authorized(&s, "CS"));
        assert!(cs_authorized(&s, "CONN2"));
        // A tag listed nowhere is rejected (the effective set is non-empty).
        assert!(!cs_authorized(&s, "NOPE"));
    }

    #[test]
    /// OC-R-074 — a connector's effective accept-set is its own list unioned with the charge-point-wide list.
    fn ut_scope_authorize_inherits_cs_only() {
        let s = store(&["CS"]);
        with_rfids_mut(&s, |s| s.add(Scope::connector(1), "CONN1".into()));
        // Connector 1 accepts its own tag and the inherited CS tag.
        assert!(scope_authorized(&s, Scope::connector(1), "CONN1"));
        assert!(scope_authorized(&s, Scope::connector(1), "CS"));
        // Another connector's tag is NOT inherited sideways.
        assert!(!scope_authorized(&s, Scope::connector(2), "CONN1"));
        // Connector 2 still inherits CS (its own list is empty, CS is not).
        assert!(scope_authorized(&s, Scope::connector(2), "CS"));
        assert!(!scope_authorized(&s, Scope::connector(2), "NOPE"));
    }
}
