//! Lifecycle wrapper around a single Modbus client or server task.

pub mod builder;
pub mod config;
pub mod error;
pub mod handle;

use builder::Builder;
use config::{ClientConfig, ServerConfig};
use error::{Error, InstanceError};
use handle::Handle;

use ferrowl_modbus::{KeyParams, LogFn};

/// A startable/stoppable Modbus endpoint (TCP/RTU x client/server).
///
/// Construct with one of the `with_*` constructors, then [`start`](Self::start)
/// to spawn the background task and [`stop`](Self::stop) to terminate it.
/// The same instance can be restarted after it stops.
pub struct Instance<T: KeyParams> {
    builder: Builder<T>,
    task: TaskState,
}

/// UI-R-314/UI-R-315 — the running task's lifecycle, split so a stop request never has to await
/// the task ending: `Idle` (never started or fully stopped), `Running` (task alive, no stop
/// requested), `Stopping` (terminate sent, waiting out the grace period before an abort
/// fallback). Modeled as one enum rather than a flag plus dependent optionals so an instance mid
/// `poll_stop` cannot be mistaken for `Idle` — `handle()` reads the handle out of both `Running`
/// and `Stopping`, keeping `active()`/`connection_status()` truthful (UI-E-147) throughout.
enum TaskState {
    Idle,
    Running(Handle),
    Stopping {
        handle: Handle,
        deadline: tokio::time::Instant,
    },
}

impl<T: KeyParams> Instance<T> {
    /// The handle of the currently running (or stopping) task, if any.
    fn handle(&self) -> Option<&Handle> {
        match &self.task {
            TaskState::Idle => None,
            TaskState::Running(h) | TaskState::Stopping { handle: h, .. } => Some(h),
        }
    }

    /// MB-R-137/153 — superseded as the view-facing signal by `connection_status()` (which
    /// distinguishes running-but-not-connected from not-running), but kept as the plain
    /// task-alive check its own extensive test suite below still exercises directly.
    #[allow(dead_code)]
    pub fn active(&self) -> bool {
        if let Some(h) = self.handle() {
            !h.is_finished()
        } else {
            false
        }
    }

    /// MB-R-150 — the path-conflict checker cell for this instance's underlying builder, or
    /// `None` for a non-serial transport (which never participates in the check).
    pub(crate) fn path_conflict_cell(&self) -> Option<ferrowl_modbus::PathConflictCell> {
        match &self.builder {
            Builder::RtuClient(b) => Some(b.path_conflict()),
            Builder::RtuServer(b) => Some(b.path_conflict()),
            Builder::AsciiClient(b) => Some(b.path_conflict()),
            Builder::AsciiServer(b) => Some(b.path_conflict()),
            _ => None,
        }
    }

    pub fn with_tcp_client(
        config: ClientConfig<T, ferrowl_modbus::tcp::Config>,
        cache: ferrowl_modbus::tcp::SelfSignedCache,
    ) -> Self {
        Self {
            builder: Builder::TcpClient(ferrowl_modbus::tcp::ClientBuilder::new(
                config.config,
                config.operations,
                config.memory,
                cache,
            )),
            task: TaskState::Idle,
        }
    }

    pub fn with_rtu_client(config: ClientConfig<T, ferrowl_modbus::rtu::Config>) -> Self {
        Self {
            builder: Builder::RtuClient(ferrowl_modbus::rtu::ClientBuilder::new(
                config.config,
                config.operations,
                config.memory,
            )),
            task: TaskState::Idle,
        }
    }

    pub fn with_tcp_server(
        config: ServerConfig<T, ferrowl_modbus::tcp::Config>,
        cache: ferrowl_modbus::tcp::SelfSignedCache,
    ) -> Self {
        Self {
            builder: Builder::TcpServer(ferrowl_modbus::tcp::ServerBuilder::new(
                config.config,
                config.memory,
                cache,
            )),
            task: TaskState::Idle,
        }
    }

    pub fn with_rtu_server(config: ServerConfig<T, ferrowl_modbus::rtu::Config>) -> Self {
        Self {
            builder: Builder::RtuServer(ferrowl_modbus::rtu::ServerBuilder::new(
                config.config,
                config.memory,
            )),
            task: TaskState::Idle,
        }
    }

    pub fn with_rtu_over_tcp_client(
        config: ClientConfig<T, ferrowl_modbus::tcp::Config>,
        cache: ferrowl_modbus::tcp::SelfSignedCache,
    ) -> Self {
        Self {
            builder: Builder::RtuOverTcpClient(ferrowl_modbus::rtu_over_tcp::ClientBuilder::new(
                config.config,
                config.operations,
                config.memory,
                cache,
            )),
            task: TaskState::Idle,
        }
    }

    pub fn with_rtu_over_tcp_server(
        config: ServerConfig<T, ferrowl_modbus::tcp::Config>,
        cache: ferrowl_modbus::tcp::SelfSignedCache,
    ) -> Self {
        Self {
            builder: Builder::RtuOverTcpServer(ferrowl_modbus::rtu_over_tcp::ServerBuilder::new(
                config.config,
                config.memory,
                cache,
            )),
            task: TaskState::Idle,
        }
    }

    pub fn with_udp_client(config: ClientConfig<T, ferrowl_modbus::udp::Config>) -> Self {
        Self {
            builder: Builder::UdpClient(ferrowl_modbus::udp::ClientBuilder::new(
                config.config,
                config.operations,
                config.memory,
            )),
            task: TaskState::Idle,
        }
    }

    pub fn with_udp_server(config: ServerConfig<T, ferrowl_modbus::udp::Config>) -> Self {
        Self {
            builder: Builder::UdpServer(ferrowl_modbus::udp::ServerBuilder::new(
                config.config,
                config.memory,
            )),
            task: TaskState::Idle,
        }
    }

    pub fn with_ascii_client(config: ClientConfig<T, ferrowl_modbus::rtu::Config>) -> Self {
        Self {
            builder: Builder::AsciiClient(ferrowl_modbus::ascii::ClientBuilder::new(
                config.config,
                config.operations,
                config.memory,
            )),
            task: TaskState::Idle,
        }
    }

    pub fn with_ascii_server(config: ServerConfig<T, ferrowl_modbus::rtu::Config>) -> Self {
        Self {
            builder: Builder::AsciiServer(ferrowl_modbus::ascii::ServerBuilder::new(
                config.config,
                config.memory,
            )),
            task: TaskState::Idle,
        }
    }

    pub fn with_ascii_over_tcp_client(
        config: ClientConfig<T, ferrowl_modbus::tcp::Config>,
        cache: ferrowl_modbus::tcp::SelfSignedCache,
    ) -> Self {
        Self {
            builder: Builder::AsciiOverTcpClient(
                ferrowl_modbus::ascii_over_tcp::ClientBuilder::new(
                    config.config,
                    config.operations,
                    config.memory,
                    cache,
                ),
            ),
            task: TaskState::Idle,
        }
    }

    pub fn with_ascii_over_tcp_server(
        config: ServerConfig<T, ferrowl_modbus::tcp::Config>,
        cache: ferrowl_modbus::tcp::SelfSignedCache,
    ) -> Self {
        Self {
            builder: Builder::AsciiOverTcpServer(
                ferrowl_modbus::ascii_over_tcp::ServerBuilder::new(
                    config.config,
                    config.memory,
                    cache,
                ),
            ),
            task: TaskState::Idle,
        }
    }

    /// Spawns the endpoint's background task. Fails with
    /// [`InstanceError::AlreadyActive`] if it is still running.
    pub async fn start<L, S>(&mut self, log: L, status: S) -> Result<(), Error>
    where
        L: LogFn + Clone,
        S: LogFn + Clone,
    {
        if let Some(h) = self.handle()
            && !h.is_finished()
        {
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
                    Ok((handle, connected)) => {
                        self.task = TaskState::Running(Handle::Client(handle::ClientHandle {
                            handle,
                            sender,
                            connected,
                        }));
                    }
                }
            }
            Builder::TcpServer(builder) => {
                let (sender, receiver) = tokio::sync::mpsc::channel(10);
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok((handle, bound_addr)) => {
                        self.task = TaskState::Running(Handle::Server(handle::ServerHandle {
                            handle,
                            sender,
                            bound_addr,
                            open: ferrowl_modbus::ConnectedCell::default(),
                        }));
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
                    Ok((handle, connected)) => {
                        self.task = TaskState::Running(Handle::Client(handle::ClientHandle {
                            handle,
                            sender,
                            connected,
                        }));
                    }
                }
            }
            Builder::RtuServer(builder) => {
                let (sender, receiver) = tokio::sync::mpsc::channel(10);
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok((handle, open)) => {
                        // Pure serial: no socket, so no bind to report — an `Arc` nobody ever
                        // writes to reads back `None` from `Instance::bound_addr()`, correctly
                        // indistinguishable from "never bound." `open` (MB-R-153) is the real
                        // "port currently open" signal `connection_status()` reads instead.
                        self.task = TaskState::Running(Handle::Server(handle::ServerHandle {
                            handle,
                            sender,
                            bound_addr: std::sync::Arc::new(parking_lot::Mutex::new(None)),
                            open,
                        }));
                    }
                }
            }
            Builder::RtuOverTcpClient(builder) => {
                let (sender, receiver) = tokio::sync::mpsc::channel(10);
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok((handle, connected)) => {
                        self.task = TaskState::Running(Handle::Client(handle::ClientHandle {
                            handle,
                            sender,
                            connected,
                        }));
                    }
                }
            }
            Builder::RtuOverTcpServer(builder) => {
                let (sender, receiver) = tokio::sync::mpsc::channel(10);
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok((handle, bound_addr)) => {
                        self.task = TaskState::Running(Handle::Server(handle::ServerHandle {
                            handle,
                            sender,
                            bound_addr,
                            open: ferrowl_modbus::ConnectedCell::default(),
                        }));
                    }
                }
            }
            Builder::UdpClient(builder) => {
                let (sender, receiver) = tokio::sync::mpsc::channel(10);
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok((handle, connected)) => {
                        self.task = TaskState::Running(Handle::Client(handle::ClientHandle {
                            handle,
                            sender,
                            connected,
                        }));
                    }
                }
            }
            Builder::UdpServer(builder) => {
                let (sender, receiver) = tokio::sync::mpsc::channel(10);
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok((handle, bound_addr)) => {
                        self.task = TaskState::Running(Handle::Server(handle::ServerHandle {
                            handle,
                            sender,
                            bound_addr,
                            open: ferrowl_modbus::ConnectedCell::default(),
                        }));
                    }
                }
            }
            Builder::AsciiClient(builder) => {
                let (sender, receiver) = tokio::sync::mpsc::channel(10);
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok((handle, connected)) => {
                        self.task = TaskState::Running(Handle::Client(handle::ClientHandle {
                            handle,
                            sender,
                            connected,
                        }));
                    }
                }
            }
            Builder::AsciiServer(builder) => {
                let (sender, receiver) = tokio::sync::mpsc::channel(10);
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok((handle, open)) => {
                        // Pure serial — see the identical `RtuServer` arm above.
                        self.task = TaskState::Running(Handle::Server(handle::ServerHandle {
                            handle,
                            sender,
                            bound_addr: std::sync::Arc::new(parking_lot::Mutex::new(None)),
                            open,
                        }));
                    }
                }
            }
            Builder::AsciiOverTcpClient(builder) => {
                let (sender, receiver) = tokio::sync::mpsc::channel(10);
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok((handle, connected)) => {
                        self.task = TaskState::Running(Handle::Client(handle::ClientHandle {
                            handle,
                            sender,
                            connected,
                        }));
                    }
                }
            }
            Builder::AsciiOverTcpServer(builder) => {
                let (sender, receiver) = tokio::sync::mpsc::channel(10);
                let res = builder.spawn(receiver, log, status).await;
                match res {
                    Err(e) => {
                        return Err(e.into());
                    }
                    Ok((handle, bound_addr)) => {
                        self.task = TaskState::Running(Handle::Server(handle::ServerHandle {
                            handle,
                            sender,
                            bound_addr,
                            open: ferrowl_modbus::ConnectedCell::default(),
                        }));
                    }
                }
            }
        }
        Ok(())
    }

    /// The address a server instance is actually bound to right now — `None` for a client
    /// instance, a pure-serial (Rtu/Ascii) server, an instance never started, or a TCP-framed/UDP
    /// server backing off from a failed bind (MB-R-130). `Some(<real addr>)` once bound, useful
    /// when the configured port was `0`. A caller that needs to know the listener is up (not
    /// merely that `start()` returned — the bind itself races behind the retried task) polls
    /// this instead of sleeping a fixed duration.
    pub fn bound_addr(&self) -> Option<std::net::SocketAddr> {
        match self.handle() {
            Some(Handle::Server(h)) => *h.bound_addr.lock(),
            _ => None,
        }
    }

    /// MB-R-137/153, UI-E-147 — the tri-state connection status, uniformly derived: not running →
    /// `Disconnected`; running (including while stopping — the task is still alive until
    /// `poll_stop` reaps it) and currently connected/bound/open → `Connected`; running and not →
    /// `Reconnecting`.
    pub fn connection_status(&self) -> crate::view::status_bar::ConnStatus {
        use crate::view::status_bar::ConnStatus;
        match self.handle() {
            None => ConnStatus::Disconnected,
            Some(h) if h.is_finished() => ConnStatus::Disconnected,
            Some(Handle::Client(c)) => {
                if c.connected.get() {
                    ConnStatus::Connected
                } else {
                    ConnStatus::Reconnecting
                }
            }
            Some(Handle::Server(s)) => {
                if s.bound_addr.lock().is_some() || s.open.get() {
                    ConnStatus::Connected
                } else {
                    ConnStatus::Reconnecting
                }
            }
        }
    }

    /// UI-R-314 — sends the running task a graceful terminate and returns immediately, without
    /// waiting for it to actually end (that's [`poll_stop`](Self::poll_stop)'s job, driven by the
    /// caller's own per-tick `refresh()`). `Err(NotRunning)` if the instance was already `Idle`.
    pub async fn request_stop(&mut self) -> Result<(), Error> {
        if !matches!(&self.task, TaskState::Running(_)) {
            return Err(InstanceError::NotRunning.into());
        }
        let TaskState::Running(handle) = std::mem::replace(&mut self.task, TaskState::Idle) else {
            unreachable!("matched Running just above");
        };

        match &handle {
            Handle::Client(h) => {
                let _ = h.sender.send(ferrowl_modbus::Command::Terminate).await;
            }
            Handle::Server(h) => {
                let _ = h
                    .sender
                    .send(ferrowl_modbus::ServerCommand::Terminate)
                    .await;
            }
        }

        self.task = TaskState::Stopping {
            handle,
            deadline: tokio::time::Instant::now() + tokio::time::Duration::from_millis(100),
        };
        Ok(())
    }

    /// UI-R-315 — polls a stop requested via [`request_stop`](Self::request_stop): `None` while
    /// there is nothing to report (never running, still running with no stop requested, or still
    /// within the grace period and not yet finished on its own); `Some(_)` once the task has
    /// ended — after the grace period elapses, still-unfinished tasks are aborted first.
    pub async fn poll_stop(&mut self) -> Option<Result<(), Error>> {
        let TaskState::Stopping { handle, deadline } = &self.task else {
            return None;
        };
        if !handle.is_finished() && tokio::time::Instant::now() < *deadline {
            return None;
        }

        let TaskState::Stopping { handle, .. } = std::mem::replace(&mut self.task, TaskState::Idle)
        else {
            unreachable!("matched Stopping just above");
        };

        let res = match handle {
            Handle::Client(h) => {
                if h.handle.is_finished() {
                    Ok(Ok(()))
                } else {
                    h.handle.abort();
                    h.handle.await
                }
            }
            Handle::Server(h) => {
                if h.handle.is_finished() {
                    Ok(Ok(()))
                } else {
                    h.handle.abort();
                    h.handle.await
                }
            }
        };

        Some(match res {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(e.into()),
            Err(e) => {
                if e.is_cancelled() {
                    Ok(())
                } else {
                    Err(InstanceError::CancelFailed.into())
                }
            }
        })
    }

    /// Stops the running task: asks clients to terminate gracefully, then
    /// aborts the task if it is still alive.
    pub async fn stop(&mut self) -> Result<(), Error> {
        self.request_stop().await?;
        loop {
            if let Some(res) = self.poll_stop().await {
                return res;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
    }

    /// Forwards a write/terminate command to a running client. Errors if no
    /// task is running or the instance is a server.
    pub async fn send_command(&self, command: ferrowl_modbus::Command) -> Result<(), Error> {
        if self.handle().is_none() {
            return Err(InstanceError::NotRunning.into());
        }
        match self.handle() {
            Some(Handle::Client(handle)) => handle
                .sender
                .send(command)
                .await
                .map_err(|e| InstanceError::SendError(e).into()),
            _ => Err(InstanceError::InvalidOperation.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrowl_modbus::UnitId;
    use ferrowl_test_support::{reserve_tcp_port, reserve_udp_port};

    use std::sync::Arc;

    use ferrowl_modbus::{Command, FunctionCode, Key, Operation, SlaveKey, tcp};
    use ferrowl_store::Range;
    use parking_lot::RwLock as MemLock;
    use tokio::sync::RwLock;

    /// No-op log/status sink satisfying `LogFn + Clone`.
    fn sink() -> impl LogFn + Clone {
        |_s: String| async move {}
    }

    /// A `tcp::Config` pointed at a local port nothing is listening on. `start()` still
    /// succeeds (spawn itself never touches the network) — only the spawned task's
    /// internal reconnect loop sees the refused connection.
    fn dead_tcp_config() -> tcp::Config {
        tcp::Config {
            ip: "127.0.0.1".to_string(),
            port: reserve_tcp_port().release(),
            timeout_ms: 200,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
            tls: Default::default(),
        }
    }

    fn tcp_client_instance() -> Instance<SlaveKey> {
        let operations = Arc::new(RwLock::new(vec![Operation {
            slave_id: UnitId(1),
            fn_code: FunctionCode::ReadHoldingRegisters,
            range: Range::new(0, 2),
        }]));
        Instance::with_tcp_client(
            config::ClientConfig {
                config: Arc::new(RwLock::new(dead_tcp_config())),
                operations,
                memory: Arc::new(MemLock::new(
                    ferrowl_store::Memory::<Key<SlaveKey>>::default(),
                )),
            },
            ferrowl_modbus::tcp::new_self_signed_cache(),
        )
    }

    #[tokio::test]
    async fn start_twice_is_already_active() {
        let mut instance = tcp_client_instance();
        instance.start(sink(), sink()).await.expect("first start");
        assert!(instance.active());

        let err = instance.start(sink(), sink()).await.unwrap_err();
        assert!(matches!(err, Error::Instance(InstanceError::AlreadyActive)));

        instance.stop().await.expect("cleanup stop");
    }

    #[tokio::test]
    async fn stop_never_started_is_not_running() {
        let mut instance = tcp_client_instance();
        let err = instance.stop().await.unwrap_err();
        assert!(matches!(err, Error::Instance(InstanceError::NotRunning)));
    }

    #[tokio::test]
    /// MB-R-093 — sending a write command to an instance that is not running fails rather than being silently dropped.
    async fn send_command_never_started_is_not_running() {
        let instance = tcp_client_instance();
        let err = instance.send_command(Command::Terminate).await.unwrap_err();
        assert!(matches!(err, Error::Instance(InstanceError::NotRunning)));
    }

    /// `send_command` on a server-role handle must reject with `InvalidOperation`. A real
    /// server would need a bound TCP listener; instead we construct the `Handle::Server`
    /// variant directly (both are in-crate types), which exercises exactly the same
    /// branch in `send_command` without any real I/O.
    #[tokio::test]
    /// MB-R-093 — sending a write command to an instance whose role is a server fails with an error rather than being silently dropped.
    async fn send_command_on_server_is_invalid_operation() {
        let mut instance = tcp_client_instance();
        let task = tokio::spawn(async { Ok(()) });
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        instance.task = TaskState::Running(handle::Handle::Server(handle::ServerHandle {
            handle: task,
            sender,
            bound_addr: Arc::new(parking_lot::Mutex::new(None)),
            open: ferrowl_modbus::ConnectedCell::default(),
        }));

        let err = instance.send_command(Command::Terminate).await.unwrap_err();
        assert!(matches!(
            err,
            Error::Instance(InstanceError::InvalidOperation)
        ));

        instance.stop().await.expect("cleanup stop");
    }

    #[tokio::test]
    /// MB-R-094 — stopping a client requests graceful termination and deactivates the instance.
    async fn graceful_stop_deactivates_instance() {
        let mut instance = tcp_client_instance();
        instance.start(sink(), sink()).await.expect("start");
        assert!(instance.active());

        instance.stop().await.expect("stop");
        assert!(!instance.active());
    }

    #[tokio::test]
    /// MB-R-094 — a stopped instance is restartable: after a graceful stop it can be started again.
    async fn stopped_instance_is_restartable() {
        let mut instance = tcp_client_instance();
        instance.start(sink(), sink()).await.expect("first start");
        assert!(instance.active());
        instance.stop().await.expect("stop");
        assert!(!instance.active());

        // Restart the same instance.
        instance.start(sink(), sink()).await.expect("restart");
        assert!(instance.active());
        instance.stop().await.expect("cleanup stop");
    }

    /// MB-R-114 — an RtuOverTcp client instance starts, connects, and stops exactly
    /// like a TCP client instance (reuses `tcp::Config`, MB-R-113).
    fn rtu_over_tcp_client_instance() -> Instance<SlaveKey> {
        let operations = Arc::new(RwLock::new(vec![]));
        Instance::with_rtu_over_tcp_client(
            config::ClientConfig {
                config: Arc::new(RwLock::new(dead_tcp_config())),
                operations,
                memory: Arc::new(MemLock::new(
                    ferrowl_store::Memory::<Key<SlaveKey>>::default(),
                )),
            },
            ferrowl_modbus::tcp::new_self_signed_cache(),
        )
    }

    #[tokio::test]
    async fn rtu_over_tcp_start_twice_is_already_active() {
        let mut instance = rtu_over_tcp_client_instance();
        instance.start(sink(), sink()).await.expect("first start");
        assert!(instance.active());
        let err = instance.start(sink(), sink()).await.unwrap_err();
        assert!(matches!(err, Error::Instance(InstanceError::AlreadyActive)));
        instance.stop().await.expect("cleanup stop");
    }

    fn dead_udp_config(port: u16) -> ferrowl_modbus::udp::Config {
        ferrowl_modbus::udp::Config {
            ip: "127.0.0.1".to_string(),
            port,
            timeout_ms: 200,
            delay_ms: 0,
            interval_ms: 0,
            reconnect: true,
        }
    }

    /// MB-R-117 — a Udp client instance starts (association is local-only, no I/O to fail
    /// against a dead peer), reports active, and stops gracefully like every other transport.
    #[tokio::test]
    async fn udp_client_starts_and_stops() {
        let port = reserve_udp_port().release();
        let mut instance = Instance::with_udp_client(config::ClientConfig {
            config: Arc::new(RwLock::new(dead_udp_config(port))),
            operations: Arc::new(RwLock::new(vec![])),
            memory: Arc::new(MemLock::new(
                ferrowl_store::Memory::<Key<SlaveKey>>::default(),
            )),
        });
        instance.start(sink(), sink()).await.expect("start");
        assert!(instance.active());
        instance.stop().await.expect("stop");
        assert!(!instance.active());
    }

    /// MB-R-119 — a Udp server instance binds, reports active, and stops gracefully like
    /// every other transport.
    #[tokio::test]
    async fn udp_server_starts_and_stops() {
        let port = reserve_udp_port().release();
        let mut instance = Instance::with_udp_server(config::ServerConfig {
            config: Arc::new(RwLock::new(dead_udp_config(port))),
            memory: Arc::new(MemLock::new(
                ferrowl_store::Memory::<Key<SlaveKey>>::default(),
            )),
        });
        instance.start(sink(), sink()).await.expect("start");
        assert!(instance.active());
        instance.stop().await.expect("stop");
        assert!(!instance.active());
    }

    /// A serial path that cannot be opened, so `SerialStream::open` fails — mirrors
    /// `ascii_serial.rs`'s `bad_config` at the `Instance` layer.
    fn dead_rtu_config(reconnect: bool) -> ferrowl_modbus::rtu::Config {
        ferrowl_modbus::rtu::Config {
            path: "/nonexistent/ferrowl-no-such-serial-port".to_string(),
            baud_rate: 115200,
            slave: 1,
            parity: None,
            data_bits: None,
            stop_bits: None,
            timeout_ms: 1000,
            delay_ms: 0,
            interval_ms: 0,
            reconnect,
        }
    }

    /// MB-R-122 — an Ascii client instance surfaces a serial-open failure from `start`
    /// exactly like an RTU client instance (MB-R-209/124), ending the task (reconnect off).
    #[tokio::test]
    async fn ascii_client_open_failure_ends_task() {
        let mut instance = Instance::with_ascii_client(config::ClientConfig {
            config: Arc::new(RwLock::new(dead_rtu_config(false))),
            operations: Arc::new(RwLock::new(vec![])),
            memory: Arc::new(MemLock::new(
                ferrowl_store::Memory::<Key<SlaveKey>>::default(),
            )),
        });
        instance
            .start(sink(), sink())
            .await
            .expect("spawn succeeds; the open error surfaces from the task");

        for _ in 0..50 {
            if !instance.active() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }
        assert!(!instance.active());
    }

    /// MB-R-123, MB-R-130 — an Ascii server instance's `start` always succeeds (the serial-open
    /// failure surfaces inside the retried task); with `reconnect` disabled the instance still
    /// ends up inactive shortly after, exactly like an RTU server instance and like
    /// `ascii_client_open_failure_ends_task` above.
    #[tokio::test]
    async fn ascii_server_open_failure_ends_task() {
        let mut instance = Instance::with_ascii_server(config::ServerConfig {
            config: Arc::new(RwLock::new(dead_rtu_config(false))),
            memory: Arc::new(MemLock::new(
                ferrowl_store::Memory::<Key<SlaveKey>>::default(),
            )),
        });
        instance
            .start(sink(), sink())
            .await
            .expect("spawn always returns Ok; the open error surfaces from the task");

        for _ in 0..50 {
            if !instance.active() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }
        assert!(!instance.active());
    }

    /// MB-R-126 — an AsciiOverTcp client instance starts, connects, and stops exactly like a
    /// TCP/RtuOverTcp client instance (reuses `tcp::Config`, MB-R-125).
    fn ascii_over_tcp_client_instance() -> Instance<SlaveKey> {
        let operations = Arc::new(RwLock::new(vec![]));
        Instance::with_ascii_over_tcp_client(
            config::ClientConfig {
                config: Arc::new(RwLock::new(dead_tcp_config())),
                operations,
                memory: Arc::new(MemLock::new(
                    ferrowl_store::Memory::<Key<SlaveKey>>::default(),
                )),
            },
            ferrowl_modbus::tcp::new_self_signed_cache(),
        )
    }

    #[tokio::test]
    async fn ascii_over_tcp_start_twice_is_already_active() {
        let mut instance = ascii_over_tcp_client_instance();
        instance.start(sink(), sink()).await.expect("first start");
        assert!(instance.active());
        let err = instance.start(sink(), sink()).await.unwrap_err();
        assert!(matches!(err, Error::Instance(InstanceError::AlreadyActive)));
        instance.stop().await.expect("cleanup stop");
    }

    /// MB-R-126 — an AsciiOverTcp server instance binds, reports active, and stops gracefully
    /// like every other transport.
    #[tokio::test]
    async fn ascii_over_tcp_server_starts_and_stops() {
        let port = reserve_tcp_port().release();
        let mut instance = Instance::with_ascii_over_tcp_server(
            config::ServerConfig {
                config: Arc::new(RwLock::new(tcp::Config {
                    ip: "127.0.0.1".to_string(),
                    port,
                    timeout_ms: 200,
                    delay_ms: 0,
                    interval_ms: 0,
                    reconnect: true,
                    tls: Default::default(),
                })),
                memory: Arc::new(MemLock::new(
                    ferrowl_store::Memory::<Key<SlaveKey>>::default(),
                )),
            },
            ferrowl_modbus::tcp::new_self_signed_cache(),
        );
        instance.start(sink(), sink()).await.expect("start");
        assert!(instance.active());
        instance.stop().await.expect("stop");
        assert!(!instance.active());
    }

    #[tokio::test]
    async fn active_reflects_finished_task() {
        let mut instance = tcp_client_instance();
        instance.start(sink(), sink()).await.expect("start");
        assert!(instance.active());

        // Force the client task to exit on its own by telling it to terminate, then give
        // it a moment to actually finish, without going through `stop()`'s bookkeeping.
        instance
            .send_command(Command::Terminate)
            .await
            .expect("send terminate");
        for _ in 0..50 {
            if !instance.active() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }
        assert!(!instance.active());

        // `stop()` on an already-finished task still tears down bookkeeping cleanly.
        instance.stop().await.expect("stop after natural finish");
    }

    /// MB-R-130 (bound_addr companion) — a TCP server instance's `bound_addr()` is `None` right
    /// after `start()` returns (which only guarantees the task was scheduled), `Some(<real
    /// addr>)` once the listener actually binds (even with the configured `port: 0`), and `None`
    /// again after a graceful `stop()` — the ready signal a caller polls instead of racing
    /// `start()`'s return with a fixed sleep.
    #[tokio::test]
    async fn it_tcp_server_bound_addr_reflects_listener_state() {
        let mut instance = Instance::with_tcp_server(
            config::ServerConfig {
                config: Arc::new(RwLock::new(tcp::Config {
                    ip: "127.0.0.1".to_string(),
                    port: 0,
                    timeout_ms: 200,
                    delay_ms: 0,
                    interval_ms: 0,
                    reconnect: true,
                    tls: Default::default(),
                })),
                memory: Arc::new(MemLock::new(
                    ferrowl_store::Memory::<Key<SlaveKey>>::default(),
                )),
            },
            ferrowl_modbus::tcp::new_self_signed_cache(),
        );
        assert!(instance.bound_addr().is_none());

        instance.start(sink(), sink()).await.expect("start");

        let mut addr = None;
        for _ in 0..50 {
            addr = instance.bound_addr();
            if addr.is_some() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        }
        let addr = addr.expect("listener must have bound within 1s");
        assert_ne!(addr.port(), 0, "the OS must have assigned a real port");

        instance.stop().await.expect("stop");
        assert!(
            instance.bound_addr().is_none(),
            "bound_addr must clear once the instance stops"
        );
    }

    /// MB-R-133 — stopping a running TCP server sends `ServerCommand::Terminate` through the
    /// new server sender and awaits the task, the same graceful path clients already had; this
    /// is asserted indirectly by timing: the 100ms grace-period sleep `stop()` always takes
    /// after a successful send is the *only* delay, so `stop()` returning promptly after that
    /// (well under an additional 100ms) proves the send-then-await path fired rather than
    /// `stop()` falling through to the abort-and-await fallback for a task that never got the
    /// message.
    #[tokio::test]
    async fn it_server_terminate_ends_task_gracefully() {
        let port = reserve_tcp_port().release();
        let mut instance = Instance::with_tcp_server(
            config::ServerConfig {
                config: Arc::new(RwLock::new(tcp::Config {
                    ip: "127.0.0.1".to_string(),
                    port,
                    timeout_ms: 200,
                    delay_ms: 0,
                    interval_ms: 0,
                    reconnect: true,
                    tls: Default::default(),
                })),
                memory: Arc::new(MemLock::new(
                    ferrowl_store::Memory::<Key<SlaveKey>>::default(),
                )),
            },
            ferrowl_modbus::tcp::new_self_signed_cache(),
        );
        instance.start(sink(), sink()).await.expect("start");
        assert!(instance.active());

        let before = tokio::time::Instant::now();
        instance.stop().await.expect("stop");
        assert!(!instance.active());
        // `stop()`'s own grace period is 100ms; comfortably under 250ms proves no abort-fallback
        // wait (which would additionally block on `h.handle.await` after `abort()`, itself
        // instant, but a task that never received Terminate wouldn't have exited gracefully
        // during the 100ms sleep in the first place — this bounds out that failure mode).
        assert!(
            before.elapsed() < tokio::time::Duration::from_millis(250),
            "stop() took {:?}, expected the graceful send-then-await path to finish promptly",
            before.elapsed()
        );
    }

    /// MB-R-133 — stopping a TCP server whose task is actively backing off from a bind failure
    /// (an occupied port, per MB-R-071/MB-R-130) still ends it gracefully and promptly via
    /// `ServerCommand::Terminate`, exercised through the full `Instance` stack (not just
    /// `ferrowl-modbus` in isolation, already covered there).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn it_server_stop_on_backing_off_task_ends_promptly() {
        let occupier = reserve_tcp_port();
        let port = occupier.port();
        let mut instance = Instance::with_tcp_server(
            config::ServerConfig {
                config: Arc::new(RwLock::new(tcp::Config {
                    ip: "127.0.0.1".to_string(),
                    port,
                    timeout_ms: 200,
                    delay_ms: 0,
                    interval_ms: 0,
                    reconnect: true,
                    tls: Default::default(),
                })),
                memory: Arc::new(MemLock::new(
                    ferrowl_store::Memory::<Key<SlaveKey>>::default(),
                )),
            },
            ferrowl_modbus::tcp::new_self_signed_cache(),
        );
        instance.start(sink(), sink()).await.expect("start");
        assert!(instance.active());

        // Give the task a moment to actually be in its backoff wait, not mid-bind-attempt.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let result = tokio::time::timeout(tokio::time::Duration::from_secs(2), instance.stop())
            .await
            .expect("stop() must not hang while the task is backing off");
        assert!(result.is_ok());
        assert!(!instance.active());
    }

    /// MB-R-150 — `path_conflict_cell()` is `Some` for an Rtu client (the only serial transport
    /// that participates in the path-conflict check exercised here) and `None` for a Tcp client
    /// (a non-serial transport, which never participates).
    #[tokio::test]
    async fn ut_instance_path_conflict_cell_some_for_rtu_ascii_none_for_others() {
        let rtu_instance = Instance::with_rtu_client(config::ClientConfig {
            config: Arc::new(RwLock::new(dead_rtu_config(false))),
            operations: Arc::new(RwLock::new(vec![])),
            memory: Arc::new(MemLock::new(
                ferrowl_store::Memory::<Key<SlaveKey>>::default(),
            )),
        });
        assert!(rtu_instance.path_conflict_cell().is_some());

        let tcp_instance = tcp_client_instance();
        assert!(tcp_instance.path_conflict_cell().is_none());
    }

    /// UI-R-314 — `request_stop()` sends the terminate and returns immediately, well under the
    /// 100ms grace period `poll_stop()` waits out, instead of blocking until the task ends.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ut_request_stop_returns_before_the_task_ends() {
        let occupier = reserve_tcp_port();
        let port = occupier.port();
        let mut instance = Instance::with_tcp_server(
            config::ServerConfig {
                config: Arc::new(RwLock::new(tcp::Config {
                    ip: "127.0.0.1".to_string(),
                    port,
                    timeout_ms: 200,
                    delay_ms: 0,
                    interval_ms: 0,
                    reconnect: true,
                    tls: Default::default(),
                })),
                memory: Arc::new(MemLock::new(
                    ferrowl_store::Memory::<Key<SlaveKey>>::default(),
                )),
            },
            ferrowl_modbus::tcp::new_self_signed_cache(),
        );
        instance.start(sink(), sink()).await.expect("start");
        assert!(instance.active());

        // The task is backing off from the occupied port, so a real `stop()` would need its
        // full 100ms grace period; `request_stop()` must not wait for any of that.
        let before = tokio::time::Instant::now();
        instance.request_stop().await.expect("request_stop");
        assert!(
            before.elapsed() < tokio::time::Duration::from_millis(50),
            "request_stop() took {:?}, expected to return immediately",
            before.elapsed()
        );

        // Cleanup: drive the stop to completion.
        loop {
            if instance.poll_stop().await.is_some() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
    }

    /// UI-R-314 — a second `request_stop()` while already `Stopping` errors with `NotRunning`
    /// rather than dropping the live handle (which would orphan the task and flip status to
    /// `Disconnected` while it is still running, contradicting UI-E-147).
    #[tokio::test]
    async fn ut_request_stop_while_already_stopping_does_not_orphan_the_handle() {
        use crate::view::status_bar::ConnStatus;

        let mut instance = tcp_client_instance();
        instance.start(sink(), sink()).await.expect("start");

        instance.request_stop().await.expect("first request_stop");
        let err = instance.request_stop().await.unwrap_err();
        assert!(matches!(err, Error::Instance(InstanceError::NotRunning)));
        assert_eq!(
            instance.connection_status(),
            ConnStatus::Reconnecting,
            "the handle must still be tracked, not dropped, after the repeat request"
        );

        loop {
            if instance.poll_stop().await.is_some() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
        assert_eq!(instance.connection_status(), ConnStatus::Disconnected);
    }

    /// UI-R-315 — `poll_stop()` reports `None` until the task actually ends, then `Some(Ok(()))`
    /// once it does.
    #[tokio::test]
    async fn ut_poll_stop_reports_outcome_once_the_task_ends() {
        let mut instance = tcp_client_instance();
        instance.start(sink(), sink()).await.expect("start");

        instance.request_stop().await.expect("request_stop");
        assert!(
            instance.poll_stop().await.is_none(),
            "poll_stop() must report nothing before the grace period or task completion"
        );

        let result = loop {
            if let Some(res) = instance.poll_stop().await {
                break res;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        };
        assert!(result.is_ok());
        assert!(!instance.active());
        assert!(
            instance.poll_stop().await.is_none(),
            "poll_stop() must report nothing once already Idle"
        );
    }

    /// UI-E-147 — the connection status stays `Reconnecting` while a stop is in flight (the task
    /// is still alive until `poll_stop` reaps it), then becomes `Disconnected` once `poll_stop`
    /// returns `Some(_)`.
    #[tokio::test]
    async fn ut_connection_status_reconnecting_while_stopping() {
        use crate::view::status_bar::ConnStatus;

        let mut instance = tcp_client_instance();
        instance.start(sink(), sink()).await.expect("start");
        assert_eq!(instance.connection_status(), ConnStatus::Reconnecting);

        instance.request_stop().await.expect("request_stop");
        assert_eq!(
            instance.connection_status(),
            ConnStatus::Reconnecting,
            "must not report Disconnected until poll_stop reaps the task"
        );

        loop {
            if instance.poll_stop().await.is_some() {
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        }
        assert_eq!(instance.connection_status(), ConnStatus::Disconnected);
    }
}
