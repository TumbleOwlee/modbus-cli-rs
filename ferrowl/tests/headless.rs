//! Integration smoke tests for `ferrowl run` (headless/CI mode). Drives the actual compiled
//! binary as a subprocess since `ferrowl` is bin-only (no lib target to call `headless::run`
//! from directly), asserting the exit-code contract documented in the README.

use ferrowl_test_support::reserve_temp_dir;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ferrowl"))
}

#[test]
/// CL-R-032 — a headless run reaching its --duration deadline exits 0.
fn it_runs_a_modbus_server_and_exits_clean() {
    let device = concat!(env!("CARGO_MANIFEST_DIR"), "/../configs/evse.toml");
    let module = format!(
        "name=it-headless-1,device={device},transport=tcp,ip=127.0.0.1,port=15920,role=server"
    );
    let output = bin()
        .args(["run", "--module", &module, "--duration", "1"])
        .output()
        .expect("failed to run ferrowl binary");

    assert!(
        output.status.success(),
        "expected exit 0, got {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("it-headless-1"),
        "expected drained log line naming the module, got: {stdout}"
    );
}

#[test]
/// CL-R-030, CL-R-049 — a module whose device config fails to load makes the headless run exit 1 with an `Error:`-prefixed diagnostic on stderr.
fn it_fails_hard_on_a_missing_device_config() {
    let module = "name=it-headless-bad,device=/no/such/device.toml,transport=tcp,ip=127.0.0.1,port=15921,role=server";
    let output = bin()
        .args(["run", "--module", module, "--duration", "1"])
        .output()
        .expect("failed to run ferrowl binary");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr
            .lines()
            .any(|line| line.starts_with("Error:") && line.contains("failed to load")),
        "expected an `Error:`-prefixed load-failure line, got: {stderr}"
    );
}

/// MB-R-140, MB-R-145 — a monitor device-type config file's `definitions` list defaults to
/// empty when omitted; an explicit empty list is a legitimate "no interpretations yet" file.
fn write_monitor_device(path: &std::path::Path) {
    std::fs::write(path, "definitions = []\n").expect("write monitor device config");
}

#[test]
/// MB-R-140, MB-R-141 — `ferrowl run --session` with a `role = "monitor"` module builds and
/// starts a working monitor tab: `MonitorBuilder::spawn` always returns `Ok` (the actual
/// serial open, and its failure/retry, happen inside the spawned task), so a headless run
/// with a monitor module on a non-existent serial path still starts and reaches its
/// `--duration` deadline cleanly.
fn it_headless_run_starts_monitor_module() {
    let dir = reserve_temp_dir("ferrowl_it_headless_monitor");
    let device_path = dir.join("device.toml");
    write_monitor_device(&device_path);

    let session_path = dir.join("session.toml");
    std::fs::write(
        &session_path,
        format!(
            r#"
[[modules]]
type = "modbus"
name = "it-headless-monitor"
device = "{}"
role = "monitor"

[modules.endpoint]
transport = "rtu"
path = "/dev/ttyNONE-it-headless-monitor"
baud_rate = 9600
"#,
            device_path.to_str().unwrap().replace('\\', "\\\\")
        ),
    )
    .expect("write session file");

    let output = bin()
        .args([
            "run",
            "--session",
            session_path.to_str().unwrap(),
            "--duration",
            "1",
        ])
        .output()
        .expect("failed to run ferrowl binary");

    assert!(
        output.status.success(),
        "expected exit 0, got {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("it-headless-monitor"),
        "expected drained log line naming the monitor module, got: {stdout}"
    );
}

#[test]
/// CL-R-030, one role over — a monitor module referencing a device file that fails
/// `load_monitor_device` hard-fails the headless run (exit 1), same as today's
/// `load_device` failure for `Role::Client`/`Role::Server`. `role = "monitor"` is only
/// resolvable via `--session` (typed `ModuleSpec`) — the `--module key=val` flag's parser
/// deliberately only accepts `client`/`server`.
fn it_headless_fails_on_monitor_module_with_bad_device_path() {
    let dir = reserve_temp_dir("ferrowl_it_headless_monitor_bad");
    let session_path = dir.join("bad_device_session.toml");
    std::fs::write(
        &session_path,
        r#"
[[modules]]
type = "modbus"
name = "it-headless-monitor-bad"
device = "/no/such/monitor-device.toml"
role = "monitor"

[modules.endpoint]
transport = "rtu"
path = "/dev/ttyNONE-it-headless-monitor-bad"
baud_rate = 9600
"#,
    )
    .expect("write session file");

    let output = bin()
        .args([
            "run",
            "--session",
            session_path.to_str().unwrap(),
            "--duration",
            "1",
        ])
        .output()
        .expect("failed to run ferrowl binary");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr
            .lines()
            .any(|line| line.starts_with("Error:") && line.contains("failed to load")),
        "expected an `Error:`-prefixed load-failure line, got: {stderr}"
    );
}
