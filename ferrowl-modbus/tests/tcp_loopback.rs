//! End-to-end TCP test: a ferrowl Modbus TCP server and client talk over a
//! loopback socket. Drives the shared client loop (`client_core`) through every
//! read function code and every write command, plus graceful termination.

// Integration-test crate: an unwrap that fails is the test failing, same as an assertion.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use ferrowl_codec::Kind as RegKind;
use ferrowl_modbus::tcp;
use ferrowl_modbus::{
    Address, Command, FunctionCode, Key, Operation, ServerCommand, SlaveKey, UnitId, Word,
};
use ferrowl_store::{CellKind, CellType, Memory, Range};
use ferrowl_test_support::reserve_tcp_port;
use parking_lot::Mutex;
use parking_lot::RwLock as MemLock;
use tokio::sync::{RwLock, mpsc};
use tokio::time::sleep;

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

/// Polls a `ServerBuilder::spawn`-returned `BoundAddr` until the listener actually binds,
/// instead of racing it with a fixed sleep (MB-R-130 companion — `spawn()` only guarantees the
/// task was scheduled, not that its first bind attempt has run).
async fn wait_bound_addr(bound_addr: &Arc<Mutex<Option<std::net::SocketAddr>>>) {
    for _ in 0..50 {
        if bound_addr.lock().is_some() {
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!("listener did not bind within 1s");
}

/// A log sink that records every line, so a test can assert on what the client logged.
/// `LogFn + Clone` is satisfied by a move-closure capturing an `Arc`.
fn capturing() -> (impl ferrowl_modbus::LogFn + Clone, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = log.clone();
    let f = move |s: String| {
        let sink = sink.clone();
        async move {
            sink.lock().push(s);
        }
    };
    (f, log)
}

fn config(port: u16) -> tcp::Config {
    tcp::Config {
        ip: "127.0.0.1".to_string(),
        port,
        timeout_ms: 1000,
        delay_ms: 0,
        interval_ms: 0,
        reconnect: true,
        tls: Default::default(),
    }
}

fn config_no_reconnect(port: u16) -> tcp::Config {
    tcp::Config {
        reconnect: false,
        ..config(port)
    }
}

/// Server memory seeded with distinct values in all four register tables.
fn server_mem() -> Mem {
    let mut mem = Memory::<Key<SlaveKey>>::default();
    mem.add_ranges(
        key(RegKind::Coil),
        &CellKind::read_write(CellType::Coil),
        &[Range::new(0, 8)],
    );
    mem.write(
        key(RegKind::Coil),
        &CellType::Coil,
        &Range::new(0, 4),
        &[1, 0, 1, 0],
    )
    .unwrap();
    mem.add_ranges(
        key(RegKind::DiscreteInput),
        &CellKind::read_write(CellType::Coil),
        &[Range::new(0, 4)],
    );
    mem.write(
        key(RegKind::DiscreteInput),
        &CellType::Coil,
        &Range::new(0, 4),
        &[0, 1, 1, 0],
    )
    .unwrap();
    mem.add_ranges(
        key(RegKind::InputRegister),
        &CellKind::read_write(CellType::Register),
        &[Range::new(0, 4)],
    );
    mem.write(
        key(RegKind::InputRegister),
        &CellType::Register,
        &Range::new(0, 4),
        &[100, 200, 300, 400],
    )
    .unwrap();
    mem.add_ranges(
        key(RegKind::HoldingRegister),
        &CellKind::read_write(CellType::Register),
        &[Range::new(0, 8)],
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

/// Client memory with the same regions declared but no values (the client fills them from reads).
fn client_mem() -> Mem {
    let mut mem = Memory::<Key<SlaveKey>>::default();
    mem.add_ranges(
        key(RegKind::Coil),
        &CellKind::read_write(CellType::Coil),
        &[Range::new(0, 8)],
    );
    mem.add_ranges(
        key(RegKind::DiscreteInput),
        &CellKind::read_write(CellType::Coil),
        &[Range::new(0, 4)],
    );
    mem.add_ranges(
        key(RegKind::InputRegister),
        &CellKind::read_write(CellType::Register),
        &[Range::new(0, 4)],
    );
    mem.add_ranges(
        key(RegKind::HoldingRegister),
        &CellKind::read_write(CellType::Register),
        &[Range::new(0, 8)],
    );
    Arc::new(MemLock::new(mem))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-035 — the client polls every read operation and writes each result into the shared store
/// (and accepts write commands, MB-R-046, and terminates gracefully, MB-R-049).
/// MB-R-037 — polling advances round-robin, so all four operations are read in one pass.
/// MB-R-039 — polling runs on a fixed tick of `interval_ms`.
/// MB-R-204 — `interval_ms` of 0 (this config) is accepted, not rejected.
/// MB-R-041 — the poll loop issues exactly the four read function codes (coils, discrete inputs, input registers, holding registers).
async fn tcp_client_polls_server_and_executes_commands() {
    let port = reserve_tcp_port().release();
    let srv_mem = server_mem();
    let cli_mem = client_mem();

    let (_srv_tx, srv_rx) = mpsc::channel::<ServerCommand>(1);
    let (server, bound_addr) = tcp::ServerBuilder::new(
        Arc::new(RwLock::new(config(port))),
        srv_mem.clone(),
        tcp::new_self_signed_cache(),
    )
    .spawn(srv_rx, sink(), sink())
    .await
    .expect("server failed to start");
    wait_bound_addr(&bound_addr).await;

    // Operations cover every read function code the client supports.
    let operations = Arc::new(RwLock::new(vec![
        Operation {
            slave_id: UnitId(1),
            fn_code: FunctionCode::ReadCoils,
            range: Range::new(0, 4),
        },
        Operation {
            slave_id: UnitId(1),
            fn_code: FunctionCode::ReadDiscreteInputs,
            range: Range::new(0, 4),
        },
        Operation {
            slave_id: UnitId(1),
            fn_code: FunctionCode::ReadInputRegisters,
            range: Range::new(0, 4),
        },
        Operation {
            slave_id: UnitId(1),
            fn_code: FunctionCode::ReadHoldingRegisters,
            range: Range::new(0, 4),
        },
    ]));

    let (client_log, client_lines) = capturing();
    let (tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(config(port))),
        operations,
        cli_mem.clone(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, client_log, sink())
    .await
    .expect("client failed to connect");

    // Let the client poll every operation at least once.
    sleep(Duration::from_millis(800)).await;

    // MB-R-204: a 1 ms tick over four round-robin operations logs a "successful" line per read;
    // a coarser tick (e.g. the crate's non-zero default) could not clear a fraction of this
    // count in the same window, so this floor pins the tick rate, not just that 0 was accepted.
    let successful_reads = client_lines
        .lock()
        .iter()
        .filter(|l| l.contains("successful"))
        .count();
    assert!(
        successful_reads > 50,
        "expected far more than 50 successful reads in 800ms from a 1 ms tick, got {successful_reads}"
    );

    {
        let g = cli_mem.read();
        assert_eq!(
            g.read(
                key(RegKind::HoldingRegister),
                &CellType::Register,
                &Range::new(0, 4)
            )
            .unwrap(),
            vec![10, 20, 30, 40]
        );
        assert_eq!(
            g.read(
                key(RegKind::InputRegister),
                &CellType::Register,
                &Range::new(0, 4)
            )
            .unwrap(),
            vec![100, 200, 300, 400]
        );
        assert_eq!(
            g.read(key(RegKind::Coil), &CellType::Coil, &Range::new(0, 4))
                .unwrap(),
            vec![1, 0, 1, 0]
        );
        assert_eq!(
            g.read(
                key(RegKind::DiscreteInput),
                &CellType::Coil,
                &Range::new(0, 4)
            )
            .unwrap(),
            vec![0, 1, 1, 0]
        );
    }

    // Exercise every write command against the server.
    tx.send(Command::WriteSingleRegister(
        UnitId(1),
        Address(0),
        Word(99),
    ))
    .await
    .unwrap();
    tx.send(Command::WriteMultipleRegister(
        UnitId(1),
        Address(1),
        vec![5, 6].into_iter().map(Word).collect(),
    ))
    .await
    .unwrap();
    tx.send(Command::WriteSingleCoil(UnitId(1), Address(5), true))
        .await
        .unwrap();
    tx.send(Command::WriteMultipleCoils(
        UnitId(1),
        Address(6),
        vec![true, false],
    ))
    .await
    .unwrap();
    sleep(Duration::from_millis(600)).await;

    {
        let g = srv_mem.read();
        assert_eq!(
            g.read(
                key(RegKind::HoldingRegister),
                &CellType::Register,
                &Range::new(0, 3)
            )
            .unwrap(),
            vec![99, 5, 6]
        );
        assert_eq!(
            g.read(key(RegKind::Coil), &CellType::Coil, &Range::new(5, 3))
                .unwrap(),
            vec![1, 1, 0]
        );
    }

    // Graceful termination returns Ok and ends the client task.
    tx.send(Command::Terminate).await.unwrap();
    let joined = tokio::time::timeout(Duration::from_secs(5), client)
        .await
        .expect("client did not terminate in time")
        .expect("client task panicked");
    assert!(joined.is_ok());

    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-043 — Modbus exceptions do not disconnect the client; it retries then skips the operation
/// (and rejected write commands are logged without disconnecting, MB-R-047).
async fn tcp_client_handles_server_rejections() {
    let port = reserve_tcp_port().release();
    // Server with no registered regions: every request for slave 1 is rejected.
    let srv_mem: Mem = Arc::new(MemLock::new(Memory::<Key<SlaveKey>>::default()));
    let (_srv_tx, srv_rx) = mpsc::channel::<ServerCommand>(1);
    let (server, bound_addr) = tcp::ServerBuilder::new(
        Arc::new(RwLock::new(config(port))),
        srv_mem,
        tcp::new_self_signed_cache(),
    )
    .spawn(srv_rx, sink(), sink())
    .await
    .expect("server failed to start");
    wait_bound_addr(&bound_addr).await;

    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: UnitId(1),
        fn_code: FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 2),
    }]));
    let (tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(config(port))),
        operations,
        client_mem(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, sink(), sink())
    .await
    .expect("client failed to connect");

    // Several poll cycles let the exception-retry counter pass MAX_RETRIES.
    sleep(Duration::from_millis(800)).await;

    // Writes the server rejects -> the "invalid" command branches.
    tx.send(Command::WriteSingleRegister(UnitId(1), Address(0), Word(1)))
        .await
        .unwrap();
    tx.send(Command::WriteMultipleRegister(
        UnitId(1),
        Address(0),
        vec![1, 2].into_iter().map(Word).collect(),
    ))
    .await
    .unwrap();
    tx.send(Command::WriteSingleCoil(UnitId(1), Address(0), true))
        .await
        .unwrap();
    tx.send(Command::WriteMultipleCoils(
        UnitId(1),
        Address(0),
        vec![true],
    ))
    .await
    .unwrap();
    sleep(Duration::from_millis(600)).await;

    tx.send(Command::Terminate).await.unwrap();
    let joined = tokio::time::timeout(Duration::from_secs(5), client)
        .await
        .expect("client did not terminate in time")
        .expect("client task panicked");
    assert!(joined.is_ok());
    server.abort();
}

#[tokio::test]
/// MB-R-069 — an `ip`/`port` pair that does not parse as a socket address fails with a TCP address
/// error, for both the client and the server. `spawn()` itself always returns `Ok` now
/// (MB-R-130/MB-R-134); the server-side address error surfaces from the joined task instead, and
/// never retries even with `reconnect` on (a malformed address never fixes itself).
async fn tcp_unparseable_address_is_error() {
    use ferrowl_modbus::{Error, TcpError};

    let mut bad = config(502);
    bad.ip = "not.an.ip.address".to_string();

    // Client side (`Client` isn't `Debug`, so match the result rather than `unwrap_err`).
    assert!(matches!(
        tcp::Client::connect(&bad, &tcp::new_self_signed_cache()).await,
        Err(Error::Tcp(TcpError::Address(_)))
    ));

    let mem: Mem = Arc::new(MemLock::new(Memory::<Key<SlaveKey>>::default()));
    let (_tx, rx) = mpsc::channel::<ServerCommand>(1);
    let (handle, _bound_addr) = tcp::ServerBuilder::new(
        Arc::new(RwLock::new(bad)),
        mem,
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, sink(), sink())
    .await
    .expect("spawn always returns Ok");
    let server_err = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("task should end promptly, not retry, on an address error")
        .expect("task must not panic")
        .unwrap_err();
    assert!(matches!(server_err, Error::Tcp(TcpError::Address(_))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-163 — an endpoint consults only its own role's TLS policy: the other role's policy is
/// never validated. (The other clause — a non-`None` policy applies TLS for its own role — is
/// already covered by `tcp_tls_client.rs`/`tcp_tls_server.rs`; not re-pinned here.) Both
/// poisoned policies below are rejected by `validate()` for the role they belong to, so a leak
/// surfaces as a configuration error instead of a silent pass.
async fn it_endpoint_ignores_the_other_roles_tls_policy() {
    use ferrowl_util::tls::{CertSource, CertVerification, ClientTlsPolicy, ServerTlsPolicy};

    // Client half: a client endpoint's `server` policy must never be consulted.
    {
        let port = reserve_tcp_port().release();
        let srv_mem = server_mem();
        let (_srv_tx, srv_rx) = mpsc::channel::<ServerCommand>(1);
        let (server, bound_addr) = tcp::ServerBuilder::new(
            Arc::new(RwLock::new(config(port))),
            srv_mem,
            tcp::new_self_signed_cache(),
        )
        .spawn(srv_rx, sink(), sink())
        .await
        .expect("server failed to start");
        wait_bound_addr(&bound_addr).await;

        let mut cfg = config(port);
        cfg.tls = tcp::ModbusTlsConfig {
            server: ServerTlsPolicy::Mutual {
                identity: CertSource::SelfSigned {},
                verification: CertVerification::RootStore {
                    extra_ca_files: vec![],
                },
            },
            client: ClientTlsPolicy::None {},
        };

        let connected = tcp::Client::connect(&cfg, &tcp::new_self_signed_cache()).await;
        assert!(
            connected.is_ok(),
            "the poisoned server policy must not affect a client connect: {}",
            connected.err().map(|e| e.to_string()).unwrap_or_default()
        );

        let operations = Arc::new(RwLock::new(vec![Operation {
            slave_id: UnitId(1),
            fn_code: FunctionCode::ReadHoldingRegisters,
            range: Range::new(0, 4),
        }]));
        let cli_mem = client_mem();
        let (tx, rx) = mpsc::channel::<Command>(16);
        let (client, _connected) = tcp::ClientBuilder::new(
            Arc::new(RwLock::new(cfg)),
            operations,
            cli_mem.clone(),
            tcp::new_self_signed_cache(),
        )
        .spawn(rx, sink(), sink())
        .await
        .expect("client failed to connect");

        sleep(Duration::from_millis(400)).await;
        assert_eq!(
            cli_mem
                .read()
                .read(
                    key(RegKind::HoldingRegister),
                    &CellType::Register,
                    &Range::new(0, 4)
                )
                .unwrap(),
            vec![10, 20, 30, 40],
            "a real plain read must succeed through the poisoned-server-policy client"
        );

        tx.send(Command::Terminate).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), client).await;
        server.abort();
    }

    // Server half: a server endpoint's `client` policy must never be consulted.
    {
        let port = reserve_tcp_port().release();
        let srv_mem = server_mem();
        let mut cfg = config(port);
        cfg.tls = tcp::ModbusTlsConfig {
            server: ServerTlsPolicy::None {},
            client: ClientTlsPolicy::Mutual {
                verification: CertVerification::Skip {},
                identity: CertSource::Ephemeral {},
            },
        };
        let (_srv_tx, srv_rx) = mpsc::channel::<ServerCommand>(1);
        let (server, bound_addr) = tcp::ServerBuilder::new(
            Arc::new(RwLock::new(cfg)),
            srv_mem,
            tcp::new_self_signed_cache(),
        )
        .spawn(srv_rx, sink(), sink())
        .await
        .expect("server failed to start");
        wait_bound_addr(&bound_addr).await;

        let operations = Arc::new(RwLock::new(vec![Operation {
            slave_id: UnitId(1),
            fn_code: FunctionCode::ReadHoldingRegisters,
            range: Range::new(0, 4),
        }]));
        let cli_mem = client_mem();
        let (tx, rx) = mpsc::channel::<Command>(16);
        let (client, _connected) = tcp::ClientBuilder::new(
            Arc::new(RwLock::new(config(port))),
            operations,
            cli_mem.clone(),
            tcp::new_self_signed_cache(),
        )
        .spawn(rx, sink(), sink())
        .await
        .expect("client failed to connect");

        sleep(Duration::from_millis(400)).await;
        assert_eq!(
            cli_mem
                .read()
                .read(
                    key(RegKind::HoldingRegister),
                    &CellType::Register,
                    &Range::new(0, 4)
                )
                .unwrap(),
            vec![10, 20, 30, 40],
            "a plain client must read successfully through the poisoned-client-policy server"
        );

        tx.send(Command::Terminate).await.unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), client).await;
        server.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-070 — a TCP server accepts connections in a loop, serving multiple concurrent clients
/// against the same shared store.
async fn tcp_server_serves_concurrent_clients() {
    let port = reserve_tcp_port().release();
    let srv_mem = server_mem();

    let (_srv_tx, srv_rx) = mpsc::channel::<ServerCommand>(1);
    let (server, bound_addr) = tcp::ServerBuilder::new(
        Arc::new(RwLock::new(config(port))),
        srv_mem,
        tcp::new_self_signed_cache(),
    )
    .spawn(srv_rx, sink(), sink())
    .await
    .expect("server failed to start");
    wait_bound_addr(&bound_addr).await;

    // Two independent clients connect at the same time and both read from the one server.
    let ops = || {
        Arc::new(RwLock::new(vec![Operation {
            slave_id: UnitId(1),
            fn_code: FunctionCode::ReadHoldingRegisters,
            range: Range::new(0, 4),
        }]))
    };
    let mem_a = client_mem();
    let mem_b = client_mem();
    let (tx_a, rx_a) = mpsc::channel::<Command>(16);
    let (tx_b, rx_b) = mpsc::channel::<Command>(16);
    let (client_a, _connected_a) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(config(port))),
        ops(),
        mem_a.clone(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx_a, sink(), sink())
    .await
    .expect("client A failed to connect");
    let (client_b, _connected_b) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(config(port))),
        ops(),
        mem_b.clone(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx_b, sink(), sink())
    .await
    .expect("client B failed to connect");

    sleep(Duration::from_millis(600)).await;

    for m in [&mem_a, &mem_b] {
        assert_eq!(
            m.read()
                .read(
                    key(RegKind::HoldingRegister),
                    &CellType::Register,
                    &Range::new(0, 4)
                )
                .unwrap(),
            vec![10, 20, 30, 40]
        );
    }

    tx_a.send(Command::Terminate).await.unwrap();
    tx_b.send(Command::Terminate).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), client_a).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), client_b).await;
    server.abort();
}

#[tokio::test]
/// MB-R-068 — a TCP client connect attempt to a port with no listener fails.
async fn tcp_client_connect_refused_is_error() {
    // Nothing is listening on this port, so the connect fails.
    let port = reserve_tcp_port().release();
    assert!(
        tcp::Client::connect(&config(port), &tcp::new_self_signed_cache())
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-055 — with reconnect disabled, a failed connect ends the client task with that error.
async fn tcp_client_reconnect_false_dies_on_refused_connect() {
    // No listener; with reconnect off the spawned task's join result carries the connect error
    // instead of retrying forever.
    let port = reserve_tcp_port().release();
    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: UnitId(1),
        fn_code: FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 2),
    }]));
    let (_tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(config_no_reconnect(port))),
        operations,
        client_mem(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, sink(), sink())
    .await
    .expect("spawn itself always succeeds; the connect error surfaces from the task");

    let joined = tokio::time::timeout(Duration::from_secs(5), client)
        .await
        .expect("client task did not finish in time")
        .expect("client task panicked");
    assert!(joined.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-050 — with reconnect enabled, a refused connect is retried after a backoff and connects once a listener appears.
async fn tcp_client_reconnect_true_connects_once_a_listener_appears() {
    // Nothing is listening yet: the client's first connect attempt fails. With reconnect on it
    // keeps retrying in the background; once a server starts on the port, it should connect and
    // start reading.
    let port = reserve_tcp_port().release();
    let srv_mem = server_mem();
    let cli_mem = client_mem();

    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: UnitId(1),
        fn_code: FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 4),
    }]));
    let (tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(config(port))),
        operations,
        cli_mem.clone(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, sink(), sink())
    .await
    .expect("client failed to connect");

    // Let the first (failing) connect attempt happen before the server exists.
    sleep(Duration::from_millis(200)).await;

    let (_srv_tx, srv_rx) = mpsc::channel::<ServerCommand>(1);
    let (server, bound_addr) = tcp::ServerBuilder::new(
        Arc::new(RwLock::new(config(port))),
        srv_mem,
        tcp::new_self_signed_cache(),
    )
    .spawn(srv_rx, sink(), sink())
    .await
    .expect("server failed to start");
    wait_bound_addr(&bound_addr).await;

    // The 1s initial backoff plus poll delay must elapse before the client retries and reads.
    sleep(Duration::from_millis(2000)).await;

    {
        let g = cli_mem.read();
        assert_eq!(
            g.read(
                key(RegKind::HoldingRegister),
                &CellType::Register,
                &Range::new(0, 4)
            )
            .unwrap(),
            vec![10, 20, 30, 40]
        );
    }

    tx.send(Command::Terminate).await.unwrap();
    let joined = tokio::time::timeout(Duration::from_secs(5), client)
        .await
        .expect("client did not terminate in time")
        .expect("client task panicked");
    assert!(joined.is_ok());
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-053 — Terminate aborts a reconnect backoff wait immediately and ends the client task with success.
async fn tcp_client_terminate_during_backoff_exits_promptly() {
    // No listener, so the client sits in its reconnect backoff (up to 1s initially). Sending
    // Terminate must abort that wait immediately rather than sleeping it out.
    let port = reserve_tcp_port().release();
    let operations = Arc::new(RwLock::new(vec![]));
    let (tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(config(port))),
        operations,
        client_mem(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, sink(), sink())
    .await
    .expect("spawn itself always succeeds");

    // Give the first failed connect attempt time to happen and enter the backoff wait.
    sleep(Duration::from_millis(100)).await;
    tx.send(Command::Terminate).await.unwrap();

    // Well under the 1s initial backoff: proves Terminate interrupts the wait rather than
    // sleeping it out.
    let joined = tokio::time::timeout(Duration::from_millis(500), client)
        .await
        .expect("Terminate did not interrupt the reconnect backoff promptly")
        .expect("client task panicked");
    assert!(joined.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-036 — the operation list is shared and mutable at runtime; an operation added after the
/// client is polling is picked up on a later poll cycle without any reconnect.
async fn tcp_client_operation_list_mutated_at_runtime() {
    let port = reserve_tcp_port().release();
    let srv_mem = server_mem();
    let cli_mem = client_mem();

    let (_srv_tx, srv_rx) = mpsc::channel::<ServerCommand>(1);
    let (server, bound_addr) = tcp::ServerBuilder::new(
        Arc::new(RwLock::new(config(port))),
        srv_mem,
        tcp::new_self_signed_cache(),
    )
    .spawn(srv_rx, sink(), sink())
    .await
    .expect("server failed to start");
    wait_bound_addr(&bound_addr).await;

    // Start with a single operation reading the holding registers.
    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: UnitId(1),
        fn_code: FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 4),
    }]));
    let (tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(config(port))),
        operations.clone(),
        cli_mem.clone(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, sink(), sink())
    .await
    .expect("client failed to connect");

    sleep(Duration::from_millis(300)).await;
    // The input-register table has not been read yet: still zeros in the client store.
    assert_eq!(
        cli_mem
            .read()
            .read(
                key(RegKind::InputRegister),
                &CellType::Register,
                &Range::new(0, 4)
            )
            .unwrap(),
        vec![0, 0, 0, 0]
    );

    // Add an input-register operation at runtime — no reconnect.
    operations.write().await.push(Operation {
        slave_id: UnitId(1),
        fn_code: FunctionCode::ReadInputRegisters,
        range: Range::new(0, 4),
    });
    sleep(Duration::from_millis(400)).await;

    assert_eq!(
        cli_mem
            .read()
            .read(
                key(RegKind::InputRegister),
                &CellType::Register,
                &Range::new(0, 4)
            )
            .unwrap(),
        vec![100, 200, 300, 400]
    );

    tx.send(Command::Terminate).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), client).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-056 — the connection settings (here the endpoint) are re-read from the shared config on
/// every connection attempt, so an edit takes effect on the next reconnect.
async fn tcp_client_rereads_config_on_reconnect() {
    let good_port = reserve_tcp_port().release();
    let bad_port = reserve_tcp_port().release();
    let srv_mem = server_mem();
    let cli_mem = client_mem();

    // A server listens on `good_port`; the client is initially pointed at `bad_port` (no listener).
    let (_srv_tx, srv_rx) = mpsc::channel::<ServerCommand>(1);
    let (server, bound_addr) = tcp::ServerBuilder::new(
        Arc::new(RwLock::new(config(good_port))),
        srv_mem,
        tcp::new_self_signed_cache(),
    )
    .spawn(srv_rx, sink(), sink())
    .await
    .expect("server failed to start");
    wait_bound_addr(&bound_addr).await;

    let shared_cfg = Arc::new(RwLock::new(config(bad_port)));
    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: UnitId(1),
        fn_code: FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 4),
    }]));
    let (tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = tcp::ClientBuilder::new(
        shared_cfg.clone(),
        operations,
        cli_mem.clone(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, sink(), sink())
    .await
    .expect("spawn succeeds; first connect fails against the empty port");

    // First connect attempt fails; while it backs off, repoint the config at the live server.
    sleep(Duration::from_millis(200)).await;
    shared_cfg.write().await.port = good_port;

    // The 1 s initial backoff plus a poll tick must elapse before the re-read connect and read.
    sleep(Duration::from_millis(2000)).await;
    assert_eq!(
        cli_mem
            .read()
            .read(
                key(RegKind::HoldingRegister),
                &CellType::Register,
                &Range::new(0, 4)
            )
            .unwrap(),
        vec![10, 20, 30, 40]
    );

    tx.send(Command::Terminate).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), client).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-052 — the backoff resets to 1 s after a connection run that got at least one read through,
/// so the reconnect logged after a successful run is "1s" even though the backoff had already grown.
async fn tcp_client_backoff_resets_after_successful_run() {
    let port = reserve_tcp_port().release();
    let srv_mem = server_mem();
    let cli_mem = client_mem();

    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: UnitId(1),
        fn_code: FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 4),
    }]));
    let (tx, rx) = mpsc::channel::<Command>(16);
    let (log, lines) = capturing();
    // No server yet: the first connect fails and the backoff grows to 2 s.
    let (client, _connected) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(config(port))),
        operations,
        cli_mem.clone(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, log, sink())
    .await
    .expect("spawn succeeds; first connect fails");

    // Bring the server up during the first (1 s) backoff so the second attempt connects and reads.
    sleep(Duration::from_millis(500)).await;
    let (_srv_tx, srv_rx) = mpsc::channel::<ServerCommand>(1);
    let (server, bound_addr) = tcp::ServerBuilder::new(
        Arc::new(RwLock::new(config(port))),
        srv_mem,
        tcp::new_self_signed_cache(),
    )
    .spawn(srv_rx, sink(), sink())
    .await
    .expect("server failed to start");
    wait_bound_addr(&bound_addr).await;

    // Let the client connect and get at least one read through (marks the run successful).
    sleep(Duration::from_millis(2000)).await;
    assert_eq!(
        cli_mem
            .read()
            .read(
                key(RegKind::HoldingRegister),
                &CellType::Register,
                &Range::new(0, 4)
            )
            .unwrap(),
        vec![10, 20, 30, 40]
    );

    // Drop the server: the run ends with a transport error, and because it read successfully the
    // backoff is reset to 1 s.
    server.abort();
    sleep(Duration::from_millis(1500)).await;

    let logged = lines.lock().clone();
    let first_success = logged
        .iter()
        .position(|l| l.contains("successful"))
        .expect("expected a successful read");
    let reconnect_after_success = logged[first_success..]
        .iter()
        .find(|l| l.contains("Reconnecting in"))
        .expect("expected a reconnect log after the successful run");
    // The backoff had already grown to 2 s from the first failed connect; the reset brings the
    // post-success reconnect back to 1 s.
    assert!(
        reconnect_after_success.contains("in 1s"),
        "post-success reconnect backoff was not reset to 1s: {reconnect_after_success}"
    );

    tx.send(Command::Terminate).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), client).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-048 — each read addresses the slave id carried by the operation, independent of any slave
/// id configured on the transport (the TCP transport configures none).
async fn tcp_client_addresses_operation_slave_id() {
    let port = reserve_tcp_port().release();

    // Server memory declared only under slave id 7. A request for any other slave finds no region.
    let k7 = || {
        Key::new(SlaveKey {
            slave_id: UnitId(7),
            kind: RegKind::HoldingRegister,
        })
    };
    let mut sm = Memory::<Key<SlaveKey>>::default();
    sm.add_ranges(
        k7(),
        &CellKind::read_write(CellType::Register),
        &[Range::new(0, 4)],
    );
    sm.write(
        k7(),
        &CellType::Register,
        &Range::new(0, 4),
        &[11, 22, 33, 44],
    )
    .unwrap();
    let srv_mem: Mem = Arc::new(MemLock::new(sm));

    let (_srv_tx, srv_rx) = mpsc::channel::<ServerCommand>(1);
    let (server, bound_addr) = tcp::ServerBuilder::new(
        Arc::new(RwLock::new(config(port))),
        srv_mem,
        tcp::new_self_signed_cache(),
    )
    .spawn(srv_rx, sink(), sink())
    .await
    .expect("server failed to start");
    wait_bound_addr(&bound_addr).await;

    // Client store keyed on slave 7 too; the operation targets slave 7.
    let mut cm = Memory::<Key<SlaveKey>>::default();
    cm.add_ranges(
        k7(),
        &CellKind::read_write(CellType::Register),
        &[Range::new(0, 4)],
    );
    let cli_mem: Mem = Arc::new(MemLock::new(cm));

    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: UnitId(7),
        fn_code: FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 4),
    }]));
    let (tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(config(port))),
        operations,
        cli_mem.clone(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, sink(), sink())
    .await
    .expect("client failed to connect");

    sleep(Duration::from_millis(400)).await;
    // The read only succeeds if the request was addressed to slave 7 (set from the operation).
    assert_eq!(
        cli_mem
            .read()
            .read(k7(), &CellType::Register, &Range::new(0, 4))
            .unwrap(),
        vec![11, 22, 33, 44]
    );

    tx.send(Command::Terminate).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), client).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-038 — the client waits `delay_ms` before its first poll on a connection.
async fn tcp_client_delays_before_first_poll() {
    let port = reserve_tcp_port().release();
    let srv_mem = server_mem();
    let cli_mem = client_mem();

    let (_srv_tx, srv_rx) = mpsc::channel::<ServerCommand>(1);
    let (server, bound_addr) = tcp::ServerBuilder::new(
        Arc::new(RwLock::new(config(port))),
        srv_mem,
        tcp::new_self_signed_cache(),
    )
    .spawn(srv_rx, sink(), sink())
    .await
    .expect("server failed to start");
    wait_bound_addr(&bound_addr).await;

    // A long start delay: nothing should be read until it elapses.
    let cfg = tcp::Config {
        delay_ms: 600,
        ..config(port)
    };
    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: UnitId(1),
        fn_code: FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 4),
    }]));
    let (tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(cfg)),
        operations,
        cli_mem.clone(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, sink(), sink())
    .await
    .expect("client failed to connect");

    // Well before the 600ms delay elapses: nothing polled yet.
    sleep(Duration::from_millis(250)).await;
    assert_eq!(
        cli_mem
            .read()
            .read(
                key(RegKind::HoldingRegister),
                &CellType::Register,
                &Range::new(0, 4)
            )
            .unwrap(),
        vec![0, 0, 0, 0]
    );

    // After the delay plus a poll tick: the values are in.
    sleep(Duration::from_millis(600)).await;
    assert_eq!(
        cli_mem
            .read()
            .read(
                key(RegKind::HoldingRegister),
                &CellType::Register,
                &Range::new(0, 4)
            )
            .unwrap(),
        vec![10, 20, 30, 40]
    );

    tx.send(Command::Terminate).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), client).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-040 — every read is bounded by `timeout_ms`; a server that accepts the connection but never
/// answers makes the read time out, which (with reconnect off) ends the client task with an error.
async fn tcp_client_read_times_out_when_server_silent() {
    // A raw TCP listener that accepts connections but never speaks Modbus.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let silent = tokio::spawn(async move {
        let mut held = Vec::new();
        loop {
            if let Ok((stream, _)) = listener.accept().await {
                held.push(stream); // keep it open; never reply
            }
        }
    });

    let cfg = tcp::Config {
        timeout_ms: 300,
        ..config_no_reconnect(port)
    };
    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: UnitId(1),
        fn_code: FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 2),
    }]));
    let (_tx, rx) = mpsc::channel::<Command>(16);
    let (client, _connected) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(cfg)),
        operations,
        client_mem(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, sink(), sink())
    .await
    .expect("connect (TCP handshake) succeeds against the silent listener");

    // timeout_ms is 300ms; the task must end with an error well before this bound.
    let joined = tokio::time::timeout(Duration::from_secs(3), client)
        .await
        .expect("read did not time out within the bound")
        .expect("client task panicked");
    assert!(joined.is_err());
    silent.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-044 — a successful read resets the retry counter, so an operation that was failing and then
/// recovers keeps polling successfully instead of staying skipped.
async fn tcp_client_success_resets_retry_counter() {
    let port = reserve_tcp_port().release();
    // Server starts with no region declared: every read for slave 1 is rejected (exceptions).
    let cli_mem = client_mem();
    let srv_mem: Mem = Arc::new(MemLock::new(Memory::<Key<SlaveKey>>::default()));
    let (_srv_tx, srv_rx) = mpsc::channel::<ServerCommand>(1);
    let (server, bound_addr) = tcp::ServerBuilder::new(
        Arc::new(RwLock::new(config(port))),
        srv_mem.clone(),
        tcp::new_self_signed_cache(),
    )
    .spawn(srv_rx, sink(), sink())
    .await
    .expect("server failed to start");
    wait_bound_addr(&bound_addr).await;

    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: UnitId(1),
        fn_code: FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 4),
    }]));
    let (tx, rx) = mpsc::channel::<Command>(16);
    let (log, lines) = capturing();
    let (client, _connected) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(config(port))),
        operations,
        cli_mem.clone(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, log, sink())
    .await
    .expect("client failed to connect");

    // Let several exception-retry cycles run against the region-less server.
    sleep(Duration::from_millis(500)).await;

    // Now declare and seed the region: reads start succeeding, which resets the retry counter.
    {
        let mut g = srv_mem.write();
        g.add_ranges(
            key(RegKind::HoldingRegister),
            &CellKind::read_write(CellType::Register),
            &[Range::new(0, 4)],
        );
        g.write(
            key(RegKind::HoldingRegister),
            &CellType::Register,
            &Range::new(0, 4),
            &[10, 20, 30, 40],
        )
        .unwrap();
    }
    sleep(Duration::from_millis(500)).await;

    // The recovered operation keeps polling and fills the client store.
    assert_eq!(
        cli_mem
            .read()
            .read(
                key(RegKind::HoldingRegister),
                &CellType::Register,
                &Range::new(0, 4)
            )
            .unwrap(),
        vec![10, 20, 30, 40]
    );
    // A "successful" read line was logged after recovery, proving the counter was reset and the
    // operation resumed rather than staying permanently invalid.
    assert!(
        lines.lock().iter().any(|l| l.contains("successful")),
        "expected a successful read after recovery"
    );

    tx.send(Command::Terminate).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), client).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
/// MB-R-205 — a missed poll tick delays the schedule, never fires catch-up ticks: driving the
/// real client poll loop (`client_core::run`) through a server whose first reply is slow enough
/// to miss several tick periods, the *next* successful read lands roughly one `interval_ms`
/// later, not immediately — which a catch-up burst would do instead.
async fn it_missed_tick_delays_not_bursts() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // A raw TCP listener speaking just enough Modbus TCP to answer one ReadHoldingRegisters
    // request per read: the very first reply is deliberately slow, every later one instant, so
    // the poll loop's ticker has time to miss several ticks while awaiting that first response.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let first = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let first = first.clone();
            tokio::spawn(async move {
                loop {
                    let mut req = [0u8; 12];
                    if stream.read_exact(&mut req).await.is_err() {
                        break;
                    }
                    if first.swap(false, std::sync::atomic::Ordering::SeqCst) {
                        sleep(Duration::from_millis(150)).await;
                    }
                    let resp = [req[0], req[1], 0, 0, 0, 7, req[6], 3, 4, 0, 0, 0, 0];
                    if stream.write_all(&resp).await.is_err() {
                        break;
                    }
                }
            });
        }
    });

    let cfg = tcp::Config {
        interval_ms: 30,
        timeout_ms: 2000,
        ..config(port)
    };
    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: UnitId(1),
        fn_code: FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 2),
    }]));
    let (_tx, rx) = mpsc::channel::<Command>(16);

    let hits: Arc<Mutex<Vec<std::time::Instant>>> = Arc::new(Mutex::new(Vec::new()));
    let hits_sink = hits.clone();
    let log = move |s: String| {
        let hits_sink = hits_sink.clone();
        async move {
            if s.contains("successful") {
                hits_sink.lock().push(std::time::Instant::now());
            }
        }
    };

    let (_client, _connected) = tcp::ClientBuilder::new(
        Arc::new(RwLock::new(cfg)),
        operations,
        client_mem(),
        tcp::new_self_signed_cache(),
    )
    .spawn(rx, log, sink())
    .await
    .expect("connect succeeds");

    // Three successful reads: the first lands after the slow reply (having missed several 30ms
    // periods while awaiting it); the loop's very next tick is always immediate, whatever the
    // missed-tick policy, since one is already overdue — that is the second read. The policy
    // only shows up in the *third* read's gap from the second: `Delay` reschedules a fresh
    // 30ms-spaced tick from the catch-up point, where a catch-up burst would fire it right away
    // too.
    tokio::time::timeout(Duration::from_secs(3), async {
        while hits.lock().len() < 3 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("expected three successful reads within the bound");

    let recorded = hits.lock().clone();
    let gap = recorded[2].duration_since(recorded[1]);
    assert!(
        gap >= Duration::from_millis(20),
        "a missed tick must not queue a catch-up burst: the read after the catch-up tick landed \
         after {gap:?}, too soon for a fresh interval_ms=30ms tick to have been waited for"
    );

    server.abort();
}
