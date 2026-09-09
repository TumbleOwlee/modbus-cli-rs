use crate::LogFn;
use rust_modbus::{
    Client, ClientFraming, ClientTransport, ExceptionCode, RequestPdu, ResponsePdu, UnitId,
};
use std::future::Future;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

/// Log-line prefix for a genuine bridge failure (BR-R-013's "`[bridge]`-sourced error
/// line", mirroring headless run's `[sim]`).
pub const ERROR_PREFIX: &str = "[bridge]";

/// Owns one downstream connection (BR-R-006) and answers forwarded requests (BR-R-007)
/// while reconnecting in the background on failure (BR-R-010, mirrors MB-R-050–056).
/// Cloning shares the same underlying connection state — every clone forwards onto the
/// same physical link.
pub struct DownstreamHandle<S, F> {
    state: Arc<Mutex<Option<Client<S, F>>>>,
    reconnect_needed: Arc<Notify>,
}

// Written by hand rather than `#[derive(Clone)]`: a derive would add `S: Clone, F: Clone`
// bounds even though only the `Arc`s are ever cloned — neither the connected transport nor
// the framing marker needs to be `Clone` for this to be sound.
impl<S, F> Clone for DownstreamHandle<S, F> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            reconnect_needed: self.reconnect_needed.clone(),
        }
    }
}

impl<S, F> DownstreamHandle<S, F>
where
    S: ClientTransport<F> + Send + 'static,
    F: ClientFraming + Send + 'static,
{
    /// Spawns the background reconnector and returns a handle. `connect` is retried with
    /// doubling backoff (`ferrowl_util::backoff::BackoffPolicy::default()`'s `initial`..=`max`,
    /// MB-R-051's exact rule) whenever the link is down and `reconnect` is true; with
    /// `reconnect` false a lost/failed
    /// connection is never retried. `log` receives lifecycle
    /// lines (unprefixed) and failure lines (prefixed `ERROR_PREFIX`).
    pub fn spawn<C, Fut, L>(mut connect: C, reconnect: bool, log: L) -> Self
    where
        C: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = Result<Client<S, F>, crate::Error>> + Send,
        L: LogFn + Clone + Send + 'static,
    {
        let state: Arc<Mutex<Option<Client<S, F>>>> = Arc::new(Mutex::new(None));
        let reconnect_needed = Arc::new(Notify::new());
        let handle = Self {
            state: state.clone(),
            reconnect_needed: reconnect_needed.clone(),
        };
        let task_reconnect_needed = reconnect_needed.clone();
        tokio::spawn(async move {
            loop {
                task_reconnect_needed.notified().await;
                let policy = ferrowl_util::backoff::BackoffPolicy::default();
                let mut backoff = policy.initial;
                loop {
                    match connect().await {
                        Ok(client) => {
                            log.invoke("downstream connected".to_string()).await;
                            *state.lock().await = Some(client);
                            break;
                        }
                        Err(e) => {
                            log.invoke(format!(
                                "{ERROR_PREFIX} downstream connect failed: {e}. Reconnecting in {}s.",
                                backoff.as_secs()
                            ))
                            .await;
                            if !reconnect {
                                // A connect failure with reconnect disabled never retries: the
                                // task ends here, so a later notify_one() (from a subsequent
                                // forward() failure) has no listener and is silently absorbed.
                                return;
                            }
                            tokio::time::sleep(backoff).await;
                            backoff = (backoff * 2).min(policy.max);
                        }
                    }
                }
                if !reconnect {
                    // Connected once; an exchange failure afterwards must not retry either —
                    // end the task rather than waiting for another notify_one().
                    return;
                }
            }
        });
        reconnect_needed.notify_one(); // BR-R-006 — connect immediately at startup.
        handle
    }

    /// Forward one decoded upstream request (BR-R-007). `None` state (never connected yet,
    /// or reconnecting after a failure) answers `GatewayPathUnavailable` immediately —
    /// never blocks waiting for a reconnect (edge-cases.md: "a request arriving upstream
    /// during downstream backoff gets the BR-R-010 exception rather than blocking
    /// indefinitely"). A connected client that fails the exchange (timeout or the
    /// connection dropping mid-request) answers `GatewayTargetDeviceFailedToRespond`, drops
    /// the now-desynchronized client, and wakes the reconnector.
    pub async fn forward(
        &self,
        unit: UnitId,
        request: RequestPdu,
    ) -> Result<Option<ResponsePdu>, ExceptionCode> {
        let mut guard = self.state.lock().await;
        match guard.as_mut() {
            None => Err(ExceptionCode::GatewayPathUnavailable),
            Some(client) => match client.call(unit, request).await {
                Ok(resp) => Ok(resp),
                Err(_e) => {
                    *guard = None;
                    drop(guard);
                    self.reconnect_needed.notify_one();
                    Err(ExceptionCode::GatewayTargetDeviceFailedToRespond)
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_modbus::{
        Address, Client as RustClient, ClientConfig, FrameTransport, Framing, Quantity,
        RegisterValue, Rtu,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

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

    /// BR-R-010 — before the first connect succeeds, `forward` answers
    /// `GatewayPathUnavailable` immediately, never blocking on the pending connect.
    #[tokio::test]
    async fn ut_forward_before_first_connect_is_gateway_path_unavailable() {
        let (log, _lines) = recording_log();
        let handle: DownstreamHandle<FrameTransport<DuplexStream, Rtu>, Rtu> =
            DownstreamHandle::spawn(
                || async {
                    std::future::pending::<
                        Result<RustClient<FrameTransport<DuplexStream, Rtu>, Rtu>, crate::Error>,
                    >()
                    .await
                },
                true,
                log,
            );

        let result = tokio::time::timeout(
            Duration::from_millis(200),
            handle.forward(
                UnitId(1),
                RequestPdu::ReadHoldingRegisters {
                    address: Address(0),
                    quantity: Quantity(1),
                },
            ),
        )
        .await
        .expect("forward must not block");

        assert_eq!(result, Err(ExceptionCode::GatewayPathUnavailable));
    }

    /// BR-R-007 — a successful downstream exchange relays the response unmodified.
    #[tokio::test]
    async fn ut_forward_success_relays_response_unmodified() {
        let (client_end, mut peer) = tokio::io::duplex(256);
        let (log, _lines) = recording_log();
        let handle: DownstreamHandle<FrameTransport<DuplexStream, Rtu>, Rtu> =
            DownstreamHandle::spawn(
                {
                    let mut client_end = Some(client_end);
                    move || {
                        let client_end = client_end.take();
                        async move {
                            Ok(RustClient::new(FrameTransport::<_, Rtu>::new(
                                client_end.expect("connect called once"),
                            )))
                        }
                    }
                },
                true,
                log,
            );

        // Let the reconnector connect before forwarding.
        tokio::time::sleep(Duration::from_millis(20)).await;

        let expected = ResponsePdu::ReadHoldingRegisters {
            registers: vec![RegisterValue(42)],
        };
        let respond = {
            let expected = expected.clone();
            async move {
                let mut buf = [0u8; 64];
                let n = peer.read(&mut buf).await.unwrap();
                let frame = Rtu::encode_response(&UnitId(1), &expected).unwrap();
                let _ = n;
                peer.write_all(&frame).await.unwrap();
            }
        };
        let (result, _) = tokio::join!(
            handle.forward(
                UnitId(1),
                RequestPdu::ReadHoldingRegisters {
                    address: Address(0),
                    quantity: Quantity(1),
                },
            ),
            respond
        );

        assert_eq!(result, Ok(Some(expected)));
    }

    /// BR-R-018, BR-R-006 — a downstream timeout answers `GatewayTargetDeviceFailedToRespond`
    /// and wakes the reconnector for a second connect attempt (mirrors MB-R-050–056's
    /// backoff-driven reconnect); a subsequent connect failure logs a `[bridge]`-prefixed
    /// failure line (`forward` itself carries no `log`, only the reconnector does — see
    /// `DownstreamHandle::spawn`).
    #[tokio::test]
    async fn ut_forward_timeout_answers_target_failed_and_triggers_reconnect() {
        let (log, lines) = recording_log();
        let connects = Arc::new(AtomicUsize::new(0));
        let handle: DownstreamHandle<FrameTransport<DuplexStream, Rtu>, Rtu> = {
            let connects = connects.clone();
            DownstreamHandle::spawn(
                move || {
                    let attempt = connects.fetch_add(1, Ordering::SeqCst) + 1;
                    async move {
                        if attempt == 2 {
                            // The reconnect attempt (woken by the forward failure below)
                            // fails once, producing a [bridge]-prefixed log line.
                            return Err(crate::Error::Modbus(crate::ModbusError::Exception(
                                ExceptionCode::GatewayPathUnavailable,
                            )));
                        }
                        let (client_end, _peer_never_answers) = tokio::io::duplex(256);
                        Ok(RustClient::with_config(
                            FrameTransport::<_, Rtu>::new(client_end),
                            ClientConfig {
                                response_timeout: Duration::from_millis(50),
                            },
                        ))
                    }
                },
                true,
                log,
            )
        };

        tokio::time::sleep(Duration::from_millis(20)).await;

        let result = handle
            .forward(
                UnitId(1),
                RequestPdu::ReadHoldingRegisters {
                    address: Address(0),
                    quantity: Quantity(1),
                },
            )
            .await;

        assert_eq!(
            result,
            Err(ExceptionCode::GatewayTargetDeviceFailedToRespond)
        );

        // Give the woken reconnector time to attempt (and fail) a second connect.
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert!(
            connects.load(Ordering::SeqCst) >= 2,
            "expected the reconnector to attempt a second connect"
        );
        assert!(
            lines.lock().iter().any(|l| l.starts_with(ERROR_PREFIX)),
            "expected a [bridge]-prefixed failure line: {:?}",
            lines.lock()
        );
    }

    /// BR-R-009 — a broadcast forwarded to the downstream is fire-and-forget: `forward`
    /// returns `Ok(None)` promptly, without waiting for (and without classifying the absence
    /// of) a response.
    #[tokio::test]
    async fn ut_forward_broadcast_downstream_is_fire_and_forget() {
        let (client_end, _peer_never_answers) = tokio::io::duplex(256);
        let (log, _lines) = recording_log();
        let handle: DownstreamHandle<FrameTransport<DuplexStream, Rtu>, Rtu> = {
            let mut client_end = Some(client_end);
            DownstreamHandle::spawn(
                move || {
                    let client_end = client_end.take();
                    async move {
                        Ok(RustClient::new(FrameTransport::<_, Rtu>::new(
                            client_end.expect("connect called once"),
                        )))
                    }
                },
                true,
                log,
            )
        };

        tokio::time::sleep(Duration::from_millis(20)).await;

        let result = tokio::time::timeout(
            Duration::from_millis(200),
            handle.forward(
                UnitId(0),
                RequestPdu::WriteSingleRegister {
                    address: Address(0),
                    value: RegisterValue(1),
                },
            ),
        )
        .await
        .expect("broadcast forward must not block");

        assert_eq!(result, Ok(None));
    }

    /// BR-E-004, BR-R-010 — with `reconnect = false`, a downstream connect failure is
    /// never retried: every subsequent `forward` answers `GatewayPathUnavailable` and the
    /// `connect` closure is invoked only once.
    #[tokio::test]
    async fn ut_reconnect_false_never_retries() {
        let (log, _lines) = recording_log();
        let connects = Arc::new(AtomicUsize::new(0));
        let handle: DownstreamHandle<FrameTransport<DuplexStream, Rtu>, Rtu> = {
            let connects = connects.clone();
            DownstreamHandle::spawn(
                move || {
                    let connects = connects.clone();
                    async move {
                        connects.fetch_add(1, Ordering::SeqCst);
                        Err(crate::Error::Modbus(crate::ModbusError::Exception(
                            ExceptionCode::GatewayPathUnavailable,
                        )))
                    }
                },
                false,
                log,
            )
        };

        tokio::time::sleep(Duration::from_millis(20)).await;

        let first = handle
            .forward(
                UnitId(1),
                RequestPdu::ReadHoldingRegisters {
                    address: Address(0),
                    quantity: Quantity(1),
                },
            )
            .await;
        let second = handle
            .forward(
                UnitId(1),
                RequestPdu::ReadHoldingRegisters {
                    address: Address(0),
                    quantity: Quantity(1),
                },
            )
            .await;

        assert_eq!(first, Err(ExceptionCode::GatewayPathUnavailable));
        assert_eq!(second, Err(ExceptionCode::GatewayPathUnavailable));
        assert_eq!(connects.load(Ordering::SeqCst), 1);
    }
}
