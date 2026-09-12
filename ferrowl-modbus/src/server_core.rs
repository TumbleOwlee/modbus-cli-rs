//! Transport-agnostic Modbus server request handler shared by the TCP and RTU servers.

use crate::common::serial_config_from;
use crate::tcp::Config;
use crate::tcp::tls::build_server_tls_config;
use crate::{
    ConnectedCell, Error, Key, KeyParams, LogFn, PathConflictCell, SerialError, ServerCommand,
    TcpError,
};

use ferrowl_store::{CellType, Memory, Range};
use ferrowl_util::backoff::{AttemptOutcome, BackoffPolicy, run_with_backoff};
use parking_lot::RwLock;
use rust_modbus::{
    Connection, ExceptionCode, FunctionCode, Quantity, RegisterValue, RequestPdu, ResponsePdu,
    Server as ModbusServer, ServerFraming, Service, TcpListener, TlsListener, UnitId, open_serial,
};
use std::fmt::Display;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex as AsyncMutex;

/// Shared body of the four read function codes: log the request, read `[addr, addr+cnt)` for the
/// `(slave, fc)` key as `cell`, log the outcome when `verbose`, and return the raw words. The
/// `name` is the only thing that varies between the read arms (and appears verbatim in the logs).
#[allow(clippy::too_many_arguments)] // request context (name/slave/fc/cell/addr/cnt) + server state
async fn handle_read<T, L>(
    name: &str,
    slave: UnitId,
    fc: FunctionCode,
    cell: CellType,
    addr: u16,
    cnt: u16,
    memory: &Arc<RwLock<Memory<Key<T>>>>,
    log: &L,
    verbose: bool,
) -> Result<Vec<u16>, ExceptionCode>
where
    T: KeyParams,
    L: LogFn + Clone,
{
    log.invoke(format!(
        "{name} request received for slave ID {slave} and range [{}, {}).",
        addr,
        addr as usize + cnt as usize
    ))
    .await;
    let key = Key {
        id: T::from_slave_fn(slave, fc),
    };
    // Scoped so the (sync) guard is dropped before any log `.await` below.
    let result = {
        let guard = memory.read();
        guard.read(key, &cell, &Range::new(addr as usize, cnt as usize))
    };
    match result {
        Ok(v) => {
            if verbose {
                log.invoke(format!(
                    "{name} request for slave ID {slave} and range [{}, {}) successful.",
                    addr,
                    addr as usize + cnt as usize
                ))
                .await;
            }
            Ok(v)
        }
        Err(e) => {
            if verbose {
                log.invoke(format!(
                    "{name} request for slave ID {slave} and range [{}, {}) failed: {e}.",
                    addr,
                    addr as usize + cnt as usize
                ))
                .await;
            }
            Err(ExceptionCode::IllegalDataAddress)
        }
    }
}

/// Shared body of the two multi-write function codes (registers/coils): write `values` at `addr`
/// for the `(slave, fc)` key as `cell`, log the outcome, and return the count written (for the
/// response). Coil callers pass their bits already widened to `u16`.
#[allow(clippy::too_many_arguments)] // request context (name/slave/fc/cell/addr/values) + server state
async fn handle_write_multi<T, L>(
    name: &str,
    slave: UnitId,
    fc: FunctionCode,
    cell: CellType,
    addr: u16,
    values: &[u16],
    memory: &Arc<RwLock<Memory<Key<T>>>>,
    log: &L,
    verbose: bool,
) -> Result<u16, ExceptionCode>
where
    T: KeyParams,
    L: LogFn + Clone,
{
    log.invoke(format!(
        "{name} request received for slave ID {slave}, range [{}, {}), and values {values:?}.",
        addr,
        addr as usize + values.len()
    ))
    .await;
    let key = Key {
        id: T::from_slave_fn(slave, fc),
    };
    // Scoped so the (sync) guard is dropped before any log `.await` below.
    let result = {
        let mut guard = memory.write();
        guard.write(key, &cell, &Range::new(addr as usize, values.len()), values)
    };
    match result {
        Ok(()) => {
            if verbose {
                log.invoke(format!(
                    "{name} request for slave ID {slave}, range [{}, {}), and values {values:?} successful.",
                    addr,
                    addr as usize + values.len()
                ))
                .await;
            }
            Ok(values.len() as u16)
        }
        Err(e) => {
            if verbose {
                log.invoke(format!(
                    "{name} request for slave ID {slave}, range [{}, {}), and values {values:?} failed: {e}.",
                    addr,
                    addr as usize + values.len()
                ))
                .await;
            }
            Err(ExceptionCode::IllegalDataAddress)
        }
    }
}

/// Shared body of the two single-write function codes (register/coil): write `stored` at `addr`
/// for the `(slave, fc)` key as `cell`, logging `value` (the protocol-level value — a `u16` for a
/// register, a `bool` for a coil) so the log text matches the wire request.
#[allow(clippy::too_many_arguments)] // request context (name/slave/fc/cell/addr/value/stored) + server state
async fn handle_write_single<T, L, V>(
    name: &str,
    slave: UnitId,
    fc: FunctionCode,
    cell: CellType,
    addr: u16,
    value: V,
    stored: u16,
    memory: &Arc<RwLock<Memory<Key<T>>>>,
    log: &L,
    verbose: bool,
) -> Result<(), ExceptionCode>
where
    T: KeyParams,
    L: LogFn + Clone,
    V: Display,
{
    log.invoke(format!(
        "{name} request received for slave ID {slave}, address {addr}, and value {value}."
    ))
    .await;
    let key = Key {
        id: T::from_slave_fn(slave, fc),
    };
    // Scoped so the (sync) guard is dropped before any log `.await` below.
    let result = {
        let mut guard = memory.write();
        guard.write(key, &cell, &Range::new(addr as usize, 1), &[stored])
    };
    match result {
        Ok(()) => {
            if verbose {
                log.invoke(format!(
                    "{name} request for slave ID {slave}, address {addr}, and value {value} successful."
                ))
                .await;
            }
            Ok(())
        }
        Err(e) => {
            if verbose {
                log.invoke(format!(
                    "{name} request for slave ID {slave}, address {addr}, and value {value} failed: {e}."
                ))
                .await;
            }
            Err(ExceptionCode::IllegalDataAddress)
        }
    }
}

/// MB-R-128 — `true` when `slave` has no declared region in any register table: every key
/// `T::all_kinds_for(slave)` produces is absent from `memory`. A region declared for a
/// *different* table than a particular failing request's own still makes this `false`: a slave
/// with any declared region anywhere is a known slave, so the check needs no knowledge of which
/// `MemoryError` variant the failing request itself hit.
fn slave_has_no_region<T>(slave: UnitId, memory: &Arc<RwLock<Memory<Key<T>>>>) -> bool
where
    T: KeyParams,
{
    let guard = memory.read();
    T::all_kinds_for(slave)
        .into_iter()
        .all(|id| !guard.contains_key(&Key { id }))
}

/// The original body of [`handle_request`], answering unconditionally (i.e. before MB-R-128's
/// silence translation is applied). Kept as its own function, rather than inlined into
/// `handle_request`, so every arm's `?` short-circuits *this* function's `Result` — not
/// `handle_request`'s `Result<Option<ResponsePdu>, ExceptionCode>` — which would otherwise skip
/// the translation step entirely on the first early return.
async fn answer_request<T, L>(
    slave: UnitId,
    request: RequestPdu,
    memory: &Arc<RwLock<Memory<Key<T>>>>,
    log: &L,
    verbose: bool,
) -> Result<ResponsePdu, ExceptionCode>
where
    T: KeyParams,
    L: LogFn + Clone,
{
    match request {
        RequestPdu::ReadCoils { address, quantity } => {
            let v = handle_read(
                "ReadCoils",
                slave,
                FunctionCode::ReadCoils,
                CellType::Coil,
                address.0,
                quantity.0,
                memory,
                log,
                verbose,
            )
            .await?;
            Ok(ResponsePdu::ReadCoils {
                coils: v.into_iter().map(|b| b != 0).collect(),
            })
        }
        RequestPdu::ReadDiscreteInputs { address, quantity } => {
            let v = handle_read(
                "ReadDiscreteInputs",
                slave,
                FunctionCode::ReadDiscreteInputs,
                CellType::Coil,
                address.0,
                quantity.0,
                memory,
                log,
                verbose,
            )
            .await?;
            Ok(ResponsePdu::ReadDiscreteInputs {
                inputs: v.into_iter().map(|b| b != 0).collect(),
            })
        }
        RequestPdu::ReadInputRegisters { address, quantity } => {
            let v = handle_read(
                "ReadInputRegisters",
                slave,
                FunctionCode::ReadInputRegisters,
                CellType::Register,
                address.0,
                quantity.0,
                memory,
                log,
                verbose,
            )
            .await?;
            Ok(ResponsePdu::ReadInputRegisters {
                registers: v.into_iter().map(RegisterValue).collect(),
            })
        }
        RequestPdu::ReadHoldingRegisters { address, quantity } => {
            let v = handle_read(
                "ReadHoldingRegisters",
                slave,
                FunctionCode::ReadHoldingRegisters,
                CellType::Register,
                address.0,
                quantity.0,
                memory,
                log,
                verbose,
            )
            .await?;
            Ok(ResponsePdu::ReadHoldingRegisters {
                registers: v.into_iter().map(RegisterValue).collect(),
            })
        }
        RequestPdu::WriteMultipleRegisters { address, registers } => {
            let values: Vec<u16> = registers.iter().map(|v| v.0).collect();
            let len = handle_write_multi(
                "WriteMultipleRegisters",
                slave,
                FunctionCode::WriteMultipleRegisters,
                CellType::Register,
                address.0,
                &values,
                memory,
                log,
                verbose,
            )
            .await?;
            Ok(ResponsePdu::WriteMultipleRegisters {
                address,
                quantity: Quantity(len),
            })
        }
        RequestPdu::WriteSingleRegister { address, value } => {
            handle_write_single(
                "WriteSingleRegister",
                slave,
                FunctionCode::WriteSingleRegister,
                CellType::Register,
                address.0,
                value.0,
                value.0,
                memory,
                log,
                verbose,
            )
            .await?;
            Ok(ResponsePdu::WriteSingleRegister { address, value })
        }
        RequestPdu::WriteMultipleCoils { address, coils } => {
            let values: Vec<u16> = coils.iter().map(|v| *v as u16).collect();
            let len = handle_write_multi(
                "WriteMultipleCoils",
                slave,
                FunctionCode::WriteMultipleCoils,
                CellType::Coil,
                address.0,
                &values,
                memory,
                log,
                verbose,
            )
            .await?;
            Ok(ResponsePdu::WriteMultipleCoils {
                address,
                quantity: Quantity(len),
            })
        }
        RequestPdu::WriteSingleCoil { address, value } => {
            handle_write_single(
                "WriteSingleCoil",
                slave,
                FunctionCode::WriteSingleCoil,
                CellType::Coil,
                address.0,
                value,
                value as u16,
                memory,
                log,
                verbose,
            )
            .await?;
            Ok(ResponsePdu::WriteSingleCoil { address, value })
        }
        RequestPdu::ReadWriteMultipleRegisters {
            read_address,
            read_quantity,
            write_address,
            registers,
        } => {
            let (read_addr, cnt, write_addr) = (read_address.0, read_quantity.0, write_address.0);
            let values: Vec<u16> = registers.iter().map(|v| v.0).collect();
            log.invoke(format!(
                "ReadWriteMultipleRegisrters request received for slave ID {slave}, read address {read_addr}, count {cnt}, write address {write_addr}, and values {values:?}."
            ))
            .await;
            let key = Key {
                id: T::from_slave_fn(slave, FunctionCode::ReadWriteMultipleRegisters),
            };
            // The four checks/ops below must be atomic against concurrent requests, so they all
            // run under one scoped (sync) guard; it's dropped before any log `.await`.
            enum Outcome {
                NotAddressable(ferrowl_store::MemoryError),
                Rejected(ferrowl_store::MemoryError),
                Ok(Vec<u16>),
            }
            let outcome = {
                let mut guard = memory.write();
                match guard.readable(
                    &key,
                    &CellType::Register,
                    &Range::new(read_addr as usize, cnt as usize),
                ) {
                    Err(e) => Outcome::NotAddressable(e),
                    Ok(()) => match guard.writable(
                        &key,
                        &CellType::Register,
                        &Range::new(write_addr as usize, values.len()),
                    ) {
                        Err(e) => Outcome::NotAddressable(e),
                        Ok(()) => match guard.read(
                            key.clone(),
                            &CellType::Register,
                            &Range::new(read_addr as usize, cnt as usize),
                        ) {
                            Err(e) => Outcome::Rejected(e),
                            Ok(v) => match guard.write(
                                key,
                                &CellType::Register,
                                &Range::new(write_addr as usize, values.len()),
                                &values,
                            ) {
                                Err(e) => Outcome::Rejected(e),
                                Ok(()) => Outcome::Ok(v),
                            },
                        },
                    },
                }
            };
            match outcome {
                Outcome::NotAddressable(e) => {
                    if verbose {
                        log.invoke(format!(
                            "ReadWriteMultipleRegisrters request for slave ID {slave}, read address {read_addr}, count {cnt}, write address {write_addr}, and values {values:?} failed: {e}."
                        ))
                        .await;
                    }
                    Err(ExceptionCode::IllegalDataAddress)
                }
                Outcome::Rejected(e) => {
                    if verbose {
                        log.invoke(format!(
                            "ReadWriteMultipleRegisrters request for slave ID {slave}, read address {read_addr}, count {cnt}, write address {write_addr}, and values {values:?} failed: {e}."
                        ))
                        .await;
                    }
                    Err(ExceptionCode::IllegalDataAddress)
                }
                Outcome::Ok(v) => {
                    if verbose {
                        log.invoke(format!(
                            "ReadWriteMultipleRegisrters request for slave ID {slave}, read address {read_addr}, count {cnt}, write address {write_addr}, and values {values:?} successful."
                        ))
                        .await;
                    }
                    Ok(ResponsePdu::ReadWriteMultipleRegisters {
                        registers: v.into_iter().map(RegisterValue).collect(),
                    })
                }
            }
        }
        // MB-R-059: everything the server does not implement — report-server-id,
        // mask-write-register, diagnostics, comm-event, file record, FIFO queue,
        // read-device-identification (MEI), and any custom code — is one refusal, so a
        // function code the frame layer learns to decode later cannot silently acquire a
        // different answer here. MB-R-066 still requires the received line.
        other => {
            log.invoke(format!(
                "{} request received for slave ID {slave}. Unsupported function.",
                other.function(),
            ))
            .await;
            Err(ExceptionCode::IllegalFunction)
        }
    }
}

/// Handle one inbound Modbus server request against `memory`, shared by every server transport
/// (TCP, RTU, RTU-over-TCP).
///
/// Every arm logs a "request received" line. When `verbose` is set, each arm additionally logs
/// per-request success/failure; every production caller now passes `verbose = true` (MB-R-067) —
/// `verbose = false` remains reachable only for tests exercising the quiet path. `physical_serial`
/// gates MB-R-128: on a real `Rtu`/`Ascii` link, a request addressed to a slave id with no
/// declared region in any table is applied to the store and answered with silence (`Ok(None)`)
/// rather than the ordinary `IllegalDataAddress` exception (MB-R-057).
pub(crate) async fn handle_request<T, L>(
    slave: UnitId,
    request: RequestPdu,
    memory: &Arc<RwLock<Memory<Key<T>>>>,
    log: &L,
    verbose: bool,
    physical_serial: bool,
) -> Result<Option<ResponsePdu>, ExceptionCode>
where
    T: KeyParams,
    L: LogFn + Clone,
{
    let result = answer_request(slave, request, memory, log, verbose).await;
    match result {
        Ok(pdu) => Ok(Some(pdu)),
        // MB-R-128 — silence, not an exception, when the slave id itself is wholly unmapped on
        // a physical Rtu/Ascii link; an address merely outside a declared region for a mapped
        // slave still falls through to the ordinary exception below.
        Err(ExceptionCode::IllegalDataAddress)
            if physical_serial && slave_has_no_region(slave, memory) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Which event a server's [`Server`] treats as "the serve loop did something useful"
/// (MB-R-132): connection-oriented transports (TCP, `RtuOverTcp`, `AsciiOverTcp`) reset on
/// accepting a connection; datagram/single-link transports (RTU, `Ascii`, `Udp`) have no
/// separate "connect" step, so they reset on reading a request/datagram instead. Set via
/// [`Server::with_reset_on`] by each transport's own server module (MB-R-130–134's `spawn()`
/// wiring, not this shared core) — unset, a `Server` still runs (both branches are safe
/// no-ops against a `Server::new`-default `activity` nobody reads).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResetOn {
    Connect,
    Request,
}

/// Per-connection Modbus server service shared by every server transport (TCP, RTU,
/// RTU-over-TCP): every request is answered directly from the shared `memory` via
/// [`handle_request`]. `verbose` toggles the per-request success/failure logging — every
/// production caller sets it (MB-R-067). (The transport-specific bind/accept vs serial-open
/// setup stays in `tcp::server`/`rtu::server`/`rtu_over_tcp::server`.)
pub(crate) struct Server<T, L>
where
    T: KeyParams,
    L: LogFn + Clone,
{
    memory: Arc<RwLock<Memory<Key<T>>>>,
    log: L,
    verbose: bool,
    physical_serial: bool,
    /// MB-R-132's per-serve-loop "did something useful" flag. Defaults to a private flag
    /// nobody outside this `Server` reads — a caller that never opts in via
    /// [`with_reset_on`](Self::with_reset_on) gets `on_connect`/`on_request` that flip a flag
    /// harmlessly into the void, not a behavior change.
    activity: Arc<AtomicBool>,
    reset_on: ResetOn,
}

impl<T, L> Server<T, L>
where
    T: KeyParams,
    L: LogFn + Clone,
{
    pub(crate) fn new(
        memory: Arc<RwLock<Memory<Key<T>>>>,
        log: L,
        verbose: bool,
        physical_serial: bool,
    ) -> Self {
        Self {
            memory,
            log,
            verbose,
            physical_serial,
            activity: Arc::new(AtomicBool::new(false)),
            reset_on: ResetOn::Connect,
        }
    }

    /// Opts this server into MB-R-132's activity tracking: `activity` is the flag a caller's
    /// own reconnect loop reads back after the serve loop ends, and `reset_on` picks which
    /// event sets it (see [`ResetOn`]). Additive — a caller that never calls this keeps the
    /// `Server::new` default, which is inert (nothing outside this `Server` ever observes it).
    pub(crate) fn with_reset_on(mut self, activity: Arc<AtomicBool>, reset_on: ResetOn) -> Self {
        self.activity = activity;
        self.reset_on = reset_on;
        self
    }
}

impl<T, L> Service for Server<T, L>
where
    T: KeyParams,
    L: LogFn + Clone,
{
    // Taken by `&self` and awaited inside the connection's own task, so the handler suspends
    // normally on the store's lock and the log sink rather than blocking a worker thread. The
    // connection itself is not part of the answer: every request is served from the shared
    // store, whichever link carried it (MB-R-057).
    async fn on_request(
        &self,
        _conn: &Connection,
        unit: UnitId,
        request: RequestPdu,
    ) -> Result<Option<ResponsePdu>, ExceptionCode> {
        let result = handle_request(
            unit,
            request,
            &self.memory,
            &self.log,
            self.verbose,
            self.physical_serial,
        )
        .await;
        // MB-R-132 — "at least one request was ... read", not "answered successfully": a
        // refused request (an `Err(ExceptionCode)`) still counts, so this flips unconditionally
        // on the outcome, not just on `Ok`.
        if self.reset_on == ResetOn::Request {
            self.activity.store(true, Ordering::Relaxed);
        }
        result
    }

    // MB-R-132 (connection-oriented half) — called once per connection, before any request is
    // read (SV-R-032); always accepts (MB-R-057/MB-R-065 untouched, every connection is served).
    async fn on_connect(&self, _conn: &Connection) -> rust_modbus::Acceptance {
        if self.reset_on == ResetOn::Connect {
            self.activity.store(true, Ordering::Relaxed);
        }
        rust_modbus::Acceptance::Accept
    }

    // A framing or I/O failure on the wire never reaches `on_request`; this reports it (the
    // former `on_process_error` callback) whenever `verbose` is set — every production caller
    // now sets it (MB-R-067).
    async fn on_error(&self, _conn: &Connection, error: &rust_modbus::Error) {
        if self.verbose {
            self.log
                .invoke(format!("Server processing failed. [{error}]"))
                .await;
        }
    }

    // No `Connection` names the peer here — the handshake never got far enough to be
    // accepted (MB-R-111, server logging half). `Error::TlsHandshake.peer_cert` only
    // carries the raw offered DER, never a parsed subject, so the certificate's byte
    // length is the most specific identity this can report without adding an x509
    // parser dependency.
    async fn on_tls_handshake_failed(
        &self,
        peer: std::net::SocketAddr,
        error: &rust_modbus::Error,
    ) {
        let detail = match error {
            rust_modbus::Error::TlsHandshake {
                source,
                peer_cert: Some(cert),
            } => {
                format!(
                    "{source}; the client presented a certificate (fingerprint {}) that was rejected",
                    sha256_fingerprint(cert)
                )
            }
            rust_modbus::Error::TlsHandshake {
                source,
                peer_cert: None,
            } => source.to_string(),
            other => other.to_string(),
        };
        self.log
            .invoke(format!("TLS handshake with {peer} failed: {detail}."))
            .await;
    }
}

/// A SHA-256 fingerprint of a certificate's raw DER bytes, colon-separated hex
/// (the conventional certificate-fingerprint display) — the crate exposes only the
/// raw DER, no parsed subject, so a fingerprint is the strongest identity a log line
/// can carry without adding an x509 parser dependency. A byte length is not: two
/// distinct certificates of the same length are indistinguishable by length alone
/// (MB-R-111).
fn sha256_fingerprint(cert: &rustls_pki_types::CertificateDer<'_>) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, cert.as_ref());
    digest
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// The address a listener/socket transport (Tcp, RtuOverTcp, AsciiOverTcp, Udp) is actually
/// bound to right now — `None` until the first successful bind, cleared again once the serve
/// loop for that bind ends. Lets a caller observe the already-spec'd bind/retry behavior
/// (MB-R-130, MB-R-134) instead of racing it: `spawn()` returns before the first bind attempt
/// has necessarily run, so this is the only way to know the listener is actually up (mirrors
/// `ferrowl_ocpp::csms::Server::local_addr`, OC-R-083). RTU and Ascii (physical serial, no socket
/// address) have no equivalent: `open_serial` is a single synchronous local-path open with no
/// listen/accept step for a peer to race against, unlike a network bind a remote client can dial
/// before it lands.
pub(crate) type BoundAddr = std::sync::Arc<parking_lot::Mutex<Option<std::net::SocketAddr>>>;

/// How a driven serve loop ended (MB-R-130/MB-R-131/MB-R-133).
pub(crate) enum ServeEnd {
    /// A `ServerCommand::Terminate`, or the command channel closing, ended the loop gracefully
    /// (MB-R-133) — includes the ordinary "serving future returned `Ok(())`" case, since both
    /// paths mean "no retry is wanted."
    Terminated,
    /// The serve loop itself ended with a failure (a bind/open failure surfacing from a listener
    /// or serial link, MB-R-130/MB-R-131) with no command ever received.
    Failed(rust_modbus::Error),
}

/// Drives one `serve_fut` (whatever a transport's `serve`/`serve_framed`/`serve_tls`/`serve_link`
/// call returns) against a `ServerCommand` channel, racing the two: a `Terminate` (or the channel
/// closing) requests a graceful `handle.shutdown()` and waits for `serve_fut` to actually end
/// before returning [`ServeEnd::Terminated`] (MB-R-133); `serve_fut` ending on its own — `Ok(())`
/// (a graceful stop the caller didn't ask for — treated the same as `Terminated`, since either
/// way no retry is wanted) or `Err(e)` (MB-R-130/MB-R-131, [`ServeEnd::Failed`]) — returns
/// immediately without ever touching `handle`.
pub(crate) async fn drive_serve<Fut>(
    serve_fut: Fut,
    handle: rust_modbus::ServerHandle,
    commands: &mut tokio::sync::mpsc::Receiver<crate::ServerCommand>,
) -> ServeEnd
where
    Fut: std::future::Future<Output = rust_modbus::Result<()>>,
{
    tokio::pin!(serve_fut);
    loop {
        tokio::select! {
            result = &mut serve_fut => {
                return match result {
                    Ok(()) => ServeEnd::Terminated,
                    Err(e) => ServeEnd::Failed(e),
                };
            }
            cmd = commands.recv() => {
                if matches!(cmd, None | Some(crate::ServerCommand::Terminate)) {
                    let _ = tokio::join!(handle.shutdown(), &mut serve_fut);
                    return ServeEnd::Terminated;
                }
            }
        }
    }
}

/// Waits out a bind/open backoff, aborting early on `ServerCommand::Terminate` or the command
/// channel closing. Deliberately simpler than [`ferrowl_util::backoff::wait_backoff`]:
/// `ServerCommand` has only one variant, so any command received aborts the wait and there is no
/// "drop a non-terminate command" case to log.
pub(crate) async fn wait_reconnect_backoff(
    receiver: &mut tokio::sync::mpsc::Receiver<crate::ServerCommand>,
    backoff: std::time::Duration,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(backoff) => false,
        _ = receiver.recv() => true,
    }
}

/// Drive the bind/serve retry loop shared by every TCP-family server transport (Tcp,
/// RtuOverTcp, Ascii-over-TCP — all three ride a raw [`TcpListener`]/[`TlsListener`] and share
/// `crate::tcp::Config`; only the [`ServerFraming`] `F` they hand to `serve_framed`/`serve_tls`
/// differs). Binds the configured TCP address and serves with `F`'s framing, retrying a bind or
/// mid-serve failure per [`BackoffPolicy`] when `config.reconnect` is set (MB-R-071/MB-R-114/
/// MB-R-126 revised, MB-R-130–134). Each accepted connection answers from the shared `memory`
/// via a [`Server`] with the caller's `verbose`/`physical_serial` flags. Plain TCP unless
/// `config.tls` is set (MB-R-104/MB-R-115/MB-R-127), in which case the listener terminates TLS
/// on each accepted connection. A TLS configuration error (MB-R-107/MB-R-108) always ends the
/// task immediately regardless of `reconnect` — retrying an invalid configuration can never
/// succeed.
#[allow(clippy::too_many_arguments)] // config/memory/receiver/log/status/bound_addr + verbose/physical_serial flags
pub(crate) async fn run_tcp_family<T, F, L, St>(
    config: Arc<tokio::sync::RwLock<Config>>,
    memory: Arc<RwLock<Memory<Key<T>>>>,
    receiver: tokio::sync::mpsc::Receiver<ServerCommand>,
    log: L,
    status: St,
    bound_addr: BoundAddr,
    verbose: bool,
    physical_serial: bool,
    cache: crate::tcp::tls::SelfSignedCache,
) -> Result<(), Error>
where
    T: KeyParams,
    F: ServerFraming + Send + 'static,
    F::Header: Send + Sync,
    L: LogFn + Clone,
    St: LogFn + Clone,
{
    // Shared between `attempt` and `wait_abortable`, called strictly sequentially by
    // `run_with_backoff` and never concurrently — same technique as
    // `client_core::run_reconnect_loop`.
    let receiver = AsyncMutex::new(receiver);
    let activity = Arc::new(AtomicBool::new(false));

    let attempt = || {
        let config = config.clone();
        let memory = memory.clone();
        let log = log.clone();
        let activity = activity.clone();
        let receiver = &receiver;
        let bound_addr = bound_addr.clone();
        let cache = cache.clone();
        async move {
            activity.store(false, Ordering::Relaxed);
            let guard = config.read().await;
            let reconnect = guard.reconnect;
            let addr: SocketAddr = match format!("{}:{}", guard.ip, guard.port).parse() {
                Ok(addr) => addr,
                Err(e) => {
                    return AttemptOutcome::Failed {
                        error: Error::Tcp(TcpError::Address(e)),
                        // A malformed address never fixes itself by retrying; matches the
                        // pre-retry behavior, which failed this unconditionally too.
                        reconnect: false,
                        reset: false,
                    };
                }
            };
            let server = ModbusServer::new(
                Server::new(memory.clone(), log.clone(), verbose, physical_serial)
                    .with_reset_on(activity.clone(), ResetOn::Connect),
            );
            let policy = guard.server_tls_policy();
            match &policy {
                ferrowl_util::tls::ServerTlsPolicy::None {} => {
                    drop(guard);
                    let mut receiver = receiver.lock().await;
                    // `ServerCommand` has only one variant, so any command received aborts the
                    // race and there is no "park a non-terminate command" case here.
                    let mut parked: std::collections::VecDeque<ServerCommand> =
                        std::collections::VecDeque::new();
                    let bind_result = crate::common::race_terminate(
                        TcpListener::bind(addr),
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
                            error: Error::Server(e),
                            reconnect,
                            reset: false,
                        },
                        Some(Ok(listener)) => {
                            let bound = match listener.local_addr() {
                                Ok(addr) => addr,
                                Err(e) => {
                                    return AttemptOutcome::Failed {
                                        error: Error::Server(e),
                                        reconnect,
                                        reset: false,
                                    };
                                }
                            };
                            *bound_addr.lock() = Some(bound);
                            let handle = server.handle();
                            let end = drive_serve(
                                server.serve_framed::<F>(listener),
                                handle,
                                &mut receiver,
                            )
                            .await;
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
                _ => {
                    let build_result = build_server_tls_config(&policy, &guard.ip, &cache);
                    drop(guard);
                    match build_result {
                        Err(e) => AttemptOutcome::Failed {
                            error: Error::Tcp(e),
                            // A TLS configuration error never fixes itself by retrying
                            // (MB-R-107/MB-R-108) — always ends the task, regardless of
                            // `reconnect`.
                            reconnect: false,
                            reset: false,
                        },
                        // `None {}` was already handled above, so this arm only ever runs
                        // for `Tls`/`Mutual`, both of which always build `Some((..))`.
                        Ok(None) => unreachable!(
                            "build_server_tls_config returns None only for None (mode), already handled"
                        ),
                        Ok(Some((tls_config, used_fallback))) => {
                            if used_fallback {
                                log.invoke(
                                    "No cert_file/key_file/self_signed configured for this TLS \
                                     server; falling back to an ephemeral self-signed certificate."
                                        .to_string(),
                                )
                                .await;
                            }
                            let mut receiver = receiver.lock().await;
                            let mut parked: std::collections::VecDeque<ServerCommand> =
                                std::collections::VecDeque::new();
                            let bind_result = crate::common::race_terminate(
                                TlsListener::bind(addr, tls_config),
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
                                    error: Error::Server(e),
                                    reconnect,
                                    reset: false,
                                },
                                Some(Ok(listener)) => {
                                    let bound = match listener.local_addr() {
                                        Ok(addr) => addr,
                                        Err(e) => {
                                            return AttemptOutcome::Failed {
                                                error: Error::Server(e),
                                                reconnect,
                                                reset: false,
                                            };
                                        }
                                    };
                                    *bound_addr.lock() = Some(bound);
                                    let handle = server.handle();
                                    let end = drive_serve(
                                        server.serve_tls::<F>(listener),
                                        handle,
                                        &mut receiver,
                                    )
                                    .await;
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

/// Drive the open/serve retry loop shared by every serial-family server transport (Rtu, Ascii —
/// both ride an OS serial port opened via `open_serial::<F>` and share `crate::rtu::Config`;
/// only the framing `F` they hand to `serve_link` differs). Retries an open or mid-serve failure
/// per [`BackoffPolicy`] when `config.reconnect` is set (MB-R-071/MB-R-075/MB-R-124 revised,
/// MB-R-130–134). Uses `ResetOn::Request`, not `Connect`: a serial link's own `on_connect` fires
/// once immediately, before any request is read, so it cannot be the "did something useful"
/// signal — only reading a request/datagram counts. Before every OS-level open, `path_conflict`
/// is checked against the freshly `~`-expanded path (MB-R-150); a conflict is a `Failed` outcome
/// carrying the ordinary reconnect setting (so it retries on the usual backoff and recovers once
/// the conflict clears) and is logged before returning, since a `reconnect: true` server would
/// otherwise retry it forever with no observable trace at all.
#[allow(clippy::too_many_arguments)] // config/memory/receiver/log/status/path_conflict/open + verbose/physical_serial flags
pub(crate) async fn run_serial_family<T, F, L, St>(
    config: Arc<tokio::sync::RwLock<crate::rtu::Config>>,
    memory: Arc<RwLock<Memory<Key<T>>>>,
    receiver: tokio::sync::mpsc::Receiver<ServerCommand>,
    log: L,
    status: St,
    path_conflict: PathConflictCell,
    open: ConnectedCell,
    verbose: bool,
    physical_serial: bool,
) -> Result<(), Error>
where
    T: KeyParams,
    F: ServerFraming + Send + 'static,
    F::Header: Send + Sync,
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
        let path_conflict = path_conflict.clone();
        let open = open.clone();
        async move {
            activity.store(false, Ordering::Relaxed);
            let guard = config.read().await;
            let reconnect = guard.reconnect;
            let serial = match serial_config_from(
                guard.baud_rate,
                guard.data_bits,
                guard.stop_bits,
                guard.parity.as_deref(),
            ) {
                Ok(serial) => serial,
                Err(e) => {
                    return AttemptOutcome::Failed {
                        error: e.into(),
                        reconnect: false, // a bad serial-config value never fixes itself
                        reset: false,
                    };
                }
            };
            let path = guard.path.clone();
            drop(guard);
            let expanded = ferrowl_util::path::expand(&path);
            let expanded = expanded.to_string_lossy().into_owned();
            if let Some(other) = path_conflict.check(&expanded) {
                log.invoke(format!(
                    "Serial path '{expanded}' is already in use by module '{other}' in this \
                     session; skipping open."
                ))
                .await;
                return AttemptOutcome::Failed {
                    error: Error::PathConflict {
                        path: expanded,
                        other,
                    },
                    reconnect,
                    reset: false,
                };
            }
            let mut receiver = receiver.lock().await;
            let mut parked: std::collections::VecDeque<ServerCommand> =
                std::collections::VecDeque::new();
            // MB-E-091 — `open_serial` is synchronous with no await point, so this race cannot
            // preempt it mid-call; a terminate is honored at the first surrounding await instead.
            let open_result = crate::common::race_terminate(
                async { open_serial::<F>(&path, serial) },
                &mut receiver,
                |_: &ServerCommand| true,
                &mut parked,
            )
            .await;
            debug_assert!(
                parked.is_empty(),
                "ServerCommand has only one variant; nothing is ever parked"
            );
            match open_result {
                None => AttemptOutcome::Done,
                Some(Err(e)) => AttemptOutcome::Failed {
                    error: SerialError::Error(e).into(),
                    reconnect,
                    reset: false,
                },
                Some(Ok(transport)) => {
                    open.set(true);
                    let server = ModbusServer::new(
                        Server::new(memory.clone(), log.clone(), verbose, physical_serial)
                            .with_reset_on(activity.clone(), ResetOn::Request),
                    );
                    let handle = server.handle();
                    let end =
                        drive_serve(server.serve_link(transport), handle, &mut receiver).await;
                    open.set(false);
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
    use super::*;
    use crate::ServerCommand;
    use crate::SlaveKey;
    use ferrowl_codec::Kind as RegKind;
    use ferrowl_store::CellKind;
    use rust_modbus::{
        Address, Ascii, Client as RmClient, DiagnosticSubFunction, FileNumber, FileRecordRead,
        FileRecordWrite, FrameTransport, Mask, MeiRequest, ReadDeviceIdCode, RecordLength,
        RecordNumber, Rtu, Server as ModbusServer, ServerHandle, Tcp,
    };
    use std::sync::Mutex;
    use std::time::Duration;

    /// Build a memory map for slave `1`, holding registers `[0,4)`, seeded with `seed` at addr 0,
    /// wrapped in the `Arc<RwLock<_>>` that `handle_request` expects.
    fn seeded_memory(seed: &[u16]) -> Arc<RwLock<Memory<Key<SlaveKey>>>> {
        let key = Key {
            id: SlaveKey {
                slave_id: UnitId(1),
                kind: RegKind::HoldingRegister,
            },
        };
        let mut mem = Memory::<Key<SlaveKey>>::default();
        mem.add_ranges(
            key.clone(),
            &CellKind::read_write(CellType::Register),
            &[Range::new(0, 4)],
        );
        if !seed.is_empty() {
            mem.write(key, &CellType::Register, &Range::new(0, seed.len()), seed)
                .unwrap();
        }
        Arc::new(RwLock::new(mem))
    }

    /// A `LogFn` that records every line into a shared buffer for assertions.
    fn recording_log() -> (impl LogFn + Clone, Arc<Mutex<Vec<String>>>) {
        let buf = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = buf.clone();
        let log = move |s: String| {
            let sink = sink.clone();
            async move {
                sink.lock().unwrap().push(s);
            }
        };
        (log, buf)
    }

    #[tokio::test]
    /// MB-R-178 (server logging half) — a TLS handshake failure logs the peer address
    /// and the underlying failure, including the rejected client certificate's size
    /// when one was offered (the crate only exposes the raw DER, no parsed subject).
    async fn ut_on_tls_handshake_failed_logs_peer_and_error() {
        let mem = seeded_memory(&[]);
        let (log, buf) = recording_log();
        let server = Server::new(mem, log, true, false);

        let peer: std::net::SocketAddr = "127.0.0.1:5502".parse().unwrap();
        let error = rust_modbus::Error::TlsHandshake {
            source: tokio_rustls::rustls::Error::General("bad certificate".to_string()),
            peer_cert: Some(rustls_pki_types::CertificateDer::from(vec![1, 2, 3, 4])),
        };
        server.on_tls_handshake_failed(peer, &error).await;

        let lines = buf.lock().unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("127.0.0.1:5502"), "line: {}", lines[0]);
        assert!(lines[0].contains("bad certificate"), "line: {}", lines[0]);
        // SHA-256 of [1, 2, 3, 4], colon-separated hex (computed from the SHA-256
        // standard, not from this crate's own output).
        assert!(
            lines[0].contains(
                "9f:64:a7:47:e1:b9:7f:13:1f:ab:b6:b4:47:29:6c:9b:6f:02:01:e7:9f:b3:c5:35:6e:6c:77:e8:9b:6a:80:6a"
            ),
            "line: {}",
            lines[0]
        );
    }

    // The current-thread flavor is the point of this test: `memory` is a synchronous
    // `parking_lot` lock and `on_request` an ordinary async fn, so the request path must need no
    // worker thread to bridge blocking work onto.
    #[tokio::test]
    /// MB-R-057 — the server answers an inbound request directly from the shared store.
    async fn ut_server_call_works_on_current_thread_runtime() {
        let mem = seeded_memory(&[10, 20]);
        let (log, _) = recording_log();
        let server = Server::new(mem, log, true, false);

        // Served over an in-memory duplex link, which is the same code path a socket takes:
        // the accept loop is the only thing a real listener adds.
        let (server_end, client_end) = tokio::io::duplex(256);
        let modbus = ModbusServer::new(server);
        let handle = modbus.handle();
        let serving = tokio::spawn(modbus.serve_link(FrameTransport::<_, Tcp>::new(server_end)));

        let mut client: RmClient<_, Tcp> = RmClient::new(FrameTransport::new(client_end));
        let registers = client
            .read_holding_registers(UnitId(1), Address(0), Quantity(2))
            .await
            .unwrap();
        assert_eq!(registers, vec![RegisterValue(10), RegisterValue(20)]);

        handle.shutdown().await;
        let _ = serving.await;
    }

    #[tokio::test]
    /// MB-R-132 — with `ResetOn::Connect`, the activity flag flips as soon as a connection is
    /// accepted, before any request is read; a request alone (`ResetOn::Request` not selected)
    /// does not move it further, and it starts `false`.
    async fn ut_server_on_connect_sets_activity_when_reset_on_connect() {
        let mem = seeded_memory(&[10, 20]);
        let (log, _) = recording_log();
        let activity = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server =
            Server::new(mem, log, true, false).with_reset_on(activity.clone(), ResetOn::Connect);

        let (server_end, client_end) = tokio::io::duplex(256);
        let modbus = ModbusServer::new(server);
        let handle = modbus.handle();
        let serving = tokio::spawn(modbus.serve_link(FrameTransport::<_, Tcp>::new(server_end)));

        // Hold the client end open (this is what "a connection was accepted" means for a
        // single already-established link) without ever sending a request.
        let _client_end = client_end;
        assert!(
            !activity.load(std::sync::atomic::Ordering::Relaxed),
            "must start false"
        );
        for _ in 0..50 {
            if activity.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            activity.load(std::sync::atomic::Ordering::Relaxed),
            "on_connect must set activity for ResetOn::Connect"
        );

        handle.shutdown().await;
        let _ = serving.await;
    }

    #[tokio::test]
    /// MB-R-132 — with `ResetOn::Request`, the activity flag stays false until a request is
    /// actually read (accepting the connection alone does not move it), and flips regardless of
    /// whether the request was answered or refused.
    async fn ut_server_on_request_sets_activity_when_reset_on_request() {
        // Slave 1 has a declared region, slave 9 does not: the request below targets 9, an
        // unmapped slave, which is refused with `IllegalDataAddress` (not physical-serial here,
        // so it is a real exception, not silence) — proving the flag flips on a refused request
        // too, not only a successfully-answered one.
        let mem = seeded_memory(&[10, 20]);
        let (log, _) = recording_log();
        let activity = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server =
            Server::new(mem, log, true, false).with_reset_on(activity.clone(), ResetOn::Request);

        let (server_end, client_end) = tokio::io::duplex(256);
        let modbus = ModbusServer::new(server);
        let handle = modbus.handle();
        let serving = tokio::spawn(modbus.serve_link(FrameTransport::<_, Tcp>::new(server_end)));

        // Connection accepted, no request sent yet: must still be false.
        let mut client: RmClient<_, Tcp> = RmClient::new(FrameTransport::new(client_end));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !activity.load(std::sync::atomic::Ordering::Relaxed),
            "on_connect alone must not set activity for ResetOn::Request"
        );

        let _ = client
            .read_holding_registers(UnitId(9), Address(0), Quantity(1))
            .await;

        for _ in 0..50 {
            if activity.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            activity.load(std::sync::atomic::Ordering::Relaxed),
            "a refused request must still set activity for ResetOn::Request"
        );

        handle.shutdown().await;
        let _ = serving.await;
    }

    #[tokio::test]
    /// MB-R-103 — an RTU server applies a request addressed to slave id 0 to the store exactly as
    /// it would any other, and emits no response frame for it.
    async fn ut_rtu_broadcast_request_is_applied_and_unanswered() {
        // Both the broadcast address and slave 1 have declared regions: the broadcast write
        // lands in slave 0's, and the follow-up read of slave 1 is what proves no stray
        // broadcast response was left sitting in the stream ahead of it.
        let mut mem = Memory::<Key<SlaveKey>>::default();
        for slave in [UnitId(0), UnitId(1)] {
            let key = Key {
                id: SlaveKey {
                    slave_id: slave,
                    kind: RegKind::HoldingRegister,
                },
            };
            mem.add_ranges(
                key.clone(),
                &CellKind::read_write(CellType::Register),
                &[Range::new(0, 4)],
            );
            mem.write(key, &CellType::Register, &Range::new(0, 2), &[10, 20])
                .unwrap();
        }
        let mem = Arc::new(RwLock::new(mem));
        let (log, _) = recording_log();
        let server = Server::new(mem.clone(), log, true, true);

        let (server_end, client_end) = tokio::io::duplex(256);
        let modbus = ModbusServer::new(server);
        let handle = modbus.handle();
        let serving = tokio::spawn(modbus.serve_link(FrameTransport::<_, Rtu>::new(server_end)));

        let mut client: RmClient<_, Rtu> = RmClient::new(FrameTransport::new(client_end));
        // Returns as soon as the frame is written — nothing is awaited, because nothing answers.
        client
            .write_single_register(UnitId(0), Address(1), RegisterValue(0x1234))
            .await
            .unwrap();

        // The store took the write all the same.
        let key = Key {
            id: SlaveKey {
                slave_id: UnitId(0),
                kind: RegKind::HoldingRegister,
            },
        };
        let mut applied = Vec::new();
        for _ in 0..50 {
            applied = mem
                .read()
                .read(key.clone(), &CellType::Register, &Range::new(0, 2))
                .unwrap();
            if applied == vec![10, 0x1234] {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(applied, vec![10, 0x1234]);

        // The next exchange lines up, so the broadcast put no frame on the wire.
        let registers = client
            .read_holding_registers(UnitId(1), Address(0), Quantity(2))
            .await
            .unwrap();
        assert_eq!(registers, vec![RegisterValue(10), RegisterValue(20)]);

        handle.shutdown().await;
        let _ = serving.await;
    }

    #[tokio::test]
    /// MB-R-057 — a holding-register read is answered from the values stored in the shared store.
    async fn ut_handle_read_holding_returns_seeded_values() {
        let mem = seeded_memory(&[10, 20]);
        let (log, _) = recording_log();
        let resp = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadHoldingRegisters {
                address: Address(0),
                quantity: Quantity(2),
            },
            &mem,
            &log,
            true,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        assert!(
            matches!(resp, ResponsePdu::ReadHoldingRegisters { registers: v } if v == vec![RegisterValue(10), RegisterValue(20)])
        );
    }

    #[tokio::test]
    /// MB-R-060 — a read against a slave with no declared regions is answered with `IllegalDataAddress`.
    async fn ut_handle_read_unknown_slave_is_illegal_data_address() {
        let mem = seeded_memory(&[10, 20]);
        let (log, _) = recording_log();
        // Slave 2 has no registered ranges, so the lookup fails.
        let err = handle_request::<SlaveKey, _>(
            UnitId(2),
            RequestPdu::ReadHoldingRegisters {
                address: Address(0),
                quantity: Quantity(2),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
    }

    #[tokio::test]
    /// MB-R-058 — a write-single-register request is answered and its value persisted in the store.
    async fn ut_handle_write_single_register_persists() {
        let mem = seeded_memory(&[]);
        let (log, _) = recording_log();
        handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::WriteSingleRegister {
                address: Address(1),
                value: RegisterValue(99),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        let resp = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadHoldingRegisters {
                address: Address(1),
                quantity: Quantity(1),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        assert!(
            matches!(resp, ResponsePdu::ReadHoldingRegisters { registers: v } if v == vec![RegisterValue(99)])
        );
    }

    #[tokio::test]
    /// `verbose` gates outcome logging directly: on, the per-request success/failure is logged
    /// alongside the "received" line; off, only "received" is logged. Every production caller
    /// now passes `verbose = true` (MB-R-067) — this exercises both branches of the mechanism.
    async fn ut_handle_verbose_logs_outcome_quiet_when_off() {
        let mem = seeded_memory(&[1, 2]);

        // verbose = true: a "received" line plus a "successful" line.
        let (log, buf) = recording_log();
        handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadHoldingRegisters {
                address: Address(0),
                quantity: Quantity(2),
            },
            &mem,
            &log,
            true,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        let verbose = buf.lock().unwrap().clone();
        assert_eq!(verbose.len(), 2);
        assert!(verbose[0].contains("received"));
        assert!(verbose[1].contains("successful"));

        // verbose = false: only the "received" line.
        let (log, buf) = recording_log();
        handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadHoldingRegisters {
                address: Address(0),
                quantity: Quantity(2),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        let quiet = buf.lock().unwrap().clone();
        assert_eq!(quiet.len(), 1);
        assert!(quiet[0].contains("received"));
    }

    /// Build a memory for slave `1` of the given register `kind`/value `ty` over `[0, len)`,
    /// optionally seeded with `seed` at addr 0. ReadWrite so both read and write paths work.
    fn seeded(
        kind: RegKind,
        ty: CellType,
        len: usize,
        seed: &[u16],
    ) -> Arc<RwLock<Memory<Key<SlaveKey>>>> {
        let key = Key {
            id: SlaveKey {
                slave_id: UnitId(1),
                kind,
            },
        };
        let mut mem = Memory::<Key<SlaveKey>>::default();
        mem.add_ranges(
            key.clone(),
            &CellKind::read_write(ty),
            &[Range::new(0, len)],
        );
        if !seed.is_empty() {
            mem.write(key, &ty, &Range::new(0, seed.len()), seed)
                .unwrap();
        }
        Arc::new(RwLock::new(mem))
    }

    // WriteMultipleCoils: the written range's length comes from the request.

    #[tokio::test]
    /// MB-R-062 — a multi-coil write is answered with the address written and the number of values written.
    async fn ut_write_multiple_coils_persists_every_bit() {
        let mem = seeded(RegKind::Coil, CellType::Coil, 8, &[]);
        let (log, _) = recording_log();
        let coils = vec![true, false, true, true, false];

        let resp = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::WriteMultipleCoils {
                address: Address(1),
                coils: coils.clone(),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        // The write range length must equal values.len(), not 1: `Memory::write` rejects a range
        // whose length disagrees with the value count, which would reject every multi-coil write.
        assert!(matches!(
            resp,
            ResponsePdu::WriteMultipleCoils {
                address: Address(1),
                quantity: Quantity(5)
            }
        ));

        let read = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadCoils {
                address: Address(1),
                quantity: Quantity(5),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        assert!(matches!(read, ResponsePdu::ReadCoils { coils: v } if v == coils));
    }

    #[tokio::test]
    /// MB-R-060 — a multi-coil write overrunning the declared region is answered with `IllegalDataAddress`.
    async fn ut_write_multiple_coils_out_of_range_is_illegal_data_address() {
        let mem = seeded(RegKind::Coil, CellType::Coil, 8, &[]);
        let (log, _) = recording_log();
        // addr 6 + 5 coils overruns the registered [0, 8) region.
        let err = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::WriteMultipleCoils {
                address: Address(6),
                coils: vec![true; 5],
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
    }

    #[tokio::test]
    /// MB-R-061 — a coil read reports each stored word as set when it is non-zero.
    async fn ut_read_coils_returns_seeded_bits() {
        let mem = seeded(RegKind::Coil, CellType::Coil, 4, &[1, 0, 1, 0]);
        let (log, _) = recording_log();
        let resp = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadCoils {
                address: Address(0),
                quantity: Quantity(4),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        assert!(
            matches!(resp, ResponsePdu::ReadCoils { coils: v } if v == vec![true, false, true, false])
        );
    }

    #[tokio::test]
    /// MB-R-060 — a coil read outside the declared region is answered with `IllegalDataAddress`.
    async fn ut_read_coils_out_of_range_is_illegal_data_address() {
        let mem = seeded(RegKind::Coil, CellType::Coil, 4, &[]);
        let (log, _) = recording_log();
        let err = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadCoils {
                address: Address(10),
                quantity: Quantity(2),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
    }

    #[tokio::test]
    /// MB-R-058 — the server answers read-discrete-inputs from the stored bits.
    async fn ut_read_discrete_inputs_returns_seeded_bits() {
        let mem = seeded(RegKind::DiscreteInput, CellType::Coil, 3, &[0, 1, 1]);
        let (log, _) = recording_log();
        let resp = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadDiscreteInputs {
                address: Address(0),
                quantity: Quantity(3),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        assert!(
            matches!(resp, ResponsePdu::ReadDiscreteInputs { inputs: v } if v == vec![false, true, true])
        );
    }

    #[tokio::test]
    /// MB-R-060 — a discrete-input read against an undeclared slave is answered with `IllegalDataAddress`.
    async fn ut_read_discrete_inputs_unknown_slave_is_illegal_data_address() {
        let mem = seeded(RegKind::DiscreteInput, CellType::Coil, 3, &[]);
        let (log, _) = recording_log();
        let err = handle_request::<SlaveKey, _>(
            UnitId(2),
            RequestPdu::ReadDiscreteInputs {
                address: Address(0),
                quantity: Quantity(3),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
    }

    #[tokio::test]
    /// MB-R-057 — an input-register read is answered from the values stored in the shared store.
    async fn ut_read_input_registers_returns_seeded_values() {
        let mem = seeded(RegKind::InputRegister, CellType::Register, 3, &[7, 8, 9]);
        let (log, _) = recording_log();
        let resp = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadInputRegisters {
                address: Address(0),
                quantity: Quantity(3),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        assert!(
            matches!(resp, ResponsePdu::ReadInputRegisters { registers: v } if v == vec![RegisterValue(7), RegisterValue(8), RegisterValue(9)])
        );
    }

    #[tokio::test]
    /// MB-R-060 — an input-register read outside the declared region is answered with `IllegalDataAddress`.
    async fn ut_read_input_registers_out_of_range_is_illegal_data_address() {
        let mem = seeded(RegKind::InputRegister, CellType::Register, 3, &[]);
        let (log, _) = recording_log();
        let err = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadInputRegisters {
                address: Address(2),
                quantity: Quantity(5),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
    }

    #[tokio::test]
    /// MB-R-060 — a holding-register read outside the declared region is answered with `IllegalDataAddress`.
    async fn ut_read_holding_registers_out_of_range_is_illegal_data_address() {
        let mem = seeded(RegKind::HoldingRegister, CellType::Register, 4, &[]);
        let (log, _) = recording_log();
        let err = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadHoldingRegisters {
                address: Address(3),
                quantity: Quantity(4),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
    }

    #[tokio::test]
    /// MB-R-061 — a coil write stores a set coil, observable on read-back.
    async fn ut_write_single_coil_persists() {
        let mem = seeded(RegKind::Coil, CellType::Coil, 4, &[]);
        let (log, _) = recording_log();
        let resp = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::WriteSingleCoil {
                address: Address(2),
                value: true,
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        assert!(matches!(
            resp,
            ResponsePdu::WriteSingleCoil {
                address: Address(2),
                value: true
            }
        ));
        let read = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadCoils {
                address: Address(2),
                quantity: Quantity(1),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        assert!(matches!(read, ResponsePdu::ReadCoils { coils: v } if v == vec![true]));
    }

    #[tokio::test]
    /// MB-R-060 — a single-coil write outside the declared region is answered with `IllegalDataAddress`.
    async fn ut_write_single_coil_out_of_range_is_illegal_data_address() {
        let mem = seeded(RegKind::Coil, CellType::Coil, 4, &[]);
        let (log, _) = recording_log();
        let err = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::WriteSingleCoil {
                address: Address(9),
                value: true,
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
    }

    #[tokio::test]
    /// MB-R-060 — a single-register write outside the declared region is answered with `IllegalDataAddress`.
    async fn ut_write_single_register_out_of_range_is_illegal_data_address() {
        let mem = seeded(RegKind::HoldingRegister, CellType::Register, 4, &[]);
        let (log, _) = recording_log();
        let err = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::WriteSingleRegister {
                address: Address(99),
                value: RegisterValue(1),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
    }

    #[tokio::test]
    /// MB-R-062 — a multi-register write is answered with the address written and the number of values written.
    async fn ut_write_multiple_registers_persists_all() {
        let mem = seeded(RegKind::HoldingRegister, CellType::Register, 8, &[]);
        let (log, _) = recording_log();
        let resp = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::WriteMultipleRegisters {
                address: Address(1),
                registers: vec![11, 22, 33].into_iter().map(RegisterValue).collect(),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        assert!(matches!(
            resp,
            ResponsePdu::WriteMultipleRegisters {
                address: Address(1),
                quantity: Quantity(3)
            }
        ));
        let read = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadHoldingRegisters {
                address: Address(1),
                quantity: Quantity(3),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        assert!(
            matches!(read, ResponsePdu::ReadHoldingRegisters { registers: v } if v == vec![RegisterValue(11), RegisterValue(22), RegisterValue(33)])
        );
    }

    #[tokio::test]
    /// MB-R-060 — a multi-register write outside the declared region is answered with `IllegalDataAddress`.
    async fn ut_write_multiple_registers_out_of_range_is_illegal_data_address() {
        let mem = seeded(RegKind::HoldingRegister, CellType::Register, 4, &[]);
        let (log, _) = recording_log();
        let err = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::WriteMultipleRegisters {
                address: Address(3),
                registers: vec![1, 2, 3].into_iter().map(RegisterValue).collect(),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
    }

    // ReadWriteMultipleRegisters reads and writes the same holding region.

    #[tokio::test]
    /// MB-R-063 — a read/write-multiple request applies the write and returns the values read before it.
    async fn ut_read_write_multiple_registers_writes_then_returns_read() {
        let mem = seeded(
            RegKind::HoldingRegister,
            CellType::Register,
            8,
            &[5, 6, 7, 8],
        );
        let (log, _) = recording_log();
        // Read [0,2), write [2,4) = [77, 88].
        let resp = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadWriteMultipleRegisters {
                read_address: Address(0),
                read_quantity: Quantity(2),
                write_address: Address(2),
                registers: vec![77, 88].into_iter().map(RegisterValue).collect(),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        assert!(
            matches!(resp, ResponsePdu::ReadWriteMultipleRegisters { registers: v } if v == vec![RegisterValue(5), RegisterValue(6)])
        );
        let read = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadHoldingRegisters {
                address: Address(2),
                quantity: Quantity(2),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        assert!(
            matches!(read, ResponsePdu::ReadHoldingRegisters { registers: v } if v == vec![RegisterValue(77), RegisterValue(88)])
        );
    }

    #[tokio::test]
    /// MB-R-064 — a read/write-multiple whose write range is not writable is answered `IllegalDataAddress` and applies no write.
    async fn ut_read_write_multiple_registers_out_of_range_is_illegal_data_address() {
        let mem = seeded(
            RegKind::HoldingRegister,
            CellType::Register,
            4,
            &[1, 2, 3, 4],
        );
        let (log, _) = recording_log();
        let err = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadWriteMultipleRegisters {
                read_address: Address(0),
                read_quantity: Quantity(2),
                write_address: Address(10),
                registers: vec![1, 2].into_iter().map(RegisterValue).collect(),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
    }

    #[tokio::test]
    /// MB-R-065 — the server answers any slave id for which regions are declared; it does not filter by a configured slave id.
    async fn ut_server_serves_any_declared_slave() {
        // Regions declared for two different slave ids; the server has no "configured" slave.
        let mut mem = Memory::<Key<SlaveKey>>::default();
        for &slave in &[3u8, 9u8] {
            let key = Key {
                id: SlaveKey {
                    slave_id: UnitId(slave),
                    kind: RegKind::HoldingRegister,
                },
            };
            mem.add_ranges(
                key.clone(),
                &CellKind::read_write(CellType::Register),
                &[Range::new(0, 2)],
            );
            mem.write(
                key,
                &CellType::Register,
                &Range::new(0, 2),
                &[slave as u16, 0],
            )
            .unwrap();
        }
        let mem = Arc::new(RwLock::new(mem));
        let (log, _) = recording_log();

        // Both slaves are answered from their own declared regions.
        for &slave in &[3u8, 9u8] {
            let resp = handle_request::<SlaveKey, _>(
                UnitId(slave),
                RequestPdu::ReadHoldingRegisters {
                    address: Address(0),
                    quantity: Quantity(2),
                },
                &mem,
                &log,
                false,
                false,
            )
            .await
            .unwrap()
            .expect("request answered");
            assert!(
                matches!(resp, ResponsePdu::ReadHoldingRegisters { registers: v } if v[0] == RegisterValue(slave as u16))
            );
        }
    }

    #[tokio::test]
    /// MB-R-066 — a "request received" line is logged for every inbound request, including a rejected function code.
    async fn ut_logs_request_received_including_rejected() {
        let mem = seeded(
            RegKind::HoldingRegister,
            CellType::Register,
            4,
            &[10, 20, 30, 40],
        );

        // A supported request logs "request received".
        let (log, buf) = recording_log();
        handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadHoldingRegisters {
                address: Address(0),
                quantity: Quantity(2),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap()
        .expect("request answered");
        assert!(
            buf.lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("request received"))
        );

        // A rejected function code still logs "request received" before the IllegalFunction reply.
        let (log, buf) = recording_log();
        let err = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReportServerId,
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalFunction);
        assert!(
            buf.lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("request received"))
        );
    }

    #[tokio::test]
    /// MB-R-059 — report-server-id is rejected with `IllegalFunction`.
    async fn ut_report_server_id_is_illegal_function() {
        let mem = seeded(RegKind::HoldingRegister, CellType::Register, 4, &[]);
        let (log, _) = recording_log();
        let err = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReportServerId,
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalFunction);
    }

    // Verbose logging: every arm's success/failure log branch.

    #[tokio::test]
    /// MB-R-067 — a TCP (verbose) server logs a success outcome for every supported request.
    async fn ut_verbose_logs_success_for_every_request() {
        macro_rules! ok {
            ($mem:expr, $req:expr) => {{
                let mem = $mem;
                let (log, buf) = recording_log();
                handle_request::<SlaveKey, _>(UnitId(1), $req, &mem, &log, true, false)
                    .await
                    .unwrap()
                    .expect("request answered");
                assert!(
                    buf.lock().unwrap().iter().any(|l| l.contains("successful")),
                    "missing success log line"
                );
            }};
        }
        ok!(
            seeded(RegKind::Coil, CellType::Coil, 8, &[1, 0, 1, 0, 1, 0, 1, 0]),
            RequestPdu::ReadCoils {
                address: Address(0),
                quantity: Quantity(4)
            }
        );
        ok!(
            seeded(RegKind::Coil, CellType::Coil, 8, &[]),
            RequestPdu::WriteSingleCoil {
                address: Address(0),
                value: true
            }
        );
        ok!(
            seeded(RegKind::Coil, CellType::Coil, 8, &[]),
            RequestPdu::WriteMultipleCoils {
                address: Address(0),
                coils: vec![true, false, true]
            }
        );
        ok!(
            seeded(RegKind::DiscreteInput, CellType::Coil, 4, &[1, 1, 1, 1]),
            RequestPdu::ReadDiscreteInputs {
                address: Address(0),
                quantity: Quantity(4)
            }
        );
        ok!(
            seeded(RegKind::InputRegister, CellType::Register, 4, &[1, 2, 3, 4]),
            RequestPdu::ReadInputRegisters {
                address: Address(0),
                quantity: Quantity(4)
            }
        );
        ok!(
            seeded(RegKind::HoldingRegister, CellType::Register, 8, &[]),
            RequestPdu::WriteSingleRegister {
                address: Address(0),
                value: RegisterValue(9)
            }
        );
        ok!(
            seeded(RegKind::HoldingRegister, CellType::Register, 8, &[]),
            RequestPdu::WriteMultipleRegisters {
                address: Address(0),
                registers: vec![1, 2, 3].into_iter().map(RegisterValue).collect()
            }
        );
        ok!(
            seeded(
                RegKind::HoldingRegister,
                CellType::Register,
                8,
                &[5, 6, 7, 8]
            ),
            RequestPdu::ReadWriteMultipleRegisters {
                read_address: Address(0),
                read_quantity: Quantity(2),
                write_address: Address(2),
                registers: vec![7, 8].into_iter().map(RegisterValue).collect()
            }
        );
    }

    #[tokio::test]
    /// MB-R-067 — a TCP (verbose) server logs a failure outcome for every rejected request.
    async fn ut_verbose_logs_failure_for_every_request() {
        macro_rules! fail {
            ($mem:expr, $req:expr) => {{
                let mem = $mem;
                let (log, buf) = recording_log();
                let _ =
                    handle_request::<SlaveKey, _>(UnitId(1), $req, &mem, &log, true, false).await;
                assert!(
                    buf.lock().unwrap().iter().any(|l| l.contains("failed")),
                    "missing failure log line"
                );
            }};
        }
        fail!(
            seeded(RegKind::Coil, CellType::Coil, 4, &[]),
            RequestPdu::ReadCoils {
                address: Address(10),
                quantity: Quantity(2)
            }
        );
        fail!(
            seeded(RegKind::Coil, CellType::Coil, 4, &[]),
            RequestPdu::WriteSingleCoil {
                address: Address(9),
                value: true
            }
        );
        fail!(
            seeded(RegKind::Coil, CellType::Coil, 4, &[]),
            RequestPdu::WriteMultipleCoils {
                address: Address(6),
                coils: vec![true; 5]
            }
        );
        fail!(
            seeded(RegKind::DiscreteInput, CellType::Coil, 4, &[]),
            RequestPdu::ReadDiscreteInputs {
                address: Address(10),
                quantity: Quantity(2)
            }
        );
        fail!(
            seeded(RegKind::InputRegister, CellType::Register, 4, &[]),
            RequestPdu::ReadInputRegisters {
                address: Address(10),
                quantity: Quantity(2)
            }
        );
        fail!(
            seeded(RegKind::HoldingRegister, CellType::Register, 4, &[]),
            RequestPdu::ReadHoldingRegisters {
                address: Address(10),
                quantity: Quantity(2)
            }
        );
        fail!(
            seeded(RegKind::HoldingRegister, CellType::Register, 4, &[]),
            RequestPdu::WriteSingleRegister {
                address: Address(99),
                value: RegisterValue(1)
            }
        );
        fail!(
            seeded(RegKind::HoldingRegister, CellType::Register, 4, &[]),
            RequestPdu::WriteMultipleRegisters {
                address: Address(3),
                registers: vec![1, 2, 3].into_iter().map(RegisterValue).collect()
            }
        );
        // Write address out of range -> writable check fails (verbose failure branch).
        fail!(
            seeded(
                RegKind::HoldingRegister,
                CellType::Register,
                4,
                &[1, 2, 3, 4]
            ),
            RequestPdu::ReadWriteMultipleRegisters {
                read_address: Address(0),
                read_quantity: Quantity(2),
                write_address: Address(10),
                registers: vec![1, 2].into_iter().map(RegisterValue).collect()
            }
        );
    }

    #[tokio::test]
    /// MB-R-059 — every function code outside the nine the server implements is rejected with
    /// `IllegalFunction`, whatever the frame layer is able to decode.
    async fn ut_unsupported_function_codes_are_illegal() {
        let mem = seeded(RegKind::HoldingRegister, CellType::Register, 4, &[]);
        let (log, _) = recording_log();
        for req in [
            RequestPdu::MaskWriteRegister {
                address: Address(0),
                and_mask: Mask(0),
                or_mask: Mask(0),
            },
            RequestPdu::EncapsulatedInterfaceTransport(MeiRequest::ReadDeviceIdentification {
                read_device_id_code: ReadDeviceIdCode::Basic,
                object_id: 0,
            }),
            RequestPdu::ReportServerId,
            RequestPdu::ReadExceptionStatus,
            RequestPdu::GetCommEventCounter,
            RequestPdu::GetCommEventLog,
            RequestPdu::Diagnostics {
                sub_function: DiagnosticSubFunction::ReturnQueryData,
                data: vec![0x1234],
            },
            RequestPdu::ReadFileRecord {
                records: vec![FileRecordRead {
                    file_number: FileNumber(1),
                    record_number: RecordNumber(0),
                    record_length: RecordLength(1),
                }],
            },
            RequestPdu::WriteFileRecord {
                records: vec![FileRecordWrite {
                    file_number: FileNumber(1),
                    record_number: RecordNumber(0),
                    values: vec![RegisterValue(1)],
                }],
            },
            RequestPdu::ReadFifoQueue {
                address: Address(0),
            },
            RequestPdu::Custom {
                code: 0x65,
                data: vec![],
            },
        ] {
            let err = handle_request::<SlaveKey, _>(UnitId(1), req, &mem, &log, false, false)
                .await
                .unwrap_err();
            assert_eq!(err, ExceptionCode::IllegalFunction);
        }
    }

    #[test]
    /// MB-R-128 — a slave id is "wholly unmapped" only when every register table is unregistered
    /// for it; a region in any one table (even a different one than the request under test)
    /// disqualifies it.
    fn ut_slave_has_no_region_true_when_wholly_unmapped() {
        let mem: Arc<RwLock<Memory<Key<SlaveKey>>>> = Arc::new(RwLock::new(Memory::default()));
        assert!(slave_has_no_region(UnitId(9), &mem));
    }

    #[test]
    /// MB-R-128 — a region declared in one table (Coil) still counts as "at least one region is
    /// declared" for the slave id as a whole, even though this test's own interest is elsewhere.
    fn ut_slave_has_no_region_false_when_any_table_is_mapped() {
        let mut mem = Memory::<Key<SlaveKey>>::default();
        mem.add_ranges(
            Key {
                id: SlaveKey {
                    slave_id: UnitId(9),
                    kind: RegKind::Coil,
                },
            },
            &CellKind::read_write(CellType::Coil),
            &[Range::new(0, 4)],
        );
        let mem = Arc::new(RwLock::new(mem));
        assert!(!slave_has_no_region(UnitId(9), &mem));
    }

    #[tokio::test]
    /// MB-R-128 — a physical-serial server withholds the response for a wholly-unmapped slave id.
    async fn ut_handle_request_silent_when_wholly_unmapped_and_physical_serial() {
        let mem = seeded_memory(&[10, 20]); // slave 1 only
        let (log, _) = recording_log();
        let resp = handle_request::<SlaveKey, _>(
            UnitId(9),
            RequestPdu::ReadHoldingRegisters {
                address: Address(0),
                quantity: Quantity(2),
            },
            &mem,
            &log,
            false,
            true, // physical_serial
        )
        .await
        .unwrap();
        assert_eq!(resp, None);
    }

    #[tokio::test]
    /// MB-R-128 — the same wholly-unmapped slave id gets the ordinary exception, not silence, on
    /// every non-`Rtu`/`Ascii` transport (edge-cases.md row 1).
    async fn ut_handle_request_exception_when_wholly_unmapped_and_not_physical_serial() {
        let mem = seeded_memory(&[10, 20]);
        let (log, _) = recording_log();
        let err = handle_request::<SlaveKey, _>(
            UnitId(9),
            RequestPdu::ReadHoldingRegisters {
                address: Address(0),
                quantity: Quantity(2),
            },
            &mem,
            &log,
            false,
            false, // physical_serial
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
    }

    #[tokio::test]
    /// MB-R-128 — a slave id with a region declared in a different table still gets the ordinary
    /// exception on a physical-serial server; only a *wholly* unmapped slave id is silenced.
    async fn ut_handle_request_exception_when_region_declared_in_other_table_even_physical_serial()
    {
        let mut mem = Memory::<Key<SlaveKey>>::default();
        mem.add_ranges(
            Key {
                id: SlaveKey {
                    slave_id: UnitId(9),
                    kind: RegKind::Coil,
                },
            },
            &CellKind::read_write(CellType::Coil),
            &[Range::new(0, 4)],
        );
        let mem = Arc::new(RwLock::new(mem));
        let (log, _) = recording_log();
        // Request targets HoldingRegister — unmapped for slave 9 — but slave 9 has a Coil region.
        let err = handle_request::<SlaveKey, _>(
            UnitId(9),
            RequestPdu::ReadHoldingRegisters {
                address: Address(0),
                quantity: Quantity(2),
            },
            &mem,
            &log,
            false,
            true, // physical_serial
        )
        .await
        .unwrap_err();
        assert_eq!(err, ExceptionCode::IllegalDataAddress);
    }

    #[tokio::test]
    /// MB-R-128 — end-to-end over a real `Rtu` frame: a request to an unmapped, non-broadcast
    /// slave id is applied to the store and answered with silence. Unlike MB-R-103's broadcast
    /// case, the client here has no way to know in advance that nothing will answer, so (unlike
    /// `ut_rtu_broadcast_request_is_applied_and_unanswered`) it genuinely waits out its response
    /// timeout — that timeout, not an early return, is what proves no frame came back.
    async fn ut_rtu_unmapped_slave_request_is_applied_and_unanswered() {
        let mut mem = Memory::<Key<SlaveKey>>::default();
        let key1 = Key {
            id: SlaveKey {
                slave_id: UnitId(1),
                kind: RegKind::HoldingRegister,
            },
        };
        mem.add_ranges(
            key1.clone(),
            &CellKind::read_write(CellType::Register),
            &[Range::new(0, 4)],
        );
        mem.write(key1, &CellType::Register, &Range::new(0, 2), &[10, 20])
            .unwrap();
        let mem = Arc::new(RwLock::new(mem));
        let (log, _) = recording_log();
        let server = Server::new(mem.clone(), log, true, true); // physical_serial

        let (server_end, client_end) = tokio::io::duplex(256);
        let modbus = ModbusServer::new(server);
        let handle = modbus.handle();
        let serving = tokio::spawn(modbus.serve_link(FrameTransport::<_, Rtu>::new(server_end)));

        let mut client: RmClient<_, Rtu> = RmClient::with_config(
            FrameTransport::new(client_end),
            rust_modbus::ClientConfig {
                response_timeout: std::time::Duration::from_millis(50),
            },
        );
        // Slave 5 has no declared region at all — the client waits out its response timeout
        // because nothing answers (the timeout itself is the proof no response frame arrived;
        // MB-R-090 desynchronizes the client afterward, so a follow-up request over the same
        // link is not a meaningful further check).
        let err = client
            .write_single_register(UnitId(5), Address(1), RegisterValue(0x1234))
            .await
            .unwrap_err();
        assert!(matches!(err, rust_modbus::Error::Timeout { .. }));

        handle.shutdown().await;
        let _ = serving.await;
    }

    #[tokio::test]
    /// MB-R-128 — identical to the `Rtu` case above but over `Ascii` framing (the requirement
    /// names both transports explicitly).
    async fn ut_ascii_unmapped_slave_request_is_applied_and_unanswered() {
        let mut mem = Memory::<Key<SlaveKey>>::default();
        let key1 = Key {
            id: SlaveKey {
                slave_id: UnitId(1),
                kind: RegKind::HoldingRegister,
            },
        };
        mem.add_ranges(
            key1.clone(),
            &CellKind::read_write(CellType::Register),
            &[Range::new(0, 4)],
        );
        mem.write(key1, &CellType::Register, &Range::new(0, 2), &[10, 20])
            .unwrap();
        let mem = Arc::new(RwLock::new(mem));
        let (log, _) = recording_log();
        let server = Server::new(mem.clone(), log, true, true);

        let (server_end, client_end) = tokio::io::duplex(256);
        let modbus = ModbusServer::new(server);
        let handle = modbus.handle();
        let serving = tokio::spawn(modbus.serve_link(FrameTransport::<_, Ascii>::new(server_end)));

        let mut client: RmClient<_, Ascii> = RmClient::with_config(
            FrameTransport::new(client_end),
            rust_modbus::ClientConfig {
                response_timeout: std::time::Duration::from_millis(50),
            },
        );
        let err = client
            .write_single_register(UnitId(5), Address(1), RegisterValue(0x1234))
            .await
            .unwrap_err();
        assert!(matches!(err, rust_modbus::Error::Timeout { .. }));

        handle.shutdown().await;
        let _ = serving.await;
    }

    #[tokio::test]
    /// MB-R-128 — over `Tcp` (representative of the four non-Rtu/Ascii transports), an unmapped
    /// slave id still gets an ordinary exception response end-to-end — silence must not leak past
    /// the `physical_serial` gate.
    async fn ut_tcp_unmapped_slave_still_answers_exception() {
        let mem = seeded_memory(&[10, 20]); // slave 1 only
        let (log, _) = recording_log();
        let server = Server::new(mem, log, true, false); // physical_serial = false

        let (server_end, client_end) = tokio::io::duplex(256);
        let modbus = ModbusServer::new(server);
        let handle = modbus.handle();
        let serving = tokio::spawn(modbus.serve_link(FrameTransport::<_, Tcp>::new(server_end)));

        let mut client: RmClient<_, Tcp> = RmClient::new(FrameTransport::new(client_end));
        let err = client
            .read_holding_registers(UnitId(9), Address(0), Quantity(2))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            rust_modbus::Error::Exception {
                exception: ExceptionCode::IllegalDataAddress,
                ..
            }
        ));

        handle.shutdown().await;
        let _ = serving.await;
    }

    /// A minimal `rust_modbus::Server` (with no memory/regions declared) plus its handle — just
    /// enough plumbing for `drive_serve` tests, which exercise the shared command/shutdown
    /// dispatch, not request handling.
    fn minimal_modbus_server() -> (
        ModbusServer<Server<SlaveKey, impl LogFn + Clone>>,
        ServerHandle,
    ) {
        let mem = seeded_memory(&[]);
        let (log, _) = recording_log();
        let modbus = ModbusServer::new(Server::new(mem, log, true, false));
        let handle = modbus.handle();
        (modbus, handle)
    }

    #[tokio::test]
    /// MB-R-133, MB-E-092 — a `ServerCommand::Terminate` on the command channel ends
    /// `drive_serve` with `ServeEnd::Terminated`, via the graceful `handle.shutdown()` path
    /// (proved by the peer end staying open and connected throughout — nothing here ever drops
    /// or aborts the link); the serve loop is already running and ends on its own rather than
    /// being torn down early.
    async fn ut_drive_serve_terminate_ends_gracefully() {
        let (modbus, handle) = minimal_modbus_server();
        let (server_end, client_end) = tokio::io::duplex(256);
        let serve_fut = modbus.serve_link(FrameTransport::<_, Tcp>::new(server_end));
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ServerCommand>(1);

        let driving = tokio::spawn(async move { drive_serve(serve_fut, handle, &mut rx).await });
        tx.send(ServerCommand::Terminate).await.unwrap();

        let end = tokio::time::timeout(Duration::from_secs(5), driving)
            .await
            .expect("drive_serve did not return promptly")
            .unwrap();
        assert!(matches!(end, ServeEnd::Terminated));
        drop(client_end); // kept alive until here: the link was never dropped mid-test.
    }

    #[tokio::test]
    /// MB-R-133 — the command channel closing (every sender dropped) ends `drive_serve` the same
    /// way as an explicit `Terminate`, with `ServeEnd::Terminated`.
    async fn ut_drive_serve_channel_close_ends_gracefully() {
        let (modbus, handle) = minimal_modbus_server();
        let (server_end, client_end) = tokio::io::duplex(256);
        let serve_fut = modbus.serve_link(FrameTransport::<_, Tcp>::new(server_end));
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ServerCommand>(1);
        drop(tx);

        let end = tokio::time::timeout(
            Duration::from_secs(5),
            drive_serve(serve_fut, handle, &mut rx),
        )
        .await
        .expect("drive_serve did not return promptly");
        assert!(matches!(end, ServeEnd::Terminated));
        drop(client_end);
    }

    #[tokio::test]
    /// MB-R-130, MB-R-131 — a mid-serve transport failure (the `serve_fut` itself resolving
    /// `Err`) surfaces from `drive_serve` as `ServeEnd::Failed`, without a command ever being
    /// sent — proven with a synthetic future rather than a real link failure, since
    /// `serve_link`'s own failure mode is infallible by the crate's own documentation (its
    /// `Result` exists only so a *future* serving failure needs no API change); `drive_serve` is
    /// generic over any `Future<Output = rust_modbus::Result<()>>`, so this exercises its actual
    /// dispatch logic exactly as a real `serve`/`serve_framed`/`serve_tls` failure would (the
    /// code path a real listener-bind/accept failure is wired onto).
    async fn ut_drive_serve_transport_failure_surfaces_as_failed() {
        let (_modbus, handle) = minimal_modbus_server();
        let serve_fut = async { Err(rust_modbus::Error::TrailingBytes { extra: 1 }) };
        let (_tx, mut rx) = tokio::sync::mpsc::channel::<ServerCommand>(1);

        let end = tokio::time::timeout(
            Duration::from_secs(5),
            drive_serve(serve_fut, handle, &mut rx),
        )
        .await
        .expect("drive_serve did not return promptly");
        assert!(matches!(end, ServeEnd::Failed(_)));
    }

    #[test]
    /// BR-R-019 — the gateway exception codes exist and carry the wire bytes the Modbus spec
    /// assigns them, even though ferrowl's own server never emits either.
    fn ut_gateway_exception_codes_carry_their_wire_bytes() {
        assert_eq!(
            ExceptionCode::GatewayPathUnavailable.encode().unwrap(),
            0x0A
        );
        assert_eq!(
            ExceptionCode::GatewayTargetDeviceFailedToRespond
                .encode()
                .unwrap(),
            0x0B
        );
    }

    #[tokio::test]
    /// BR-R-019 — no failing request a ferrowl Modbus server can be asked to answer ever yields
    /// a gateway exception code; only the four `api-contract.md` 0x01-0x04 codes it maps to.
    async fn ut_server_exception_mapping_never_yields_a_gateway_code() {
        let mem = seeded_memory(&[10, 20]);
        let (log, _) = recording_log();
        let mut codes = Vec::new();

        // Unmapped slave, every read function code.
        for request in [
            RequestPdu::ReadCoils {
                address: Address(0),
                quantity: Quantity(2),
            },
            RequestPdu::ReadDiscreteInputs {
                address: Address(0),
                quantity: Quantity(2),
            },
            RequestPdu::ReadInputRegisters {
                address: Address(0),
                quantity: Quantity(2),
            },
            RequestPdu::ReadHoldingRegisters {
                address: Address(0),
                quantity: Quantity(2),
            },
        ] {
            let err = handle_request::<SlaveKey, _>(UnitId(2), request, &mem, &log, false, false)
                .await
                .unwrap_err();
            codes.push(err);
        }

        // Out-of-range address/quantity on a mapped slave (region is `[0, 4)`).
        let err = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReadHoldingRegisters {
                address: Address(3),
                quantity: Quantity(4),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        codes.push(err);

        // Unsupported function.
        let err = handle_request::<SlaveKey, _>(
            UnitId(1),
            RequestPdu::ReportServerId,
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        codes.push(err);

        // Write to an unmapped slave.
        let err = handle_request::<SlaveKey, _>(
            UnitId(2),
            RequestPdu::WriteSingleRegister {
                address: Address(0),
                value: RegisterValue(1),
            },
            &mem,
            &log,
            false,
            false,
        )
        .await
        .unwrap_err();
        codes.push(err);

        assert!(!codes.is_empty());
        for code in codes {
            assert!(
                matches!(
                    code,
                    ExceptionCode::IllegalFunction
                        | ExceptionCode::IllegalDataAddress
                        | ExceptionCode::IllegalDataValue
                        | ExceptionCode::ServerDeviceFailure
                ),
                "unexpected exception code: {code:?}"
            );
        }
    }

    #[tokio::test]
    /// MB-R-221, MB-E-090 — a terminate arriving while a server's listener bind is in flight
    /// aborts the race immediately, well inside the bind's own timeout, matching the abort every
    /// `run_tcp_family`/`run_serial_family`/`run_udp` bind race performs.
    async fn ut_terminate_during_bind_is_abandoned() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<ServerCommand>(1);
        tx.send(ServerCommand::Terminate).await.unwrap();
        let mut parked = std::collections::VecDeque::new();

        let out = tokio::time::timeout(
            Duration::from_millis(250),
            crate::common::race_terminate(
                std::future::pending::<()>(),
                &mut rx,
                |_: &ServerCommand| true,
                &mut parked,
            ),
        )
        .await
        .expect("race_terminate did not return promptly");

        assert!(out.is_none(), "the pending bind is abandoned");
        assert!(parked.is_empty(), "ServerCommand has only one variant");
    }
}
