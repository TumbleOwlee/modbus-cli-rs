use crate::{Error, Result, Script, module::LogLevel, module::LogSink, module::Module};
use mlua::{HookTriggers, Lua, StdLib, UserData, VmState};
use std::{
    cell::Cell,
    collections::HashMap,
    hash::Hash,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// SC-R-034 — the execution hook fires this often, in VM instructions.
const HOOK_INSTRUCTION_INTERVAL: u32 = 1_000;
/// SC-R-034 — wall-clock cap on one cycle (or one on-demand run, SC-R-035).
const HOOK_WALL_CLOCK_CAP: Duration = Duration::from_millis(1_000);

/// Lua context handling module and script loading
pub struct Context<K>
where
    K: Hash + Eq + Default,
{
    lua: Lua,
    scripts: HashMap<K, Script>,
    /// SC-R-034 — wall-clock origin of the cycle (or on-demand run) currently executing; read by
    /// the hook installed in `install_execution_hook`, reset by every script-invoking method
    /// below before it runs anything.
    cycle_start: Rc<Cell<Instant>>,
}

impl<K> Default for Context<K>
where
    K: Hash + Eq + Default,
{
    fn default() -> Self {
        Self {
            lua: Lua::default(),
            scripts: HashMap::default(),
            cycle_start: Rc::new(Cell::new(Instant::now())),
        }
    }
}

impl<K> Context<K>
where
    K: Hash + Eq + Default,
{
    /// SC-R-034, SC-R-039 — install the execution hook on this context's Lua VM. `stop` is
    /// `Some` only for a sim-thread context (SC-R-012); an on-demand run (SC-R-035) passes
    /// `None`, so its hook enforces the wall-clock cap only. An error raised here propagates
    /// through the executing Lua code exactly like any other runtime error, so it is caught by
    /// `Script::exec` and reaches the caller (`call`/`call_all`/`refresh`/`refresh_all`) through
    /// the existing per-script error path — no new error-handling code is needed for SC-R-039.
    pub(crate) fn install_execution_hook(&mut self, stop: Option<Arc<AtomicBool>>) -> Result<()> {
        let cycle_start = self.cycle_start.clone();
        self.lua.set_hook(
            HookTriggers::new().every_nth_instruction(HOOK_INSTRUCTION_INTERVAL),
            move |_lua, _debug| {
                if let Some(stop) = &stop
                    && stop.load(Ordering::Relaxed)
                {
                    return Err(mlua::Error::RuntimeError(
                        "sim thread stop requested".to_string(),
                    ));
                }
                if cycle_start.get().elapsed() > HOOK_WALL_CLOCK_CAP {
                    return Err(mlua::Error::RuntimeError(format!(
                        "script exceeded the {}ms execution cap",
                        HOOK_WALL_CLOCK_CAP.as_millis()
                    )));
                }
                Ok(VmState::Continue)
            },
        )
    }

    /// SC-R-034 — resets the wall-clock origin the hook measures "since the current cycle began"
    /// against. Called by every script-invoking method below, before it runs anything.
    fn begin_cycle(&self) {
        self.cycle_start.set(Instant::now());
    }

    /// Add a new module to the lua context
    pub fn add_module<T>(&mut self, value: T) -> Result<()>
    where
        T: 'static + Module + UserData,
    {
        let globals = self.lua.globals();
        globals.set(T::module(), value)
    }

    /// Enable the standard libraries a sim script is allowed to use.
    ///
    /// Deliberately **not** `StdLib::ALL_SAFE`: that set only drops FFI and `debug`, leaving `io`,
    /// `os` (including `os.execute`), and `package`/`require` reachable -- so any script in a
    /// device or session config could read/write files and spawn processes, and loading a config
    /// would be equivalent to running an untrusted program. Sim scripts model device behavior;
    /// they have no legitimate need for the filesystem, the shell, or dynamic library loading.
    ///
    /// Only the pure computation libraries are kept (`string`, `table`, `math`, `utf8`,
    /// `coroutine`). Clock access, which a sim genuinely needs, is provided by the sandboxed
    /// `C_Time` module instead of `os`.
    ///
    /// `mlua` constructs a `Lua` with `ALL_SAFE` already loaded, so `load_std_libs` can only add
    /// libraries, never remove them -- the unwanted ones (`io`, `os`, `package`) and the base
    /// library's dynamic-code loaders (`load`, `loadfile`, `dofile`, `require`), none of which a
    /// `StdLib` flag can gate off, are therefore removed by clearing them from the globals.
    pub fn enable_stdlib(&mut self) -> Result<()> {
        let safe = StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE;
        self.lua.load_std_libs(safe)?;
        let globals = self.lua.globals();
        for name in [
            "io",
            "os",
            "package",
            "load",
            "loadfile",
            "dofile",
            "loadstring",
            "require",
        ] {
            globals.set(name, mlua::Value::Nil)?;
        }
        Ok(())
    }

    /// Override the global `print` so output goes to the host log instead of stdout
    /// (stdout would corrupt the TUI alternate screen). Mirrors real print semantics:
    /// arguments are converted with tostring semantics and joined by tabs.
    pub fn redirect_print<S: LogSink + 'static>(&mut self, sink: S) -> Result<()> {
        let f = self
            .lua
            .create_function(move |_, args: mlua::Variadic<mlua::Value>| {
                let line = args
                    .iter()
                    .map(mlua::Value::to_string) // tostring semantics, honors __tostring
                    .collect::<std::result::Result<Vec<_>, _>>()?
                    .join("\t");
                sink.log(LogLevel::Info, &line);
                Ok(())
            })?;
        self.lua.globals().set("print", f)
    }

    pub fn iter<'a>(&'a self) -> std::collections::hash_map::Iter<'a, K, Script> {
        self.scripts.iter()
    }

    pub fn iter_mut<'a>(&'a mut self) -> std::collections::hash_map::IterMut<'a, K, Script> {
        self.scripts.iter_mut()
    }

    /// Execute a loaded script specified by specific key
    pub fn call(&mut self, key: &K) -> Result<()> {
        self.begin_cycle();
        match self.scripts.get_mut(key) {
            Some(script) => script.exec(),
            None => Ok(()),
        }
    }

    /// Execute a loaded script specified by specific key while skipping it if it has been executed
    /// in the last timeframe of given duration
    pub fn refresh(&mut self, key: &K, since: std::time::Duration) -> Result<()> {
        self.begin_cycle();
        match self.scripts.get_mut(key) {
            Some(script) if script.since_last_execution() >= since => script.exec(),
            _ => Ok(()),
        }
    }

    pub fn call_all(&mut self) -> std::result::Result<(), Vec<Error>> {
        self.begin_cycle();
        Self::exec_collecting_errors(self.iter_mut())
    }

    /// Execute all loaded scripts while skipping all scripts executed in the last timeframe of
    /// given duration
    pub fn refresh_all(
        &mut self,
        since: std::time::Duration,
    ) -> std::result::Result<(), Vec<Error>> {
        self.begin_cycle();
        Self::exec_collecting_errors(
            self.iter_mut()
                .filter(|(_, v)| v.since_last_execution() >= since),
        )
    }

    /// Execute every script yielded by `scripts`, collecting the errors of the failing ones
    fn exec_collecting_errors<'a>(
        scripts: impl Iterator<Item = (&'a K, &'a mut Script)>,
    ) -> std::result::Result<(), Vec<Error>>
    where
        K: 'a,
    {
        let errors: Vec<_> = scripts
            .map(|(_, v)| v.exec())
            .filter(std::result::Result::is_err)
            .map(|e| e.expect_err("filter kept only Err results"))
            .collect();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Load the script and store it under the given key unless another script is already loaded
    /// for the key
    pub fn load_script(&mut self, key: K, script: &str) -> Result<()> {
        let func = self.lua.load(script).into_function()?;
        if let std::collections::hash_map::Entry::Vacant(e) = self.scripts.entry(key) {
            e.insert(Script::init(func));
            Ok(())
        } else {
            Err(mlua::Error::BindError)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Context;
    use std::time::Duration;

    fn key(s: &str) -> String {
        s.to_string()
    }

    #[test]
    /// SC-R-003 — a script is compiled into a callable function at load time; invalid Lua fails the load.
    fn ut_load_script_rejects_invalid_lua() {
        let mut ctx = Context::<String>::default();
        assert!(
            ctx.load_script(key("bad"), "this is not ! valid lua")
                .is_err()
        );
    }

    #[test]
    /// SC-R-005 — loading a second script under an existing name is rejected; no silent overwrite.
    fn ut_load_script_rejects_duplicate_key() {
        let mut ctx = Context::<String>::default();
        assert!(ctx.load_script(key("a"), "local x = 1").is_ok());
        // Second load under the same key must not overwrite; it returns a bind error.
        assert!(ctx.load_script(key("a"), "local y = 2").is_err());
    }

    #[test]
    fn ut_call_missing_key_is_ok() {
        let mut ctx = Context::<String>::default();
        // No script registered for the key: nothing to run, so it succeeds vacuously.
        assert!(ctx.call(&key("nope")).is_ok());
    }

    #[test]
    /// SC-R-032 — a runtime error raised by a script is surfaced rather than swallowed.
    fn ut_call_runs_script_and_surfaces_runtime_error() {
        let mut ctx = Context::<String>::default();
        ctx.load_script(key("ok"), "local x = 1 + 1").unwrap();
        ctx.load_script(key("boom"), "error('kaboom')").unwrap();
        assert!(ctx.call(&key("ok")).is_ok());
        let err = ctx.call(&key("boom")).unwrap_err();
        assert!(err.to_string().contains("kaboom"));
    }

    #[test]
    /// SC-R-007, SC-R-040 — io/os/package and the base dynamic-code loaders are removed from the globals (seen as nil).
    fn ut_sandbox_denies_filesystem_shell_and_dynamic_loading() {
        // A script in a config is untrusted input; the sandbox must not give it the filesystem,
        // the shell, or a way to pull in more code. Each of these globals must be absent.
        let mut ctx = Context::<String>::default();
        ctx.enable_stdlib().unwrap();
        // The whole table/global is gone, so `os.execute` et al. are unreachable -- an indexing
        // attempt would even throw "index a nil value" rather than return nil.
        for global in [
            "io",
            "os",
            "package",
            "require",
            "load",
            "loadfile",
            "dofile",
            "loadstring",
        ] {
            ctx.load_script(key(global), &format!("assert({global} == nil)"))
                .unwrap();
            assert!(
                ctx.call(&key(global)).is_ok(),
                "sandbox leaks `{global}` to scripts"
            );
        }
    }

    #[test]
    /// SC-R-006 — the pure-computation stdlib subset (string/table/math/…) stays reachable.
    fn ut_sandbox_keeps_pure_computation_libraries() {
        let mut ctx = Context::<String>::default();
        ctx.enable_stdlib().unwrap();
        ctx.load_script(
            key("pure"),
            "assert(string.upper('a') == 'A'); assert(math.floor(1.5) == 1); \
             assert(table.concat({'x'}) == 'x')",
        )
        .unwrap();
        assert!(ctx.call(&key("pure")).is_ok());
    }

    #[test]
    /// SC-R-032 — one script's error is collected and does not stop the others in the context.
    fn ut_call_all_collects_errors() {
        let mut ctx = Context::<String>::default();
        ctx.load_script(key("ok"), "local x = 1").unwrap();
        ctx.load_script(key("boom"), "error('x')").unwrap();
        let errs = ctx.call_all().unwrap_err();
        assert_eq!(errs.len(), 1);
    }

    #[test]
    /// SC-R-044 — refresh runs a script at most once per interval, skipping one that ran more recently.
    fn ut_refresh_skips_recently_executed_script() {
        let mut ctx = Context::<String>::default();
        // A failing script that would error if executed.
        ctx.load_script(key("boom"), "error('x')").unwrap();
        // Just loaded, so its last-execution age is ~0; a one-hour window skips it entirely,
        // proving the throttle: no execution means no error.
        assert!(ctx.refresh(&key("boom"), Duration::from_secs(3600)).is_ok());
        // Without the throttle the same script does run and surfaces its error.
        assert!(ctx.call(&key("boom")).is_err());
    }
}
