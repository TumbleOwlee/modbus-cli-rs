//! Transport-agnostic Modbus client loop shared by every Modbus client transport.
//!
//! Holds the framing-generic connect helpers for both transport families (`connect_serial` for
//! `Rtu`/`Ascii` over a serial port, `connect_tcp_family` for `Tcp`/`RtuOverTcp`/`Ascii` over a
//! socket), the `spawn_client_task`/`ClientTiming` pair every transport's `ClientBuilder::spawn`
//! delegates to, and the read/run loop and command execution that follow a successful connect,
//! identical across every transport once a `ClientCore` exists.

use crate::common::serial_config_from;
use crate::tcp::tls::{ClientStream, SelfSignedCache, build_client_tls_config};
use crate::{
    Command, Error, Key, KeyParams, LogFn, ModbusError, Operation, PathConflictCell, RunConfig,
    SerialError, TcpError,
};

use ferrowl_store::Memory;
use parking_lot::RwLock as MemLock;
use rust_modbus::{
    Address, Client, ClientFraming, ClientTransport, ExceptionCode, FrameTransport, Framing,
    FunctionCode, Quantity, RegisterValue, SerialStream, TcpConfig, UnitId, connect_tcp_framed,
    connect_tls_framed, open_serial,
};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::sync::mpsc::Receiver;
use tokio::task::JoinHandle;
use tokio::time::sleep;

/// Outcome of one connection attempt, as reported by a transport's `connect` closure passed to
/// [`ClientCore::run_reconnect_loop`]. Bundles the just-read config snapshot (`reconnect` plus
/// the timing fields `run` needs) alongside the connection result itself, since `reconnect` must
/// be known even when the connect attempt failed.
pub(crate) struct ConnectAttempt<S, F> {
    pub(crate) reconnect: bool,
    pub(crate) timeout_ms: usize,
    pub(crate) delay_ms: usize,
    pub(crate) interval_ms: usize,
    pub(crate) client: Result<ClientCore<S, F>, Error>,
}

/// Number of consecutive Modbus exceptions tolerated before the client skips the operation.
pub(crate) const MAX_RETRIES: u32 = 3;

/// Logs the "about to read" intent line shared by every read function code.
async fn log_read_intent<L>(log: &L, name: &str, slave_id: UnitId, start: usize, end: usize)
where
    L: LogFn,
{
    log.invoke(format!(
        "Perform {name} request for slave ID {slave_id} and range [{start}, {end})."
    ))
    .await;
}

/// Converts a coil/discrete-input bit vector to the `u16` shape the shared memory store uses.
pub(crate) fn bits_to_words(bits: Vec<bool>) -> Vec<u16> {
    bits.into_iter().map(|b| if b { 1 } else { 0 }).collect()
}

/// Unwraps register values into the bare `u16` shape the shared memory store uses.
pub(crate) fn words(registers: Vec<RegisterValue>) -> Vec<u16> {
    registers.into_iter().map(|v| v.0).collect()
}

/// Classifies a completed timeout+request result into the single `ModbusError` shape shared
/// by every read and write outcome.
/// A device's refusal arrives as `Error::Exception`, alongside — not inside — the transport
/// failures, so the exception path (retry, no disconnect) is separated here rather than by
/// the shape of the `Result`.
pub(crate) fn classify<V>(
    result: Result<Result<V, rust_modbus::Error>, tokio::time::error::Elapsed>,
) -> Result<V, ModbusError> {
    match result {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(rust_modbus::Error::Exception { exception, .. })) => {
            Err(ModbusError::Exception(exception))
        }
        Ok(Err(e)) => Err(ModbusError::Error(e)),
        Err(e) => Err(ModbusError::Timeout(e)),
    }
}

/// Owns a connected Modbus client and drives the read/command loop. Transport-neutral: the TCP
/// and RTU `Client` types each establish the connection, then hand the client here.
pub(crate) struct ClientCore<S, F> {
    pub(crate) client: Client<S, F>,
}

/// Opens the configured serial port under framing `F`, shared by the ASCII and RTU clients.
///
/// The port is not bound to a slave address: each request carries the slave id of the
/// operation or command that issued it (MB-R-048).
///
/// MB-R-150 — before the OS-level open, `path_conflict` is checked against the freshly
/// `~`-expanded path; a conflict short-circuits with `Error::PathConflict` and skips the open
/// attempt entirely.
pub(crate) fn connect_serial<F>(
    config: &crate::rtu::Config,
    path_conflict: &PathConflictCell,
) -> Result<ClientCore<FrameTransport<SerialStream, F>, F>, Error>
where
    F: Framing + ClientFraming + Send,
    F::Header: Sync,
{
    let serial = serial_config_from(
        config.baud_rate,
        config.data_bits,
        config.stop_bits,
        config.parity.as_deref(),
    )?;
    let expanded = ferrowl_util::path::expand(&config.path);
    let expanded = expanded.to_string_lossy();
    if let Some(other) = path_conflict.check(&expanded) {
        return Err(Error::PathConflict {
            path: expanded.into_owned(),
            other,
        });
    }
    match open_serial::<F>(&config.path, serial) {
        Ok(transport) => Ok(ClientCore {
            client: Client::new(transport),
        }),
        Err(e) => Err(SerialError::Error(e).into()),
    }
}

/// Opens a TCP connection to `config.ip:config.port`, bounded by the configured timeout. Plain
/// TCP unless the endpoint's client TLS policy is set (MB-R-104/MB-R-115/MB-R-127), in which case
/// the same timeout bounds the TCP connect and the TLS handshake together. `F` selects the on-wire
/// framing; establishing the socket does not differ by framing, only what is read off it does.
pub(crate) async fn connect_tcp_family<F>(
    config: &crate::tcp::Config,
    cache: &SelfSignedCache,
) -> Result<ClientCore<FrameTransport<ClientStream, F>, F>, Error>
where
    F: Framing + ClientFraming + Send,
    F::Header: Sync,
{
    let addr: SocketAddr = format!("{}:{}", config.ip, config.port)
        .parse()
        .map_err(|e| Error::Tcp(TcpError::Address(e)))?;
    let tls_config = build_client_tls_config(&config.client_tls_policy(), cache)?;
    let attempt = async {
        match tls_config {
            None => connect_tcp_framed::<F>(addr, TcpConfig::default())
                .await
                .map(|t| ClientStream::Plain(t.into_inner())),
            Some(tls) => connect_tls_framed::<F>(addr, TcpConfig::default(), tls)
                .await
                .map(|t| ClientStream::Tls(Box::new(t.into_inner()))),
        }
    };
    match tokio::time::timeout(
        std::time::Duration::from_millis(config.timeout_ms as u64),
        attempt,
    )
    .await
    {
        Ok(Ok(stream)) => Ok(ClientCore {
            client: Client::<_, _>::new(FrameTransport::new(stream)),
        }),
        Ok(Err(e)) => Err(TcpError::Error(e).into()),
        Err(e) => Err(TcpError::Timeout(e).into()),
    }
}

/// The four connect-loop timings every client config carries; the only fields
/// `spawn_client_task` needs to read out of a transport-specific `Config`.
pub(crate) trait ClientTiming {
    fn reconnect(&self) -> bool;
    fn timeout_ms(&self) -> usize;
    fn delay_ms(&self) -> usize;
    fn interval_ms(&self) -> usize;
}

impl ClientTiming for crate::rtu::Config {
    fn reconnect(&self) -> bool {
        self.reconnect
    }
    fn timeout_ms(&self) -> usize {
        self.timeout_ms
    }
    fn delay_ms(&self) -> usize {
        self.delay_ms
    }
    fn interval_ms(&self) -> usize {
        self.interval_ms
    }
}

impl ClientTiming for crate::tcp::Config {
    fn reconnect(&self) -> bool {
        self.reconnect
    }
    fn timeout_ms(&self) -> usize {
        self.timeout_ms
    }
    fn delay_ms(&self) -> usize {
        self.delay_ms
    }
    fn interval_ms(&self) -> usize {
        self.interval_ms
    }
}

impl ClientTiming for crate::udp::Config {
    fn reconnect(&self) -> bool {
        self.reconnect
    }
    fn timeout_ms(&self) -> usize {
        self.timeout_ms
    }
    fn delay_ms(&self) -> usize {
        self.delay_ms
    }
    fn interval_ms(&self) -> usize {
        self.interval_ms
    }
}

/// The connect callback's return type: a boxed future borrowing only the config guard.
pub(crate) type BoxedConnect<'a, S, F> =
    Pin<Box<dyn Future<Output = Result<ClientCore<S, F>, Error>> + Send + 'a>>;

/// Spawns a TCP-family client's reconnect loop: the `connect_tcp_family` counterpart to
/// `spawn_client_task`, pinning the socket type so `Tcp`, `RtuOverTcp` and `Ascii` over a socket
/// differ only in `F`.
pub(crate) fn spawn_tcp_family_client<T, F, L, St>(
    config: Arc<RwLock<crate::tcp::Config>>,
    operations: Arc<RwLock<Vec<Operation>>>,
    memory: Arc<MemLock<Memory<Key<T>>>>,
    cache: SelfSignedCache,
    receiver: Receiver<Command>,
    log: L,
    status: St,
) -> (JoinHandle<Result<(), Error>>, crate::ConnectedCell)
where
    T: KeyParams,
    F: Framing + ClientFraming + Send + Sync + 'static,
    F::Header: Send + Sync,
    L: LogFn + Clone,
    St: LogFn + Clone,
{
    spawn_client_task(
        config,
        operations,
        memory,
        receiver,
        log,
        status,
        move |cfg: &crate::tcp::Config| -> BoxedConnect<'_, FrameTransport<ClientStream, F>, F> {
            let cache = cache.clone();
            Box::pin(async move { connect_tcp_family::<F>(cfg, &cache).await })
        },
    )
}

/// Spawns a client's reconnect loop as a tokio task and hands back its handle and
/// [`ConnectedCell`]. `connect` is the only per-transport part: it receives the config under the
/// read guard held for the whole attempt and yields a connected [`ClientCore`], exactly as each
/// transport's `Client::connect` does.
pub(crate) fn spawn_client_task<T, Cfg, L, St, C, S, F>(
    config: Arc<RwLock<Cfg>>,
    operations: Arc<RwLock<Vec<Operation>>>,
    memory: Arc<MemLock<Memory<Key<T>>>>,
    receiver: Receiver<Command>,
    log: L,
    status: St,
    connect: C,
) -> (JoinHandle<Result<(), Error>>, crate::ConnectedCell)
where
    T: KeyParams,
    Cfg: ClientTiming + Send + Sync + 'static,
    L: LogFn + Clone,
    St: LogFn + Clone,
    S: ClientTransport<F> + Send + 'static,
    F: ClientFraming + Send + 'static,
    F::Header: Send,
    C: for<'a> Fn(&'a Cfg) -> BoxedConnect<'a, S, F> + Send + Sync + 'static,
{
    let connected = crate::ConnectedCell::default();
    let connected_for_task = connected.clone();
    let connect = Arc::new(connect);
    let handle = tokio::task::spawn(async move {
        ClientCore::run_reconnect_loop(
            receiver,
            log,
            status,
            operations,
            memory,
            move || {
                let config = config.clone();
                let connect = connect.clone();
                async move {
                    let guard = config.read().await;
                    let attempt = ConnectAttempt {
                        reconnect: guard.reconnect(),
                        timeout_ms: guard.timeout_ms(),
                        delay_ms: guard.delay_ms(),
                        interval_ms: guard.interval_ms(),
                        client: connect(&guard).await,
                    };
                    drop(guard);
                    attempt
                }
            },
            connected_for_task,
        )
        .await
    });
    (handle, connected)
}

impl<S, F> ClientCore<S, F>
where
    S: ClientTransport<F>,
    F: ClientFraming,
{
    pub(crate) async fn read<L>(
        &mut self,
        op: &Operation,
        timeout_ms: usize,
        log: &L,
    ) -> (&'static str, Result<Vec<u16>, ModbusError>)
    where
        L: LogFn,
    {
        let start = op.range.start();
        let end = op.range.end();
        let Ok(count) = u16::try_from(end - start) else {
            return (
                "Unknown",
                Err(ModbusError::Exception(ExceptionCode::IllegalDataValue)),
            );
        };
        // MB-R-101: on a framing that has a broadcast address (RTU; never TCP), a read
        // addressed to it cannot be answered by anyone. Refused here, in the same local-
        // exception shape as an over-long range, so it follows the retry path of MB-R-043
        // instead of surfacing as a transport error that would disconnect the client.
        if F::is_broadcast(op.slave_id) {
            log.invoke(format!(
                "Read request for slave ID {} skipped: address 0 is the broadcast address, which no device answers.",
                op.slave_id
            ))
            .await;
            return (
                "Broadcast",
                Err(ModbusError::Exception(ExceptionCode::IllegalDataAddress)),
            );
        }
        // MB-R-149: a start address that doesn't fit in the wire's u16 field is refused
        // locally, in the same shape as the count guard above, so it follows the same
        // exception-retry path as MB-R-043 instead of silently truncating.
        let Ok(start_addr) = u16::try_from(start) else {
            return (
                "Unknown",
                Err(ModbusError::Exception(ExceptionCode::IllegalDataValue)),
            );
        };
        let (address, quantity) = (Address(start_addr), Quantity(count));
        match op.fn_code {
            FunctionCode::ReadCoils => {
                log_read_intent(log, "ReadCoils", op.slave_id, start, end).await;
                let res = tokio::time::timeout(
                    Duration::from_millis(timeout_ms as u64),
                    self.client.read_coils(op.slave_id, address, quantity),
                )
                .await;
                ("ReadCoils", classify(res).map(bits_to_words))
            }
            FunctionCode::ReadDiscreteInputs => {
                log_read_intent(log, "ReadDiscreteInputs", op.slave_id, start, end).await;
                let res = tokio::time::timeout(
                    Duration::from_millis(timeout_ms as u64),
                    self.client
                        .read_discrete_inputs(op.slave_id, address, quantity),
                )
                .await;
                ("ReadDiscreteInputs", classify(res).map(bits_to_words))
            }
            FunctionCode::ReadInputRegisters => {
                log_read_intent(log, "ReadInputRegisters", op.slave_id, start, end).await;
                let res = tokio::time::timeout(
                    Duration::from_millis(timeout_ms as u64),
                    self.client
                        .read_input_registers(op.slave_id, address, quantity),
                )
                .await;
                ("ReadInputRegisters", classify(res).map(words))
            }
            FunctionCode::ReadHoldingRegisters => {
                log_read_intent(log, "ReadHoldingRegisters", op.slave_id, start, end).await;
                let res = tokio::time::timeout(
                    Duration::from_millis(timeout_ms as u64),
                    self.client
                        .read_holding_registers(op.slave_id, address, quantity),
                )
                .await;
                ("ReadHoldingRegisters", classify(res).map(words))
            }
            _ => (
                "Unknown",
                Err(ModbusError::Exception(ExceptionCode::IllegalFunction)),
            ),
        }
    }

    /// Classifies a completed write result and logs the outcome with the same four-way shape
    /// (timeout / io error / exception / success) shared by every write command. Disconnects and
    /// returns an error on timeout or io error; logs and continues (`Ok(())`) otherwise.
    /// `invalid_word` covers the one wording inconsistency between commands ("invalid" vs.
    /// "failed" for the exception case).
    async fn handle_write_result<V, L>(
        &mut self,
        result: Result<Result<V, rust_modbus::Error>, tokio::time::error::Elapsed>,
        label: &str,
        detail: &str,
        invalid_word: &str,
        log: &L,
    ) -> Result<(), Error>
    where
        L: LogFn,
    {
        match classify(result) {
            Ok(_) => {
                log.invoke(format!(
                    "{label} request to {detail} successfully executed."
                ))
                .await;
                Ok(())
            }
            Err(ModbusError::Exception(e)) => {
                log.invoke(format!(
                    "{label} request to {detail} {invalid_word}. [{e:?}]"
                ))
                .await;
                Ok(())
            }
            Err(ModbusError::Error(e)) => {
                log.invoke(format!(
                    "{label} request to {detail} failed. Disconnecting client. [{e:?}]"
                ))
                .await;
                Err(ModbusError::Error(e).into())
            }
            Err(ModbusError::Timeout(e)) => {
                log.invoke(format!(
                    "{label} request to {detail} timed out. Disconnecting client. [{e:?}]"
                ))
                .await;
                Err(ModbusError::Timeout(e).into())
            }
        }
    }

    /// Runs one poll cycle: reads the next operation in rotation and writes the result into
    /// `memory`, advancing (or retrying) the round-robin index. Broken out of `run` so the tick
    /// arm of its `select!` stays a single call. Sets `*had_success` on a successful read, so the
    /// caller's reconnect backoff can tell a live-then-dropped connection from a connection that
    /// never got a single read through.
    #[allow(clippy::too_many_arguments)]
    async fn poll_once<T, L>(
        &mut self,
        operations: &Arc<RwLock<Vec<Operation>>>,
        memory: &Arc<MemLock<Memory<Key<T>>>>,
        timeout_ms: usize,
        log: &L,
        index: &mut usize,
        retries: &mut u32,
        had_success: &mut bool,
    ) -> Result<(), Error>
    where
        T: KeyParams,
        L: LogFn,
    {
        let operations = operations.read().await;
        let count = operations.len();
        if *index >= count {
            *index = 0;
        }
        let operation = operations.get(*index).map(|v| (*v).clone());

        if let Some(operation) = operation {
            let fc = operation.fn_code;
            let range = operation.range.clone();
            let start = range.start();
            let end = range.end();
            match self.read(&operation, timeout_ms, log).await {
                (s, Ok(values)) => {
                    *had_success = true;
                    let key = Key {
                        id: T::from_slave_fn(operation.slave_id, fc),
                    };
                    // Scoped so the (sync) guard is dropped before the log `.await`s below.
                    let ok = {
                        let mut guard = memory.write();
                        guard.write_unchecked(key, &range, &values)
                    };
                    if !ok {
                        log.invoke(format!(
                            "{s} Failed because of failing memory update for [{start}, {end})."
                        ))
                        .await;
                    } else {
                        let mut hex_str = String::with_capacity(values.len() * 3 + 4);
                        hex_str += "[";
                        let mut first = true;
                        for v in values.iter() {
                            if !first {
                                hex_str += &format!(" {:04x}", *v);
                            } else {
                                hex_str += &format!("{:04x}", *v);
                            }
                            first = false;
                        }
                        hex_str += "]";
                        log.invoke(format!("{s} request to read [{start}, {end}) successful. Received values {hex_str}."))
                            .await;
                    }
                    *index = (*index + 1) % count;
                    *retries = 0;
                }
                (s, Err(ModbusError::Timeout(e))) => {
                    log.invoke(format!(
                            "{s} request to read [{start}, {end}) timed out. Disconnecting client. [{e:?}]"
                        )).await;
                    return Err(ModbusError::Timeout(e).into());
                }
                (s, Err(ModbusError::Error(e))) => {
                    log.invoke(format!(
                        "{s} request to read [{start}, {end}) failed. Disconnecting client. [{e:?}]"
                    ))
                    .await;
                    return Err(ModbusError::Error(e).into());
                }
                (s, Err(ModbusError::Exception(e))) => {
                    *retries += 1;
                    if *retries >= MAX_RETRIES {
                        log.invoke(format!(
                            "{s} request to read [{start}, {end}) invalid. [{e}]"
                        ))
                        .await;
                        *index = (*index + 1) % count;
                        *retries = 0;
                    }
                }
            }
        }
        Ok(())
    }

    /// Runs the read/command loop against the connected client until a graceful
    /// `Command::Terminate` (or the command channel closing) or a transport error. Returns
    /// whether at least one read succeeded during this run alongside the outcome, so the
    /// caller's reconnect backoff can reset after a connection that was live for a while rather
    /// than one that never got a read through.
    pub(crate) async fn run<T, L, St>(
        mut self,
        operations: Arc<RwLock<Vec<Operation>>>,
        memory: Arc<MemLock<Memory<Key<T>>>>,
        commands: &mut crate::common::Commands<'_, Command>,
        config: RunConfig<L, St>,
    ) -> (bool, Result<(), Error>)
    where
        T: KeyParams,
        L: LogFn,
        St: LogFn,
    {
        let RunConfig {
            log,
            status,
            timeout_ms,
            delay_ms,
            interval_ms,
        } = config;

        sleep(Duration::from_millis(delay_ms as u64)).await;

        // `interval_ms` of 0 means "as fast as possible"; tokio's interval requires a non-zero
        // period, and firing every 1ms is indistinguishable from that in practice.
        let mut ticker = tokio::time::interval(Duration::from_millis(interval_ms.max(1) as u64));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let mut index = 0;
        let mut retries = 0;
        let mut had_success = false;
        loop {
            tokio::select! {
                // Perform next read of registers
                _ = ticker.tick() => {
                    if let Err(e) = self
                        .poll_once(&operations, &memory, timeout_ms, &log, &mut index, &mut retries, &mut had_success)
                        .await
                    {
                        return (had_success, Err(e));
                    }
                }
                // Execute next command if available. `None` means every sender was dropped
                // (e.g. the owning instance was torn down without sending `Terminate`); treat
                // that the same as an explicit `Terminate`.
                cmd = commands.recv() => match cmd.unwrap_or(Command::Terminate) {
                    Command::Terminate => {
                        log.invoke("Client gracefully terminated.".to_string())
                            .await;
                        status.invoke("Client disconnected".to_string()).await;
                        return (had_success, Ok(()));
                    }
                    Command::WriteSingleCoil(slave, addr, coil) => {
                        let result = tokio::time::timeout(
                            Duration::from_millis(timeout_ms as u64),
                            self.client.write_single_coil(slave, addr, coil),
                        )
                        .await;
                        if let Err(e) = self
                            .handle_write_result(result, "WriteSingleCoil", &format!("{addr} with {coil}"), "invalid", &log)
                            .await
                        {
                            return (had_success, Err(e));
                        }
                    }
                    Command::WriteMultipleCoils(slave, addr, coils) => {
                        let result = tokio::time::timeout(
                            Duration::from_millis(timeout_ms as u64),
                            self.client.write_multiple_coils(slave, addr, &coils),
                        )
                        .await;
                        if let Err(e) = self
                            .handle_write_result(result, "WriteMultipleCoils", &format!("{addr} with {coils:?}"), "failed", &log)
                            .await
                        {
                            return (had_success, Err(e));
                        }
                    }
                    Command::WriteSingleRegister(slave, addr, value) => {
                        let result = tokio::time::timeout(
                            Duration::from_millis(timeout_ms as u64),
                            self.client.write_single_register(slave, addr, value),
                        )
                        .await;
                        if let Err(e) = self
                            .handle_write_result(result, "WriteSingleRegister", &format!("{addr} with {value}"), "invalid", &log)
                            .await
                        {
                            return (had_success, Err(e));
                        }
                    }
                    Command::WriteMultipleRegister(slave, addr, values) => {
                        let result = tokio::time::timeout(
                            Duration::from_millis(timeout_ms as u64),
                            self.client.write_multiple_registers(slave, addr, &values),
                        )
                        .await;
                        if let Err(e) = self
                            .handle_write_result(result, "WriteMultipleRegister", &format!("{addr} with {values:?}"), "invalid", &log)
                            .await
                        {
                            return (had_success, Err(e));
                        }
                    }
                }
            }
        }
    }

    /// Drives a transport's connect-retry-run loop: repeatedly calls `connect` to obtain a
    /// [`ConnectAttempt`], runs the resulting client via [`ClientCore::run`] until it exits, and
    /// on failure waits an exponential backoff (capped, reset after a run that got at least one
    /// read through) before retrying. `Command::Terminate` (or the command channel closing) ends
    /// the loop cleanly at any point; with `reconnect` unset for the current config snapshot, a
    /// transport error ends the loop instead of backing off. `connect` alone differs between the
    /// TCP and RTU transports (socket dial vs. serial open); everything else here is shared.
    ///
    /// Built on the shared [`ferrowl_util::backoff::run_with_backoff`] driver (MB-R-051): this
    /// function supplies the `attempt` (connect, then run) and `wait_abortable` (backoff wait,
    /// abortable by `Command::Terminate`/channel close) closures the driver calls.
    pub(crate) async fn run_reconnect_loop<T, L, St, C, Fut>(
        receiver: Receiver<Command>,
        log: L,
        status: St,
        operations: Arc<RwLock<Vec<Operation>>>,
        memory: Arc<MemLock<Memory<Key<T>>>>,
        connect: C,
        connected: crate::ConnectedCell,
    ) -> Result<(), Error>
    where
        T: KeyParams,
        L: LogFn + Clone,
        St: LogFn + Clone,
        C: FnMut() -> Fut,
        Fut: Future<Output = ConnectAttempt<S, F>>,
    {
        // Shared (not moved) between the two closures below, which the driver never calls
        // concurrently — `attempt` and `wait_abortable` always run one fully to completion
        // before the other starts, so lock contention can't happen. An async `Mutex` (rather
        // than `&mut` reborrowed per call) is required for both: an `FnMut` closure that returns
        // a future holding a per-call reborrow of a captured `&mut` upvar cannot compile (the
        // reborrow's lifetime is tied to that one call, but the returned future needs to outlive
        // it) — the driver calls `attempt`/`wait_abortable` repeatedly, so both need interior
        // mutability instead of a plain `&mut`. `tokio::sync::Mutex` rather than `std::cell::
        // RefCell`: the whole loop is spawned onto a multi-threaded runtime (`tokio::spawn`
        // requires `Send`), and a `RefCell` guard held across an `.await` is not `Send`.
        let receiver = Mutex::new(receiver);
        let connect = Mutex::new(connect);

        let attempt = || {
            let log = log.clone();
            let status = status.clone();
            let operations = operations.clone();
            let memory = memory.clone();
            let receiver = &receiver;
            let connect = &connect;
            let connected = connected.clone();
            async move {
                let mut guard = receiver.lock().await;
                let mut parked: std::collections::VecDeque<Command> =
                    std::collections::VecDeque::new();
                let conn_attempt = {
                    let mut connect_guard = connect.lock().await;
                    crate::common::race_terminate(
                        (*connect_guard)(),
                        &mut guard,
                        |cmd: &Command| matches!(cmd, Command::Terminate),
                        &mut parked,
                    )
                    .await
                };
                let conn_attempt = match conn_attempt {
                    Some(conn_attempt) => conn_attempt,
                    None => {
                        // MB-R-220 — terminate (or channel close) arrived while the connect
                        // attempt itself was in flight: the attempt is abandoned, never having
                        // reached a connected state.
                        drop(guard);
                        status.invoke("Client disconnected".to_string()).await;
                        return ferrowl_util::backoff::AttemptOutcome::Done;
                    }
                };
                let reconnect = conn_attempt.reconnect;
                let run_config = RunConfig {
                    log: log.clone(),
                    status: status.clone(),
                    timeout_ms: conn_attempt.timeout_ms,
                    delay_ms: conn_attempt.delay_ms,
                    interval_ms: conn_attempt.interval_ms,
                };

                let core = match conn_attempt.client {
                    Ok(core) => {
                        // MB-R-137 — the transport is now actually connected, not merely "a
                        // dial attempt was scheduled": flips the tri-state status to Connected.
                        connected.set(true);
                        core
                    }
                    Err(e) => {
                        // MB-E-093 — a command parked while this failed attempt was in flight is
                        // dropped here, with the same log line as a command arriving while
                        // backing off: there is no connection loop left to hand it to.
                        for _ in 0..parked.len() {
                            log.invoke(
                                "Command dropped: client is disconnected and reconnecting."
                                    .to_string(),
                            )
                            .await;
                        }
                        if !reconnect {
                            log.invoke(format!("{e} Reconnect disabled; client stopping."))
                                .await;
                            status.invoke("Client disconnected".to_string()).await;
                        } else {
                            log.invoke(format!("{e}")).await;
                        }
                        return ferrowl_util::backoff::AttemptOutcome::Failed {
                            error: e,
                            reconnect,
                            reset: false,
                        };
                    }
                };

                let mut commands = crate::common::Commands::new(&mut guard, parked);
                let (had_success, result) = core
                    .run::<T, _, _>(operations, memory, &mut commands, run_config)
                    .await;
                drop(guard);
                // MB-R-137 — the run() loop has ended (gracefully or not): no longer connected,
                // whether the next step is a retry (Reconnecting) or the task itself ending
                // (Disconnected, which the caller derives separately from task-alive).
                connected.set(false);
                match result {
                    Ok(()) => ferrowl_util::backoff::AttemptOutcome::Done,
                    Err(e) => {
                        if !reconnect {
                            // run() already logged the underlying disconnect; just surface the
                            // status change before the task ends.
                            status.invoke("Client disconnected".to_string()).await;
                        } else {
                            log.invoke(format!("{e}")).await;
                        }
                        ferrowl_util::backoff::AttemptOutcome::Failed {
                            error: e,
                            reconnect,
                            reset: had_success,
                        }
                    }
                }
            }
        };

        let wait_abortable = |backoff: Duration| {
            let log = log.clone();
            let status = status.clone();
            let receiver = &receiver;
            async move {
                log.invoke(format!("Reconnecting in {}s.", backoff.as_secs()))
                    .await;
                let mut guard = receiver.lock().await;
                // Any non-terminate command received while disconnected is dropped with a log
                // line rather than queued for after reconnect.
                let aborted = ferrowl_util::backoff::wait_backoff(
                    &mut guard,
                    backoff,
                    "Command dropped: client is disconnected and reconnecting.",
                    |cmd: &Command| matches!(cmd, Command::Terminate),
                    |msg| log.invoke(msg),
                )
                .await;
                drop(guard);
                if aborted {
                    status.invoke("Client disconnected".to_string()).await;
                }
                aborted
            }
        };

        ferrowl_util::backoff::run_with_backoff(
            ferrowl_util::backoff::BackoffPolicy::default(),
            attempt,
            wait_abortable,
        )
        .await
    }
}

// `poll_once`/`run`/`handle_write_result` all drive a real connected `Client` (over an actual
// TCP socket or serial port), so exercising them meaningfully needs a live transport; that
// end-to-end coverage lives in `tests/tcp_loopback.rs` (round-robin advance,
// retry-then-skip past `MAX_RETRIES`, every write outcome, reconnect/backoff, graceful
// termination). What's unit-testable in isolation here is the pure classification/conversion
// logic those methods build on.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConnectedCell, SlaveKey};
    use ferrowl_store::Range;
    use rust_modbus::{FrameTransport, Rtu};
    use tokio::io::{AsyncReadExt, DuplexStream};

    type ReadResult<V> = Result<Result<V, rust_modbus::Error>, tokio::time::error::Elapsed>;

    /// An RTU-framed client over an in-memory duplex link, plus the peer end of that link.
    /// Nothing ever answers on the peer end: the broadcast tests below are about what the
    /// client does *without* a response.
    fn rtu_client_over_duplex() -> (
        ClientCore<FrameTransport<DuplexStream, Rtu>, Rtu>,
        DuplexStream,
    ) {
        let (client_end, peer) = tokio::io::duplex(256);
        let core = ClientCore {
            client: Client::new(FrameTransport::<_, Rtu>::new(client_end)),
        };
        (core, peer)
    }

    /// A `LogFn` that records every line into a shared buffer for assertions.
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

    /// Reads whatever the peer end received within a short window, or nothing if the wire
    /// stayed silent.
    async fn drain_peer(peer: &mut DuplexStream) -> Vec<u8> {
        let mut buf = [0u8; 64];
        match tokio::time::timeout(Duration::from_millis(100), peer.read(&mut buf)).await {
            Ok(Ok(n)) => buf[..n].to_vec(),
            _ => Vec::new(),
        }
    }

    #[tokio::test]
    /// MB-R-101 — an RTU read addressed to slave id 0 fails locally, never reaching the wire,
    /// and fails as a Modbus exception so it follows the retry path of MB-R-043 rather than
    /// disconnecting the client.
    async fn ut_rtu_broadcast_read_is_refused_locally() {
        let (mut core, mut peer) = rtu_client_over_duplex();
        let (log, lines) = recording_log();
        let op = Operation {
            slave_id: UnitId(0),
            fn_code: FunctionCode::ReadHoldingRegisters,
            range: Range::new(0, 2),
        };

        let (label, result) = core.read(&op, 200, &log).await;

        assert_eq!(label, "Broadcast");
        assert!(matches!(
            result,
            Err(ModbusError::Exception(ExceptionCode::IllegalDataAddress))
        ));
        assert!(
            lines.lock().iter().any(|l| l.contains("broadcast address")),
            "the refusal is logged: {:?}",
            lines.lock()
        );
        assert!(
            drain_peer(&mut peer).await.is_empty(),
            "a broadcast read must not reach the wire"
        );
    }

    #[tokio::test]
    /// MB-R-149 — a read whose start address does not fit in `u16` (reachable only from a
    /// hand-edited config; the planner's `addr: u16` cannot produce one) is refused locally as
    /// `IllegalDataValue`, never reaching the wire, following the same exception-retry path as
    /// MB-R-043.
    async fn ut_rtu_start_address_overflow_is_refused_locally() {
        let (mut core, mut peer) = rtu_client_over_duplex();
        let (log, _lines) = recording_log();
        let op = Operation {
            slave_id: UnitId(1),
            fn_code: FunctionCode::ReadHoldingRegisters,
            range: Range::new(70_000, 1),
        };

        let (label, result) = core.read(&op, 200, &log).await;

        assert_eq!(label, "Unknown");
        assert!(matches!(
            result,
            Err(ModbusError::Exception(ExceptionCode::IllegalDataValue))
        ));
        assert!(
            drain_peer(&mut peer).await.is_empty(),
            "an out-of-range start address must not reach the wire"
        );
    }

    #[tokio::test]
    /// MB-R-149 — a read whose computed count (`end - start`) does not fit in `u16` is refused
    /// locally as `IllegalDataValue`, never reaching the wire, following the same exception-retry
    /// path as MB-R-043. (Pre-existing guard, now covered under MB-R-149.)
    async fn ut_rtu_range_count_overflow_is_refused_locally() {
        let (mut core, mut peer) = rtu_client_over_duplex();
        let (log, _lines) = recording_log();
        let op = Operation {
            slave_id: UnitId(1),
            fn_code: FunctionCode::ReadHoldingRegisters,
            range: Range::new(0, 70_000),
        };

        let (label, result) = core.read(&op, 200, &log).await;

        assert_eq!(label, "Unknown");
        assert!(matches!(
            result,
            Err(ModbusError::Exception(ExceptionCode::IllegalDataValue))
        ));
        assert!(
            drain_peer(&mut peer).await.is_empty(),
            "an over-long range must not reach the wire"
        );
    }

    #[tokio::test]
    /// MB-R-102 — an RTU write addressed to slave id 0 is transmitted without awaiting a
    /// response, logged as executed, and does not disconnect the client.
    async fn ut_rtu_broadcast_write_is_fire_and_forget() {
        let (core, mut peer) = rtu_client_over_duplex();
        let (log, lines) = recording_log();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Command>(4);
        tx.send(Command::WriteSingleRegister(
            UnitId(0),
            Address(1),
            RegisterValue(7),
        ))
        .await
        .unwrap();
        tx.send(Command::Terminate).await.unwrap();

        // No operations, so the poll ticker has nothing to do: the run is the two commands.
        let (_had_success, result) = core
            .run::<SlaveKey, _, _>(
                Arc::new(RwLock::new(Vec::new())),
                Arc::new(MemLock::new(Memory::<Key<SlaveKey>>::default())),
                &mut crate::common::Commands::new(&mut rx, std::collections::VecDeque::new()),
                RunConfig {
                    log,
                    status: |_s: String| async move {},
                    timeout_ms: 200,
                    delay_ms: 0,
                    interval_ms: 60_000,
                },
            )
            .await;

        // Nothing answered, yet the write neither timed out nor ended the run: the client
        // stayed up all the way to the graceful terminate.
        assert!(result.is_ok(), "a broadcast write must not disconnect");
        assert!(
            lines
                .lock()
                .iter()
                .any(|l| l.contains("WriteSingleRegister") && l.contains("successfully executed")),
            "the write is logged as executed: {:?}",
            lines.lock()
        );
        assert!(
            !drain_peer(&mut peer).await.is_empty(),
            "a broadcast write is still transmitted"
        );
    }

    #[test]
    /// MB-R-042 — coil/discrete-input reads map to one word per bit: `1` for set, `0` for clear.
    fn ut_bits_to_words_maps_true_false_to_one_zero() {
        assert_eq!(
            bits_to_words(vec![true, false, true, true]),
            vec![1u16, 0, 1, 1]
        );
    }

    #[test]
    /// MB-R-042 — an empty coil/discrete read maps to no words.
    fn ut_bits_to_words_empty_is_empty() {
        assert_eq!(bits_to_words(vec![]), Vec::<u16>::new());
    }

    #[test]
    fn ut_classify_success_unwraps_value() {
        let res: ReadResult<u16> = Ok(Ok(42));
        assert_eq!(classify(res).unwrap(), 42);
    }

    #[test]
    /// MB-R-043 — a read returning a Modbus exception is classified as an exception (retried, not a disconnect).
    fn ut_classify_exception_maps_to_modbus_exception() {
        let res: ReadResult<u16> = Ok(Err(rust_modbus::Error::Exception {
            function: FunctionCode::ReadHoldingRegisters,
            exception: ExceptionCode::IllegalDataAddress,
        }));
        let e = classify(res).unwrap_err();
        assert!(matches!(
            e,
            ModbusError::Exception(ExceptionCode::IllegalDataAddress)
        ));
    }

    #[test]
    /// MB-R-045 — a transport error is classified as an error (disconnects the client).
    fn ut_classify_transport_error_maps_to_modbus_error() {
        let res: ReadResult<u16> = Ok(Err(rust_modbus::Error::Io {
            kind: std::io::ErrorKind::ConnectionReset,
        }));
        let e = classify(res).unwrap_err();
        assert!(matches!(e, ModbusError::Error(_)));
    }

    #[tokio::test]
    /// MB-R-045 — a timed-out read is classified as a timeout (disconnects the client).
    async fn ut_classify_elapsed_maps_to_modbus_timeout() {
        // A zero-duration timeout against a never-resolving future always elapses immediately,
        // giving a real `Elapsed` without needing any transport.
        let elapsed = tokio::time::timeout(Duration::from_millis(0), std::future::pending::<()>())
            .await
            .unwrap_err();
        let res: ReadResult<u16> = Err(elapsed);
        let e = classify(res).unwrap_err();
        assert!(matches!(e, ModbusError::Timeout(_)));
    }

    #[tokio::test]
    /// MB-R-054 — a non-terminate command arriving while backing off is dropped (with a log line), not queued.
    async fn ut_backoff_drops_non_terminate_command() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Command>(4);
        let lines = Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
        let sink = lines.clone();
        let log = move |s: String| {
            let sink = sink.clone();
            async move {
                sink.lock().push(s);
            }
        };
        // A non-terminate command sent before the backoff elapses must be dropped, so the wait
        // still runs to completion and returns `false` (not aborted).
        tx.send(Command::WriteSingleRegister(
            UnitId(1),
            Address(0),
            RegisterValue(42),
        ))
        .await
        .unwrap();
        let aborted = ferrowl_util::backoff::wait_backoff(
            &mut rx,
            Duration::from_millis(50),
            "Command dropped: client is disconnected and reconnecting.",
            |cmd: &Command| matches!(cmd, Command::Terminate),
            log,
        )
        .await;
        assert!(!aborted);
        assert!(lines.lock().iter().any(|l| l.contains("Command dropped")));
    }

    #[tokio::test]
    /// MB-R-054 — Terminate (and the command channel closing) aborts the backoff wait immediately.
    async fn ut_backoff_aborts_on_terminate_or_close() {
        let sink = |_s: String| async move {};
        // Terminate aborts (returns true).
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Command>(4);
        tx.send(Command::Terminate).await.unwrap();
        assert!(
            ferrowl_util::backoff::wait_backoff(
                &mut rx,
                Duration::from_secs(30),
                "Command dropped: client is disconnected and reconnecting.",
                |cmd: &Command| matches!(cmd, Command::Terminate),
                &sink,
            )
            .await
        );
        // The channel closing aborts too.
        let (tx2, mut rx2) = tokio::sync::mpsc::channel::<Command>(4);
        drop(tx2);
        assert!(
            ferrowl_util::backoff::wait_backoff(
                &mut rx2,
                Duration::from_secs(30),
                "Command dropped: client is disconnected and reconnecting.",
                |cmd: &Command| matches!(cmd, Command::Terminate),
                sink,
            )
            .await
        );
    }

    #[tokio::test]
    /// MB-R-137 — the `ConnectedCell` threaded through `run_reconnect_loop` is `false` until a
    /// connect attempt succeeds, `true` while the resulting `run()` loop is active, and `false`
    /// again once that loop ends (here: a graceful `Terminate`).
    async fn ut_run_reconnect_loop_connected_cell_reflects_connect_run_disconnect_lifecycle() {
        let (client_end, _peer) = tokio::io::duplex(256);
        let core = ClientCore {
            client: Client::new(FrameTransport::<_, Rtu>::new(client_end)),
        };
        let (log, _log_lines) = recording_log();
        let (status, _status_lines) = recording_log();
        let (tx, rx) = tokio::sync::mpsc::channel::<Command>(4);
        let connected = ConnectedCell::default();

        let mut once = Some(core);
        let connect = move || {
            let core = once
                .take()
                .expect("connect is called exactly once in this test");
            async move {
                ConnectAttempt {
                    reconnect: true,
                    timeout_ms: 200,
                    delay_ms: 0,
                    interval_ms: 60_000,
                    client: Ok(core),
                }
            }
        };

        assert!(!connected.get(), "not connected before the loop starts");

        let connected_for_task = connected.clone();
        let handle = tokio::spawn(async move {
            ClientCore::run_reconnect_loop::<SlaveKey, _, _, _, _>(
                rx,
                log,
                status,
                Arc::new(RwLock::new(Vec::new())),
                Arc::new(MemLock::new(Memory::<Key<SlaveKey>>::default())),
                connect,
                connected_for_task,
            )
            .await
        });

        let mut waited = 0;
        while !connected.get() && waited < 100 {
            tokio::time::sleep(Duration::from_millis(5)).await;
            waited += 1;
        }
        assert!(connected.get(), "connected once the run() loop is active");

        tx.send(Command::Terminate).await.unwrap();
        let result = handle.await.unwrap();

        assert!(result.is_ok(), "graceful terminate ends the loop cleanly");
        assert!(!connected.get(), "not connected once the loop has ended");
    }

    #[tokio::test]
    /// MB-R-220 — a terminate arriving while the connect attempt itself is in flight aborts it
    /// immediately and ends the client task with success, without waiting for the attempt to
    /// resolve.
    async fn ut_terminate_during_connect_ends_loop_ok() {
        let (log, _log_lines) = recording_log();
        let (status, _status_lines) = recording_log();
        let (tx, rx) = tokio::sync::mpsc::channel::<Command>(4);
        let connected = ConnectedCell::default();

        let connect = move || {
            std::future::pending::<ConnectAttempt<FrameTransport<DuplexStream, Rtu>, Rtu>>()
        };

        tx.send(Command::Terminate).await.unwrap();

        let result = tokio::time::timeout(
            Duration::from_millis(250),
            ClientCore::run_reconnect_loop::<SlaveKey, _, _, _, _>(
                rx,
                log,
                status,
                Arc::new(RwLock::new(Vec::new())),
                Arc::new(MemLock::new(Memory::<Key<SlaveKey>>::default())),
                connect,
                connected,
            ),
        )
        .await
        .expect("terminate during connect must not hang");

        assert!(
            result.is_ok(),
            "abandoned connect attempt ends with success"
        );
    }

    #[tokio::test]
    /// MB-E-093 — a command sent while a connect attempt is still in flight is parked, not
    /// dropped, and is delivered to the connection loop once the attempt succeeds: a race that
    /// drops non-terminate commands during connect would silently discard this write instead of
    /// executing it.
    async fn ut_command_sent_during_connect_is_executed_after_connect() {
        let (core, mut peer) = rtu_client_over_duplex();
        let (log, _log_lines) = recording_log();
        let (status, _status_lines) = recording_log();
        let (tx, rx) = tokio::sync::mpsc::channel::<Command>(4);
        let connected = ConnectedCell::default();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();

        let mut once = Some(core);
        let mut ready_rx = Some(ready_rx);
        let connect = move || {
            let core = once
                .take()
                .expect("connect is called exactly once in this test");
            let ready_rx = ready_rx
                .take()
                .expect("connect is called exactly once in this test");
            async move {
                ready_rx.await.ok();
                ConnectAttempt {
                    reconnect: true,
                    timeout_ms: 200,
                    delay_ms: 0,
                    interval_ms: 60_000,
                    client: Ok(core),
                }
            }
        };

        let handle = tokio::spawn(async move {
            ClientCore::run_reconnect_loop::<SlaveKey, _, _, _, _>(
                rx,
                log,
                status,
                Arc::new(RwLock::new(Vec::new())),
                Arc::new(MemLock::new(Memory::<Key<SlaveKey>>::default())),
                connect,
                connected,
            )
            .await
        });

        // Sent while the connect attempt is still pending: must be parked, not dropped.
        tx.send(Command::WriteSingleRegister(
            UnitId(0),
            Address(1),
            RegisterValue(0x1234),
        ))
        .await
        .unwrap();

        ready_tx.send(()).unwrap();

        let wire = drain_peer(&mut peer).await;
        assert!(
            wire.len() >= 6,
            "the write parked during connect must reach the wire once connected: {wire:?}"
        );
        // RTU ADU: slave id, function code (6 = WriteSingleRegister), address (u16 BE), value
        // (u16 BE), then a 2-byte CRC this assertion does not need to recompute.
        assert_eq!(
            &wire[..6],
            &[0x00, 0x06, 0x00, 0x01, 0x12, 0x34],
            "the parked write's slave id/function/address/value must reach the wire unchanged"
        );

        tx.send(Command::Terminate).await.unwrap();
        let result = tokio::time::timeout(Duration::from_millis(500), handle)
            .await
            .expect("loop did not end promptly")
            .unwrap();
        assert!(result.is_ok());
    }

    #[tokio::test]
    /// MB-E-091 — documentation-level test: unlike `ut_terminate_during_connect_ends_loop_ok`'s
    /// pending, genuinely-abortable future, a synchronous open has no await point for a race to
    /// preempt, so it always runs to completion (here: an immediate failure against a
    /// nonexistent path) exactly once per attempt, and a pre-queued terminate takes effect only
    /// at the await surrounding it — never by cutting the open itself short. `calls == 1` proves
    /// no second attempt happened; if the terminate were somehow observed before the open ran,
    /// or if `reconnect` retried past it, this count would differ.
    async fn ut_terminate_around_serial_open_ends_loop_ok() {
        let (log, _log_lines) = recording_log();
        let (status, _status_lines) = recording_log();
        let (tx, rx) = tokio::sync::mpsc::channel::<Command>(4);
        let connected = ConnectedCell::default();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_for_connect = calls.clone();

        let connect = move || {
            calls_for_connect.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // Mirrors `open_serial`'s synchronous, no-await-point call against a path that does
            // not exist: it fails at once, inside the closure, before any `.await`.
            let open_result: Result<ClientCore<FrameTransport<DuplexStream, Rtu>, Rtu>, Error> =
                Err(Error::PathConflict {
                    path: "/dev/nonexistent".to_string(),
                    other: "other-module".to_string(),
                });
            async move {
                ConnectAttempt {
                    reconnect: true,
                    timeout_ms: 200,
                    delay_ms: 0,
                    interval_ms: 60_000,
                    client: open_result,
                }
            }
        };

        tx.send(Command::Terminate).await.unwrap();

        let result = tokio::time::timeout(
            Duration::from_millis(250),
            ClientCore::run_reconnect_loop::<SlaveKey, _, _, _, _>(
                rx,
                log,
                status,
                Arc::new(RwLock::new(Vec::new())),
                Arc::new(MemLock::new(Memory::<Key<SlaveKey>>::default())),
                connect,
                connected,
            ),
        )
        .await
        .expect("terminate around the serial open must not hang");

        assert!(
            result.is_ok(),
            "loop ends with success once terminate is honored"
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the open runs exactly once before the terminate is observed at the next await"
        );
    }

    #[test]
    /// MB-R-042 — a successful coil read is mapped through to one word per bit.
    fn ut_classify_maps_bits_through_to_words_on_success() {
        // Same classify() call the ReadCoils/ReadDiscreteInputs arms make, chained with
        // `.map(bits_to_words)` as `read()` does.
        let res: ReadResult<Vec<bool>> = Ok(Ok(vec![true, false]));
        let words = classify(res).map(bits_to_words).unwrap();
        assert_eq!(words, vec![1u16, 0]);
    }

    #[tokio::test]
    /// MB-R-157 — three consecutive Modbus exceptions on the same operation skip it (advance the
    /// round-robin index) and reset the retry counter; a fourth exception on the next rotation
    /// starts counting again rather than skipping immediately.
    async fn ut_three_consecutive_exceptions_skip_advance_and_reset_the_counter() {
        let (mut core, mut peer) = rtu_client_over_duplex();
        let (log, lines) = recording_log();
        let operations = Arc::new(RwLock::new(vec![
            Operation {
                slave_id: UnitId(0),
                fn_code: FunctionCode::ReadHoldingRegisters,
                range: Range::new(0, 2),
            },
            Operation {
                slave_id: UnitId(0),
                fn_code: FunctionCode::ReadHoldingRegisters,
                range: Range::new(10, 2),
            },
        ]));
        let memory = Arc::new(MemLock::new(Memory::<Key<SlaveKey>>::default()));
        let mut index = 0usize;
        let mut retries = 0u32;
        let mut had_success = false;

        let invalid_count = |lines: &Arc<parking_lot::Mutex<Vec<String>>>| -> usize {
            lines
                .lock()
                .iter()
                .filter(|l| l.contains("invalid."))
                .count()
        };

        core.poll_once::<SlaveKey, _>(
            &operations,
            &memory,
            200,
            &log,
            &mut index,
            &mut retries,
            &mut had_success,
        )
        .await
        .unwrap();
        core.poll_once::<SlaveKey, _>(
            &operations,
            &memory,
            200,
            &log,
            &mut index,
            &mut retries,
            &mut had_success,
        )
        .await
        .unwrap();
        assert_eq!(invalid_count(&lines), 0);
        assert_eq!(index, 0);
        assert_eq!(retries, 2);

        core.poll_once::<SlaveKey, _>(
            &operations,
            &memory,
            200,
            &log,
            &mut index,
            &mut retries,
            &mut had_success,
        )
        .await
        .unwrap();
        assert_eq!(invalid_count(&lines), 1);
        assert!(
            lines
                .lock()
                .iter()
                .any(|l| l.contains("invalid.") && l.contains("[0, 2)"))
        );
        assert_eq!(index, 1);
        assert_eq!(retries, 0);

        core.poll_once::<SlaveKey, _>(
            &operations,
            &memory,
            200,
            &log,
            &mut index,
            &mut retries,
            &mut had_success,
        )
        .await
        .unwrap();
        assert_eq!(invalid_count(&lines), 1);
        assert_eq!(index, 1);
        assert_eq!(retries, 1);

        core.poll_once::<SlaveKey, _>(
            &operations,
            &memory,
            200,
            &log,
            &mut index,
            &mut retries,
            &mut had_success,
        )
        .await
        .unwrap();
        core.poll_once::<SlaveKey, _>(
            &operations,
            &memory,
            200,
            &log,
            &mut index,
            &mut retries,
            &mut had_success,
        )
        .await
        .unwrap();
        assert_eq!(invalid_count(&lines), 2);
        assert!(
            lines
                .lock()
                .iter()
                .any(|l| l.contains("invalid.") && l.contains("[10, 12)"))
        );
        assert_eq!(index, 0);

        assert!(!had_success);
        assert!(
            drain_peer(&mut peer).await.is_empty(),
            "broadcast reads never reach the wire"
        );
    }
}
