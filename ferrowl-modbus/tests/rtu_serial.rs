//! RTU (serial) transport tests. A real serial loopback needs hardware (or a named
//! PTY the RTU builders can open by path), which isn't portable in CI, so these cover
//! the serial-open failure paths — which is where the RTU-specific lifecycle behavior
//! (open-once-at-start for the server, reconnect-on-open-failure for the client) is
//! observable.

// Integration-test crate: an unwrap that fails is the test failing, same as an assertion.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use ferrowl_modbus::rtu;
use ferrowl_modbus::{
    Command, Error, FunctionCode, Key, Operation, SerialError, ServerCommand, SlaveKey, UnitId,
};
use ferrowl_store::{Memory, Range};
use parking_lot::RwLock as MemLock;
use tokio::sync::{RwLock, mpsc};
use tokio::time::sleep;

type Mem = Arc<MemLock<Memory<Key<SlaveKey>>>>;

fn sink() -> impl ferrowl_modbus::LogFn + Clone {
    |_s: String| async move {}
}

fn empty_mem() -> Mem {
    Arc::new(MemLock::new(Memory::<Key<SlaveKey>>::default()))
}

/// A serial path that cannot be opened, so `SerialStream::open` fails.
fn bad_config(reconnect: bool) -> rtu::Config {
    rtu::Config {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// MB-R-075, MB-R-130 — with `reconnect` enabled (the default), a serial-open failure
/// does not fail an RTU server's start: `spawn()` returns `Ok(handle)`, and the task keeps
/// retrying the open on the shared backoff policy instead of ending.
async fn rtu_server_open_failure_retries_while_reconnect_enabled() {
    let (_tx, rx) = mpsc::channel::<ServerCommand>(1);
    let (handle, _open) =
        rtu::ServerBuilder::new(Arc::new(RwLock::new(bad_config(true))), empty_mem())
            .spawn(rx, sink(), sink())
            .await
            .expect("spawn always returns Ok");
    sleep(Duration::from_millis(200)).await;
    assert!(
        !handle.is_finished(),
        "an open failure with reconnect enabled must keep retrying, not end the task"
    );
    handle.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// MB-R-208, MB-R-134 — with `reconnect` disabled, a serial-open failure fails the RTU
/// server: `spawn()` still returns `Ok(handle)`, but the joined task carries the serial error.
/// MB-R-074 — the RTU server opens the port once at start, with no accept loop deferring it, so
/// that first open is what fails here.
async fn rtu_server_open_failure_reconnect_false_ends_task() {
    let (_tx, rx) = mpsc::channel::<ServerCommand>(1);
    let (handle, _open) =
        rtu::ServerBuilder::new(Arc::new(RwLock::new(bad_config(false))), empty_mem())
            .spawn(rx, sink(), sink())
            .await
            .expect("spawn always returns Ok");
    let result = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("task should end promptly, not retry, with reconnect disabled")
        .expect("task must not panic");
    assert!(matches!(result, Err(Error::Serial(SerialError::Error(_)))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// MB-R-133 — a `ServerCommand::Terminate` sent while the RTU server is backing off from a
/// serial-open failure ends the task gracefully (`Ok(())`), not with the open error.
async fn rtu_server_terminate_while_backing_off_ends_task_ok() {
    let (tx, rx) = mpsc::channel::<ServerCommand>(1);
    let (handle, _open) =
        rtu::ServerBuilder::new(Arc::new(RwLock::new(bad_config(true))), empty_mem())
            .spawn(rx, sink(), sink())
            .await
            .expect("spawn always returns Ok");
    sleep(Duration::from_millis(100)).await;
    tx.send(ServerCommand::Terminate).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("Terminate did not end the retrying server in time")
        .expect("task must not panic");
    assert!(result.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// MB-R-209 — for an RTU client, a serial-open failure is a failed connection attempt; with
/// reconnect disabled it ends the client task with the error.
async fn rtu_client_open_failure_reconnect_false_dies() {
    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: UnitId(1),
        fn_code: FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 2),
    }]));
    let (_tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = rtu::ClientBuilder::new(
        Arc::new(RwLock::new(bad_config(false))),
        operations,
        empty_mem(),
    )
    .spawn(rx, sink(), sink())
    .await
    .expect("spawn succeeds; the open error surfaces from the task");

    let joined = tokio::time::timeout(Duration::from_secs(5), client)
        .await
        .expect("client task did not finish in time")
        .expect("client task panicked");
    assert!(joined.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// MB-R-209 — for an RTU client, a serial-open failure with reconnect enabled is subject to the
/// reconnect rules: the task keeps retrying rather than dying, and Terminate ends it cleanly.
async fn rtu_client_open_failure_reconnect_true_retries() {
    let operations = Arc::new(RwLock::new(vec![]));
    let (tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = rtu::ClientBuilder::new(
        Arc::new(RwLock::new(bad_config(true))),
        operations,
        empty_mem(),
    )
    .spawn(rx, sink(), sink())
    .await
    .expect("spawn succeeds");

    // The open keeps failing; with reconnect on, the task must still be alive (backing off), not
    // finished. Terminate then ends it with success.
    sleep(Duration::from_millis(200)).await;
    tx.send(Command::Terminate).await.unwrap();
    let joined = tokio::time::timeout(Duration::from_secs(5), client)
        .await
        .expect("Terminate did not end the retrying client in time")
        .expect("client task panicked");
    assert!(joined.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// BR-R-010, BR-R-006 — a bridge RTU downstream against an unopenable serial port answers
/// every `forward` with `GatewayPathUnavailable` and keeps retrying the open in the background
/// (MB-R-050–056's backoff), logging a `[bridge]`-prefixed line each attempt.
async fn bridge_rtu_downstream_open_failure_answers_gateway_path_unavailable_and_retries() {
    let lines = Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
    let log = {
        let lines = lines.clone();
        move |s: String| {
            let lines = lines.clone();
            async move {
                lines.lock().push(s);
            }
        }
    };

    let downstream = ferrowl_modbus::bridge::spawn_rtu_downstream(bad_config(true), log);

    let result = tokio::time::timeout(
        Duration::from_millis(200),
        downstream.forward(
            UnitId(1),
            rust_modbus::RequestPdu::ReadHoldingRegisters {
                address: rust_modbus::Address(0),
                quantity: rust_modbus::Quantity(1),
            },
        ),
    )
    .await
    .expect("forward must not block on a pending open");
    assert_eq!(
        result,
        Err(rust_modbus::ExceptionCode::GatewayPathUnavailable)
    );

    // The open keeps failing on a 1s-then-doubling backoff (INITIAL_BACKOFF): poll until a
    // second [bridge]-prefixed attempt has been logged, proving the retry loop is alive.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let count = lines
            .lock()
            .iter()
            .filter(|l| l.starts_with(ferrowl_modbus::bridge::ERROR_PREFIX))
            .count();
        if count >= 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected at least two [bridge]-prefixed open-failure lines within the deadline: {:?}",
            lines.lock()
        );
        sleep(Duration::from_millis(20)).await;
    }
}
