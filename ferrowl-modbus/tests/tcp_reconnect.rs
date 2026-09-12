//! Server-side bind-failure retry for TCP-framed server transports (MB-R-071 revised,
//! MB-R-114, MB-R-126, MB-R-130-134). Kept apart from `tcp_tls_server.rs`'s TLS-configuration
//! tests: this file is about the retry/backoff axis, not certificate handling.

// Integration-test crate: an unwrap that fails is the test failing, same as an assertion.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use ferrowl_codec::Kind as RegKind;
use ferrowl_modbus::{Address, Key, ServerCommand, SlaveKey, UnitId};
use ferrowl_store::{CellKind, CellType, Memory, Range};
use ferrowl_test_support::reserve_tcp_port;
use parking_lot::RwLock as MemLock;
use tokio::sync::{RwLock, mpsc};

type Mem = Arc<MemLock<Memory<Key<SlaveKey>>>>;

fn key(kind: RegKind) -> Key<SlaveKey> {
    Key::new(SlaveKey {
        slave_id: UnitId(1),
        kind,
    })
}

/// A no-op log/status sink. `LogFn + Clone` is satisfied by a capture-free closure.
fn sink() -> impl ferrowl_modbus::LogFn + Clone {
    |_s: String| async move {}
}

fn server_mem() -> Mem {
    let mut mem = Memory::<Key<SlaveKey>>::default();
    mem.add_ranges(
        key(RegKind::HoldingRegister),
        &CellKind::read_write(CellType::Register),
        &[Range::new(0, 4)],
    );
    mem.write(
        key(RegKind::HoldingRegister),
        &CellType::Register,
        &Range::new(0, 4),
        &[10, 20, 30, 40],
    )
    .unwrap();
    Arc::new(MemLock::new(mem))
}

fn tcp_config(port: u16, reconnect: bool) -> ferrowl_modbus::tcp::Config {
    ferrowl_modbus::tcp::Config {
        ip: "127.0.0.1".to_string(),
        port,
        timeout_ms: 1000,
        delay_ms: 0,
        interval_ms: 0,
        reconnect,
        tls: Default::default(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-071, MB-R-130, NF-R-059 — with `reconnect` enabled (the default), a TCP server whose listen port is
/// already occupied does not fail its start: `spawn()` still returns `Ok`, the task keeps
/// retrying the bind, and once the occupier drops, the very next attempt succeeds and a real
/// client can connect.
async fn tcp_server_bind_failure_retries_then_succeeds() {
    let occupier = reserve_tcp_port();
    let port = occupier.port();

    let (_sender, receiver) = mpsc::channel::<ServerCommand>(1);
    let (handle, _bound_addr) = ferrowl_modbus::tcp::ServerBuilder::new(
        Arc::new(RwLock::new(tcp_config(port, true))),
        server_mem(),
        ferrowl_modbus::tcp::new_self_signed_cache(),
    )
    .spawn(receiver, sink(), sink())
    .await
    .expect("spawn always returns Ok; the bind failure surfaces from the task instead");

    // Bind is failing and retrying, not ending the task.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !handle.is_finished(),
        "task must still be retrying the bind"
    );

    drop(occupier);
    // The default backoff's first wait is 1s (MB-R-051); give it enough room to retry and bind.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let mut client =
        rust_modbus::Client::<_, rust_modbus::Tcp>::new(rust_modbus::FrameTransport::new(
            tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .expect("server should have rebound after the occupier dropped"),
        ));
    let registers = client
        .read_holding_registers(UnitId(1), Address(0), rust_modbus::Quantity(2))
        .await
        .expect("a real client should be able to read once the retry rebinds");
    assert_eq!(
        registers,
        vec![
            rust_modbus::RegisterValue(10),
            rust_modbus::RegisterValue(20)
        ]
    );

    handle.abort();
}

#[tokio::test]
/// MB-R-134 — with `reconnect` disabled, a TCP server bind failure ends the task with the
/// error: `spawn()` itself still returns `Ok(handle)`, but awaiting `handle` resolves to
/// `Err(Error::Server(_))`.
async fn tcp_server_bind_failure_reconnect_false_ends_task() {
    let occupier = reserve_tcp_port();
    let port = occupier.port();

    let (_sender, receiver) = mpsc::channel::<ServerCommand>(1);
    let (handle, _bound_addr) = ferrowl_modbus::tcp::ServerBuilder::new(
        Arc::new(RwLock::new(tcp_config(port, false))),
        server_mem(),
        ferrowl_modbus::tcp::new_self_signed_cache(),
    )
    .spawn(receiver, sink(), sink())
    .await
    .expect("spawn always returns Ok");

    let result = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("task should end promptly, not retry, with reconnect disabled")
        .expect("task must not panic");
    assert!(matches!(result, Err(ferrowl_modbus::Error::Server(_))));

    drop(occupier);
}

#[tokio::test]
/// MB-R-133 — `ServerCommand::Terminate`, sent while the server is backing off after a bind
/// failure, ends the task promptly with `Ok(())` rather than waiting out the whole backoff.
async fn tcp_server_terminate_while_backing_off_ends_task_ok() {
    let occupier = reserve_tcp_port();
    let port = occupier.port();

    let (sender, receiver) = mpsc::channel::<ServerCommand>(1);
    let (handle, _bound_addr) = ferrowl_modbus::tcp::ServerBuilder::new(
        Arc::new(RwLock::new(tcp_config(port, true))),
        server_mem(),
        ferrowl_modbus::tcp::new_self_signed_cache(),
    )
    .spawn(receiver, sink(), sink())
    .await
    .expect("spawn always returns Ok");

    // Give the first bind attempt (and its failure) time to happen, so the task is certainly
    // in its backoff wait by the time Terminate is sent.
    tokio::time::sleep(Duration::from_millis(100)).await;
    sender.send(ServerCommand::Terminate).await.unwrap();

    let result = tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("Terminate must abort the backoff wait promptly, not wait it out")
        .expect("task must not panic");
    assert!(result.is_ok());

    drop(occupier);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-114 — an RtuOverTcp server (reuses `tcp::Config`, MB-R-113) retries a bind-failure the
/// same way TCP does.
async fn rtu_over_tcp_server_bind_failure_retries_then_succeeds() {
    let occupier = reserve_tcp_port();
    let port = occupier.port();

    let (_sender, receiver) = mpsc::channel::<ServerCommand>(1);
    let (handle, _bound_addr) = ferrowl_modbus::rtu_over_tcp::ServerBuilder::new(
        Arc::new(RwLock::new(tcp_config(port, true))),
        server_mem(),
        ferrowl_modbus::tcp::new_self_signed_cache(),
    )
    .spawn(receiver, sink(), sink())
    .await
    .expect("spawn always returns Ok");

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !handle.is_finished(),
        "task must still be retrying the bind"
    );

    drop(occupier);
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let mut client =
        rust_modbus::Client::<_, rust_modbus::RtuOverTcp>::new(rust_modbus::FrameTransport::new(
            tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .expect("server should have rebound after the occupier dropped"),
        ));
    let registers = client
        .read_holding_registers(UnitId(1), Address(0), rust_modbus::Quantity(2))
        .await
        .expect("a real client should be able to read once the retry rebinds");
    assert_eq!(
        registers,
        vec![
            rust_modbus::RegisterValue(10),
            rust_modbus::RegisterValue(20)
        ]
    );

    handle.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-126 — an AsciiOverTcp server (reuses `tcp::Config`, MB-R-113) retries a bind-failure the
/// same way TCP does.
async fn ascii_over_tcp_server_bind_failure_retries_then_succeeds() {
    let occupier = reserve_tcp_port();
    let port = occupier.port();

    let (_sender, receiver) = mpsc::channel::<ServerCommand>(1);
    let (handle, _bound_addr) = ferrowl_modbus::ascii_over_tcp::ServerBuilder::new(
        Arc::new(RwLock::new(tcp_config(port, true))),
        server_mem(),
        ferrowl_modbus::tcp::new_self_signed_cache(),
    )
    .spawn(receiver, sink(), sink())
    .await
    .expect("spawn always returns Ok");

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !handle.is_finished(),
        "task must still be retrying the bind"
    );

    drop(occupier);
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let mut client =
        rust_modbus::Client::<_, rust_modbus::Ascii>::new(rust_modbus::FrameTransport::new(
            tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .expect("server should have rebound after the occupier dropped"),
        ));
    let registers = client
        .read_holding_registers(UnitId(1), Address(0), rust_modbus::Quantity(2))
        .await
        .expect("a real client should be able to read once the retry rebinds");
    assert_eq!(
        registers,
        vec![
            rust_modbus::RegisterValue(10),
            rust_modbus::RegisterValue(20)
        ]
    );

    handle.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-221, MB-E-090 — a terminate arriving around a server's listener bind ends the task with
/// success rather than waiting for the bind to complete.
async fn it_terminate_while_server_bind_pending_ends_task_ok() {
    let port = reserve_tcp_port().release();

    let (sender, receiver) = mpsc::channel::<ServerCommand>(1);
    let (handle, _bound_addr) = ferrowl_modbus::tcp::ServerBuilder::new(
        Arc::new(RwLock::new(tcp_config(port, true))),
        server_mem(),
        ferrowl_modbus::tcp::new_self_signed_cache(),
    )
    .spawn(receiver, sink(), sink())
    .await
    .expect("spawn always returns Ok");

    sender.send(ServerCommand::Terminate).await.unwrap();

    let result = tokio::time::timeout(Duration::from_millis(500), handle)
        .await
        .expect("terminate around the bind must not hang")
        .expect("task must not panic");
    assert!(result.is_ok(), "the server task must end with success");
}
