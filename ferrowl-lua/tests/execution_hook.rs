//! Integration tests for the SC-R-034 execution hook (instruction + wall-clock ceiling) and
//! SC-R-039 (hook errors flow through the normal per-script error path). Builds contexts through
//! [`ContextBuilder`] exactly as production sim/on-demand call sites do.

// Integration-test crate: an unwrap that fails is the test failing, same as an assertion.
#![allow(clippy::unwrap_used)]

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ferrowl_lua::ContextBuilder;
use ferrowl_lua::module::{Has, Read, RegisterModule, ValueType, Write};

const INFINITE_LOOP: &str = "while true do end";

/// A `Write`-only host register double that records whether it was ever written to, so a "good"
/// script's side effect can be observed independently of its return value.
#[derive(Clone, Default)]
struct RecordingHandle(Arc<Mutex<bool>>);

impl Read for RecordingHandle {
    fn read(&self, _name: String) -> ferrowl_lua::Result<ValueType> {
        Ok(ValueType::Int(0))
    }
}
impl Has for RecordingHandle {
    fn has(&self, _name: String) -> ferrowl_lua::Result<bool> {
        Ok(true)
    }
}
impl Write for RecordingHandle {
    fn write(&self, _name: String, _value: ValueType) -> ferrowl_lua::Result<()> {
        *self.0.lock().unwrap() = true;
        Ok(())
    }
}

#[test]
/// NF-R-050, SC-R-047 — with no stop flag attached (the on-demand-run shape, SC-R-035), an infinite loop
/// is still interrupted, by the unconditional wall-clock cap alone.
fn it_wall_clock_cap_interrupts_infinite_loop_without_stop_flag() {
    let mut ctx = ContextBuilder::<String>::default()
        .with_script("runaway".to_string(), INFINITE_LOOP)
        .build()
        .expect("context build failed");

    let start = Instant::now();
    let err = ctx.call(&"runaway".to_string()).unwrap_err();
    let elapsed = start.elapsed();

    assert!(
        err.to_string().contains("execution cap"),
        "expected an execution-cap error, got: {err}"
    );
    assert!(
        elapsed >= Duration::from_millis(1_000),
        "hook fired before the 1,000ms cap was reached: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(3_000),
        "hook did not bound the runaway loop promptly: {elapsed:?}"
    );
}

#[test]
/// SC-R-012, SC-R-046 — an already-set stop flag interrupts a runaway script almost
/// immediately, well before the 1,000ms wall-clock cap would fire on its own.
fn it_stop_flag_interrupts_infinite_loop_almost_immediately() {
    let stop = Arc::new(AtomicBool::new(true));
    let mut ctx = ContextBuilder::<String>::default()
        .with_stop_flag(stop)
        .with_script("runaway".to_string(), INFINITE_LOOP)
        .build()
        .expect("context build failed");

    let start = Instant::now();
    let err = ctx.call(&"runaway".to_string()).unwrap_err();
    let elapsed = start.elapsed();

    assert!(
        err.to_string().contains("stop"),
        "expected a stop-flag error, got: {err}"
    );
    assert!(
        elapsed < Duration::from_millis(500),
        "stop flag was not observed promptly: {elapsed:?}"
    );
}

#[test]
/// SC-R-046 — a stop flag attached but never set is a no-op: the hook still enforces the
/// wall-clock cap on its own, so the resulting error is the wall-clock one, not a stop error.
fn it_stop_flag_not_set_does_not_interrupt_before_cap() {
    let stop = Arc::new(AtomicBool::new(false));
    let mut ctx = ContextBuilder::<String>::default()
        .with_stop_flag(stop)
        .with_script("runaway".to_string(), INFINITE_LOOP)
        .build()
        .expect("context build failed");

    let err = ctx.call(&"runaway".to_string()).unwrap_err();

    assert!(
        err.to_string().contains("execution cap"),
        "expected an execution-cap error (stop flag never set), got: {err}"
    );
    assert!(!err.to_string().contains("stop"));
}

#[test]
/// SC-R-039, SC-R-065 — a hook-raised error (here, a stop request) is isolated to the script
/// that raised it: `call_all` collects exactly that one error, and a sibling script in the same
/// cycle still runs to completion rather than the cycle stopping.
fn it_hook_error_isolated_other_script_in_same_cycle_still_runs() {
    let stop = Arc::new(AtomicBool::new(true));
    let recorder = RecordingHandle::default();
    let mut ctx = ContextBuilder::<String>::default()
        .with_stop_flag(stop)
        .with_module(RegisterModule::init(recorder.clone()))
        .with_script("bad".to_string(), INFINITE_LOOP)
        .with_script("good".to_string(), r#"C_Register:Set("x", 1)"#)
        .build()
        .expect("context build failed");

    let errors = ctx.call_all().expect_err("the runaway script must error");

    assert_eq!(errors.len(), 1, "only the runaway script should error");
    assert!(errors[0].to_string().contains("stop"));
    assert!(
        *recorder.0.lock().unwrap(),
        "the sibling script must still have run that cycle"
    );
}

#[test]
/// SC-R-066 — a stop flag set *while a call is already running* (not before it starts, unlike
/// `it_stop_flag_interrupts_infinite_loop_almost_immediately`) is still caught well within
/// `SC-R-047`'s fixed 1,000ms cap, measured from the moment the flag is actually set (the hook
/// firing that observes it), not from the call's own start.
fn it_stop_set_mid_call_completes_within_cap_from_when_set() {
    let stop = Arc::new(AtomicBool::new(false));
    let mut ctx = ContextBuilder::<String>::default()
        .with_stop_flag(stop.clone())
        .with_script("runaway".to_string(), INFINITE_LOOP)
        .build()
        .expect("context build failed");

    let set_at = Arc::new(Mutex::new(None::<Instant>));
    let set_at_writer = set_at.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        *set_at_writer.lock().unwrap() = Some(Instant::now());
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    let err = ctx.call(&"runaway".to_string()).unwrap_err();
    let elapsed_from_set = set_at.lock().unwrap().expect("flag was set").elapsed();

    assert!(err.to_string().contains("stop"));
    assert!(
        elapsed_from_set < Duration::from_millis(1_000),
        "stop-and-join did not complete within the 1,000ms cap measured from the flag being \
         set: {elapsed_from_set:?}"
    );
}

#[test]
/// An ordinary short script is unaffected by the hook, on both `call` and `call_all`.
fn it_normal_short_script_unaffected_by_hook() {
    let mut ctx = ContextBuilder::<String>::default()
        .with_script("ok".to_string(), "local x = 1 + 1")
        .build()
        .expect("context build failed");

    assert!(ctx.call(&"ok".to_string()).is_ok());
    assert!(ctx.call_all().is_ok());
}
