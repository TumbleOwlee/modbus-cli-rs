//! MB-R-142/143/144 — the monitor's decode/match state machine and receive-loop driver.

use super::record::{MonitorRecord, RecordStatus, SharedRecordLog, TableShape};
use super::table::SharedObservedTable;
use crate::common::serial_config_from;
use crate::server_core::wait_reconnect_backoff;
use crate::{
    ConnectedCell, Error, Key, LogFn, PathConflictCell, SerialError, ServerCommand, SlaveKey,
};

use ferrowl_codec::Kind;
use ferrowl_util::backoff::{AttemptOutcome, BackoffPolicy, run_with_backoff};
use rust_modbus::{
    AduReader, Direction, Framing, RegisterValue, RequestPdu, ResponsePdu, TransportConfig, UnitId,
    open_serial,
};
use std::time::Instant;

/// State of the monitor's decode/match state machine (MB-R-142): either awaiting the next
/// request, or awaiting the response to a request already seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MatchState {
    ExpectRequest,
    ExpectResponse {
        slave: UnitId,
        function: rust_modbus::FunctionCode,
        request: RequestPdu,
    },
}

/// How the monitor's receive-loop driver ended.
pub(crate) enum MonitorEnd {
    Terminated,
    Failed(rust_modbus::Error),
}

/// MB-R-142 — decode one raw ADU per the transport's framing `F` and advance `state`
/// accordingly, logging a completed pairing (MB-R-143), an unmatched request (MB-R-143), or a
/// discarded malformed frame (MB-R-142's CRC/LRC/malformed carve-out) as it goes, and applying
/// each MB-R-143-logged slave id's traffic to `table` (MB-R-144): a matched non-exception
/// transaction writes words, while a matched exception, an unmatched request, or a broadcast
/// with nothing to write still marks that slave id as seen.
pub(crate) async fn process_frame<F, L>(
    bytes: Vec<u8>,
    state: MatchState,
    log: &L,
    table: &SharedObservedTable,
    records: &SharedRecordLog,
) -> MatchState
where
    F: Framing<Header = UnitId>,
    L: LogFn,
{
    match state {
        MatchState::ExpectRequest => match F::decode_request(&bytes) {
            Err(e) => {
                log.invoke(format!("malformed frame discarded: {e}")).await;
                MatchState::ExpectRequest
            }
            Ok((slave, request)) => handle_new_request(slave, request, log, table, records).await,
        },
        MatchState::ExpectResponse {
            slave,
            function,
            request,
        } => match F::decode_response(&bytes) {
            Ok((resp_slave, response))
                if resp_slave == slave && response.function() == function =>
            {
                log_complete(slave, &request, Some(&response), log).await;
                push_matched_record(slave, &request, &response, records);
                apply_matched(slave, &request, &response, table);
                MatchState::ExpectRequest
            }
            _ => match F::decode_request(&bytes) {
                Ok((new_slave, new_request)) => {
                    log_unmatched(slave, &request, log).await;
                    push_unmatched_record(slave, &request, records);
                    handle_new_request(new_slave, new_request, log, table, records).await
                }
                Err(_) => {
                    log.invoke("malformed frame discarded while awaiting response".to_string())
                        .await;
                    MatchState::ExpectResponse {
                        slave,
                        function,
                        request,
                    }
                }
            },
        },
    }
}

/// MB-R-142 — begin awaiting a response to `request`, unless `slave` is the broadcast address
/// (MB-R-101/103), in which case it is logged complete on its own (MB-R-143) and, if
/// write-shaped, applied to `table` immediately (MB-R-144) — a broadcast never gets a response.
/// Every request reaching here marks `slave` seen in `table` (MB-R-144), regardless of how it
/// is eventually resolved — matched success, matched exception, unmatched, or broadcast.
async fn handle_new_request<L: LogFn>(
    slave: UnitId,
    request: RequestPdu,
    log: &L,
    table: &SharedObservedTable,
    records: &SharedRecordLog,
) -> MatchState {
    table.write().mark_seen(slave);
    if slave == UnitId(0) {
        log_complete(slave, &request, None, log).await;
        push_broadcast_record(slave, &request, records);
        apply_broadcast(slave, &request, table);
        MatchState::ExpectRequest
    } else {
        let function = request.function();
        MatchState::ExpectResponse {
            slave,
            function,
            request,
        }
    }
}

/// MB-R-143 — one log entry per completed request/response pairing (or, for a broadcast, per
/// request on its own).
async fn log_complete<L: LogFn>(
    slave: UnitId,
    request: &RequestPdu,
    response: Option<&ResponsePdu>,
    log: &L,
) {
    let msg = match response {
        Some(response) => {
            format!("slave {slave} complete: request={request:?} response={response:?}")
        }
        None => format!("slave {slave} complete (broadcast): request={request:?}"),
    };
    log.invoke(msg).await;
}

/// MB-R-143 — one log entry per unmatched request: no response frame arrived before the next
/// request began.
async fn log_unmatched<L: LogFn>(slave: UnitId, request: &RequestPdu, log: &L) {
    log.invoke(format!(
        "slave {slave} unmatched (no response): request={request:?}"
    ))
    .await;
}

/// MB-R-146 — a matched pair's captured record: `Ok` unless the response carries an exception
/// code (`Exception(code)`), in which case the shape is derived from the request alone (no
/// value was transacted, per the record-status-to-value gating note).
fn push_matched_record(
    slave: UnitId,
    request: &RequestPdu,
    response: &ResponsePdu,
    records: &SharedRecordLog,
) {
    let operation = request.function();
    let (status, shape) = match response {
        ResponsePdu::Exception(e) => (
            RecordStatus::Exception(e.exception),
            shape_address_only(request),
        ),
        _ => (RecordStatus::Ok, shape_from_pair(request, response)),
    };
    records.write().push(
        slave,
        MonitorRecord {
            timestamp: Instant::now(),
            status,
            operation,
            shape,
        },
    );
}

/// MB-R-146 — an unmatched request's captured record: address/quantity known from the request
/// alone, no values (nothing was transacted).
fn push_unmatched_record(slave: UnitId, request: &RequestPdu, records: &SharedRecordLog) {
    records.write().push(
        slave,
        MonitorRecord {
            timestamp: Instant::now(),
            status: RecordStatus::Unmatched,
            operation: request.function(),
            shape: shape_address_only(request),
        },
    );
}

/// MB-R-146 — a broadcast request's captured record: always `Ok` (a broadcast never gets a
/// response to fail), shape/values from the request's own carried value(s), same as
/// `apply_broadcast`'s table write.
fn push_broadcast_record(slave: UnitId, request: &RequestPdu, records: &SharedRecordLog) {
    records.write().push(
        slave,
        MonitorRecord {
            timestamp: Instant::now(),
            status: RecordStatus::Ok,
            operation: request.function(),
            shape: shape_from_broadcast(request),
        },
    );
}

/// MB-R-146's `TableShape` for a matched, non-exception pair — same 9-operation coverage as
/// `apply_matched`'s own match, but returning the shape instead of writing to the table.
fn shape_from_pair(request: &RequestPdu, response: &ResponsePdu) -> Option<TableShape> {
    match (request, response) {
        (RequestPdu::ReadCoils { address, quantity }, ResponsePdu::ReadCoils { coils }) => {
            Some(TableShape {
                kind: Kind::Coil,
                address: address.0,
                quantity: quantity.0,
                write_address: None,
                write_quantity: None,
                values: coils.iter().map(|b| u16::from(*b)).collect(),
            })
        }
        (
            RequestPdu::ReadDiscreteInputs { address, quantity },
            ResponsePdu::ReadDiscreteInputs { inputs },
        ) => Some(TableShape {
            kind: Kind::DiscreteInput,
            address: address.0,
            quantity: quantity.0,
            write_address: None,
            write_quantity: None,
            values: inputs.iter().map(|b| u16::from(*b)).collect(),
        }),
        (
            RequestPdu::ReadHoldingRegisters { address, quantity },
            ResponsePdu::ReadHoldingRegisters { registers },
        ) => Some(TableShape {
            kind: Kind::HoldingRegister,
            address: address.0,
            quantity: quantity.0,
            write_address: None,
            write_quantity: None,
            values: registers.iter().map(|r| r.0).collect(),
        }),
        (
            RequestPdu::ReadInputRegisters { address, quantity },
            ResponsePdu::ReadInputRegisters { registers },
        ) => Some(TableShape {
            kind: Kind::InputRegister,
            address: address.0,
            quantity: quantity.0,
            write_address: None,
            write_quantity: None,
            values: registers.iter().map(|r| r.0).collect(),
        }),
        (RequestPdu::WriteSingleCoil { address, value }, ResponsePdu::WriteSingleCoil { .. }) => {
            Some(TableShape {
                kind: Kind::Coil,
                address: address.0,
                quantity: 1,
                write_address: None,
                write_quantity: None,
                values: vec![u16::from(*value)],
            })
        }
        (
            RequestPdu::WriteSingleRegister { address, value },
            ResponsePdu::WriteSingleRegister { .. },
        ) => Some(TableShape {
            kind: Kind::HoldingRegister,
            address: address.0,
            quantity: 1,
            write_address: None,
            write_quantity: None,
            values: vec![value.0],
        }),
        (
            RequestPdu::WriteMultipleCoils { address, coils },
            ResponsePdu::WriteMultipleCoils { .. },
        ) => Some(TableShape {
            kind: Kind::Coil,
            address: address.0,
            quantity: coils.len() as u16,
            write_address: None,
            write_quantity: None,
            values: coils.iter().map(|b| u16::from(*b)).collect(),
        }),
        (
            RequestPdu::WriteMultipleRegisters { address, registers },
            ResponsePdu::WriteMultipleRegisters { .. },
        ) => Some(TableShape {
            kind: Kind::HoldingRegister,
            address: address.0,
            quantity: registers.len() as u16,
            write_address: None,
            write_quantity: None,
            values: registers.iter().map(|r| r.0).collect(),
        }),
        (
            RequestPdu::ReadWriteMultipleRegisters {
                read_address,
                read_quantity,
                write_address,
                registers,
                ..
            },
            ResponsePdu::ReadWriteMultipleRegisters {
                registers: read_values,
            },
        ) => Some(TableShape {
            kind: Kind::HoldingRegister,
            address: read_address.0,
            quantity: read_quantity.0,
            write_address: Some(write_address.0),
            write_quantity: Some(registers.len() as u16),
            values: read_values.iter().map(|r| r.0).collect(),
        }),
        _ => None,
    }
}

/// MB-R-146's `TableShape` for a broadcast request — same 5-operation coverage as
/// `apply_broadcast`'s own match, values from the request's own carried value(s).
fn shape_from_broadcast(request: &RequestPdu) -> Option<TableShape> {
    match request {
        RequestPdu::WriteSingleCoil { address, value } => Some(TableShape {
            kind: Kind::Coil,
            address: address.0,
            quantity: 1,
            write_address: None,
            write_quantity: None,
            values: vec![u16::from(*value)],
        }),
        RequestPdu::WriteSingleRegister { address, value } => Some(TableShape {
            kind: Kind::HoldingRegister,
            address: address.0,
            quantity: 1,
            write_address: None,
            write_quantity: None,
            values: vec![value.0],
        }),
        RequestPdu::WriteMultipleCoils { address, coils } => Some(TableShape {
            kind: Kind::Coil,
            address: address.0,
            quantity: coils.len() as u16,
            write_address: None,
            write_quantity: None,
            values: coils.iter().map(|b| u16::from(*b)).collect(),
        }),
        RequestPdu::WriteMultipleRegisters { address, registers } => Some(TableShape {
            kind: Kind::HoldingRegister,
            address: address.0,
            quantity: registers.len() as u16,
            write_address: None,
            write_quantity: None,
            values: registers.iter().map(|r| r.0).collect(),
        }),
        RequestPdu::ReadWriteMultipleRegisters {
            write_address,
            registers,
            ..
        } => Some(TableShape {
            kind: Kind::HoldingRegister,
            address: write_address.0,
            quantity: registers.len() as u16,
            write_address: None,
            write_quantity: None,
            values: registers.iter().map(|r| r.0).collect(),
        }),
        _ => None,
    }
}

/// MB-R-146's `TableShape` for a request with no response available (an unmatched request, or
/// the request-only half of an exception): address/quantity are known from the request alone;
/// `values` stays empty — nothing was transacted (see the record-status-to-value gating note).
fn shape_address_only(request: &RequestPdu) -> Option<TableShape> {
    match request {
        RequestPdu::ReadCoils { address, quantity } => Some(TableShape {
            kind: Kind::Coil,
            address: address.0,
            quantity: quantity.0,
            write_address: None,
            write_quantity: None,
            values: vec![],
        }),
        RequestPdu::ReadDiscreteInputs { address, quantity } => Some(TableShape {
            kind: Kind::DiscreteInput,
            address: address.0,
            quantity: quantity.0,
            write_address: None,
            write_quantity: None,
            values: vec![],
        }),
        RequestPdu::ReadHoldingRegisters { address, quantity } => Some(TableShape {
            kind: Kind::HoldingRegister,
            address: address.0,
            quantity: quantity.0,
            write_address: None,
            write_quantity: None,
            values: vec![],
        }),
        RequestPdu::ReadInputRegisters { address, quantity } => Some(TableShape {
            kind: Kind::InputRegister,
            address: address.0,
            quantity: quantity.0,
            write_address: None,
            write_quantity: None,
            values: vec![],
        }),
        RequestPdu::WriteSingleCoil { address, .. } => Some(TableShape {
            kind: Kind::Coil,
            address: address.0,
            quantity: 1,
            write_address: None,
            write_quantity: None,
            values: vec![],
        }),
        RequestPdu::WriteSingleRegister { address, .. } => Some(TableShape {
            kind: Kind::HoldingRegister,
            address: address.0,
            quantity: 1,
            write_address: None,
            write_quantity: None,
            values: vec![],
        }),
        RequestPdu::WriteMultipleCoils { address, coils } => Some(TableShape {
            kind: Kind::Coil,
            address: address.0,
            quantity: coils.len() as u16,
            write_address: None,
            write_quantity: None,
            values: vec![],
        }),
        RequestPdu::WriteMultipleRegisters { address, registers } => Some(TableShape {
            kind: Kind::HoldingRegister,
            address: address.0,
            quantity: registers.len() as u16,
            write_address: None,
            write_quantity: None,
            values: vec![],
        }),
        RequestPdu::ReadWriteMultipleRegisters {
            read_address,
            read_quantity,
            write_address,
            registers,
            ..
        } => Some(TableShape {
            kind: Kind::HoldingRegister,
            address: read_address.0,
            quantity: read_quantity.0,
            write_address: Some(write_address.0),
            write_quantity: Some(registers.len() as u16),
            values: vec![],
        }),
        _ => None,
    }
}

/// MB-R-144 — a matched transaction updates `table`: a read-shaped request writes the
/// response's returned words; a write-shaped request writes its own carried value(s).
/// `ReadWriteMultipleRegisters` is both at once (FR-R-037's read-then-write). A response
/// carrying an exception code writes no words (there are none to write); `slave` was already
/// marked seen (MB-R-144) when the request was first decoded, in `handle_new_request`.
fn apply_matched(
    slave: UnitId,
    request: &RequestPdu,
    response: &ResponsePdu,
    table: &SharedObservedTable,
) {
    if matches!(response, ResponsePdu::Exception(_)) {
        return;
    }
    match (request, response) {
        (RequestPdu::ReadCoils { address, .. }, ResponsePdu::ReadCoils { coils }) => {
            write_bits(slave, Kind::Coil, address.0, coils, table);
        }
        (
            RequestPdu::ReadDiscreteInputs { address, .. },
            ResponsePdu::ReadDiscreteInputs { inputs },
        ) => {
            write_bits(slave, Kind::DiscreteInput, address.0, inputs, table);
        }
        (
            RequestPdu::ReadHoldingRegisters { address, .. },
            ResponsePdu::ReadHoldingRegisters { registers },
        ) => {
            write_regs(slave, Kind::HoldingRegister, address.0, registers, table);
        }
        (
            RequestPdu::ReadInputRegisters { address, .. },
            ResponsePdu::ReadInputRegisters { registers },
        ) => {
            write_regs(slave, Kind::InputRegister, address.0, registers, table);
        }
        (RequestPdu::WriteSingleCoil { address, value }, ResponsePdu::WriteSingleCoil { .. }) => {
            write_bits(
                slave,
                Kind::Coil,
                address.0,
                std::slice::from_ref(value),
                table,
            );
        }
        (
            RequestPdu::WriteSingleRegister { address, value },
            ResponsePdu::WriteSingleRegister { .. },
        ) => {
            write_words(slave, Kind::HoldingRegister, address.0, &[value.0], table);
        }
        (
            RequestPdu::WriteMultipleCoils { address, coils },
            ResponsePdu::WriteMultipleCoils { .. },
        ) => {
            write_bits(slave, Kind::Coil, address.0, coils, table);
        }
        (
            RequestPdu::WriteMultipleRegisters { address, registers },
            ResponsePdu::WriteMultipleRegisters { .. },
        ) => {
            write_regs(slave, Kind::HoldingRegister, address.0, registers, table);
        }
        (
            RequestPdu::ReadWriteMultipleRegisters {
                read_address,
                write_address,
                registers,
                ..
            },
            ResponsePdu::ReadWriteMultipleRegisters {
                registers: read_values,
            },
        ) => {
            write_regs(
                slave,
                Kind::HoldingRegister,
                read_address.0,
                read_values,
                table,
            );
            write_regs(
                slave,
                Kind::HoldingRegister,
                write_address.0,
                registers,
                table,
            );
        }
        _ => {}
    }
}

/// MB-R-144 — a broadcast (slave id 0) write-shaped request is applied to `table` immediately,
/// independent of any response (there never is one). Non-write-shaped requests (a broadcast
/// read makes no protocol sense but is not itself malformed) are a no-op here.
fn apply_broadcast(slave: UnitId, request: &RequestPdu, table: &SharedObservedTable) {
    match request {
        RequestPdu::WriteSingleCoil { address, value } => {
            write_bits(
                slave,
                Kind::Coil,
                address.0,
                std::slice::from_ref(value),
                table,
            );
        }
        RequestPdu::WriteSingleRegister { address, value } => {
            write_words(slave, Kind::HoldingRegister, address.0, &[value.0], table);
        }
        RequestPdu::WriteMultipleCoils { address, coils } => {
            write_bits(slave, Kind::Coil, address.0, coils, table);
        }
        RequestPdu::WriteMultipleRegisters { address, registers } => {
            write_regs(slave, Kind::HoldingRegister, address.0, registers, table);
        }
        RequestPdu::ReadWriteMultipleRegisters {
            write_address,
            registers,
            ..
        } => {
            write_regs(
                slave,
                Kind::HoldingRegister,
                write_address.0,
                registers,
                table,
            );
        }
        _ => {}
    }
}

fn write_words(
    slave: UnitId,
    kind: Kind,
    address: u16,
    words: &[u16],
    table: &SharedObservedTable,
) {
    table.write().write_words(
        Key::new(SlaveKey {
            slave_id: slave,
            kind,
        }),
        address,
        words,
    );
}

/// MB-R-144 — coil-family values pack into `u16` words the same way the store's
/// `CellType::Coil` cells already do (`1`/`0`), so the table is a uniform `u16`-per-address map.
fn write_bits(slave: UnitId, kind: Kind, address: u16, bits: &[bool], table: &SharedObservedTable) {
    let words: Vec<u16> = bits.iter().map(|b| u16::from(*b)).collect();
    write_words(slave, kind, address, &words, table);
}

fn write_regs(
    slave: UnitId,
    kind: Kind,
    address: u16,
    regs: &[RegisterValue],
    table: &SharedObservedTable,
) {
    let words: Vec<u16> = regs.iter().map(|r| r.0).collect();
    write_words(slave, kind, address, &words, table);
}

/// MB-R-141 — drives a monitor's receive loop: decode/match every frame `reader` yields,
/// racing an incoming [`crate::ServerCommand`] on `commands`. `activity` is set on every
/// successfully-read frame regardless of decode outcome (MB-R-132's "at least one
/// request/datagram was read" reset condition, applied by analogy).
pub(crate) async fn drive_monitor<S, F, L>(
    mut reader: rust_modbus::AduReader<S, F>,
    log: L,
    table: SharedObservedTable,
    records: SharedRecordLog,
    activity: &std::sync::atomic::AtomicBool,
    commands: &mut tokio::sync::mpsc::Receiver<crate::ServerCommand>,
) -> MonitorEnd
where
    S: tokio::io::AsyncRead + Unpin + Send,
    F: rust_modbus::Framing<Header = rust_modbus::UnitId>,
    L: LogFn + Clone,
{
    let mut state = MatchState::ExpectRequest;
    loop {
        tokio::select! {
            frame = reader.recv_adu() => match frame {
                Ok(bytes) => {
                    activity.store(true, std::sync::atomic::Ordering::Relaxed);
                    state = process_frame::<F, L>(bytes, state, &log, &table, &records).await;
                }
                Err(e) => return MonitorEnd::Failed(e),
            },
            _ = commands.recv() => return MonitorEnd::Terminated,
        }
    }
}

/// Opens the configured serial port receive-only under framing `F` and drives
/// [`drive_monitor`], retrying the open with the shared backoff policy on failure
/// (MB-R-141, MB-R-130–134). Before every OS-level open, `path_conflict` is checked against
/// the freshly `~`-expanded path (MB-R-150); a conflict is a `Failed` outcome carrying the
/// ordinary reconnect setting and is logged before returning, since a `reconnect: true` monitor
/// would otherwise retry it forever with no observable trace at all.
#[allow(clippy::too_many_arguments)] // config/table/records/receiver/log/status/path_conflict/open
pub(crate) async fn run_serial_monitor<F, L, St>(
    config: std::sync::Arc<tokio::sync::RwLock<crate::rtu::Config>>,
    table: SharedObservedTable,
    records: SharedRecordLog,
    receiver: tokio::sync::mpsc::Receiver<ServerCommand>,
    log: L,
    status: St,
    path_conflict: PathConflictCell,
    open: ConnectedCell,
) -> Result<(), Error>
where
    F: Framing<Header = UnitId> + Send + 'static,
    L: LogFn + Clone,
    St: LogFn + Clone,
{
    let receiver = tokio::sync::Mutex::new(receiver);
    let activity = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let attempt = || {
        let config = config.clone();
        let table = table.clone();
        let records = records.clone();
        let log = log.clone();
        let activity = activity.clone();
        let receiver = &receiver;
        let path_conflict = path_conflict.clone();
        let open = open.clone();
        async move {
            activity.store(false, std::sync::atomic::Ordering::Relaxed);
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
                        reconnect: false,
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
            let transport_config = match TransportConfig::from_serial(&serial) {
                Ok(cfg) => cfg,
                Err(e) => {
                    return AttemptOutcome::Failed {
                        error: SerialError::Error(e).into(),
                        reconnect: false,
                        reset: false,
                    };
                }
            };
            match open_serial::<F>(&path, serial) {
                Err(e) => AttemptOutcome::Failed {
                    error: SerialError::Error(e).into(),
                    reconnect,
                    reset: false,
                },
                Ok(transport) => {
                    open.set(true);
                    let stream = transport.into_inner();
                    let reader = AduReader::<_, F>::with_config(
                        stream,
                        Direction::Request,
                        transport_config,
                    );
                    let mut receiver = receiver.lock().await;
                    let end = drive_monitor::<_, F, _>(
                        reader,
                        log.clone(),
                        table.clone(),
                        records.clone(),
                        &activity,
                        &mut receiver,
                    )
                    .await;
                    open.set(false);
                    match end {
                        MonitorEnd::Terminated => AttemptOutcome::Done,
                        MonitorEnd::Failed(e) => AttemptOutcome::Failed {
                            error: Error::Server(e),
                            reconnect,
                            reset: activity.load(std::sync::atomic::Ordering::Relaxed),
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
    status.invoke("Monitor stopped".to_string()).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServerCommand;
    use parking_lot::RwLock;
    use rust_modbus::{Direction, Rtu};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tokio::sync::mpsc;

    fn table() -> SharedObservedTable {
        Arc::new(RwLock::new(super::super::table::ObservedTable::default()))
    }

    fn records() -> SharedRecordLog {
        Arc::new(RwLock::new(super::super::record::RecordLog::default()))
    }

    /// A log sink recording every line sent to it, for assertions.
    #[derive(Clone, Default)]
    struct RecordingLog(Arc<std::sync::Mutex<Vec<String>>>);
    impl LogFn for RecordingLog {
        fn invoke(&self, msg: String) -> impl std::future::Future<Output = ()> + Send {
            let inner = self.0.clone();
            async move {
                inner.lock().unwrap().push(msg);
            }
        }
    }
    impl RecordingLog {
        fn lines(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    fn request_bytes(slave: UnitId, pdu: &RequestPdu) -> Vec<u8> {
        Rtu::encode_request(&slave, pdu).unwrap()
    }

    fn response_bytes(slave: UnitId, pdu: &ResponsePdu) -> Vec<u8> {
        Rtu::encode_response(&slave, pdu).unwrap()
    }

    /// MB-R-193 — a matched request/response pair returns to `ExpectRequest` and logs one
    /// "complete" entry (MB-R-143).
    #[tokio::test]
    async fn ut_matched_request_response_pair_updates_state_and_logs() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let request = RequestPdu::ReadHoldingRegisters {
            address: rust_modbus::Address(0),
            quantity: rust_modbus::Quantity(2),
        };
        let bytes = request_bytes(UnitId(1), &request);
        let state =
            process_frame::<Rtu, _>(bytes, MatchState::ExpectRequest, &log, &table, &records).await;
        assert_eq!(
            state,
            MatchState::ExpectResponse {
                slave: UnitId(1),
                function: rust_modbus::FunctionCode::ReadHoldingRegisters,
                request: request.clone(),
            }
        );

        let response = ResponsePdu::ReadHoldingRegisters {
            registers: vec![RegisterValue(7), RegisterValue(8)],
        };
        let bytes = response_bytes(UnitId(1), &response);
        let state = process_frame::<Rtu, _>(bytes, state, &log, &table, &records).await;
        assert_eq!(state, MatchState::ExpectRequest);
        assert_eq!(log.lines().len(), 1);
        assert!(log.lines()[0].contains("complete"));
    }

    /// MB-R-193 — a pending request with no matching response before the next request begins
    /// is logged unmatched (MB-R-143), and the new request starts a fresh wait.
    #[tokio::test]
    async fn ut_pending_request_marked_unmatched_when_next_request_arrives() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let first = RequestPdu::ReadHoldingRegisters {
            address: rust_modbus::Address(0),
            quantity: rust_modbus::Quantity(1),
        };
        let state = process_frame::<Rtu, _>(
            request_bytes(UnitId(1), &first),
            MatchState::ExpectRequest,
            &log,
            &table,
            &records,
        )
        .await;

        let second = RequestPdu::ReadInputRegisters {
            address: rust_modbus::Address(5),
            quantity: rust_modbus::Quantity(1),
        };
        let state = process_frame::<Rtu, _>(
            request_bytes(UnitId(1), &second),
            state,
            &log,
            &table,
            &records,
        )
        .await;

        assert_eq!(
            state,
            MatchState::ExpectResponse {
                slave: UnitId(1),
                function: rust_modbus::FunctionCode::ReadInputRegisters,
                request: second,
            }
        );
        assert_eq!(log.lines().len(), 1);
        assert!(log.lines()[0].contains("unmatched"));
    }

    /// MB-R-194 — a frame that fails CRC validation is logged (Warning-worded, level-independent
    /// at this crate layer) and discarded; decoding resumes at the next boundary without a state
    /// change, in `ExpectRequest`.
    #[tokio::test]
    async fn ut_crc_failure_logged_warning_and_discarded_without_state_change() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let request = RequestPdu::ReadHoldingRegisters {
            address: rust_modbus::Address(0),
            quantity: rust_modbus::Quantity(1),
        };
        let mut bytes = request_bytes(UnitId(1), &request);
        *bytes.last_mut().unwrap() ^= 0xFF; // corrupt the CRC's high byte

        let state =
            process_frame::<Rtu, _>(bytes, MatchState::ExpectRequest, &log, &table, &records).await;
        assert_eq!(state, MatchState::ExpectRequest);
        assert_eq!(log.lines().len(), 1);
        assert!(log.lines()[0].contains("malformed frame"));
    }

    /// MB-R-194 — a malformed frame while awaiting a response leaves the awaited state
    /// unchanged (still waiting — neither the response nor a new request).
    #[tokio::test]
    async fn ut_crc_failure_while_expecting_response_does_not_change_state() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let request = RequestPdu::ReadHoldingRegisters {
            address: rust_modbus::Address(0),
            quantity: rust_modbus::Quantity(1),
        };
        let waiting = MatchState::ExpectResponse {
            slave: UnitId(1),
            function: rust_modbus::FunctionCode::ReadHoldingRegisters,
            request: request.clone(),
        };
        let mut bytes = request_bytes(UnitId(1), &request);
        *bytes.last_mut().unwrap() ^= 0xFF;

        let state = process_frame::<Rtu, _>(bytes, waiting.clone(), &log, &table, &records).await;
        assert_eq!(state, waiting);
        assert!(log.lines()[0].contains("malformed frame"));
    }

    /// MB-R-143 — a broadcast (slave id 0) request is logged complete immediately and never
    /// marked unmatched: it never enters `ExpectResponse`.
    #[tokio::test]
    async fn ut_broadcast_request_logged_complete_never_unmatched() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let request = RequestPdu::WriteSingleRegister {
            address: rust_modbus::Address(0),
            value: RegisterValue(42),
        };
        let state = process_frame::<Rtu, _>(
            request_bytes(UnitId(0), &request),
            MatchState::ExpectRequest,
            &log,
            &table,
            &records,
        )
        .await;
        assert_eq!(state, MatchState::ExpectRequest);
        assert_eq!(log.lines().len(), 1);
        assert!(log.lines()[0].contains("complete"));
        assert!(!log.lines()[0].contains("unmatched"));
    }

    /// MB-R-197 — a non-write-shaped broadcast writes no words (nothing to write) but still
    /// marks slave id 0 as seen, since it still reaches an MB-R-143 log entry.
    #[tokio::test]
    async fn ut_read_shaped_broadcast_writes_no_words_but_marks_slave_seen() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let request = RequestPdu::ReadHoldingRegisters {
            address: rust_modbus::Address(0),
            quantity: rust_modbus::Quantity(1),
        };
        process_frame::<Rtu, _>(
            request_bytes(UnitId(0), &request),
            MatchState::ExpectRequest,
            &log,
            &table,
            &records,
        )
        .await;

        assert_eq!(table.read().unit_ids(), vec![UnitId(0)]);
    }

    /// MB-R-195 — a matched read transaction writes the response's returned words into the
    /// table at the request's address range.
    #[tokio::test]
    async fn ut_matched_read_writes_response_words_into_table() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let request = RequestPdu::ReadHoldingRegisters {
            address: rust_modbus::Address(10),
            quantity: rust_modbus::Quantity(2),
        };
        let state = process_frame::<Rtu, _>(
            request_bytes(UnitId(1), &request),
            MatchState::ExpectRequest,
            &log,
            &table,
            &records,
        )
        .await;
        let response = ResponsePdu::ReadHoldingRegisters {
            registers: vec![RegisterValue(11), RegisterValue(22)],
        };
        process_frame::<Rtu, _>(
            response_bytes(UnitId(1), &response),
            state,
            &log,
            &table,
            &records,
        )
        .await;

        let key = Key::new(SlaveKey {
            slave_id: UnitId(1),
            kind: Kind::HoldingRegister,
        });
        assert_eq!(table.read().read_words(&key, 10, 2), Some(vec![11, 22]));
    }

    /// MB-R-196 — a matched write transaction writes the request's own carried value(s), not
    /// the response's, into the table.
    #[tokio::test]
    async fn ut_matched_write_writes_request_values_into_table() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let request = RequestPdu::WriteSingleRegister {
            address: rust_modbus::Address(3),
            value: RegisterValue(99),
        };
        let state = process_frame::<Rtu, _>(
            request_bytes(UnitId(1), &request),
            MatchState::ExpectRequest,
            &log,
            &table,
            &records,
        )
        .await;
        let response = ResponsePdu::WriteSingleRegister {
            address: rust_modbus::Address(3),
            value: RegisterValue(99),
        };
        process_frame::<Rtu, _>(
            response_bytes(UnitId(1), &response),
            state,
            &log,
            &table,
            &records,
        )
        .await;

        let key = Key::new(SlaveKey {
            slave_id: UnitId(1),
            kind: Kind::HoldingRegister,
        });
        assert_eq!(table.read().read_words(&key, 3, 1), Some(vec![99]));
    }

    /// MB-R-197 — an unmatched write request writes no words, but marks the slave id as seen.
    #[tokio::test]
    async fn ut_unmatched_write_writes_no_words_but_marks_slave_seen() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let request = RequestPdu::WriteSingleRegister {
            address: rust_modbus::Address(3),
            value: RegisterValue(99),
        };
        process_frame::<Rtu, _>(
            request_bytes(UnitId(1), &request),
            MatchState::ExpectRequest,
            &log,
            &table,
            &records,
        )
        .await;
        // A different request begins before any response — the pending write is unmatched.
        let other = RequestPdu::ReadHoldingRegisters {
            address: rust_modbus::Address(0),
            quantity: rust_modbus::Quantity(1),
        };
        process_frame::<Rtu, _>(
            request_bytes(UnitId(1), &other),
            MatchState::ExpectResponse {
                slave: UnitId(1),
                function: rust_modbus::FunctionCode::WriteSingleRegister,
                request,
            },
            &log,
            &table,
            &records,
        )
        .await;

        let key = Key::new(SlaveKey {
            slave_id: UnitId(1),
            kind: Kind::HoldingRegister,
        });
        assert_eq!(table.read().read_words(&key, 3, 1), None);
        assert_eq!(table.read().unit_ids(), vec![UnitId(1)]);
    }

    /// MB-R-197 — a response carrying an exception code writes no words, but marks the slave
    /// id as seen (`unit_ids()` includes it).
    #[tokio::test]
    async fn ut_exception_response_writes_no_words_but_marks_slave_seen() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let request = RequestPdu::ReadHoldingRegisters {
            address: rust_modbus::Address(0),
            quantity: rust_modbus::Quantity(1),
        };
        let state = process_frame::<Rtu, _>(
            request_bytes(UnitId(1), &request),
            MatchState::ExpectRequest,
            &log,
            &table,
            &records,
        )
        .await;
        let response = ResponsePdu::Exception(rust_modbus::ExceptionResponse {
            function: rust_modbus::FunctionCode::ReadHoldingRegisters,
            exception: rust_modbus::ExceptionCode::IllegalDataAddress,
        });
        let state = process_frame::<Rtu, _>(
            response_bytes(UnitId(1), &response),
            state,
            &log,
            &table,
            &records,
        )
        .await;
        assert_eq!(state, MatchState::ExpectRequest);

        let key = Key::new(SlaveKey {
            slave_id: UnitId(1),
            kind: Kind::HoldingRegister,
        });
        assert_eq!(table.read().read_words(&key, 0, 1), None);
        assert_eq!(table.read().unit_ids(), vec![UnitId(1)]);
    }

    /// MB-R-146 — a matched, non-exception pair captures an `Ok` record with the response's own
    /// shape (mirrors `ut_matched_read_writes_response_words_into_table`'s fixture values).
    #[tokio::test]
    async fn ut_matched_pair_pushes_ok_record_with_shape() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let request = RequestPdu::ReadHoldingRegisters {
            address: rust_modbus::Address(10),
            quantity: rust_modbus::Quantity(2),
        };
        let state = process_frame::<Rtu, _>(
            request_bytes(UnitId(1), &request),
            MatchState::ExpectRequest,
            &log,
            &table,
            &records,
        )
        .await;
        let response = ResponsePdu::ReadHoldingRegisters {
            registers: vec![RegisterValue(11), RegisterValue(22)],
        };
        process_frame::<Rtu, _>(
            response_bytes(UnitId(1), &response),
            state,
            &log,
            &table,
            &records,
        )
        .await;

        let pushed = records.read().records_for(UnitId(1));
        assert_eq!(pushed.len(), 1);
        assert_eq!(pushed[0].status, RecordStatus::Ok);
        assert_eq!(
            pushed[0].operation,
            rust_modbus::FunctionCode::ReadHoldingRegisters
        );
        let shape = pushed[0]
            .shape
            .as_ref()
            .expect("read op must carry a shape");
        assert_eq!(shape.kind, Kind::HoldingRegister);
        assert_eq!(shape.address, 10);
        assert_eq!(shape.quantity, 2);
        assert_eq!(shape.values, vec![11, 22]);
    }

    /// MB-R-146 — an unmatched request still captures a record: address/quantity known, but no
    /// values (nothing was transacted).
    #[tokio::test]
    async fn ut_unmatched_request_pushes_unmatched_record_with_empty_values() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let first = RequestPdu::ReadHoldingRegisters {
            address: rust_modbus::Address(0),
            quantity: rust_modbus::Quantity(1),
        };
        let state = process_frame::<Rtu, _>(
            request_bytes(UnitId(1), &first),
            MatchState::ExpectRequest,
            &log,
            &table,
            &records,
        )
        .await;
        let second = RequestPdu::ReadInputRegisters {
            address: rust_modbus::Address(5),
            quantity: rust_modbus::Quantity(1),
        };
        process_frame::<Rtu, _>(
            request_bytes(UnitId(1), &second),
            state,
            &log,
            &table,
            &records,
        )
        .await;

        let pushed = records.read().records_for(UnitId(1));
        assert_eq!(pushed.len(), 1);
        assert_eq!(pushed[0].status, RecordStatus::Unmatched);
        let shape = pushed[0]
            .shape
            .as_ref()
            .expect("read op must carry a shape");
        assert_eq!(shape.address, 0);
        assert_eq!(shape.quantity, 1);
        assert!(shape.values.is_empty());
    }

    /// MB-R-146 — a response carrying an exception code captures `Exception(code)`, shape known
    /// from the request alone, no values.
    #[tokio::test]
    async fn ut_exception_response_pushes_exception_record_with_empty_values() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let request = RequestPdu::ReadHoldingRegisters {
            address: rust_modbus::Address(0),
            quantity: rust_modbus::Quantity(1),
        };
        let state = process_frame::<Rtu, _>(
            request_bytes(UnitId(1), &request),
            MatchState::ExpectRequest,
            &log,
            &table,
            &records,
        )
        .await;
        let response = ResponsePdu::Exception(rust_modbus::ExceptionResponse {
            function: rust_modbus::FunctionCode::ReadHoldingRegisters,
            exception: rust_modbus::ExceptionCode::IllegalDataAddress,
        });
        process_frame::<Rtu, _>(
            response_bytes(UnitId(1), &response),
            state,
            &log,
            &table,
            &records,
        )
        .await;

        let pushed = records.read().records_for(UnitId(1));
        assert_eq!(pushed.len(), 1);
        assert_eq!(
            pushed[0].status,
            RecordStatus::Exception(rust_modbus::ExceptionCode::IllegalDataAddress)
        );
        let shape = pushed[0]
            .shape
            .as_ref()
            .expect("read op must carry a shape");
        assert_eq!(shape.address, 0);
        assert_eq!(shape.quantity, 1);
        assert!(shape.values.is_empty());
    }

    /// MB-R-146 — a matched pair whose operation isn't one of the 9 table-shaping ops still
    /// captures a record, but with no `TableShape`.
    #[tokio::test]
    async fn ut_non_table_shaping_operation_has_no_shape() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let request = RequestPdu::MaskWriteRegister {
            address: rust_modbus::Address(4),
            and_mask: rust_modbus::Mask(0x00FF),
            or_mask: rust_modbus::Mask(0x0F00),
        };
        let state = process_frame::<Rtu, _>(
            request_bytes(UnitId(1), &request),
            MatchState::ExpectRequest,
            &log,
            &table,
            &records,
        )
        .await;
        let response = ResponsePdu::MaskWriteRegister {
            address: rust_modbus::Address(4),
            and_mask: rust_modbus::Mask(0x00FF),
            or_mask: rust_modbus::Mask(0x0F00),
        };
        process_frame::<Rtu, _>(
            response_bytes(UnitId(1), &response),
            state,
            &log,
            &table,
            &records,
        )
        .await;

        let pushed = records.read().records_for(UnitId(1));
        assert_eq!(pushed.len(), 1);
        assert_eq!(pushed[0].status, RecordStatus::Ok);
        assert_eq!(
            pushed[0].operation,
            rust_modbus::FunctionCode::MaskWriteRegister
        );
        assert!(pushed[0].shape.is_none());
    }

    /// MB-E-016 — `ReadWriteMultipleRegisters` carries both its own read and write
    /// address/quantity pairs; captured `values` come from the *read* response only, never the
    /// request's own written values.
    #[tokio::test]
    async fn ut_read_write_multiple_registers_shape_has_both_addresses_and_read_values_only() {
        let log = RecordingLog::default();
        let table = table();
        let records = records();
        let request = RequestPdu::ReadWriteMultipleRegisters {
            read_address: rust_modbus::Address(10),
            read_quantity: rust_modbus::Quantity(2),
            write_address: rust_modbus::Address(50),
            registers: vec![RegisterValue(111), RegisterValue(222)],
        };
        let state = process_frame::<Rtu, _>(
            request_bytes(UnitId(1), &request),
            MatchState::ExpectRequest,
            &log,
            &table,
            &records,
        )
        .await;
        let response = ResponsePdu::ReadWriteMultipleRegisters {
            registers: vec![RegisterValue(11), RegisterValue(22)],
        };
        process_frame::<Rtu, _>(
            response_bytes(UnitId(1), &response),
            state,
            &log,
            &table,
            &records,
        )
        .await;

        let pushed = records.read().records_for(UnitId(1));
        assert_eq!(pushed.len(), 1);
        let shape = pushed[0]
            .shape
            .as_ref()
            .expect("ReadWriteMultipleRegisters must carry a shape");
        assert_eq!(shape.address, 10);
        assert_eq!(shape.quantity, 2);
        assert_eq!(shape.write_address, Some(50));
        assert_eq!(shape.write_quantity, Some(2));
        assert_eq!(shape.values, vec![11, 22]);
    }

    /// MB-R-141 — `drive_monitor` terminates on `ServerCommand::Terminate`, independent of
    /// whatever the reader is doing, and marks `activity` only for frames actually read.
    #[tokio::test]
    async fn ut_drive_monitor_terminates_on_command() {
        let (_client, server) = tokio::io::duplex(64);
        // `_client` is kept alive (not dropped) so the stream never hits EOF: the reader's
        // `recv_adu` future stays pending forever, and only the command channel can end this.
        let reader = rust_modbus::AduReader::<_, Rtu>::new(server, Direction::Request);
        let table = table();
        let records = records();
        let activity = AtomicBool::new(false);
        let (tx, mut rx) = mpsc::channel::<ServerCommand>(1);
        tx.send(ServerCommand::Terminate).await.unwrap();

        let end = drive_monitor::<_, Rtu, _>(
            reader,
            RecordingLog::default(),
            table,
            records,
            &activity,
            &mut rx,
        )
        .await;
        assert!(matches!(end, MonitorEnd::Terminated));
        assert!(!activity.load(std::sync::atomic::Ordering::Relaxed));
    }
}
