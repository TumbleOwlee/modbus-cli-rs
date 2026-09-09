//! Security-profile loopback tests layered on the same CS/CSMS engine exercised by
//! `ws_loopback_v16.rs`: HTTP Basic Auth (Security Profile 1) and TLS (Security Profile 2).
//!
//! Mutual TLS (Security Profile 3) is not covered here: building a CA-signed client certificate
//! with `rcgen` (as opposed to a single self-signed leaf) needs a second keypair plus a
//! `BasicConstraints::Ca` issuer, which is enough extra scaffolding that it's left for a
//! follow-up rather than bolted on to this pass.

// Integration-test crate: an unwrap that fails is the test failing, same as an assertion.
#![allow(clippy::unwrap_used)]
#![cfg(feature = "v1_6")]

use ferrowl_ocpp::cs::{self, CsActionHandler};
use ferrowl_ocpp::csms::{self, CsmsActionHandler};
use ferrowl_ocpp::{Action16, BasicAuth, CallError, CallErrorCode, Response16, V1_6};
use ferrowl_test_support::{TempDirGuard, reserve_temp_dir};
use ferrowl_util::tls::{CertSource, CertVerification, ClientTlsPolicy, ServerTlsPolicy};
use serde_json::json;

/// No-op log sink.
fn sink() -> impl ferrowl_ocpp::LogFn + Clone {
    |_s: String| async move {}
}

/// A log sink that records every invocation, for asserting the OC-R-095 fallback is actually
/// logged rather than only flagged.
fn capturing_sink() -> (
    impl ferrowl_ocpp::LogFn + Clone,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = {
        let lines = lines.clone();
        move |s: String| {
            lines.lock().expect("not poisoned").push(s);
            async move {}
        }
    };
    (sink, lines)
}

/// Poll until the CSMS listener has bound: `spawn` binds asynchronously, retrying a
/// failed bind with backoff (OC-R-083), so `local_addr()` is `None` until the first
/// successful bind lands.
async fn bound_addr<V: ferrowl_ocpp::Version>(server: &csms::Server<V>) -> std::net::SocketAddr {
    for _ in 0..50 {
        if let Some(addr) = server.local_addr() {
            return addr;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("CSMS listener never bound");
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
            Action16::BootNotification(_) => Ok(Response16::BootNotification(
                serde_json::from_value(json!({
                    "currentTime": "2026-01-01T00:00:00Z",
                    "interval": 300,
                    "status": "Accepted"
                }))
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

fn boot_action() -> Action16 {
    Action16::BootNotification(
        serde_json::from_value(json!({
            "chargePointModel": "Model-1",
            "chargePointVendor": "Ferrowl"
        }))
        .unwrap(),
    )
}

fn write_pem(dir: &TempDirGuard, label: &str, pem: &str) -> String {
    let path = dir.join(format!("{label}.pem"));
    std::fs::write(&path, pem).expect("failed to write test PEM file");
    path.to_string_lossy().into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-030 — a CS with Basic Auth sends the `Authorization: Basic` header and the CSMS accepts matching credentials.
async fn basic_auth_accepts_matching_credentials() {
    let auth = BasicAuth {
        username: "cp001".to_owned(),
        password: "s3cret".to_owned(),
    };

    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: Some(auth.clone()),
            tls: Default::default(),
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server failed to bind");

    let url = format!("ws://{}/ocpp/CS001", bound_addr(&server).await);
    let client = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: true,
            timeout_ms: 2000,
            basic_auth: Some(auth),
            tls: Default::default(),
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    let resp = client.call(boot_action()).await.expect("boot call failed");
    assert!(matches!(resp, Response16::BootNotification(_)));

    client.terminate().await.expect("client terminate failed");
    server.terminate().await.expect("server terminate failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-031 — a CSMS rejects an upgrade whose Basic Auth credentials do not match, answering 401.
async fn basic_auth_rejects_mismatched_credentials() {
    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: Some(BasicAuth {
                username: "cp001".to_owned(),
                password: "s3cret".to_owned(),
            }),
            tls: Default::default(),
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server failed to bind");

    let url = format!("ws://{}/ocpp/CS001", bound_addr(&server).await);
    // OC-R-048/OC-R-105: `spawn` always succeeds (the dial moved inside the retried task);
    // `reconnect: false` ends the task on the first failed dial instead of retrying forever, and
    // the credential-mismatch error surfaces from `join()`.
    let mut result = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: false,
            timeout_ms: 2000,
            basic_auth: Some(BasicAuth {
                username: "cp001".to_owned(),
                password: "wrong".to_owned(),
            }),
            tls: Default::default(),
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    assert!(
        result.join().await.is_err(),
        "connect should fail the websocket handshake on a credential mismatch"
    );

    server.terminate().await.expect("server terminate failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-031 — a CSMS rejects an upgrade with no `Authorization` header, answering 401.
async fn basic_auth_rejects_missing_credentials() {
    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: Some(BasicAuth {
                username: "cp001".to_owned(),
                password: "s3cret".to_owned(),
            }),
            tls: Default::default(),
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server failed to bind");

    let url = format!("ws://{}/ocpp/CS001", bound_addr(&server).await);
    let mut result = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: false,
            timeout_ms: 2000,
            basic_auth: None,
            tls: Default::default(),
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    assert!(
        result.join().await.is_err(),
        "connect should fail the websocket handshake with no Authorization header at all"
    );

    server.terminate().await.expect("server terminate failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-034 — a CS trusts a certificate supplied via `CertVerification::RootStore`'s `extra_ca_files`, completing a TLS loopback to the CSMS.
/// OC-R-050 — when TLS is configured, the CSMS terminates TLS on the accepted socket before the WebSocket handshake (the CS reaches the OCPP layer only through the completed TLS session).
async fn tls_loopback_over_self_signed_cert() {
    let key_pair = rcgen::KeyPair::generate().expect("keypair generation failed");
    let cert = rcgen::CertificateParams::new(vec!["127.0.0.1".to_owned()])
        .expect("cert params")
        .self_signed(&key_pair)
        .expect("self-signed cert");
    let dir = reserve_temp_dir("ferrowl_ocpp_ws_security");
    let cert_file = write_pem(&dir, "server-cert", &cert.pem());
    let key_file = write_pem(&dir, "server-key", &key_pair.serialize_pem());

    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: None,
            tls: ServerTlsPolicy::Tls {
                identity: CertSource::Files {
                    cert_file: cert_file.clone(),
                    key_file,
                },
            },
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server failed to bind");

    let url = format!("wss://{}/ocpp/CS001", bound_addr(&server).await);
    let client = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: true,
            timeout_ms: 2000,
            basic_auth: None,
            tls: ClientTlsPolicy::Tls {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec![cert_file],
                },
            },
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    let resp = client.call(boot_action()).await.expect("boot call failed");
    assert!(matches!(resp, Response16::BootNotification(_)));

    client.terminate().await.expect("client terminate failed");
    server.terminate().await.expect("server terminate failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-034 — a CS rejects a server certificate not trusted by the webpki roots or its `RootStore`'s `extra_ca_files`.
async fn tls_loopback_rejects_untrusted_cert() {
    let key_pair = rcgen::KeyPair::generate().expect("keypair generation failed");
    let cert = rcgen::CertificateParams::new(vec!["127.0.0.1".to_owned()])
        .expect("cert params")
        .self_signed(&key_pair)
        .expect("self-signed cert");
    let dir = reserve_temp_dir("ferrowl_ocpp_ws_security");
    let cert_file = write_pem(&dir, "server-cert-untrusted", &cert.pem());
    let key_file = write_pem(&dir, "server-key-untrusted", &key_pair.serialize_pem());

    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: None,
            tls: ServerTlsPolicy::Tls {
                identity: CertSource::Files {
                    cert_file,
                    key_file,
                },
            },
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server failed to bind");

    // `extra_ca_files` empty: only the webpki root store is trusted, so the self-signed cert
    // must be rejected.
    let url = format!("wss://{}/ocpp/CS001", bound_addr(&server).await);
    let mut result = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: false,
            timeout_ms: 2000,
            basic_auth: None,
            tls: ClientTlsPolicy::Tls {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec![],
                },
            },
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    assert!(
        result.join().await.is_err(),
        "connect should fail TLS verification against an untrusted self-signed cert"
    );

    server.terminate().await.expect("server terminate failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-036 — a CS with `CertVerification::Skip` accepts any server certificate and connects.
async fn self_signed_csms_with_skip_verify_client_connects() {
    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: None,
            tls: ServerTlsPolicy::Tls {
                identity: CertSource::SelfSigned {},
            },
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server failed to bind");

    let url = format!("wss://{}/ocpp/CS001", bound_addr(&server).await);
    let client = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: true,
            timeout_ms: 2000,
            basic_auth: None,
            tls: ClientTlsPolicy::Tls {
                verification: CertVerification::Skip {},
            },
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    let resp = client.call(boot_action()).await.expect("boot call failed");
    assert!(matches!(resp, Response16::BootNotification(_)));

    client.terminate().await.expect("client terminate failed");
    server.terminate().await.expect("server terminate failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-129 — with `CertVerification::RootStore` (not `Skip`), a CS rejects an untrusted self-signed CSMS certificate.
async fn self_signed_csms_without_skip_verify_client_rejects() {
    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: None,
            tls: ServerTlsPolicy::Tls {
                identity: CertSource::SelfSigned {},
            },
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server failed to bind");

    let url = format!("wss://{}/ocpp/CS001", bound_addr(&server).await);
    let mut result = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: false,
            timeout_ms: 2000,
            basic_auth: None,
            tls: ClientTlsPolicy::Tls {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec![],
                },
            },
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    assert!(
        result.join().await.is_err(),
        "connect should fail under RootStore verification: a per-start self-signed cert can't be pinned"
    );

    server.terminate().await.expect("server terminate failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-031 — Basic Auth credential checking still applies when layered over a self-signed TLS connection.
async fn basic_auth_over_self_signed_tls_checks_credentials() {
    let auth = BasicAuth {
        username: "cp001".to_owned(),
        password: "s3cret".to_owned(),
    };
    let tls = ServerTlsPolicy::Tls {
        identity: CertSource::SelfSigned {},
    };

    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: Some(auth.clone()),
            tls,
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server failed to bind");

    let url = format!("wss://{}/ocpp/CS001", bound_addr(&server).await);
    let client = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url: url.clone(),
            reconnect: true,
            timeout_ms: 2000,
            basic_auth: Some(auth),
            tls: ClientTlsPolicy::Tls {
                verification: CertVerification::Skip {},
            },
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    let resp = client.call(boot_action()).await.expect("boot call failed");
    assert!(matches!(resp, Response16::BootNotification(_)));
    client.terminate().await.expect("client terminate failed");

    let mut wrong = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: false,
            timeout_ms: 2000,
            basic_auth: Some(BasicAuth {
                username: "cp001".to_owned(),
                password: "wrong".to_owned(),
            }),
            tls: ClientTlsPolicy::Tls {
                verification: CertVerification::Skip {},
            },
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial error surfaces from the task");
    assert!(
        wrong.join().await.is_err(),
        "mismatched credentials should be rejected even over a self-signed TLS connection"
    );

    server.terminate().await.expect("server terminate failed");
}

// OC-R-040/OC-R-039's "Mutual with CaFiles{ca_files: []}" is rejected by `ServerTlsPolicy::
// validate()` before `build_server_config` ever runs -- see
// `ferrowl_ocpp::security::tests::ut_build_server_config_rejects_empty_ca_files`, which asserts
// the rejection (and its mapped `TlsError::EmptyCaFiles` variant) directly; no OCPP handshake-
// level test is needed since the policy is rejected before any socket work starts.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-035, OC-R-040, OC-R-166 — a `Mutual` CS identity is presented and a self-signed CSMS's `Mutual`
/// policy with a `CertVerification::CaFiles` list accepts it end-to-end over a real TLS+mTLS
/// handshake: the server's own self-signed identity and the CA trusted for verifying client
/// certificates are independent (unlike `build_server_config`'s unit-level coverage of the same
/// rule in `ferrowl-ocpp/src/security.rs`, this exercises the full CS/CSMS wire handshake).
async fn it_csms_self_signed_mutual_with_ca_files_accepts_connection() {
    let ca_key = rcgen::KeyPair::generate().expect("ca keypair generation failed");
    let ca_params =
        rcgen::CertificateParams::new(vec!["ferrowl-ocpp-test-ca".to_owned()]).expect("ca params");
    let ca_cert = ca_params.self_signed(&ca_key).expect("self-signed ca cert");
    let dir = reserve_temp_dir("ferrowl_ocpp_ws_security");
    let ca_file = write_pem(&dir, "mtls-ca", &ca_cert.pem());

    let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);
    let client_key = rcgen::KeyPair::generate().expect("client keypair generation failed");
    let client_cert = rcgen::CertificateParams::new(vec!["ferrowl-ocpp-test-client".to_owned()])
        .expect("client params")
        .signed_by(&client_key, &issuer)
        .expect("ca-signed client cert");
    let client_cert_file = write_pem(&dir, "mtls-client-cert", &client_cert.pem());
    let client_key_file = write_pem(&dir, "mtls-client-key", &client_key.serialize_pem());

    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: None,
            tls: ServerTlsPolicy::Mutual {
                identity: CertSource::SelfSigned {},
                verification: CertVerification::CaFiles {
                    ca_files: vec![ca_file],
                },
            },
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server should start: self-signed identity + Mutual with CaFiles is a valid policy");

    let url = format!("wss://{}/ocpp/CS001", bound_addr(&server).await);
    let client = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: true,
            timeout_ms: 2000,
            basic_auth: None,
            tls: ClientTlsPolicy::Mutual {
                // The server's identity is an ephemeral self-signed cert, unpinnable in advance.
                verification: CertVerification::Skip {},
                identity: CertSource::Files {
                    cert_file: client_cert_file,
                    key_file: client_key_file,
                },
            },
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    let resp = client.call(boot_action()).await.expect("boot call failed");
    assert!(matches!(resp, Response16::BootNotification(_)));

    client.terminate().await.expect("client terminate failed");
    server.terminate().await.expect("server terminate failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-039, OC-R-113 — a CSMS trusting *any one* of several configured CAs accepts a client
/// certificate signed by the second CA in the list, not just the first.
async fn it_csms_multi_ca_accepts_cert_signed_by_either_ca() {
    let ca1_key = rcgen::KeyPair::generate().expect("ca1 keypair");
    let ca1_params = rcgen::CertificateParams::new(vec!["ferrowl-ocpp-test-ca1".to_owned()])
        .expect("ca1 params");
    let dir = reserve_temp_dir("ferrowl_ocpp_ws_security");
    let ca1_file = write_pem(
        &dir,
        "multi-ca-1",
        &ca1_params
            .clone()
            .self_signed(&ca1_key)
            .expect("self-signed ca1")
            .pem(),
    );

    let ca2_key = rcgen::KeyPair::generate().expect("ca2 keypair");
    let ca2_params = rcgen::CertificateParams::new(vec!["ferrowl-ocpp-test-ca2".to_owned()])
        .expect("ca2 params");
    let ca2_cert = ca2_params
        .clone()
        .self_signed(&ca2_key)
        .expect("self-signed ca2");
    let ca2_file = write_pem(&dir, "multi-ca-2", &ca2_cert.pem());

    // The client certificate is signed by CA2 only -- CA1 is present in the trust store but
    // irrelevant to this handshake.
    let issuer = rcgen::Issuer::from_params(&ca2_params, &ca2_key);
    let client_key = rcgen::KeyPair::generate().expect("client keypair");
    let client_cert = rcgen::CertificateParams::new(vec!["ferrowl-ocpp-test-client2".to_owned()])
        .expect("client params")
        .signed_by(&client_key, &issuer)
        .expect("ca2-signed client cert");
    let client_cert_file = write_pem(&dir, "multi-ca-client-cert", &client_cert.pem());
    let client_key_file = write_pem(&dir, "multi-ca-client-key", &client_key.serialize_pem());

    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: None,
            tls: ServerTlsPolicy::Mutual {
                identity: CertSource::SelfSigned {},
                verification: CertVerification::CaFiles {
                    ca_files: vec![ca1_file, ca2_file],
                },
            },
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server should start with a multi-CA trust store");

    let url = format!("wss://{}/ocpp/CS001", bound_addr(&server).await);
    let client = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: true,
            timeout_ms: 2000,
            basic_auth: None,
            tls: ClientTlsPolicy::Mutual {
                verification: CertVerification::Skip {},
                identity: CertSource::Files {
                    cert_file: client_cert_file,
                    key_file: client_key_file,
                },
            },
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    let resp = client.call(boot_action()).await.expect("boot call failed");
    assert!(matches!(resp, Response16::BootNotification(_)));

    client.terminate().await.expect("client terminate failed");
    server.terminate().await.expect("server terminate failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-134 — server-role `SkipVerify` still requires a client certificate be presented (a
/// handshake with none fails, same as `Verify`), but performs no chain/identity validation: a
/// client presenting a self-signed identity trusted by nobody the server knows about is still
/// accepted. OC-R-115 — the client's self-signed identity is generated via the same
/// cache/regenerate rule as the server side.
async fn it_csms_skip_verify_accepts_untrusted_self_signed_client_identity() {
    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: None,
            tls: ServerTlsPolicy::Mutual {
                identity: CertSource::SelfSigned {},
                verification: CertVerification::Skip {},
            },
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server failed to bind");

    let url = format!("wss://{}/ocpp/CS001", bound_addr(&server).await);
    let client = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: true,
            timeout_ms: 2000,
            basic_auth: None,
            tls: ClientTlsPolicy::Mutual {
                verification: CertVerification::Skip {},
                identity: CertSource::SelfSigned {},
            },
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    let resp = client.call(boot_action()).await.expect("boot call failed");
    assert!(matches!(resp, Response16::BootNotification(_)));

    client.terminate().await.expect("client terminate failed");
    server.terminate().await.expect("server terminate failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-034, OC-R-130 — `CertVerification::CaFiles` trusts only the named CA, never the webpki
/// root store: a CSMS presenting a certificate signed by the named CA is trusted, and the same
/// policy against a differently self-signed CSMS (not signed by that CA, and not a publicly
/// trusted root either) is rejected.
async fn it_cs_cafiles_trusts_only_named_ca() {
    let ca_key = rcgen::KeyPair::generate().expect("ca keypair generation failed");
    let ca_params =
        rcgen::CertificateParams::new(vec!["ferrowl-ocpp-test-ca".to_owned()]).expect("ca params");
    let ca_cert = ca_params.self_signed(&ca_key).expect("self-signed ca cert");
    let dir = reserve_temp_dir("ferrowl_ocpp_ws_security");
    let ca_file = write_pem(&dir, "cafiles-ca", &ca_cert.pem());

    let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);
    let server_key = rcgen::KeyPair::generate().expect("server keypair generation failed");
    let server_cert = rcgen::CertificateParams::new(vec!["127.0.0.1".to_owned()])
        .expect("server params")
        .signed_by(&server_key, &issuer)
        .expect("ca-signed server cert");
    let server_cert_file = write_pem(&dir, "cafiles-server-cert", &server_cert.pem());
    let server_key_file = write_pem(&dir, "cafiles-server-key", &server_key.serialize_pem());

    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: None,
            tls: ServerTlsPolicy::Tls {
                identity: CertSource::Files {
                    cert_file: server_cert_file,
                    key_file: server_key_file,
                },
            },
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server failed to bind");

    let url = format!("wss://{}/ocpp/CS001", bound_addr(&server).await);
    let client = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: true,
            timeout_ms: 2000,
            basic_auth: None,
            tls: ClientTlsPolicy::Tls {
                verification: CertVerification::CaFiles {
                    ca_files: vec![ca_file],
                },
            },
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    let resp = client.call(boot_action()).await.expect("boot call failed");
    assert!(matches!(resp, Response16::BootNotification(_)));
    client.terminate().await.expect("client terminate failed");
    server.terminate().await.expect("server terminate failed");

    // A second, unrelated self-signed CSMS: the same CaFiles policy trusts only the first CA,
    // not the webpki root store, so this handshake must fail.
    let key_pair = rcgen::KeyPair::generate().expect("keypair generation failed");
    let other_cert = rcgen::CertificateParams::new(vec!["127.0.0.1".to_owned()])
        .expect("cert params")
        .self_signed(&key_pair)
        .expect("self-signed cert");
    let other_cert_file = write_pem(&dir, "cafiles-other-cert", &other_cert.pem());
    let other_key_file = write_pem(&dir, "cafiles-other-key", &key_pair.serialize_pem());
    let other_server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: None,
            tls: ServerTlsPolicy::Tls {
                identity: CertSource::Files {
                    cert_file: other_cert_file,
                    key_file: other_key_file,
                },
            },
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server failed to bind");

    let other_ca_file = write_pem(
        &dir,
        "cafiles-ca-2",
        &rcgen::CertificateParams::new(vec!["ferrowl-ocpp-test-ca-2".to_owned()])
            .expect("ca params")
            .self_signed(&rcgen::KeyPair::generate().expect("ca2 keypair"))
            .expect("self-signed ca2 cert")
            .pem(),
    );
    let other_url = format!("wss://{}/ocpp/CS001", bound_addr(&other_server).await);
    let mut untrusted = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url: other_url,
            reconnect: false,
            timeout_ms: 2000,
            basic_auth: None,
            tls: ClientTlsPolicy::Tls {
                verification: CertVerification::CaFiles {
                    ca_files: vec![other_ca_file],
                },
            },
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");
    assert!(
        untrusted.join().await.is_err(),
        "a server cert not signed by the named CA (and not a webpki root) must be rejected"
    );
    other_server
        .terminate()
        .await
        .expect("server terminate failed");
}

#[tokio::test]
/// OC-R-095 — a `wss://` CSMS bound with no TLS material configured logs the ephemeral-identity
/// fallback, not merely flags it internally.
async fn it_wss_csms_without_tls_material_logs_the_fallback() {
    let (sink, lines) = capturing_sink();
    let server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: None,
            tls: ServerTlsPolicy::Tls {
                identity: CertSource::Ephemeral {},
            },
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink)
    .await
    .expect("server failed to bind");

    {
        let captured = lines.lock().expect("not poisoned");
        assert!(
            captured
                .iter()
                .any(|l| l.contains("No cert_file/key_file/self_signed configured")),
            "fallback was not logged: {captured:?}"
        );
    }

    server.terminate().await.expect("server terminate failed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
/// OC-R-097 — a `ws://` CS endpoint carrying a `ClientTlsPolicy` that would fail validation
/// (empty `CertVerification::CaFiles`) connects in plaintext regardless, because the scheme
/// decides the transport and the material is inert; the same policy on a `wss://` endpoint,
/// against a real TLS-terminating CSMS, still fails with the policy-validation error itself.
async fn it_ws_endpoint_skips_client_tls_validation() {
    let plain_server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: None,
            tls: Default::default(),
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server failed to bind");

    let failing_policy = ClientTlsPolicy::Tls {
        verification: CertVerification::CaFiles { ca_files: vec![] },
    };

    let url = format!("ws://{}/ocpp/CS001", bound_addr(&plain_server).await);
    let client = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url,
            reconnect: false,
            timeout_ms: 2000,
            basic_auth: None,
            tls: failing_policy.clone(),
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");

    let resp = client
        .call(boot_action())
        .await
        .expect("ws:// must connect and answer despite the inert, invalid tls policy");
    assert!(matches!(resp, Response16::BootNotification(_)));
    client.terminate().await.expect("client terminate failed");
    plain_server
        .terminate()
        .await
        .expect("server terminate failed");

    // This CSMS terminates TLS, so only policy validation (not a handshake failure) can
    // produce the error asserted below.
    let tls_server = csms::ServerBuilder::<V1_6>::new(
        csms::Config {
            host: "127.0.0.1".to_owned(),
            port: 0,
            timeout_ms: 2000,
            reconnect: true,
            basic_auth: None,
            tls: ServerTlsPolicy::Tls {
                identity: CertSource::Ephemeral {},
            },
        },
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCsms, sink())
    .await
    .expect("server failed to bind");

    let wss_url = format!("wss://{}/ocpp/CS001", bound_addr(&tls_server).await);
    let mut wss_client = cs::ClientBuilder::<V1_6>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cs::Config {
            extra_headers: Vec::new(),
            url: wss_url,
            reconnect: false,
            timeout_ms: 2000,
            basic_auth: None,
            tls: failing_policy,
        })),
        ferrowl_ocpp::new_self_signed_cache(),
    )
    .spawn(TestCs, sink(), sink())
    .await
    .expect("spawn always returns Ok; the dial happens inside the task");
    let err = wss_client
        .join()
        .await
        .expect_err("wss:// must still validate and reject an empty CaFiles list");
    assert!(
        matches!(
            err,
            ferrowl_ocpp::Error::Tls(ferrowl_ocpp::TlsError::EmptyCaFiles)
        ),
        "expected the EmptyCaFiles policy-validation error, got {err:?}"
    );

    tls_server
        .terminate()
        .await
        .expect("server terminate failed");
}
