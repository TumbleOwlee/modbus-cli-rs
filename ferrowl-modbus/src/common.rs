//! Helpers genuinely shared by both client and server, on both transports.

use crate::SerialError;

use rust_modbus::{DataBits, Parity, SerialConfig, StopBits};

/// Build a serial port configuration from the optional serial parameters, validating each.
///
/// An unset parameter is left at the Modbus serial-line default the protocol library
/// declares (8 data bits, even parity, one stop bit — MB-R-072); only the fields the
/// device config states are overridden. The port path is not part of the configuration:
/// it is passed alongside it to `open_serial`.
pub(crate) fn serial_config_from(
    baud_rate: u32,
    data_bits: Option<u8>,
    stop_bits: Option<u8>,
    parity: Option<&str>,
) -> Result<SerialConfig, SerialError> {
    let mut config = SerialConfig {
        baud_rate,
        ..SerialConfig::default()
    };
    if let Some(v) = data_bits {
        config.data_bits = match v {
            5 => DataBits::Five,
            6 => DataBits::Six,
            7 => DataBits::Seven,
            8 => DataBits::Eight,
            _ => {
                return Err(SerialError::Configuration(
                    "Invalid data bits specified".to_string(),
                ));
            }
        };
    }
    if let Some(v) = stop_bits {
        config.stop_bits = match v {
            1 => StopBits::One,
            2 => StopBits::Two,
            _ => {
                return Err(SerialError::Configuration(
                    "Invalid stop bits specified".to_string(),
                ));
            }
        };
    }
    if let Some(v) = parity {
        let v = v.to_lowercase();
        if v == "odd" {
            config.parity = Parity::Odd;
        } else if v == "even" {
            config.parity = Parity::Even;
        } else if v == "none" {
            config.parity = Parity::None;
        } else {
            return Err(SerialError::Configuration(
                "Invalid parity specified".to_string(),
            ));
        }
    }
    Ok(config)
}

/// A command channel with a queue in front of it: whatever a connect/bind race parked while the
/// attempt was running is handed out before anything newer off the channel, so a command sent
/// concurrently with a connect is delivered once connected instead of being lost (MB-E-093).
pub(crate) struct Commands<'a, C> {
    receiver: &'a mut tokio::sync::mpsc::Receiver<C>,
    queued: std::collections::VecDeque<C>,
}

impl<'a, C> Commands<'a, C> {
    /// `queued` is what a preceding `race_terminate` parked; empty when nothing was parked.
    pub(crate) fn new(
        receiver: &'a mut tokio::sync::mpsc::Receiver<C>,
        queued: std::collections::VecDeque<C>,
    ) -> Self {
        Self { receiver, queued }
    }

    /// Cancel-safe, so it can sit in a `tokio::select!` arm exactly where `receiver.recv()` did:
    /// the queue pop is synchronous and returns without awaiting, and `Receiver::recv` is itself
    /// cancel-safe.
    pub(crate) async fn recv(&mut self) -> Option<C> {
        match self.queued.pop_front() {
            Some(c) => Some(c),
            None => self.receiver.recv().await,
        }
    }
}

/// Awaits `fut` while watching `receiver`, so an in-flight connect/bind/open is abandoned the
/// moment a terminate-matching command arrives instead of running to its own success or timeout
/// (MB-R-220, MB-R-221). Returns `Some(output)` when `fut` finished first, `None` when a
/// terminate-matching command arrived or the channel closed — the caller drops `fut`, which
/// cancels the attempt. Any other command is moved into `parked`, for the caller to hand to the
/// connection loop it is about to start (or to dispose of if the attempt failed); nothing is
/// dropped here.
pub(crate) async fn race_terminate<T, C, F>(
    fut: F,
    receiver: &mut tokio::sync::mpsc::Receiver<C>,
    is_terminate: impl Fn(&C) -> bool,
    parked: &mut std::collections::VecDeque<C>,
) -> Option<T>
where
    F: std::future::Future<Output = T>,
{
    tokio::pin!(fut);
    loop {
        tokio::select! {
            out = &mut fut => return Some(out),
            cmd = receiver.recv() => match cmd {
                None => return None,
                Some(c) if is_terminate(&c) => return None,
                Some(c) => parked.push_back(c),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LogFn;
    use std::future::Future;

    #[test]
    /// MB-R-072 — unset serial parameters leave the Modbus serial-line default (8E1) in place.
    fn ut_serial_config_valid_minimal() {
        // No optional fields set: the configuration carries the stated baud rate and the
        // library's Modbus defaults for everything else.
        let cfg = serial_config_from(9600, None, None, None).unwrap();
        assert_eq!(cfg.baud_rate, 9600);
        assert_eq!(cfg.data_bits, DataBits::Eight);
        assert_eq!(cfg.parity, Parity::Even);
        assert_eq!(cfg.stop_bits, StopBits::One);
    }

    #[test]
    /// MB-R-073 — valid `data_bits`/`stop_bits`/`parity` values are accepted.
    fn ut_serial_config_valid_full() {
        let r = serial_config_from(19200, Some(8), Some(1), Some("even"));
        assert!(r.is_ok());
    }

    #[test]
    /// MB-R-073 — `parity` accepts `even`/`odd`/`none` case-insensitively.
    fn ut_serial_config_parity_case_insensitive() {
        // Parity is lower-cased before matching, so mixed case is accepted.
        assert!(serial_config_from(9600, None, None, Some("ODD")).is_ok());
        assert!(serial_config_from(9600, None, None, Some("None")).is_ok());
    }

    #[test]
    /// MB-R-073 — a `data_bits` value other than 5/6/7/8 fails with a serial configuration error.
    fn ut_serial_config_rejects_bad_data_bits() {
        let e = serial_config_from(9600, Some(9), None, None).unwrap_err();
        assert!(matches!(e, SerialError::Configuration(_)));
        assert!(e.to_string().contains("data bits"));
    }

    #[test]
    /// MB-R-073 — a `stop_bits` value other than 1/2 fails with a serial configuration error.
    fn ut_serial_config_rejects_bad_stop_bits() {
        let e = serial_config_from(9600, None, Some(3), None).unwrap_err();
        assert!(matches!(e, SerialError::Configuration(_)));
        assert!(e.to_string().contains("stop bits"));
    }

    #[test]
    /// MB-R-073 — a `parity` value other than even/odd/none fails with a serial configuration error.
    fn ut_serial_config_rejects_bad_parity() {
        let e = serial_config_from(9600, None, None, Some("bogus")).unwrap_err();
        assert!(matches!(e, SerialError::Configuration(_)));
        assert!(e.to_string().contains("parity"));
    }

    #[test]
    /// MB-R-073 — `data_bits` accepts exactly 5, 6, 7, and 8.
    fn ut_serial_config_accepts_all_data_bit_widths() {
        for bits in [5u8, 6, 7, 8] {
            assert!(serial_config_from(9600, Some(bits), None, None).is_ok());
        }
    }

    #[test]
    /// MB-R-073 — `stop_bits` accepts exactly 1 and 2.
    fn ut_serial_config_accepts_both_stop_bits() {
        assert!(serial_config_from(9600, None, Some(1), None).is_ok());
        assert!(serial_config_from(9600, None, Some(2), None).is_ok());
    }

    // Verifies the stable `LogFn` blanket impl (replacing the former nightly `async_fn_traits`
    // bound) is satisfied by an ordinary closure returning a `Send` async block. Compile-time
    // check only — no runtime needed (this crate's tokio has no `rt` feature).
    #[test]
    fn ut_logfn_impl_for_closure_returning_async_block() {
        fn assert_logfn<L: LogFn>(_: &L) {}
        let f = move |s: String| async move {
            let _ = s.len();
        };
        assert_logfn(&f);
    }

    // The future a `LogFn` hands back must be `Send` (background tasks are spawned onto a
    // multi-threaded runtime); pin it behind a `Send` bound to lock that in.
    #[test]
    fn ut_logfn_future_is_send() {
        fn assert_send_fut<F: Future + Send>(_: &F) {}
        let f = |s: String| async move {
            let _ = s;
        };
        let fut = f.invoke("hi".to_string());
        assert_send_fut(&fut);
    }

    #[derive(Debug, PartialEq)]
    enum TestCmd {
        Terminate,
        Other(u32),
    }

    #[tokio::test]
    /// MB-R-220, MB-R-221 — `race_terminate` returns the attempt's output when it finishes
    /// before any terminate-matching command arrives.
    async fn ut_race_terminate_returns_output_when_future_wins() {
        let (_tx, mut rx) = tokio::sync::mpsc::channel::<TestCmd>(1);
        let mut parked = std::collections::VecDeque::new();
        let out = race_terminate(
            async { 42 },
            &mut rx,
            |cmd: &TestCmd| matches!(cmd, TestCmd::Terminate),
            &mut parked,
        )
        .await;
        assert_eq!(out, Some(42));
        assert!(parked.is_empty());
    }

    #[tokio::test]
    /// MB-R-220, MB-R-221 — a terminate-matching command arriving while the attempt is still
    /// pending abandons it at once instead of waiting for it to resolve.
    async fn ut_race_terminate_abandons_attempt_on_terminate() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TestCmd>(1);
        tx.send(TestCmd::Terminate).await.unwrap();
        let mut parked = std::collections::VecDeque::new();
        let out = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            race_terminate(
                std::future::pending::<()>(),
                &mut rx,
                |cmd: &TestCmd| matches!(cmd, TestCmd::Terminate),
                &mut parked,
            ),
        )
        .await
        .expect("race_terminate did not return promptly");
        assert_eq!(out, None);
    }

    #[tokio::test]
    /// MB-R-220, MB-R-221 — the command channel closing while the attempt is still pending
    /// abandons it, matching the terminate case.
    async fn ut_race_terminate_abandons_attempt_on_channel_close() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TestCmd>(1);
        drop(tx);
        let mut parked = std::collections::VecDeque::new();
        let out = tokio::time::timeout(
            std::time::Duration::from_millis(250),
            race_terminate(
                std::future::pending::<()>(),
                &mut rx,
                |cmd: &TestCmd| matches!(cmd, TestCmd::Terminate),
                &mut parked,
            ),
        )
        .await
        .expect("race_terminate did not return promptly");
        assert_eq!(out, None);
    }

    #[tokio::test]
    /// MB-E-093 — a non-terminate command arriving while the attempt is still in flight is
    /// parked, not dropped: the race continues, the future still wins, and the command survives
    /// for the next `Commands::recv()` to hand out.
    async fn ut_race_terminate_queues_non_terminate_command() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TestCmd>(1);
        tx.send(TestCmd::Other(7)).await.unwrap();
        let mut parked = std::collections::VecDeque::new();
        let out = race_terminate(
            async {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                "connected"
            },
            &mut rx,
            |cmd: &TestCmd| matches!(cmd, TestCmd::Terminate),
            &mut parked,
        )
        .await;
        assert_eq!(out, Some("connected"));
        assert_eq!(
            parked.into_iter().collect::<Vec<_>>(),
            vec![TestCmd::Other(7)]
        );
    }

    #[tokio::test]
    /// MB-E-093 — `Commands::recv` hands out whatever was parked by a preceding `race_terminate`
    /// before anything newer sent on the channel, so a command sent during the connect is
    /// delivered to the connection loop ahead of one sent after it connected.
    async fn ut_commands_hands_out_queued_before_channel() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TestCmd>(4);
        tx.send(TestCmd::Other(2)).await.unwrap();
        let mut queued = std::collections::VecDeque::new();
        queued.push_back(TestCmd::Other(1));
        let mut commands = Commands::new(&mut rx, queued);

        assert_eq!(commands.recv().await, Some(TestCmd::Other(1)));
        assert_eq!(commands.recv().await, Some(TestCmd::Other(2)));
    }
}
