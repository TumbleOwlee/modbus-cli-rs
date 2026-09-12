//! OCPP CSMS listener-bind retry with backoff (OC-R-083 revised, OC-R-108–109). Mirrors
//! `ferrowl-ocpp/tests/cs_reconnect.rs` and `ferrowl-modbus/tests/tcp_reconnect.rs` in shape: an
//! occupied ephemeral port stands in for "bind fails", freeing it later proves recovery.

// Integration-test crate: an unwrap that fails is the test failing, same as an assertion.
#![allow(clippy::unwrap_used)]
#![cfg(feature = "v1_6")]

use std::time::Duration;

use ferrowl_ocpp::cs::{self, CsActionHandler};
use ferrowl_ocpp::csms::{self, CsmsActionHandler};
use ferrowl_ocpp::{Action16, CallError, CallErrorCode, Response16, V1_6};
use tokio::net::TcpListener as TokioTcpListener;
use tokio::time::sleep;

/// No-op log/status sink. `LogFn + Clone` is satisfied by a capture-free closure.
fn sink() -> impl ferrowl_ocpp::LogFn + Clone {
    |_s: String| async move {}
}

/// CSMS handler answering the single action these tests exercise.
struct TestCsms;

impl CsmsActionHandler<V1_6> for TestCsms {
    async fn handle_call(
        &self,
        _conn: csms::ConnectionId,
        action: Action16,
    ) -> Result<Response16, CallError> {
        match action {
            Action16::Heartbeat(_) => Ok(Response16::Heartbeat(
                serde_json::from_value(
                    serde_json::json!({ "currentTime": "2026-01-01T00:00:00Z" }),
                )
                .unwrap(),
            )),
            _ => Err(CallError::new(CallErrorCode::NotImplemented, "unsupported")),
        }
    }
}

/// CS handler; these tests never receive a server-initiated Call.
struct TestCs;

impl CsActionHandler<V1_6> for TestCs {
    async fn handle_call(&self, _action: Action16) -> Result<Response16, CallError> {
        Err(CallError::new(CallErrorCode::NotImplemented, "unsupported"))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// NF-R-047, NF-R-059, OC-R-139 — a CSMS listener whose bind fails retries with backoff instead of failing
/// `spawn` synchronously; once the occupying socket is freed, the next attempt lands.
async fn csms_bind_failure_retries_then_succeeds() {
    // Occupy an ephemeral port ourselves first, then point the CSMS at that exact port.
    let occupier = TokioTcpListener::bind("127.0.0.1:0")
        .await
        .expect("occupier bind failed");
    let occupied_port = occupier.local_addr().expect("occupier addr").port();

    let server = csms::ServerBuilder::<V1_6>::new(csms::Config {
        host: "127.0.0.1".to_owned(),
        port: occupied_port,
        timeout_ms: 1000,
        reconnect: true,
        basic_auth: None,
        tls: Default::default(),
    }, ferrowl_ocpp::new_self_signed_cache())
    .spawn(TestCsms, sink())
    .await
    .expect("spawn must not fail synchronously on an occupied port — only a TLS-config-build failure does (OC-R-040)");

    // The listener never bound: it is backing off from the occupied port.
    sleep(Duration::from_millis(200)).await;
    assert!(
        server.local_addr().is_none(),
        "local_addr must stay None while the port is occupied and the bind is retrying"
    );

    // Free the port and wait past the first backoff interval; the retry must land.
    drop(occupier);
    sleep(Duration::from_millis(1200)).await;
    let addr = server
        .local_addr()
        .expect("the CSMS must have bound once the port was freed");

    // A real CS can now connect and complete a Call.
    let url = format!("ws://{addr}/ocpp/CS001");
    let client = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: false,
            timeout_ms: 1000,
            basic_auth: None,
            tls: Default::default(),
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    let hb = Action16::Heartbeat(serde_json::from_value(serde_json::json!({})).unwrap());
    let resp = tokio::time::timeout(Duration::from_secs(1), client.call(hb))
        .await
        .expect("call must not hang once the listener has bound")
        .expect("the connected CS must be able to complete a Call");
    assert!(matches!(resp, Response16::Heartbeat(_)));

    let _ = client.terminate().await;
    server.terminate().await.expect("server terminate failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-109 — terminating a CSMS while it is backing off from a failed bind aborts the wait
/// immediately and ends the module task successfully (extends OC-R-053 to the backing-off state).
async fn csms_terminate_while_backing_off_ends_task_ok() {
    let occupier = TokioTcpListener::bind("127.0.0.1:0")
        .await
        .expect("occupier bind failed");
    let occupied_port = occupier.local_addr().expect("occupier addr").port();

    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: occupied_port,
            timeout_ms: 1000,
            reconnect: true,
            basic_auth: None,
            tls: Default::default(),
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("spawn must not fail synchronously on an occupied port");

    // Give the first (failing) bind attempt time to run and enter its backoff wait.
    sleep(Duration::from_millis(100)).await;

    let result = tokio::time::timeout(Duration::from_secs(2), server.terminate())
        .await
        .expect("terminate() must not hang while the task is backing off from a failed bind");
    assert!(result.is_ok());

    drop(occupier);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// NF-R-059, OC-R-139 — with `reconnect` disabled, a CSMS listener bind failure ends the module
/// task with that error instead of retrying, mirroring `ferrowl_modbus::tcp`'s
/// `tcp_server_bind_failure_reconnect_false_ends_task` (MB-R-134).
async fn csms_bind_failure_reconnect_false_ends_task() {
    let occupier = TokioTcpListener::bind("127.0.0.1:0")
        .await
        .expect("occupier bind failed");
    let occupied_port = occupier.local_addr().expect("occupier addr").port();

    let mut server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: occupied_port,
            timeout_ms: 1000,
            reconnect: false,
            basic_auth: None,
            tls: Default::default(),
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("spawn must not fail synchronously on an occupied port");

    let result = tokio::time::timeout(Duration::from_secs(5), server.join())
        .await
        .expect("task should end promptly, not retry, with reconnect disabled");
    assert!(result.is_err(), "join must surface the bind failure");

    drop(occupier);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-176, OC-E-096 — a terminate arriving around a CSMS listener bind ends the task with
/// success rather than waiting for the bind to complete.
async fn it_csms_terminate_while_bind_pending_ends_task_ok() {
    let port = ferrowl_test_support::reserve_tcp_port().release();

    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port,
            timeout_ms: 1000,
            reconnect: true,
            basic_auth: None,
            tls: Default::default(),
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("spawn always returns Ok");

    let result = tokio::time::timeout(Duration::from_millis(500), server.terminate())
        .await
        .expect("terminate around the bind must not hang");
    assert!(result.is_ok(), "the server task must end with success");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-177, OC-E-096 — a terminate arriving while the CSMS is idle in `accept()` (bound, no
/// pending connection) ends the task promptly, pinning the existing `accept_loop` select against
/// a real bound listener rather than only its helper-level shape.
async fn it_csms_terminate_during_accept_ends_task_ok() {
    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 1000,
            reconnect: true,
            basic_auth: None,
            tls: Default::default(),
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("spawn always returns Ok");

    for _ in 0..50 {
        if server.local_addr().is_some() {
            break;
        }
        sleep(Duration::from_millis(20)).await;
    }
    assert!(
        server.local_addr().is_some(),
        "the listener must bind before this test can exercise accept()"
    );

    let result = tokio::time::timeout(Duration::from_millis(500), server.terminate())
        .await
        .expect("terminate while idle in accept() must not hang");
    assert!(result.is_ok(), "the server task must end with success");
}
