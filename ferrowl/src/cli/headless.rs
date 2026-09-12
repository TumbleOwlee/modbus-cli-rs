//! Headless / CI runner: `ferrowl run`.
//!
//! Builds the same [`ModuleView`] instances the TUI's `build_tabs` builds and starts them the
//! same way, but never touches the terminal: it ticks `refresh()` on a timer, drains each
//! module's log to stdout (and optionally a file), and exits with a code that reflects what
//! happened instead of leaving the operator to read a screen.
//!
//! Exit codes: `0` ran to completion (duration elapsed or Ctrl-C), `1` a module's device config
//! failed to load or `start` reported an error, `3` `--exit-on-error` was set and a drained log
//! line had log level Error.

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ferrowl_lua::module::{ModuleDirectory, ModuleHost};

use crate::app::{Level, LogRing};
use crate::cli::RunArgs;
use crate::config::ocpp::OcppRole;
use crate::config::script::ScriptDef;
use crate::config::{self, OcppModuleSpec, OcppSpec, Role};
use crate::module::modbus::ModbusModule as Module;
use crate::module::modbus::view::ModbusModuleView;
use crate::module::modbus::{ModbusMonitorModule as MonitorModule, ModbusMonitorModuleView};
use crate::module::ocpp::client::build_client_view;
use crate::module::ocpp::server::build_server_view;
use crate::module::view::{CommandResult, ModuleView, SharedLog};
use crate::registry::{ModuleRegistry, dedupe_names};
use crate::session::SessionSim;
use crate::view::log::format_timestamp;

/// Log source name the session-level Lua sim's drained lines are prefixed with, alongside every
/// module's own name.
const SESSION_SOURCE: &str = "session";

/// How often the loop wakes to refresh modules and drain logs (mirrors `App`'s redraw tick).
const TICK: Duration = Duration::from_millis(100);

/// Ring depth to peek per tick. Matches `crate::app::LOG_SIZE`; kept local so this module has no
/// dependency on the TUI's `App` beyond the shared log type and [`LogRing`] itself.
const LOG_PEEK: usize = 80;

/// One running module: its view (owns start/stop/refresh), display name, log channel, and how
/// many lines of the log have already been drained.
struct RunModule {
    name: String,
    view: Box<dyn ModuleView>,
    log: SharedLog,
    /// Total lines already drained, per [`LogRing::written`]. Draining is exact-by-count rather
    /// than by matching the last-seen line's content: content matching mis-resumes when a
    /// message repeats verbatim within one window (a tight sim loop logging the same error
    /// every tick, e.g.), silently skipping real lines. The only way this can still lose lines
    /// is a full ring-eviction between ticks, which is detected and reported (see [`drain_log`]).
    last_written: u64,
}

/// Build every configured module, starting each one and failing hard (unlike the TUI's
/// `build_tabs`, which skips a bad module with an `eprintln!` and keeps going) if a device config
/// fails to load or `start` reports an error.
async fn build_modules(args: &RunArgs) -> Result<Vec<RunModule>, String> {
    let mut modules = Vec::new();
    // MB-R-150 — one session-wide registry for this headless run, attached to every Rtu/Ascii
    // module immediately after construction and before it starts. No race: modules are built and
    // started one at a time in this same loop, unlike `App`'s tabs (all pre-started before the
    // TUI's own first `rebuild_registry`).
    let serial_paths = crate::module::modbus::SerialPathRegistry::new();

    for spec in args.module_specs()? {
        let view: Box<dyn ModuleView> = match spec.role {
            Role::Monitor => {
                let device = config::load_monitor_device(&spec.device).map_err(|e| {
                    format!("'{}': failed to load '{}': {e}", spec.name, spec.device)
                })?;
                let mut module = MonitorModule::new(&spec, &device);
                module.set_serial_paths(serial_paths.clone());
                Box::new(ModbusMonitorModuleView::new(module, spec.clone(), device))
            }
            Role::Client | Role::Server => {
                let device = config::load_device(&spec.device).map_err(|e| {
                    format!("'{}': failed to load '{}': {e}", spec.name, spec.device)
                })?;
                let mut module = Module::new(&spec, &device);
                module.set_serial_paths(serial_paths.clone());
                Box::new(ModbusModuleView::new(module, spec.clone(), device))
            }
        };
        modules.push(start_module(spec.name.clone(), view).await?);
    }

    for spec in args.ocpp_specs()? {
        modules.push(build_ocpp_module(spec).await?);
    }

    Ok(modules)
}

async fn build_ocpp_module(module: OcppModuleSpec) -> Result<RunModule, String> {
    let name = module.name.clone();
    let device = config::load_ocpp_device(&module.device)
        .map_err(|e| format!("'{name}': failed to load '{}': {e}", module.device))?;
    let spec = OcppSpec::from_parts(&module, &device);
    let view: Box<dyn ModuleView> = match device.role {
        OcppRole::Client => build_client_view(spec, module.device.clone(), device),
        OcppRole::Server => build_server_view(spec, module.device.clone(), device),
    };
    start_module(name, view).await
}

/// Start a module via `handle_command("start")`. Each start handler tags its own message with an
/// explicit [`Level`] (see [`CommandResult`]) — a `Level::Error` result is treated as a start
/// failure here.
async fn start_module(name: String, mut view: Box<dyn ModuleView>) -> Result<RunModule, String> {
    let log = view.log();
    if let CommandResult::Handled(Some((level, msg))) = view.handle_command("start").await {
        if level == Level::Error {
            return Err(format!("'{name}': {msg}"));
        }
        log.write().await.write(level, &msg);
    }
    Ok(RunModule {
        name,
        view,
        log,
        last_written: 0,
    })
}

/// Drain newly-appended lines from one module's log, returning them pre-formatted as
/// `[<timestamp>] <name> | <line>` (the caller just prints/writes them — keeps this testable
/// without capturing stdout). The second return value is `true` when `exit_on_error` is set and
/// one of the drained lines looked like a Lua sim error.
///
/// New-line count is computed exactly from [`LogRing::written`] deltas, not by matching the
/// last-seen line's content — content matching breaks when a message repeats verbatim within one
/// window. The ring is still bounded, though: if more lines were written since the last drain
/// than the ring can hold, the oldest of them are gone for good. That case is reported via a
/// synthetic "lines dropped" line rather than silently under-counted.
async fn drain_log(
    log: &SharedLog,
    name: &str,
    last_written: &mut u64,
    exit_on_error: bool,
) -> (Vec<String>, bool) {
    let (written, window) = {
        let guard = log.read().await;
        (guard.written(), guard.peek_n(LOG_PEEK))
    };

    let new_count = written.saturating_sub(*last_written);
    *last_written = written;

    let mut lines = Vec::new();
    let mut hit_error = false;
    if new_count == 0 {
        return (lines, hit_error);
    }

    if new_count > window.len() as u64 {
        let dropped = new_count - window.len() as u64;
        lines.push(format!(
            "[{}] [{}] {name} | ({dropped} lines dropped: ring overflowed between ticks)",
            format_timestamp(ferrowl_util::time::now_unix_ms()),
            Level::Error
        ));
    }

    let take = (new_count as usize).min(window.len());
    let start = window.len() - take;
    for (ts, level, msg) in &window[start..] {
        lines.push(format!(
            "[{}] [{}] {name} | {msg}",
            format_timestamp(*ts),
            level
        ));
        if exit_on_error && *level == Level::Error {
            hit_error = true;
        }
    }

    (lines, hit_error)
}

/// Build the session-level `C_Module` registry from every running module's
/// [`ModuleView::module_host`], keyed by name deduped the same way [`crate::registry::dedupe_names`]
/// dedupes tab names in the TUI (headless has no tab set of its own, but reuses the same helper so
/// a repeated `--module`/`--ocpp` name, or a session file listing the same name twice, doesn't
/// silently drop one module's host from `C_Module`).
fn build_registry(modules: &[RunModule]) -> ModuleRegistry {
    let names: Vec<String> = modules.iter().map(|m| m.name.clone()).collect();
    let deduped = dedupe_names(&names);

    let mut hosts: HashMap<String, Arc<dyn ModuleHost>> = HashMap::new();
    for (module, name) in modules.iter().zip(deduped.iter()) {
        if let Some(host) = module.view.module_host() {
            hosts.insert(name.clone(), host);
        }
    }

    let registry = ModuleRegistry::new();
    registry.replace_all(hosts);
    registry
}

/// Aggregate every `--session` file's session-level Lua scripts and cycle interval into one
/// config, or `None` when no session file carries any script (including the single-module
/// `--module`/`--ocpp` path, which has no session file at all). Scripts from multiple session
/// files are concatenated in file order; the interval is the last session file's, matching the
/// TUI's `session_sim_config` rule so both entry points resolve multi-file sessions identically.
fn load_session_scripts(args: &RunArgs) -> Result<Option<(Vec<ScriptDef>, Duration)>, String> {
    let mut scripts = Vec::new();
    let mut interval = None;
    for path in &args.sessions {
        let session = config::load_session(path).map_err(|e| e.to_string())?;
        interval = Some(session.interval_duration());
        scripts.extend(session.scripts);
    }
    if scripts.is_empty() {
        return Ok(None);
    }
    Ok(Some((
        scripts,
        interval.unwrap_or(Duration::from_secs_f64(1.0)),
    )))
}

/// Print+append every drained line, and fold `hit_error` into `exit_code`/`should_stop`: an
/// error line from either the module logs or the session sim's log gets the same
/// exit-code-3-and-stop treatment (the source distinction is already baked into `lines` via
/// `drain_log`'s `name` prefix).
fn emit_drained(
    lines: &[String],
    log_file: &mut Option<std::fs::File>,
    hit_error: bool,
    exit_code: &mut i32,
    should_stop: &mut bool,
) {
    for line in lines {
        println!("{line}");
        if let Some(f) = log_file.as_mut() {
            let _ = writeln!(f, "{line}");
        }
    }
    if hit_error {
        *exit_code = 3;
        *should_stop = true;
    }
}

/// Format one teardown outcome for stderr (CL-R-055, CL-R-056, CL-R-057). Anything other than an
/// `Error`-level stop message — no message, an informational one, or a view that does not handle
/// `stop` — counts as a clean stop.
fn teardown_line(name: &str, outcome: Option<(Level, String)>) -> String {
    match outcome {
        Some((Level::Error, detail)) => format!("Error: failed to stop '{name}': {detail}"),
        _ => format!("Stopped '{name}'"),
    }
}

/// Stop every module (best-effort: a stop failure is logged but does not change the exit code —
/// we're already tearing down). Returns the teardown line reported for each module, in order
/// (CL-R-055, CL-R-056).
async fn stop_all(modules: &mut [RunModule]) -> Vec<String> {
    let mut lines = Vec::new();
    for module in modules.iter_mut() {
        let result = module.view.handle_command("stop").await;
        let outcome = if let CommandResult::Handled(Some((level, msg))) = &result {
            module.log.write().await.write(*level, msg);
            Some((*level, msg.clone()))
        } else {
            None
        };
        let line = teardown_line(&module.name, outcome);
        eprintln!("{line}");
        lines.push(line);
    }
    lines
}

/// Run the headless session described by `args`. Returns the process exit code; never panics on
/// a module's own runtime errors (those surface as log lines), only on setup failure.
pub async fn run(args: &RunArgs) -> i32 {
    let mut modules = match build_modules(args).await {
        Ok(modules) => modules,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };

    let mut log_file = match crate::cli::open_log_file(args.log_file.as_deref()) {
        Ok(f) => f,
        Err((path, e)) => {
            eprintln!("Error: failed to open --log-file '{path}': {e}");
            stop_all(&mut modules).await;
            return 1;
        }
    };

    let registry = build_registry(&modules);
    let mut session_sim = match load_session_scripts(args) {
        Ok(Some((scripts, interval))) => {
            let log: SharedLog = Arc::new(tokio::sync::RwLock::new(LogRing::init()));
            let directory: Arc<dyn ModuleDirectory> = Arc::new(registry);
            let mut sim = SessionSim::new(directory, log.clone());
            sim.set_interval(interval);
            sim.set_scripts(scripts);
            Some((sim, log, 0u64))
        }
        Ok(None) => None,
        Err(e) => {
            eprintln!("Error: {e}");
            stop_all(&mut modules).await;
            return 1;
        }
    };

    let deadline = args
        .duration
        .map(|secs| Instant::now() + Duration::from_secs(secs));
    let mut exit_code = 0;

    loop {
        tokio::select! {
            _ = tokio::time::sleep(TICK) => {}
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }

        for view in modules.iter_mut().map(|m| &mut m.view) {
            view.refresh().await;
        }

        let mut should_stop = false;
        for module in modules.iter_mut() {
            let (lines, hit_error) = drain_log(
                &module.log,
                &module.name,
                &mut module.last_written,
                args.exit_on_error,
            )
            .await;
            emit_drained(
                &lines,
                &mut log_file,
                hit_error,
                &mut exit_code,
                &mut should_stop,
            );
        }

        if let Some((_, log, last_written)) = session_sim.as_mut() {
            let (lines, hit_error) =
                drain_log(log, SESSION_SOURCE, last_written, args.exit_on_error).await;
            emit_drained(
                &lines,
                &mut log_file,
                hit_error,
                &mut exit_code,
                &mut should_stop,
            );
        }

        if should_stop {
            break;
        }
        if let Some(deadline) = deadline
            && Instant::now() >= deadline
        {
            break;
        }
    }

    if let Some((sim, ..)) = session_sim.as_mut() {
        sim.stop();
        eprintln!("{}", teardown_line(SESSION_SOURCE, None));
    }
    stop_all(&mut modules).await;
    exit_code
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LogRing;
    use ferrowl_test_support::{TempDirGuard, reserve_tcp_port, reserve_temp_dir};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn new_log() -> SharedLog {
        Arc::new(RwLock::new(LogRing::init()))
    }

    #[tokio::test]
    /// CL-R-043 — log draining is exact-by-count, emitting every occurrence of a repeated line.
    async fn ut_drain_log_counts_exact_even_with_duplicate_lines() {
        let log = new_log();
        {
            let mut g = log.write().await;
            g.write(Level::Info, "dup");
            g.write(Level::Info, "dup");
            g.write(Level::Info, "dup");
        }
        let mut last_written = 0;
        let (lines, hit) = drain_log(&log, "mod", &mut last_written, false).await;
        assert_eq!(
            lines.len(),
            3,
            "every occurrence of the repeated line must be drained"
        );
        assert!(!hit);
        assert_eq!(last_written, 3);

        // Nothing new since the last drain.
        let (lines, _) = drain_log(&log, "mod", &mut last_written, false).await;
        assert!(lines.is_empty());

        // Two more duplicates land; only those two are new.
        {
            let mut g = log.write().await;
            g.write(Level::Info, "dup");
            g.write(Level::Info, "dup");
        }
        let (lines, _) = drain_log(&log, "mod", &mut last_written, false).await;
        assert_eq!(lines.len(), 2);
        assert_eq!(last_written, 5);
    }

    #[tokio::test]
    /// CL-R-052 — a ring overflow between ticks is reported with a synthetic dropped-lines line.
    async fn ut_drain_log_reports_dropped_lines_on_ring_overflow() {
        let log = new_log();
        let overflow_by = 5;
        {
            let mut g = log.write().await;
            for i in 0..(LOG_PEEK + overflow_by) {
                g.write(Level::Info, &format!("line {i}"));
            }
        }
        let mut last_written = 0;
        let (lines, _) = drain_log(&log, "mod", &mut last_written, false).await;
        assert_eq!(last_written, (LOG_PEEK + overflow_by) as u64);
        // The full window plus one marker line.
        assert_eq!(lines.len(), LOG_PEEK + 1);
        assert!(lines[0].contains(&format!("{overflow_by} lines dropped")));
    }

    #[tokio::test]
    /// CL-R-031 — a log error line flags exit-code 3 only when --exit-on-error is set.
    /// CL-R-034 — a Lua sim error (a `[sim]` line) does not by itself fail a headless run: with
    /// --exit-on-error off it surfaces without flagging the exit code.
    async fn ut_drain_log_flags_sim_error_prefix_only_when_requested() {
        let log = new_log();
        log.write().await.write(Level::Error, "[sim] boom");
        let mut last_written = 0;
        let (_, hit) = drain_log(&log, "mod", &mut last_written, false).await;
        assert!(!hit, "not flagged when --exit-on-error is off");

        let mut last_written = 0;
        let (_, hit) = drain_log(&log, "mod", &mut last_written, true).await;
        assert!(hit);
    }

    #[tokio::test]
    /// CL-R-040 — session-sim lines are drained under the source name `session`.
    async fn ut_drain_log_session_source_uses_session_prefix() {
        let log = new_log();
        log.write().await.write(Level::Info, "hello");
        let mut last_written = 0;
        let (lines, hit) = drain_log(&log, SESSION_SOURCE, &mut last_written, false).await;
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].contains("session | hello"),
            "unexpected line: {}",
            lines[0]
        );
        assert!(!hit);
    }

    #[tokio::test]
    /// CL-R-031 — an error session-sim line flags exit-code 3 under --exit-on-error.
    async fn ut_drain_log_session_sim_error_flags_exit_on_error() {
        let log = new_log();
        log.write().await.write(Level::Error, "[sim] boom");
        let mut last_written = 0;
        let (lines, hit) = drain_log(&log, SESSION_SOURCE, &mut last_written, true).await;
        assert!(hit);
        assert!(
            lines[0].contains("session | [sim] boom"),
            "unexpected line: {}",
            lines[0]
        );
    }

    fn empty_run_args(sessions: Vec<String>) -> RunArgs {
        RunArgs {
            sessions,
            modules: vec![],
            ocpp: vec![],
            duration: None,
            log_file: None,
            exit_on_error: false,
        }
    }

    #[test]
    /// CL-R-023 — no session files means no session sim is created.
    fn ut_load_session_scripts_none_without_session_files() {
        // Mirrors the single-module `--module key=val` headless path:
        // no `--session` file means no `Session`, so no session sim is even considered.
        let args = empty_run_args(vec![]);
        assert!(load_session_scripts(&args).unwrap().is_none());
    }

    #[test]
    /// CL-R-027 — session scripts concatenate across files in order; the last file's interval wins.
    fn ut_load_session_scripts_aggregates_across_files_last_interval_wins() {
        use crate::config::Session as SessionConfig;
        use ferrowl_util::convert::{Converter, FileType};

        let s1 = SessionConfig {
            version: None,
            modules: vec![],
            scripts: vec![ScriptDef {
                name: "a".into(),
                code: String::new(),
                enabled: true,
            }],
            interval: 2.0,
        };
        let s2 = SessionConfig {
            version: None,
            modules: vec![],
            scripts: vec![ScriptDef {
                name: "b".into(),
                code: String::new(),
                enabled: false,
            }],
            interval: 9.0,
        };
        let dir = reserve_temp_dir("ferrowl_headless");
        let p1 = dir.join("session1.toml");
        let p2 = dir.join("session2.toml");
        Converter::save(&s1, p1.to_str().unwrap(), FileType::Toml).unwrap();
        Converter::save(&s2, p2.to_str().unwrap(), FileType::Toml).unwrap();

        let args = empty_run_args(vec![
            p1.to_str().unwrap().to_string(),
            p2.to_str().unwrap().to_string(),
        ]);
        let (scripts, interval) = load_session_scripts(&args).unwrap().unwrap();
        assert_eq!(scripts.len(), 2, "scripts from both files are concatenated");
        assert_eq!(
            interval,
            Duration::from_secs_f64(9.0),
            "interval comes from the last session file, matching the TUI rule"
        );
    }

    // --- Integration: a real modbus module + a session-level script talking to it ------------

    fn holding_device_config() -> config::DeviceConfig {
        use crate::module::modbus::config::device::{
            AccessCfg, AlignmentCfg, EndianCfg, RegisterDef, ValueType, WordOrderCfg,
        };
        use ferrowl_codec::Kind;

        let mut definitions = std::collections::BTreeMap::new();
        definitions.insert(
            "value".to_string(),
            RegisterDef {
                slave_id: 1,
                kind: Kind::HoldingRegister,
                address: Some(0),
                is_virtual: false,
                access: AccessCfg::ReadWrite,
                value_type: ValueType::U16,
                endian: EndianCfg::Big,
                word_order: WordOrderCfg::default(),
                resolution: 1.0,
                bitmask: None,
                length: 1,
                alignment: AlignmentCfg::Left,
                values: vec![],
                update: None,
                description: String::new(),
                default: None,
            },
        );
        config::DeviceConfig {
            definitions,
            ..Default::default()
        }
    }

    /// Writes a temp device config + a session file with one modbus module and one session
    /// script, returns a [`RunArgs`] pointing at it plus the guard keeping its temp dir alive.
    /// `script_enabled` toggles whether the session script is enabled, so both the "sim runs"
    /// and "zero enabled scripts spawns nothing" cases share one fixture.
    fn session_run_args(tag: &str, script_enabled: bool) -> (RunArgs, TempDirGuard) {
        use ferrowl_util::convert::{Converter, FileType};

        let dir = reserve_temp_dir(&format!("ferrowl_headless_{tag}"));
        let device_path = dir.join("device.toml");
        Converter::save(
            &holding_device_config(),
            device_path.to_str().unwrap(),
            FileType::Toml,
        )
        .unwrap();

        let mut module_value = serde_json::to_value(config::ModuleSpec {
            name: "m".to_string(),
            device: device_path.to_str().unwrap().to_string(),
            role: config::Role::Server,
            endpoint: config::Endpoint::Tcp {
                ip: "127.0.0.1".to_string(),
                port: 0,
            },
        })
        .unwrap();
        module_value
            .as_object_mut()
            .unwrap()
            .insert("type".into(), "modbus".into());

        let session = config::Session {
            version: None,
            modules: vec![module_value],
            scripts: vec![ScriptDef {
                name: "s".to_string(),
                code: r#"C_Module:Get("m"):Register():Set("value", 42); C_Log:Info("session-script-ran")"#
                    .to_string(),
                enabled: script_enabled,
            }],
            interval: 0.05,
        };
        let session_path = dir.join("session.toml");
        Converter::save(&session, session_path.to_str().unwrap(), FileType::Toml).unwrap();

        let args = RunArgs {
            sessions: vec![session_path.to_str().unwrap().to_string()],
            modules: vec![],
            ocpp: vec![],
            duration: Some(1),
            log_file: Some(dir.join("run.log").to_str().unwrap().to_string()),
            exit_on_error: false,
        };
        (args, dir)
    }

    #[tokio::test]
    /// CL-R-023 — the runner wires the session sim and drains its log under `session`.
    async fn ut_run_wires_session_sim_and_drains_its_log() {
        let (args, _dir) = session_run_args("enabled", true);
        let log_file = args.log_file.clone().unwrap();

        let exit_code = run(&args).await;
        assert_eq!(exit_code, 0);

        let contents = std::fs::read_to_string(&log_file).unwrap();
        assert!(
            contents.contains("session | session-script-ran"),
            "expected a session-prefixed log line, got:\n{contents}"
        );
    }

    #[tokio::test]
    /// NF-R-054 — `--log-file` expands a leading `~` to the home directory.
    async fn ut_run_log_file_expands_tilde() {
        // Own tag ("tilde"), not "enabled" — sharing a tag with `ut_run_wires_session_sim_and_
        // drains_its_log` would race both tests over the same temp device/session files under
        // parallel execution.
        let (mut args, _dir) = session_run_args("tilde", true);
        let home = std::env::home_dir().expect("HOME must resolve in test environment");
        let filename = format!("ferrowl_headless_tilde_{}.log", std::process::id());
        args.log_file = Some(format!("~/{filename}"));
        let expected_path = home.join(&filename);
        let _ = std::fs::remove_file(&expected_path);

        let exit_code = run(&args).await;
        assert_eq!(exit_code, 0);

        let contents = std::fs::read_to_string(&expected_path);
        let _ = std::fs::remove_file(&expected_path);
        assert!(
            contents.unwrap().contains("session | session-script-ran"),
            "expected the log to have been written under the expanded home path"
        );
    }

    #[tokio::test]
    /// CL-R-023 — with no enabled session script, no session sim is spawned.
    async fn ut_run_with_zero_enabled_scripts_spawns_no_session_sim() {
        let (args, _dir) = session_run_args("disabled", false);
        let log_file = args.log_file.clone().unwrap();

        let exit_code = run(&args).await;
        assert_eq!(exit_code, 0);

        let contents = std::fs::read_to_string(&log_file).unwrap();
        assert!(
            !contents.contains("session |"),
            "no session source should appear in the log when no script is enabled, got:\n{contents}"
        );
    }

    // --- Run lifecycle, exit codes, and the output contract ----------------------------------

    /// A device config on disk for a headless module fixture, under `dir`.
    fn write_device(dir: &TempDirGuard) -> String {
        use ferrowl_util::convert::{Converter, FileType};
        let p = dir.join("device.toml");
        Converter::save(
            &holding_device_config(),
            p.to_str().unwrap(),
            FileType::Toml,
        )
        .unwrap();
        p.to_str().unwrap().to_string()
    }

    /// A `RunArgs` starting one modbus TCP server module on `port` for `duration` seconds.
    fn modbus_run_args(dir: &TempDirGuard, port: u16, duration: u64) -> RunArgs {
        let device = write_device(dir);
        RunArgs {
            sessions: vec![],
            modules: vec![format!(
                "name=m,device={device},transport=tcp,ip=127.0.0.1,port={port},role=server"
            )],
            ocpp: vec![],
            duration: Some(duration),
            log_file: None,
            exit_on_error: false,
        }
    }

    /// A client device whose first poll times out fast and whose client then stops instead of
    /// retrying — the shortest route to an Error-level line in a module's log ring.
    fn write_timing_out_client_device(dir: &TempDirGuard) -> String {
        use ferrowl_util::convert::{Converter, FileType};
        let mut cfg = holding_device_config();
        cfg.timeout_ms = Some(100);
        cfg.delay_ms = Some(0);
        cfg.interval_ms = Some(0);
        cfg.reconnect = Some(false);
        let p = dir.join("timeout-client.toml");
        Converter::save(&cfg, p.to_str().unwrap(), FileType::Toml).unwrap();
        p.to_str().unwrap().to_string()
    }

    /// A `RunArgs` starting one modbus TCP client module against `port`, where nothing answers.
    fn timing_out_client_run_args(
        dir: &TempDirGuard,
        port: u16,
        duration: u64,
        exit_on_error: bool,
    ) -> RunArgs {
        let device = write_timing_out_client_device(dir);
        RunArgs {
            sessions: vec![],
            modules: vec![format!(
                "name=m,device={device},transport=tcp,ip=127.0.0.1,port={port},role=client"
            )],
            ocpp: vec![],
            duration: Some(duration),
            log_file: None,
            exit_on_error,
        }
    }

    #[tokio::test]
    /// CL-R-021 — the headless runner treats a module's device-config load failure as fatal to
    /// startup, rather than skipping the module like the TUI.
    async fn ut_build_modules_fails_hard_on_bad_device() {
        let dir = reserve_temp_dir("ferrowl_cl");
        let mut args = modbus_run_args(&dir, reserve_tcp_port().release(), 1);
        args.modules = vec![
            "name=m,device=/no/such/device.toml,transport=tcp,ip=127.0.0.1,port=0,role=server"
                .into(),
        ];
        assert!(build_modules(&args).await.is_err());
    }

    #[tokio::test]
    /// MB-R-150 — headless module construction attaches one shared session-wide serial-path
    /// registry to each Rtu/Ascii module before starting it, so two server instances configured
    /// on the same nonexistent path see each other as a conflict instead of silently racing the
    /// OS for it.
    async fn ut_run_attaches_shared_serial_paths_registry_across_rtu_modules() {
        let dir = reserve_temp_dir("ferrowl_cl");
        let device = write_device(&dir);
        let log_file = dir.join("serial_paths.log").to_str().unwrap().to_string();
        let args = RunArgs {
            sessions: vec![],
            modules: vec![
                format!(
                    "name=a,device={device},transport=rtu,path=/nonexistent/mb-r-150-cl,baud=9600,role=server"
                ),
                format!(
                    "name=b,device={device},transport=rtu,path=/nonexistent/mb-r-150-cl,baud=9600,role=server"
                ),
            ],
            ocpp: vec![],
            duration: Some(1),
            log_file: Some(log_file.clone()),
            exit_on_error: false,
        };

        assert_eq!(run(&args).await, 0);
        let contents = std::fs::read_to_string(&log_file).unwrap();
        assert!(
            contents.contains("already in use by module"),
            "expected a path-conflict log line from the shared registry, got:\n{contents}"
        );
    }

    #[tokio::test]
    /// CL-R-030 — a setup failure (a module's device config fails to load) makes the run exit 1.
    async fn ut_run_returns_one_on_setup_failure() {
        let dir = reserve_temp_dir("ferrowl_cl");
        let mut args = modbus_run_args(&dir, reserve_tcp_port().release(), 1);
        args.modules = vec![
            "name=m,device=/no/such/device.toml,transport=tcp,ip=127.0.0.1,port=0,role=server"
                .into(),
        ];
        assert_eq!(run(&args).await, 1);
    }

    #[tokio::test]
    /// CL-R-020 — the runner builds and starts each module (without touching the terminal) and
    /// drains its log to the output stream.
    /// CL-R-048 — the loop refreshes every module each tick and drains its newly appended log
    /// lines to the output.
    async fn ut_run_starts_modules_and_drains_output() {
        let dir = reserve_temp_dir("ferrowl_cl");
        let mut args = modbus_run_args(&dir, reserve_tcp_port().release(), 1);
        let log_file = dir.join("starts.log").to_str().unwrap().to_string();
        args.log_file = Some(log_file.clone());

        assert_eq!(run(&args).await, 0);
        let contents = std::fs::read_to_string(&log_file).unwrap();
        assert!(
            contents.contains("m |"),
            "expected drained lines tagged with the module name, got:\n{contents}"
        );
    }

    #[tokio::test]
    /// CL-R-024 — a --duration run exits cleanly once the deadline is reached.
    /// CL-R-032 — such a run, with no exit-code-2 condition, returns exit code 0.
    async fn ut_run_duration_deadline_exits_zero() {
        let dir = reserve_temp_dir("ferrowl_cl");
        let args = modbus_run_args(&dir, reserve_tcp_port().release(), 1);
        assert_eq!(run(&args).await, 0);
    }

    #[tokio::test]
    /// CL-R-026 — on loop exit the runner stops every module: the listener refuses a connect
    /// afterward, rather than merely accepting a rebind (which OS address reuse can mask).
    async fn ut_run_stops_modules_on_exit() {
        let dir = reserve_temp_dir("ferrowl_cl");
        let port = reserve_tcp_port().release();
        assert_eq!(run(&modbus_run_args(&dir, port, 1)).await, 0);
        assert!(
            std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
            "expected the module's listener to be gone after teardown"
        );
    }

    #[tokio::test]
    /// CL-R-055 — each module the headless runner stops is reported, in list order, as
    /// `Stopped '<name>'`.
    async fn ut_stop_all_reports_each_module_in_order() {
        let (view_a, _handle_a) = crate::app::testkit::MockView::pair("a");
        let (view_b, _handle_b) = crate::app::testkit::MockView::pair("b");
        let mut modules = vec![
            RunModule {
                name: "a".to_string(),
                view: view_a.boxed(),
                log: new_log(),
                last_written: 0,
            },
            RunModule {
                name: "b".to_string(),
                view: view_b.boxed(),
                log: new_log(),
                last_written: 0,
            },
        ];
        let lines = stop_all(&mut modules).await;
        assert_eq!(
            lines,
            vec!["Stopped 'a'".to_string(), "Stopped 'b'".to_string()]
        );
    }

    #[tokio::test]
    /// CL-R-056 — a module whose stop reports an `Error`-level message is reported as
    /// `Error: failed to stop '<name>': <detail>`.
    async fn ut_stop_all_reports_a_failing_stop() {
        let (view_b, _handle_b) = crate::app::testkit::MockView::pair("b");
        let view_b = view_b.with_command_message(Level::Error, "Stop server failed: boom");
        let log = new_log();
        let mut modules = vec![RunModule {
            name: "b".to_string(),
            view: view_b.boxed(),
            log: log.clone(),
            last_written: 0,
        }];
        let lines = stop_all(&mut modules).await;
        assert_eq!(
            lines,
            vec!["Error: failed to stop 'b': Stop server failed: boom".to_string()]
        );
        let (_, window) = {
            let g = log.read().await;
            (g.written(), g.peek_n(LOG_PEEK))
        };
        assert!(
            window
                .iter()
                .any(|(_, _, msg)| msg == "Stop server failed: boom"),
            "the stop message must still reach the module's log ring"
        );
    }

    #[tokio::test]
    /// CL-R-031, BR-E-011 — with --exit-on-error set, an error line makes the run exit 3.
    async fn ut_run_exit_on_error_returns_three() {
        use ferrowl_util::convert::{Converter, FileType};
        let session = config::Session {
            version: None,
            modules: vec![],
            scripts: vec![ScriptDef {
                name: "boom".into(),
                code: "error(\"boom\")".into(),
                enabled: true,
            }],
            interval: 0.05,
        };
        let dir = reserve_temp_dir("ferrowl_cl");
        let path = dir.join("exit_on_error_session.toml");
        Converter::save(&session, path.to_str().unwrap(), FileType::Toml).unwrap();
        let args = RunArgs {
            sessions: vec![path.to_str().unwrap().to_string()],
            modules: vec![],
            ocpp: vec![],
            duration: Some(5),
            log_file: None,
            exit_on_error: true,
        };
        assert_eq!(run(&args).await, 3);
    }

    #[tokio::test]
    /// CL-R-031, BR-E-011 — with --exit-on-error set, an Error line drained from a *module's* log (not the
    /// session sim's) makes the run exit 3.
    async fn ut_run_module_error_with_exit_on_error_returns_three() {
        let dir = reserve_temp_dir("ferrowl_cl");
        let occupier = reserve_tcp_port();
        let args = timing_out_client_run_args(&dir, occupier.port(), 5, true);
        assert_eq!(run(&args).await, 3);
    }

    #[tokio::test]
    /// CL-R-031 — without --exit-on-error, a module's Error line never changes the exit code.
    /// CL-R-032 — the run instead reaches its --duration deadline and returns 0.
    async fn ut_run_module_error_without_exit_on_error_returns_zero() {
        let dir = reserve_temp_dir("ferrowl_cl");
        let occupier = reserve_tcp_port();
        let mut args = timing_out_client_run_args(&dir, occupier.port(), 1, false);
        let log_file = dir.join("module_error.log").to_str().unwrap().to_string();
        args.log_file = Some(log_file.clone());

        assert_eq!(run(&args).await, 0);
        let contents = std::fs::read_to_string(&log_file).unwrap();
        assert!(
            contents.contains("m |") && contents.contains("timed out"),
            "expected the module's timeout error line to have been drained, got:\n{contents}"
        );
    }

    #[tokio::test]
    /// CL-R-041 — --log-file is opened create-and-append: an existing file is appended to, not
    /// truncated.
    async fn ut_log_file_is_appended_not_truncated() {
        let dir = reserve_temp_dir("ferrowl_cl");
        let log_file = dir.join("append.log").to_str().unwrap().to_string();
        std::fs::write(&log_file, "PREEXISTING\n").unwrap();

        let mut args = modbus_run_args(&dir, reserve_tcp_port().release(), 1);
        args.log_file = Some(log_file.clone());
        assert_eq!(run(&args).await, 0);

        let contents = std::fs::read_to_string(&log_file).unwrap();
        assert!(
            contents.starts_with("PREEXISTING\n"),
            "the pre-existing content must be preserved, got:\n{contents}"
        );
        assert!(
            contents.contains("m |"),
            "new drained lines must be appended, got:\n{contents}"
        );
    }
}
