//! ASCII (serial) transport tests. A real serial loopback needs hardware (or a named
//! PTY the ASCII builders can open by path), which isn't portable in CI, so these cover
//! the serial-open failure paths — which is where the ASCII-specific lifecycle behavior
//! (open-once-at-start for the server, reconnect-on-open-failure for the client) is
//! observable. Mirrors `rtu_serial.rs`; ASCII reuses `rtu::Config` verbatim (MB-R-121).

// Integration-test crate: an unwrap that fails is the test failing, same as an assertion.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use ferrowl_modbus::rtu; // Config type only — MB-R-121 reuses it verbatim
use ferrowl_modbus::{
    Command, Error, FunctionCode, Key, Operation, SerialError, ServerCommand, SlaveKey, UnitId,
    ascii,
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
/// MB-R-124, MB-R-130 — with `reconnect` enabled (the default), a serial-open
/// failure does not fail an Ascii server's start: `spawn()` returns `Ok(handle)`, and the task
/// keeps retrying the open on the shared backoff policy instead of ending, exactly as MB-R-075
/// for RTU (reconnect enabled).
async fn ascii_server_open_failure_retries_while_reconnect_enabled() {
    let (_tx, rx) = mpsc::channel::<ServerCommand>(1);
    let (handle, _open) =
        ascii::ServerBuilder::new(Arc::new(RwLock::new(bad_config(true))), empty_mem())
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
/// MB-R-124, MB-R-134 — with `reconnect` disabled, a serial-open failure fails the Ascii
/// server: `spawn()` still returns `Ok(handle)`, but the joined task carries the serial error,
/// exactly as MB-R-208 for RTU. MB-R-123 — the Ascii server opens the port once at start, with
/// no accept loop deferring it, so that first open is what fails here.
async fn ascii_server_open_failure_reconnect_false_ends_task() {
    let (_tx, rx) = mpsc::channel::<ServerCommand>(1);
    let (handle, _open) =
        ascii::ServerBuilder::new(Arc::new(RwLock::new(bad_config(false))), empty_mem())
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
/// MB-R-133 — a `ServerCommand::Terminate` sent while the Ascii server is backing off from a
/// serial-open failure ends the task gracefully (`Ok(())`), not with the open error.
async fn ascii_server_terminate_while_backing_off_ends_task_ok() {
    let (tx, rx) = mpsc::channel::<ServerCommand>(1);
    let (handle, _open) =
        ascii::ServerBuilder::new(Arc::new(RwLock::new(bad_config(true))), empty_mem())
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
/// MB-R-124 — for an Ascii client, a serial-open failure is a failed connection attempt; with
/// reconnect disabled it ends the client task with the error, exactly as MB-R-209 for RTU.
async fn ascii_client_open_failure_reconnect_false_dies() {
    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: UnitId(1),
        fn_code: FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 2),
    }]));
    let (_tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = ascii::ClientBuilder::new(
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
/// MB-R-124 — for an Ascii client, a serial-open failure with reconnect enabled is subject to
/// the reconnect rules: the task keeps retrying rather than dying, and Terminate ends it
/// cleanly, exactly as MB-R-209 for RTU.
async fn ascii_client_open_failure_reconnect_true_retries() {
    let operations = Arc::new(RwLock::new(vec![]));
    let (tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = ascii::ClientBuilder::new(
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
