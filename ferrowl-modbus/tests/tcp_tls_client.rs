//! Client-side Modbus/TCP TLS tests (MB-R-109, MB-R-110, MB-R-111 client half).
//!
//! Drives `ferrowl_modbus::tcp::Client::connect` against a bare `rust_modbus`
//! TLS/TCP listener, never `ferrowl_modbus`'s own server (covered by tcp_tls_server.rs).

// Integration-test crate: an unwrap that fails is the test failing, same as an assertion.
#![allow(clippy::unwrap_used)]

use std::sync::atomic::Ordering;

use ferrowl_modbus::tcp;
use ferrowl_modbus::{Command, SlaveKey};
use ferrowl_test_support::{TempDirGuard, reserve_tcp_port, reserve_temp_dir};
use ferrowl_util::tls::{CertSource, CertVerification, ClientTlsPolicy};
use rcgen::{CertificateParams, Issuer, KeyPair};
use rust_modbus::{
    ClientCertPolicy, RootStore, ServerCertVerification, TcpConfig, TlsClientConfig, TlsListener,
    TlsServerConfig, connect_tls,
};
use tokio::io::AsyncReadExt;

fn write_pem(dir: &TempDirGuard, label: &str, pem: &str) -> String {
    let path = dir.join(format!("{label}.pem"));
    std::fs::write(&path, pem).expect("failed to write test PEM file");
    path.to_string_lossy().into_owned()
}

/// A self-signed cert/key pair, PEM-encoded, CN `127.0.0.1`.
fn self_signed_pem() -> (String, String) {
    let key = KeyPair::generate().expect("keypair generation failed");
    let cert = CertificateParams::new(vec!["127.0.0.1".to_owned()])
        .expect("cert params")
        .self_signed(&key)
        .expect("self-signed cert");
    (cert.pem(), key.serialize_pem())
}

/// A self-signed CA plus one client cert/key pair it signs, all PEM-encoded.
fn ca_and_signed_client_pem() -> (String, String, String) {
    let ca_key = KeyPair::generate().expect("ca keypair generation failed");
    let ca_params = CertificateParams::new(vec!["ferrowl-test-ca".to_owned()]).expect("ca params");
    let ca_cert = ca_params.self_signed(&ca_key).expect("self-signed ca cert");
    let issuer = Issuer::from_params(&ca_params, &ca_key);

    let client_key = KeyPair::generate().expect("client keypair generation failed");
    let client_cert = CertificateParams::new(vec!["ferrowl-test-client".to_owned()])
        .expect("client params")
        .signed_by(&client_key, &issuer)
        .expect("ca-signed client cert");

    (ca_cert.pem(), client_cert.pem(), client_key.serialize_pem())
}

/// A self-signed CA plus one server-leaf cert/key pair it signs for `127.0.0.1`, all
/// PEM-encoded — used where the leaf must pass hostname verification against a loopback
/// connect address, unlike [`ca_and_signed_client_pem`]'s client-identity leaf.
fn ca_and_signed_server_pem() -> (String, String, String) {
    let ca_key = KeyPair::generate().expect("ca keypair generation failed");
    let ca_params = CertificateParams::new(vec!["ferrowl-test-ca".to_owned()]).expect("ca params");
    let ca_cert = ca_params.self_signed(&ca_key).expect("self-signed ca cert");
    let issuer = Issuer::from_params(&ca_params, &ca_key);

    let server_key = KeyPair::generate().expect("server keypair generation failed");
    let server_cert = CertificateParams::new(vec!["127.0.0.1".to_owned()])
        .expect("server params")
        .signed_by(&server_key, &issuer)
        .expect("ca-signed server cert");

    (ca_cert.pem(), server_cert.pem(), server_key.serialize_pem())
}

fn config(port: u16, tls: tcp::ModbusTlsConfig) -> tcp::Config {
    tcp::Config {
        ip: "127.0.0.1".to_string(),
        port,
        timeout_ms: 1000,
        delay_ms: 0,
        interval_ms: 0,
        reconnect: true,
        tls,
    }
}

#[tokio::test]
/// MB-R-109 — `CertVerification::Skip` accepts a server certificate unauthenticated,
/// with no trust anchor needed.
async fn tls_client_connects_to_plain_rust_modbus_tls_server() {
    let (cert_pem, key_pem) = self_signed_pem();
    let cert_chain = rust_modbus::load_pem_cert_chain(cert_pem.as_bytes()).unwrap();
    let key = rust_modbus::load_pem_private_key(key_pem.as_bytes()).unwrap();

    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", reserve_tcp_port().release())
        .parse()
        .unwrap();
    let listener = TlsListener::bind(
        addr,
        TlsServerConfig {
            cert_chain,
            key,
            client_certs: ClientCertPolicy::None,
        },
    )
    .await
    .unwrap();
    let bound = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let _ = listener.accept().await;
    });

    let cfg = config(
        bound.port(),
        tcp::ModbusTlsConfig {
            client: ClientTlsPolicy::Tls {
                verification: CertVerification::Skip {},
            },
            ..Default::default()
        },
    );
    let client = tcp::Client::connect(&cfg, &tcp::new_self_signed_cache()).await;
    assert!(
        client.is_ok(),
        "expected connect to succeed: {}",
        client.err().map(|e| e.to_string()).unwrap_or_default()
    );

    server.abort();
}

#[tokio::test]
/// MB-R-109 — `CertVerification::RootStore` trusts a server certificate presented via
/// `extra_ca_files`, and rejects the same certificate with neither `extra_ca_files` nor
/// `Skip` set (i.e. the default `RootStore` with an empty list, against a non-CA-signed cert).
async fn it_tls_client_root_store_trusts_extra_ca() {
    let dir = reserve_temp_dir("ferrowl_modbus_tcp_tls_client");
    let (cert_pem, key_pem) = self_signed_pem();
    let cert_file = write_pem(&dir, "server-cert", &cert_pem);
    let cert_chain = rust_modbus::load_pem_cert_chain(cert_pem.as_bytes()).unwrap();
    let key = rust_modbus::load_pem_private_key(key_pem.as_bytes()).unwrap();

    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", reserve_tcp_port().release())
        .parse()
        .unwrap();
    let listener = TlsListener::bind(
        addr,
        TlsServerConfig {
            cert_chain,
            key,
            client_certs: ClientCertPolicy::None,
        },
    )
    .await
    .unwrap();
    let bound = listener.local_addr().unwrap();
    let listener = std::sync::Arc::new(listener);

    let accept_listener = listener.clone();
    let server = tokio::spawn(async move {
        loop {
            if accept_listener.accept().await.is_err() {
                break;
            }
        }
    });

    // Trusted via extra_ca_files.
    let trusted_cfg = config(
        bound.port(),
        tcp::ModbusTlsConfig {
            client: ClientTlsPolicy::Tls {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec![cert_file],
                },
            },
            ..Default::default()
        },
    );
    let trusted = tcp::Client::connect(&trusted_cfg, &tcp::new_self_signed_cache()).await;
    assert!(
        trusted.is_ok(),
        "expected trusted connect: {}",
        trusted.err().map(|e| e.to_string()).unwrap_or_default()
    );

    // Untrusted: RootStore with an empty extra list, and native roots don't know this
    // self-signed cert either.
    let untrusted_cfg = config(
        bound.port(),
        tcp::ModbusTlsConfig {
            client: ClientTlsPolicy::Tls {
                verification: CertVerification::RootStore {
                    extra_ca_files: vec![],
                },
            },
            ..Default::default()
        },
    );
    let untrusted = tcp::Client::connect(&untrusted_cfg, &tcp::new_self_signed_cache()).await;
    assert!(
        untrusted.is_err(),
        "expected untrusted connect to fail verification"
    );

    server.abort();
}

#[tokio::test]
/// MB-R-109 — `CertVerification::CaFiles` trusts only the named CA(s), never the native root
/// store: a server presenting a cert signed by the named CA is accepted, and a differently
/// self-signed server (not signed by that CA, and not in the native roots either) is rejected.
async fn it_tls_client_cafiles_trusts_only_named_ca() {
    let dir = reserve_temp_dir("ferrowl_modbus_tcp_tls_client");
    let (ca_pem, signed_cert_pem, signed_key_pem) = ca_and_signed_server_pem();
    let ca_file = write_pem(&dir, "client-cafiles-ca", &ca_pem);
    let signed_chain = rust_modbus::load_pem_cert_chain(signed_cert_pem.as_bytes()).unwrap();
    let signed_key = rust_modbus::load_pem_private_key(signed_key_pem.as_bytes()).unwrap();

    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", reserve_tcp_port().release())
        .parse()
        .unwrap();
    let listener = TlsListener::bind(
        addr,
        TlsServerConfig {
            cert_chain: signed_chain,
            key: signed_key,
            client_certs: ClientCertPolicy::None,
        },
    )
    .await
    .unwrap();
    let bound = listener.local_addr().unwrap();
    let listener = std::sync::Arc::new(listener);
    let accept_listener = listener.clone();
    let server = tokio::spawn(async move {
        loop {
            if accept_listener.accept().await.is_err() {
                break;
            }
        }
    });

    let trusted_cfg = config(
        bound.port(),
        tcp::ModbusTlsConfig {
            client: ClientTlsPolicy::Tls {
                verification: CertVerification::CaFiles {
                    ca_files: vec![ca_file],
                },
            },
            ..Default::default()
        },
    );
    let trusted = tcp::Client::connect(&trusted_cfg, &tcp::new_self_signed_cache()).await;
    assert!(
        trusted.is_ok(),
        "expected CA-signed server cert to be trusted: {}",
        trusted.err().map(|e| e.to_string()).unwrap_or_default()
    );
    server.abort();

    // A second, unrelated self-signed server: the same CaFiles policy trusts only the first
    // CA, not the native root store, so this handshake must fail.
    let (other_cert_pem, other_key_pem) = self_signed_pem();
    let other_chain = rust_modbus::load_pem_cert_chain(other_cert_pem.as_bytes()).unwrap();
    let other_key = rust_modbus::load_pem_private_key(other_key_pem.as_bytes()).unwrap();
    let other_addr: std::net::SocketAddr = format!("127.0.0.1:{}", reserve_tcp_port().release())
        .parse()
        .unwrap();
    let other_listener = TlsListener::bind(
        other_addr,
        TlsServerConfig {
            cert_chain: other_chain,
            key: other_key,
            client_certs: ClientCertPolicy::None,
        },
    )
    .await
    .unwrap();
    let other_bound = other_listener.local_addr().unwrap();
    let other_server = tokio::spawn(async move {
        let _ = other_listener.accept().await;
    });

    let ca_file_only = ca_and_signed_server_pem().0;
    let ca_file_only = write_pem(&dir, "client-cafiles-untrusted-ca", &ca_file_only);
    let untrusted_cfg = config(
        other_bound.port(),
        tcp::ModbusTlsConfig {
            client: ClientTlsPolicy::Tls {
                verification: CertVerification::CaFiles {
                    ca_files: vec![ca_file_only],
                },
            },
            ..Default::default()
        },
    );
    let untrusted = tcp::Client::connect(&untrusted_cfg, &tcp::new_self_signed_cache()).await;
    assert!(
        untrusted.is_err(),
        "a server cert not signed by the named CA (and not a native root) must be rejected"
    );
    other_server.abort();
}

#[tokio::test]
/// MB-R-110, MB-R-138 — under `ClientTlsPolicy::Mutual` with a `CertSource::SelfSigned`
/// identity, a client presents its cached ephemeral identity and a server requiring one
/// (`ClientCertPolicy::Require`, trusting exactly that self-signed cert) accepts the handshake;
/// the same server rejects a `Tls`-policy client, which presents no identity at all.
async fn it_tls_client_presents_self_signed_identity() {
    let (server_cert_pem, server_key_pem) = self_signed_pem();
    let server_cert_chain = rust_modbus::load_pem_cert_chain(server_cert_pem.as_bytes()).unwrap();
    let server_key = rust_modbus::load_pem_private_key(server_key_pem.as_bytes()).unwrap();

    // Seed the client cache with a known self-signed pair *before* connecting: `CertSource::
    // SelfSigned`'s cache-hit path (MB-R-106/MB-R-138) returns exactly whatever the cache holds,
    // so this makes the presented identity's certificate known ahead of time, letting the
    // server's `Require` trust it specifically rather than needing `AllowAny`/`Skip`, which
    // would not distinguish "an identity was presented" from "any cert would have been fine".
    let (client_cert_pem, client_key_pem) = self_signed_pem();
    let client_cert_chain = rust_modbus::load_pem_cert_chain(client_cert_pem.as_bytes()).unwrap();
    let client_key = rust_modbus::load_pem_private_key(client_key_pem.as_bytes()).unwrap();
    let seeded_cache: tcp::SelfSignedCache = tcp::new_self_signed_cache();
    *seeded_cache.lock() = Some((client_cert_chain, client_key));

    let mut trust_client_cert = RootStore::empty();
    trust_client_cert
        .add_pem(client_cert_pem.as_bytes())
        .unwrap();

    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", reserve_tcp_port().release())
        .parse()
        .unwrap();
    let listener = TlsListener::bind(
        addr,
        TlsServerConfig {
            cert_chain: server_cert_chain,
            key: server_key,
            client_certs: ClientCertPolicy::Require(trust_client_cert),
        },
    )
    .await
    .unwrap();
    let bound = listener.local_addr().unwrap();
    let listener = std::sync::Arc::new(listener);
    let accept_listener = listener.clone();
    // Counts only handshakes the *server* completed successfully: a handshake `Require`
    // rejects surfaces as `Err` from `accept()`, so this counter records the server's own
    // verdict rather than the client-side read of its own handshake future (which the TLS-1.3
    // "client completes regardless of the server's verdict" quirk can make misleading, per
    // `it_tls_client_with_no_identity_rejected_by_require`'s doc comment).
    let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let accepted_by_server = accepted.clone();
    let server = tokio::spawn(async move {
        while accept_listener.accept().await.is_ok() {
            accepted_by_server.fetch_add(1, Ordering::SeqCst);
        }
    });

    // `Mutual` presents the seeded self-signed identity, which `Require` trusts by name.
    let mutual_cfg = config(
        bound.port(),
        tcp::ModbusTlsConfig {
            client: ClientTlsPolicy::Mutual {
                verification: CertVerification::Skip {},
                identity: CertSource::SelfSigned {},
            },
            ..Default::default()
        },
    );
    let mutual = tcp::Client::connect(&mutual_cfg, &seeded_cache).await;
    assert!(
        mutual.is_ok(),
        "expected mTLS connect with a self-signed identity to succeed: {}",
        mutual.err().map(|e| e.to_string()).unwrap_or_default()
    );
    // The client-side `Ok` alone doesn't prove the server validated the certificate (see the
    // quirk note above); the server's own accept count does. `accept()`'s task may not have
    // recorded it yet even though `connect()` already returned, so poll to a deadline instead
    // of guessing a fixed delay.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while accepted.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        1,
        "the server must have completed and validated the mTLS handshake, not just the client side"
    );

    // `Tls` presents no identity at all against the same `Require` listener: the handshake
    // future can still resolve `Ok` on the client side (see `it_tls_client_with_no_identity_
    // rejected_by_require`'s doc comment for the TLS-1.3 quirk this works around), so rejection
    // is observed on the wire instead, as the server closing the connection before any
    // application data arrives.
    let mut trust_server_cert = RootStore::empty();
    trust_server_cert
        .add_pem(server_cert_pem.as_bytes())
        .unwrap();
    let no_identity_tls = TlsClientConfig {
        server_cert: ServerCertVerification::Verify(trust_server_cert),
        client_identity: None,
    };
    let transport = connect_tls(addr, TcpConfig::default(), no_identity_tls)
        .await
        .expect("client-side TLS 1.3 handshake completes even though the server will reject");
    let mut stream = transport.into_inner();
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(std::time::Duration::from_millis(500), stream.read(&mut buf))
        .await
        .expect("server should close the connection promptly, not hang");
    if let Ok(n) = read {
        assert_eq!(
            n, 0,
            "expected EOF: Require must reject a Tls-policy client presenting no identity"
        );
    }

    server.abort();
}

#[tokio::test]
/// MB-R-108 — `Mutual` with `Require` rejects a client presenting no certificate at all,
/// distinct from the self-signed-identity acceptance case above.
async fn it_tls_client_with_no_identity_rejected_by_require() {
    let dir = reserve_temp_dir("ferrowl_modbus_tcp_tls_client");
    let (server_cert_pem, server_key_pem) = self_signed_pem();
    let server_cert_file = write_pem(&dir, "mtls-require-server-cert", &server_cert_pem);
    let server_cert_chain = rust_modbus::load_pem_cert_chain(server_cert_pem.as_bytes()).unwrap();
    let server_key = rust_modbus::load_pem_private_key(server_key_pem.as_bytes()).unwrap();

    let (ca_pem, _client_cert_pem, _client_key_pem) = ca_and_signed_client_pem();
    let mut roots = RootStore::empty();
    roots.add_pem(ca_pem.as_bytes()).unwrap();

    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", reserve_tcp_port().release())
        .parse()
        .unwrap();
    let listener = TlsListener::bind(
        addr,
        TlsServerConfig {
            cert_chain: server_cert_chain,
            key: server_key,
            client_certs: ClientCertPolicy::Require(roots),
        },
    )
    .await
    .unwrap();
    let bound = listener.local_addr().unwrap();
    let listener = std::sync::Arc::new(listener);
    let accept_listener = listener.clone();
    let server = tokio::spawn(async move {
        loop {
            if accept_listener.accept().await.is_err() {
                break;
            }
        }
    });

    // TLS 1.3 quirk (confirmed empirically): the *client's* handshake future resolves
    // successfully even when the server will reject the connection for a missing/invalid
    // client certificate — client-side completion doesn't wait for the server's verdict on
    // the client's certificate message. The rejection is only observable on the wire
    // afterwards, as the server closing the connection before sending any application data.
    let mut server_trust = RootStore::empty();
    server_trust.add_pem(server_cert_pem.as_bytes()).unwrap();
    let no_identity_tls = TlsClientConfig {
        server_cert: ServerCertVerification::Verify(server_trust),
        client_identity: None,
    };
    let addr: std::net::SocketAddr = format!("127.0.0.1:{}", bound.port()).parse().unwrap();
    let transport = connect_tls(addr, TcpConfig::default(), no_identity_tls)
        .await
        .expect("client-side TLS 1.3 handshake completes even though the server will reject");
    let mut stream = transport.into_inner();
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(std::time::Duration::from_millis(500), stream.read(&mut buf))
        .await
        .expect("server should close the connection promptly, not hang");
    if let Ok(n) = read {
        assert_eq!(
            n, 0,
            "expected EOF (server closed after rejecting no identity)"
        );
    }

    drop(server_cert_file);
    server.abort();
}

#[tokio::test]
/// MB-R-111 (client half) — a TLS handshake failure, a refused connection, and a
/// timed-out connection attempt are three distinct outcomes.
async fn tls_handshake_failure_is_distinct_from_refused_and_timeout() {
    // (a) TLS configured against a plain (non-TLS) listener: handshake failure.
    let plain_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let plain_addr = plain_listener.local_addr().unwrap();
    let plain_server = tokio::spawn(async move {
        loop {
            if plain_listener.accept().await.is_err() {
                break;
            }
        }
    });
    let handshake_cfg = config(
        plain_addr.port(),
        tcp::ModbusTlsConfig {
            client: ClientTlsPolicy::Tls {
                verification: CertVerification::Skip {},
            },
            ..Default::default()
        },
    );
    let handshake_result =
        tcp::Client::connect(&handshake_cfg, &tcp::new_self_signed_cache()).await;
    assert!(
        handshake_result.is_err(),
        "expected TLS handshake against a plain listener to fail"
    );
    plain_server.abort();

    // (b) Nothing listening: connection refused.
    let refused_port = reserve_tcp_port().release();
    let refused_cfg = config(
        refused_port,
        tcp::ModbusTlsConfig {
            client: ClientTlsPolicy::Tls {
                verification: CertVerification::Skip {},
            },
            ..Default::default()
        },
    );
    let refused_result = tcp::Client::connect(&refused_cfg, &tcp::new_self_signed_cache()).await;
    assert!(
        refused_result.is_err(),
        "expected connect to a closed port to fail"
    );

    // (c) A TLS server that accepts the TCP connection but never completes the
    // handshake: a very short client timeout should time out, not hang.
    let stalling_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stalling_addr = stalling_listener.local_addr().unwrap();
    let stalling_server = tokio::spawn(async move {
        let (socket, _) = stalling_listener.accept().await.unwrap();
        // Accept the TCP connection, then simply hold it open without ever speaking
        // TLS, forcing the client's handshake to stall until it hits its timeout.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        drop(socket);
    });
    let mut timeout_cfg = config(
        stalling_addr.port(),
        tcp::ModbusTlsConfig {
            client: ClientTlsPolicy::Tls {
                verification: CertVerification::Skip {},
            },
            ..Default::default()
        },
    );
    timeout_cfg.timeout_ms = 50;
    let timeout_result = tcp::Client::connect(&timeout_cfg, &tcp::new_self_signed_cache()).await;
    assert!(
        timeout_result.is_err(),
        "expected a stalled handshake to time out"
    );
    stalling_server.abort();
}

#[tokio::test]
/// MB-R-220, MB-E-089 — a terminate arriving while a client's TLS handshake is stalled against
/// an unresponsive peer aborts the attempt at once and ends the client task with success, well
/// inside the configured 30 s timeout rather than after it elapses.
async fn it_terminate_during_tls_handshake_returns_immediately() {
    let stalling_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stalling_addr = stalling_listener.local_addr().unwrap();
    let stalling_server = tokio::spawn(async move {
        let (socket, _) = stalling_listener.accept().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        drop(socket);
    });

    let mut cfg = config(
        stalling_addr.port(),
        tcp::ModbusTlsConfig {
            client: ClientTlsPolicy::Tls {
                verification: CertVerification::Skip {},
            },
            ..Default::default()
        },
    );
    cfg.timeout_ms = 30_000;

    let (_sender, receiver) = tokio::sync::mpsc::channel::<Command>(1);
    let builder = tcp::ClientBuilder::<SlaveKey>::new(
        std::sync::Arc::new(tokio::sync::RwLock::new(cfg)),
        std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        std::sync::Arc::new(parking_lot::RwLock::new(ferrowl_store::Memory::default())),
        tcp::new_self_signed_cache(),
    );
    let (handle, _connected) = builder
        .spawn(
            receiver,
            |_s: String| async move {},
            |_s: String| async move {},
        )
        .await
        .expect("spawn always returns Ok");

    _sender.send(Command::Terminate).await.unwrap();

    let result = tokio::time::timeout(std::time::Duration::from_millis(500), handle)
        .await
        .expect("terminate during a stalled TLS handshake must not wait for the 30s timeout")
        .expect("task must not panic");
    assert!(result.is_ok(), "the client task must end successfully");
    stalling_server.abort();
}
