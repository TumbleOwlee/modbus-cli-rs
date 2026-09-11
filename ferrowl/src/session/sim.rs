//! Session-level Lua simulation: scripts run with `C_Module` access to every module in the
//! session, on a dedicated thread that lives iff at least one script is enabled. Mirrors the
//! per-module sim pattern in `lua.rs`/`module/ocpp/client/lua_sim.rs`, but keyed off the
//! session's own `scripts`/`interval` config (`config::session::Session`) rather than a device's
//! per-register `update` scripts.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ferrowl_lua::ContextBuilder;
use ferrowl_lua::module::{LogModule, ModuleDirModule, ModuleDirectory, TestModule, TimeModule};

use crate::app::Level;
use crate::config::script::ScriptDef;
use crate::lua::SimHandle;
use crate::module::ocpp::client::lua_sim::{LuaLogSink, emit, sleep_responsive};
use crate::module::view::SharedLog;

/// Owns the session-level Lua sim thread, restarting it whenever the script list or interval
/// changes (globals reset on restart — intended). Stopped whenever no script is enabled.
pub struct SessionSim {
    scripts: Vec<ScriptDef>,
    interval: Duration,
    log: SharedLog,
    directory: Arc<dyn ModuleDirectory>,
    handle: Option<SimHandle>,
}

impl SessionSim {
    /// Builds the sim around a live module directory and the session's shared log. No scripts
    /// are configured yet, so the thread starts stopped.
    pub fn new(directory: Arc<dyn ModuleDirectory>, log: SharedLog) -> Self {
        Self {
            scripts: Vec::new(),
            interval: Duration::from_secs_f64(1.0),
            log,
            directory,
            handle: None,
        }
    }

    /// Replaces the script list and restarts the sim thread (stopped if none are enabled).
    pub fn set_scripts(&mut self, scripts: Vec<ScriptDef>) {
        self.scripts = scripts;
        self.ensure();
    }

    /// Replaces the sim cycle interval and restarts the sim thread.
    pub fn set_interval(&mut self, interval: Duration) {
        self.interval = interval;
        self.ensure();
    }

    /// Stops the sim thread, if running.
    pub fn stop(&mut self) {
        self.handle = None;
    }

    /// Run one script exactly once on a short-lived detached thread, outside the sim (SC-R-035).
    /// Independent of the sim thread: no enabled filter, no loop, and no effect on `handle` — a
    /// disabled script runs, and a running sim keeps running alongside it on its own Lua state.
    /// Errors log under `[run]`, not `[sim]`.
    pub fn run_once(&self, name: String, code: String) {
        let directory = self.directory.clone();
        let log = self.log.clone();
        std::thread::spawn(move || {
            let context = ContextBuilder::<String>::default()
                .with_stdlib()
                .with_module(ModuleDirModule::init(directory))
                .with_module(TimeModule::default())
                .with_module(TestModule)
                .with_module(LogModule::init(LuaLogSink(log.clone())))
                .with_print_sink(LuaLogSink(log.clone()))
                .with_script(name.clone(), &code)
                .build();
            match context {
                Ok(mut context) => {
                    if let Err(e) = context.call(&name) {
                        emit(&log, Level::Error, &format!("[run] {e}"));
                    }
                }
                Err(e) => emit(
                    &log,
                    Level::Error,
                    &format!("[run] failed to build Lua context: {e}"),
                ),
            }
        });
    }

    /// Tears down any running sim thread, then spawns a fresh one if at least one script is
    /// enabled.
    fn ensure(&mut self) {
        self.handle = None;

        let enabled: Vec<(String, String)> = self
            .scripts
            .iter()
            .filter(|s| s.enabled)
            .map(|s| (s.name.clone(), s.code.clone()))
            .collect();
        if enabled.is_empty() {
            return;
        }

        let directory = self.directory.clone();
        let log = self.log.clone();
        let interval = self.interval;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut builder = ContextBuilder::<String>::default()
                .with_stdlib()
                .with_module(ModuleDirModule::init(directory))
                .with_module(TimeModule::default())
                .with_module(TestModule)
                .with_module(LogModule::init(LuaLogSink(log.clone())))
                .with_print_sink(LuaLogSink(log.clone()))
                .with_stop_flag(thread_stop.clone());
            for (name, code) in &enabled {
                builder = builder.with_script(name.clone(), code);
            }
            let mut context = match builder.build() {
                Ok(context) => context,
                Err(e) => {
                    emit(
                        &log,
                        Level::Error,
                        &format!("[sim] failed to build Lua context: {e}"),
                    );
                    return;
                }
            };

            while !thread_stop.load(Ordering::Relaxed) {
                if let Err(errors) = context.call_all() {
                    for e in errors {
                        emit(&log, Level::Error, &format!("[sim] {e}"));
                    }
                }
                sleep_responsive(interval, &thread_stop);
            }
            emit(&log, Level::Info, "[sim] stopped completely ");
        });

        self.handle = Some(SimHandle::from_parts(stop, handle));
    }
}

impl Drop for SessionSim {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LOG_SIZE;
    use ferrowl_lua::module::{Has, ModuleHost, Read, RegisterModule, ValueType, Write};
    use mlua::{AnyUserData, Lua, Result as LuaResult};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::RwLock as StdRwLock;

    /// `Send + Sync` in-memory register store, mirroring `module_dir.rs`'s test mock.
    #[derive(Clone, Default)]
    struct MockReadWrite {
        store: Arc<Mutex<HashMap<String, ValueType>>>,
    }
    impl Read for MockReadWrite {
        fn read(&self, name: String) -> LuaResult<ValueType> {
            self.store
                .lock()
                .unwrap()
                .get(&name)
                .cloned()
                .ok_or_else(|| mlua::Error::RuntimeError(format!("unknown '{name}'")))
        }
    }
    impl Write for MockReadWrite {
        fn write(&self, name: String, value: ValueType) -> LuaResult<()> {
            self.store.lock().unwrap().insert(name, value);
            Ok(())
        }
    }
    impl Has for MockReadWrite {
        fn has(&self, name: String) -> LuaResult<bool> {
            Ok(self.store.lock().unwrap().get(&name).is_some())
        }
    }

    struct MockHost {
        rw: MockReadWrite,
    }
    impl ModuleHost for MockHost {
        fn kind(&self) -> &'static str {
            "modbus"
        }
        fn role(&self) -> &'static str {
            "server"
        }
        fn register_accessor(&self, lua: &Lua) -> LuaResult<Option<AnyUserData>> {
            Ok(Some(
                lua.create_userdata(RegisterModule::init(self.rw.clone()))?,
            ))
        }
        fn ocpp_accessor(&self, _lua: &Lua) -> LuaResult<Option<AnyUserData>> {
            Ok(None)
        }
    }

    #[derive(Default)]
    struct MockDirectory {
        modules: StdRwLock<HashMap<String, Arc<dyn ModuleHost>>>,
    }
    impl MockDirectory {
        fn insert(&self, name: &str, host: Arc<dyn ModuleHost>) {
            self.modules.write().unwrap().insert(name.to_string(), host);
        }
    }
    impl ModuleDirectory for MockDirectory {
        fn list(&self) -> Vec<String> {
            self.modules.read().unwrap().keys().cloned().collect()
        }
        fn resolve(&self, name: &str) -> Option<Arc<dyn ModuleHost>> {
            self.modules.read().unwrap().get(name).cloned()
        }
    }

    fn log() -> SharedLog {
        Arc::new(tokio::sync::RwLock::new(crate::app::LogRing::init()))
    }

    fn directory_with_mock(rw: MockReadWrite) -> Arc<dyn ModuleDirectory> {
        let dir = Arc::new(MockDirectory::default());
        dir.insert("m", Arc::new(MockHost { rw }));
        dir as Arc<dyn ModuleDirectory>
    }

    fn script(name: &str, code: &str) -> ScriptDef {
        ScriptDef {
            name: name.to_string(),
            code: code.to_string(),
            enabled: true,
        }
    }

    /// Polls `cond` up to `timeout`, sleeping in small steps. Bounded, no raw sleep-as-sync.
    fn wait_for(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
        let step = Duration::from_millis(10);
        let mut waited = Duration::ZERO;
        while waited < timeout {
            if cond() {
                return true;
            }
            std::thread::sleep(step);
            waited += step;
        }
        cond()
    }

    #[test]
    /// SC-R-011 — an enabled session script starts the session sim with no explicit start call.
    fn ut_enabled_script_runs_without_explicit_start() {
        let rw = MockReadWrite::default();
        let directory = directory_with_mock(rw.clone());
        let log = log();
        let mut sim = SessionSim::new(directory, log);
        sim.set_interval(Duration::from_millis(20));
        sim.set_scripts(vec![script(
            "s",
            r#"C_Module:Get("m"):Register():Set("x", 1)"#,
        )]);

        assert!(wait_for(Duration::from_millis(500), || {
            matches!(rw.store.lock().unwrap().get("x"), Some(ValueType::Int(1)))
        }));
    }

    #[test]
    /// SC-R-011 — with every session script disabled, no sim thread exists.
    fn ut_all_disabled_stops() {
        let rw = MockReadWrite::default();
        let directory = directory_with_mock(rw);
        let log = log();
        let mut sim = SessionSim::new(directory, log);
        sim.set_scripts(vec![ScriptDef {
            name: "s".into(),
            code: r#"C_Module:Get("m"):Register():Set("x", 1)"#.into(),
            enabled: false,
        }]);
        assert!(sim.handle.is_none());
    }

    #[test]
    /// SC-R-061 — an enabled script with an empty code body is still handed to a sim thread;
    /// selection filters on the enabled flag alone, not on the code body (SC-E-040).
    fn ut_enabled_empty_code_script_still_starts_sim_thread() {
        let rw = MockReadWrite::default();
        let directory = directory_with_mock(rw);
        let log = log();
        let mut sim = SessionSim::new(directory, log);
        sim.set_scripts(vec![script("s", "")]);
        assert!(sim.handle.is_some());
    }

    #[test]
    /// SC-R-024 — toggling a session script's enabled flag starts then stops the sim thread.
    fn ut_toggle_scripts_starts_and_stops() {
        let rw = MockReadWrite::default();
        let directory = directory_with_mock(rw.clone());
        let log = log();
        let mut sim = SessionSim::new(directory, log);
        sim.set_interval(Duration::from_millis(20));

        sim.set_scripts(vec![script(
            "s",
            r#"C_Module:Get("m"):Register():Set("x", 1)"#,
        )]);
        assert!(sim.handle.is_some());
        assert!(wait_for(Duration::from_millis(500), || {
            matches!(rw.store.lock().unwrap().get("x"), Some(ValueType::Int(1)))
        }));

        sim.set_scripts(vec![ScriptDef {
            name: "s".into(),
            code: r#"C_Module:Get("m"):Register():Set("x", 1)"#.into(),
            enabled: false,
        }]);
        assert!(sim.handle.is_none());
    }

    #[test]
    /// SC-R-012 — stopping the session sim while a script is in a runaway loop completes promptly
    /// via the execution hook (SC-R-034), instead of the between-cycles-only check blocking the
    /// join forever.
    fn ut_stop_interrupts_runaway_session_script_promptly() {
        let rw = MockReadWrite::default();
        let directory = directory_with_mock(rw);
        let log = log();
        let mut sim = SessionSim::new(directory, log);
        sim.set_interval(Duration::from_secs(10));
        sim.set_scripts(vec![script("runaway", "while true do end")]);
        assert!(sim.handle.is_some());

        // Let the sim thread actually enter the runaway script's busy loop before requesting a
        // stop, so the assertion below exercises "interrupted mid-execution", not "never started".
        std::thread::sleep(Duration::from_millis(100));

        let start = std::time::Instant::now();
        sim.stop();
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "stop() blocked instead of the hook interrupting the runaway script"
        );
    }

    #[test]
    /// SC-R-024 — editing a session script restarts the sim on a fresh context so new code takes over.
    fn ut_code_change_restarts_with_new_behavior() {
        let rw = MockReadWrite::default();
        let directory = directory_with_mock(rw.clone());
        let log = log();
        let mut sim = SessionSim::new(directory, log);
        sim.set_interval(Duration::from_millis(20));

        sim.set_scripts(vec![script(
            "s",
            r#"C_Module:Get("m"):Register():Set("x", 1)"#,
        )]);
        assert!(wait_for(Duration::from_millis(500), || {
            matches!(rw.store.lock().unwrap().get("x"), Some(ValueType::Int(1)))
        }));

        sim.set_scripts(vec![script(
            "s",
            r#"C_Module:Get("m"):Register():Set("x", 2)"#,
        )]);
        assert!(wait_for(Duration::from_millis(500), || {
            matches!(rw.store.lock().unwrap().get("x"), Some(ValueType::Int(2)))
        }));
    }

    #[test]
    /// SC-R-032 — a session script error is logged at Error level with a `[sim]` prefix; print still logs.
    fn ut_script_error_logged_with_sim_prefix_and_print_logged() {
        let rw = MockReadWrite::default();
        let directory = directory_with_mock(rw);
        let log = log();
        let mut sim = SessionSim::new(directory, log.clone());
        sim.set_interval(Duration::from_millis(20));
        sim.set_scripts(vec![script(
            "s",
            r#"C_Log:Info("x"); C_Module:Get("nope"):Register()"#,
        )]);

        assert!(wait_for(Duration::from_millis(500), || {
            let lines: Vec<String> = log
                .blocking_read()
                .peek_n(LOG_SIZE)
                .into_iter()
                .map(|(_, _, l)| l)
                .collect();
            lines.iter().any(|l| l.contains("[sim]")) && lines.iter().any(|l| l == "x")
        }));
    }

    #[test]
    /// SC-R-032 — one session script's error does not prevent the others from running that cycle.
    fn ut_unknown_module_error_does_not_stop_loop() {
        let rw = MockReadWrite::default();
        let directory = directory_with_mock(rw.clone());
        let log = log();
        let mut sim = SessionSim::new(directory, log);
        sim.set_interval(Duration::from_millis(20));
        sim.set_scripts(vec![
            script("bad", r#"C_Module:Get("nope"):Register()"#),
            script("good", r#"C_Module:Get("m"):Register():Set("x", 9)"#),
        ]);

        assert!(wait_for(Duration::from_millis(500), || {
            matches!(rw.store.lock().unwrap().get("x"), Some(ValueType::Int(9)))
        }));
    }

    #[test]
    /// SC-R-058, SC-R-060, SC-R-043 — `run_once` executes a **disabled** script on demand and starts no sim
    /// thread: the one-shot is independent of the sim lifecycle.
    fn ut_run_once_executes_disabled_script_without_sim() {
        let rw = MockReadWrite::default();
        let directory = directory_with_mock(rw.clone());
        let mut sim = SessionSim::new(directory, log());
        sim.set_scripts(vec![ScriptDef {
            name: "s".into(),
            code: r#"C_Module:Get("m"):Register():Set("x", 5)"#.into(),
            enabled: false,
        }]);
        assert!(sim.handle.is_none(), "disabled script starts no sim");

        sim.run_once(
            "s".to_string(),
            r#"C_Module:Get("m"):Register():Set("x", 5)"#.to_string(),
        );

        assert!(wait_for(Duration::from_secs(2), || {
            matches!(rw.store.lock().unwrap().get("x"), Some(ValueType::Int(5)))
        }));
        assert!(sim.handle.is_none(), "run_once must not start a sim thread");
    }

    #[test]
    /// SC-R-054 — `run_once` loads only the script being run, not another script of the same
    /// name in the owner's list: the passed code wins even when the list holds a same-named
    /// script with different code.
    fn ut_run_once_loads_only_the_passed_script_not_the_owners_list_entry() {
        let rw = MockReadWrite::default();
        let directory = directory_with_mock(rw.clone());
        let mut sim = SessionSim::new(directory, log());
        // A long cycle interval keeps the enabled decoy's sim thread to a single write, so it
        // cannot race the assertion below.
        sim.set_interval(Duration::from_secs(60));
        sim.set_scripts(vec![ScriptDef {
            name: "s".into(),
            code: r#"C_Module:Get("m"):Register():Set("x", 99)"#.into(),
            enabled: true,
        }]);
        assert!(
            wait_for(Duration::from_millis(500), || {
                matches!(rw.store.lock().unwrap().get("x"), Some(ValueType::Int(99)))
            }),
            "positive control: the enabled decoy in the owner's list must actually run once"
        );

        sim.run_once(
            "s".to_string(),
            r#"C_Module:Get("m"):Register():Set("x", 5)"#.to_string(),
        );

        assert!(wait_for(Duration::from_secs(2), || {
            matches!(rw.store.lock().unwrap().get("x"), Some(ValueType::Int(5)))
        }));
    }

    #[test]
    /// SC-R-055 — an on-demand run calls the loaded script exactly once: an incrementing script
    /// settles at 1, never climbing further.
    fn ut_run_once_calls_the_script_exactly_once() {
        let rw = MockReadWrite::default();
        rw.store
            .lock()
            .unwrap()
            .insert("x".to_string(), ValueType::Int(0));
        let directory = directory_with_mock(rw.clone());
        let sim = SessionSim::new(directory, log());

        sim.run_once(
            "s".to_string(),
            r#"C_Module:Get("m"):Register():Set("x", (C_Module:Get("m"):Register():Get("x") or 0) + 1)"#
                .to_string(),
        );

        assert!(wait_for(Duration::from_secs(2), || {
            matches!(rw.store.lock().unwrap().get("x"), Some(ValueType::Int(1)))
        }));
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            matches!(rw.store.lock().unwrap().get("x"), Some(ValueType::Int(1))),
            "a second, unwanted call would have pushed the count past 1"
        );
    }

    #[test]
    /// SC-R-052 — an on-demand run executes on its own short-lived thread: `run_once` returns to
    /// its caller long before a script slow enough to measure has finished running.
    fn ut_run_once_runs_off_the_calling_thread() {
        let rw = MockReadWrite::default();
        let directory = directory_with_mock(rw.clone());
        let sim = SessionSim::new(directory, log());

        let start = std::time::Instant::now();
        sim.run_once(
            "s".to_string(),
            // A busy Lua loop, slow enough to measure, that only finishes by writing "done".
            r#"
                local n = 0
                for i = 1, 30000000 do n = n + 1 end
                C_Module:Get("m"):Register():Set("x", "done")
            "#
            .to_string(),
        );
        let call_elapsed = start.elapsed();

        assert!(
            wait_for(Duration::from_secs(5), || {
                matches!(rw.store.lock().unwrap().get("x"), Some(ValueType::String(s)) if s == "done")
            }),
            "the busy script never finished"
        );
        let total_elapsed = start.elapsed();

        assert!(
            call_elapsed < total_elapsed / 2,
            "run_once ({call_elapsed:?}) must return long before the script it kicked off \
             finishes ({total_elapsed:?}), proving it runs on its own thread"
        );
    }

    #[test]
    /// SC-R-053 — an on-demand run's context registers the same `C_*` modules the owner's sim
    /// thread would (here, `C_Time`, available to a sim thread via the identical `TimeModule`
    /// registration).
    fn ut_run_once_registers_same_c_star_modules_as_sim_thread() {
        let rw = MockReadWrite::default();
        let directory = directory_with_mock(rw.clone());
        let l = log();
        let sim = SessionSim::new(directory, l.clone());

        sim.run_once(
            "s".to_string(),
            r#"C_Module:Get("m"):Register():Set("x", C_Time:GetMs())"#.to_string(),
        );

        assert!(
            wait_for(Duration::from_secs(2), || {
                matches!(
                    rw.store.lock().unwrap().get("x"),
                    Some(ValueType::Float(_)) | Some(ValueType::Int(_))
                )
            }),
            "log: {:?}",
            l.blocking_read()
                .peek_n(LOG_SIZE)
                .into_iter()
                .map(|(_, _, l)| l)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    /// SC-R-059 — an on-demand run's context shares no Lua state with a sim thread's context: a
    /// Lua global set by the sim thread is invisible to a `run_once` call against the same
    /// directory.
    fn ut_run_once_shares_no_lua_state_with_sim_thread() {
        let rw = MockReadWrite::default();
        let directory = directory_with_mock(rw.clone());
        let log = log();
        let mut sim = SessionSim::new(directory, log);
        sim.set_interval(Duration::from_millis(20));
        sim.set_scripts(vec![script(
            "sim_script",
            r#"shared_global = 42; C_Module:Get("m"):Register():Set("sim_ran", true)"#,
        )]);
        assert!(
            wait_for(Duration::from_millis(500), || {
                matches!(
                    rw.store.lock().unwrap().get("sim_ran"),
                    Some(ValueType::Bool(true))
                )
            }),
            "positive control: the sim thread must have actually run and set shared_global \
             before the no-shared-state assertion means anything"
        );

        sim.run_once(
            "s".to_string(),
            r#"C_Module:Get("m"):Register():Set("x", shared_global or -1)"#.to_string(),
        );

        assert!(wait_for(Duration::from_secs(2), || {
            matches!(rw.store.lock().unwrap().get("x"), Some(ValueType::Int(-1)))
        }));
    }

    #[test]
    /// SC-R-014 — the session sim runs its script about once per cycle interval.
    fn ut_interval_honored_roughly() {
        let rw = MockReadWrite::default();
        rw.store
            .lock()
            .unwrap()
            .insert("x".to_string(), ValueType::Int(0));
        let directory = directory_with_mock(rw.clone());
        let log = log();
        let mut sim = SessionSim::new(directory, log);
        sim.set_interval(Duration::from_millis(50));
        sim.set_scripts(vec![script(
            "s",
            r#"C_Module:Get("m"):Register():Set("x", (C_Module:Get("m"):Register():Get("x") or 0) + 1)"#,
        )]);

        std::thread::sleep(Duration::from_millis(500));
        let count = match rw.store.lock().unwrap().get("x") {
            Some(ValueType::Int(v)) => *v,
            _ => 0,
        };
        assert!(count >= 3, "expected >= 3 executions, got {count}");
    }
}
