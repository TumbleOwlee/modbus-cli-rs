//! Per-module Lua simulation: a `RegisterBridge` exposes the module's `Memory` to Lua as the
//! `C_Register` global (`Get(name)`/`Set(name, value)`), and `run_sim` drives every register's
//! `update` script on a dedicated thread (because `mlua::Lua` is `!Send`).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ferrowl_codec::{Address, Register};
use ferrowl_lua::module::{
    Has, LogLevel, LogModule, LogSink, Read, RegisterModule, TestModule, TimeModule, ValueType,
    Write,
};
use ferrowl_lua::{ContextBuilder, Error, Result};
use ferrowl_modbus::{Key, SlaveKey};
use ferrowl_store::Range;

use crate::app::Level;
use crate::module::modbus::{FileSink, ModuleLog, ModuleMemory, VirtualStore, append};

mod value_conv;
use value_conv::{typed_value_from_type, value_to_type, virtual_value_from_type};

/// Bridges Lua register access (`C_Register`) to the module's shared `Memory` (fixed-address
/// registers) and `VirtualStore` (virtual registers). Runs on the dedicated simulation thread.
/// `memory` is a synchronous (`parking_lot`) lock, locked directly; `virtual_store` is still a
/// tokio `RwLock`, locked with its `blocking_*` ops (safe off a runtime worker thread).
pub struct RegisterBridge {
    memory: ModuleMemory,
    virtual_store: VirtualStore,
    registers: Arc<HashMap<String, Register>>,
}

impl RegisterBridge {
    pub fn new(
        memory: ModuleMemory,
        virtual_store: VirtualStore,
        registers: Arc<HashMap<String, Register>>,
    ) -> Self {
        Self {
            memory,
            virtual_store,
            registers,
        }
    }

    fn register(&self, name: &str) -> Result<&Register> {
        self.registers
            .get(name)
            .ok_or_else(|| Error::RuntimeError(format!("unknown register '{name}'")))
    }
}

impl Read for RegisterBridge {
    fn read(&self, name: String) -> Result<ValueType> {
        let register = self.register(&name)?;
        let addr = match register.address() {
            Address::Fixed(addr) => *addr,
            Address::Virtual => {
                return self
                    .virtual_store
                    .blocking_read()
                    .get(&name)
                    .map(|v| value_to_type(v.clone()))
                    .ok_or_else(|| {
                        Error::RuntimeError(format!("virtual register '{name}' not set"))
                    });
            }
        };
        let width = register.format().width();
        let key = Key {
            id: SlaveKey {
                slave_id: *register.slave_id(),
                kind: register.kind().clone(),
            },
        };
        let raw = self
            .memory
            .read()
            .read_unchecked(key, &Range::new(addr as usize, width))
            .ok_or_else(|| Error::RuntimeError(format!("register '{name}' not readable")))?;
        let value = register
            .decode(&raw)
            .map_err(|e| Error::RuntimeError(format!("decode '{name}': {e}")))?;
        Ok(value_to_type(value))
    }
}

impl Write for RegisterBridge {
    fn write(&self, name: String, value: ValueType) -> Result<()> {
        let register = self.register(&name)?;
        let addr = match register.address() {
            Address::Fixed(addr) => *addr,
            Address::Virtual => {
                let value = virtual_value_from_type(value, register)?;
                self.virtual_store.blocking_write().insert(name, value);
                return Ok(());
            }
        };
        let raw = match value {
            // A Lua string is already a string — no round-trip to avoid, so this still goes
            // through the shared `encode`/`:set` string path.
            ValueType::String(s) => register
                .encode(&s)
                .map_err(|e| Error::RuntimeError(format!("encode '{name}': {e}")))?,
            _ => {
                let typed = typed_value_from_type(value, register.format())?;
                register
                    .encode_value(&typed)
                    .map_err(|e| Error::RuntimeError(format!("encode '{name}': {e}")))?
            }
        };
        let key = Key {
            id: SlaveKey {
                slave_id: *register.slave_id(),
                kind: register.kind().clone(),
            },
        };
        let ok =
            self.memory
                .write()
                .write_unchecked(key, &Range::new(addr as usize, raw.len()), &raw);
        if ok {
            Ok(())
        } else {
            Err(Error::RuntimeError(format!(
                "write to '{name}' rejected (not writable)"
            )))
        }
    }
}

impl Has for RegisterBridge {
    fn has(&self, name: String) -> Result<bool> {
        Ok(self.register(&name).is_ok())
    }
}

/// Controls a running simulation thread: setting the flag and joining the handle stops it.
pub struct SimHandle {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl SimHandle {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    /// Construct a handle from its stop flag and join handle (for sibling sim runners, e.g.
    /// `session_sim`'s session-level runner).
    #[allow(dead_code)]
    pub(crate) fn from_parts(stop: Arc<AtomicBool>, handle: std::thread::JoinHandle<()>) -> Self {
        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for SimHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Spawn the simulation thread for a module. Returns `None` when there are no `update` scripts.
/// The Lua `Context` is built inside the thread because it is `!Send`. Each cycle runs every
/// script via `call_all`; script errors are surfaced into the module log (ring + file).
#[allow(clippy::too_many_arguments)]
pub fn run_sim(
    memory: ModuleMemory,
    virtual_store: VirtualStore,
    registers: HashMap<String, Register>,
    scripts: Vec<(String, String)>,
    interval: Duration,
    log: ModuleLog,
    sink: FileSink,
) -> Option<SimHandle> {
    if scripts.is_empty() {
        return None;
    }

    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let handle = std::thread::spawn(move || {
        let bridge = RegisterBridge::new(memory, virtual_store, Arc::new(registers));
        let mut builder = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(RegisterModule::init(bridge))
            .with_module(TimeModule::default())
            .with_module(TestModule)
            .with_module(LogModule::init(LuaLogSink {
                log: log.clone(),
                sink: sink.clone(),
            }))
            .with_print_sink(LuaLogSink {
                log: log.clone(),
                sink: sink.clone(),
            })
            .with_stop_flag(thread_stop.clone());
        for (name, code) in &scripts {
            builder = builder.with_script(name.clone(), code);
        }
        let mut context = match builder.build() {
            Ok(context) => context,
            Err(e) => {
                emit(
                    &log,
                    &sink,
                    Level::Error,
                    &format!("[sim] failed to build Lua context: {e}"),
                );
                return;
            }
        };

        while !thread_stop.load(Ordering::Relaxed) {
            if let Err(errors) = context.call_all() {
                for e in errors {
                    emit(&log, &sink, Level::Error, &format!("[sim] {e}"));
                }
            }
            sleep_responsive(interval, &thread_stop);
        }
        emit(&log, &sink, Level::Info, "[sim] stopped completely ");
    });

    Some(SimHandle {
        stop,
        handle: Some(handle),
    })
}

/// Run one script exactly once on a short-lived thread, outside any sim (SC-R-035). Unlike
/// [`run_sim`] there is no enabled filter and no loop: the script is executed on a fresh context
/// with the same `C_*` modules, whether or not it is enabled and whether or not a sim is running.
/// Errors are logged under `[run]`, not `[sim]`, so a dialog-driven test run is never mistaken for
/// a sim failure by headless `--exit-on-error` (CL-R-031). The thread is detached — a script with
/// no execution ceiling (SC-R-034) must not be able to block the UI by hanging.
pub fn run_script_once(
    memory: ModuleMemory,
    virtual_store: VirtualStore,
    registers: HashMap<String, Register>,
    name: String,
    code: String,
    log: ModuleLog,
    sink: FileSink,
) {
    std::thread::spawn(move || {
        let bridge = RegisterBridge::new(memory, virtual_store, Arc::new(registers));
        let context = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(RegisterModule::init(bridge))
            .with_module(TimeModule::default())
            .with_module(TestModule)
            .with_module(LogModule::init(LuaLogSink {
                log: log.clone(),
                sink: sink.clone(),
            }))
            .with_print_sink(LuaLogSink {
                log: log.clone(),
                sink: sink.clone(),
            })
            .with_script(name.clone(), &code)
            .build();
        match context {
            Ok(mut context) => {
                if let Err(e) = context.call(&name) {
                    emit(&log, &sink, Level::Error, &format!("[run] {e}"));
                }
            }
            Err(e) => emit(
                &log,
                &sink,
                Level::Error,
                &format!("[run] failed to build Lua context: {e}"),
            ),
        }
    });
}

/// Append a line to the module's ring log and file sink from the (non-runtime) sim thread.
fn emit(log: &ModuleLog, sink: &FileSink, level: Level, line: &str) {
    log.blocking_write().write(level, line);
    append(sink, line);
}

/// Routes `C_Log:Info/Warn/Error(..)` lines from a Modbus sim into the module's ring log and file
/// sink, mirroring the `emit` path used for sim diagnostics.
struct LuaLogSink {
    log: ModuleLog,
    sink: FileSink,
}

impl LogSink for LuaLogSink {
    fn log(&self, level: LogLevel, line: &str) {
        emit(&self.log, &self.sink, level.into(), line);
    }
}

/// Sleep up to `interval` in small chunks so the stop flag is observed promptly.
fn sleep_responsive(interval: Duration, stop: &AtomicBool) {
    let chunk = Duration::from_millis(50);
    let mut slept = Duration::ZERO;
    while slept < interval && !stop.load(Ordering::Relaxed) {
        let step = chunk.min(interval - slept);
        std::thread::sleep(step);
        slept += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrowl_codec::format::{BitField, Endian, Resolution, WordOrder};
    use ferrowl_codec::{Access, Format, Kind, RegisterBuilder};
    use ferrowl_modbus::SlaveKey;
    use ferrowl_modbus::UnitId;
    use ferrowl_store::{CellKind, CellType, Memory};
    use parking_lot::RwLock as MemLock;
    use tokio::sync::RwLock;

    fn holding(addr: u16) -> Register {
        RegisterBuilder::default()
            .slave_id(UnitId(1))
            .access(Access::ReadWrite)
            .kind(Kind::HoldingRegister)
            .address(Address::Fixed(addr))
            .format(Format::u16(
                Endian::Big,
                WordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ))
            .build()
            .unwrap()
    }

    /// Memory holding two U16 registers (setpoint@0, power@1), both read/write.
    fn evse_memory() -> ModuleMemory {
        let mut memory: Memory<Key<SlaveKey>> = Memory::default();
        let key = Key {
            id: SlaveKey {
                slave_id: UnitId(1),
                kind: Kind::HoldingRegister,
            },
        };
        memory.add_ranges(
            key,
            &CellKind::read_write(CellType::Register),
            &[Range::new(0, 2)],
        );
        Arc::new(MemLock::new(memory))
    }

    fn evse_registers() -> HashMap<String, Register> {
        let mut registers = HashMap::new();
        registers.insert("setpoint".to_string(), holding(0));
        registers.insert("power".to_string(), holding(1));
        registers
    }

    fn vstore() -> VirtualStore {
        Arc::new(RwLock::new(HashMap::new()))
    }

    #[test]
    /// SC-R-027 — a virtual register round-trips its natural Lua type through the C_Register bridge.
    fn ut_bridge_virtual_register_roundtrip() {
        let virtual_store = vstore();
        let mut registers = evse_registers();
        registers.insert(
            "calc".to_string(),
            RegisterBuilder::default()
                .slave_id(UnitId(1))
                .access(Access::ReadWrite)
                .kind(Kind::HoldingRegister)
                .address(ferrowl_codec::Address::Virtual)
                .format(Format::u16(
                    Endian::Big,
                    WordOrder::Normal,
                    Resolution(1.0),
                    BitField::default(),
                ))
                .build()
                .unwrap(),
        );
        let bridge = RegisterBridge::new(evse_memory(), virtual_store.clone(), Arc::new(registers));

        // Reading before any write errors; after a write the value round-trips via the store.
        assert!(bridge.read("calc".to_string()).is_err());
        bridge
            .write("calc".to_string(), ValueType::Int(7))
            .expect("virtual write");
        match bridge.read("calc".to_string()).expect("virtual read") {
            ValueType::Int(v) => assert_eq!(v, 7),
            _ => panic!("expected Int"),
        }
        assert_eq!(
            virtual_store
                .blocking_read()
                .get("calc")
                .map(|v| v.clone().unscaled().to_string()),
            Some("7".to_string())
        );
    }

    #[test]
    /// SC-R-028 — a Lua register write is applied to the module's in-memory state and reads back.
    fn ut_bridge_write_then_read() {
        let bridge = RegisterBridge::new(evse_memory(), vstore(), Arc::new(evse_registers()));
        bridge
            .write("setpoint".to_string(), ValueType::Int(100))
            .expect("write");
        match bridge.read("setpoint".to_string()).expect("read") {
            ValueType::Int(v) => assert_eq!(v, 100),
            _ => panic!("expected Int"),
        }
    }

    #[test]
    fn ut_bridge_unknown_register_errors() {
        let bridge = RegisterBridge::new(evse_memory(), vstore(), Arc::new(evse_registers()));
        assert!(bridge.read("nope".to_string()).is_err());
        assert!(bridge.write("nope".to_string(), ValueType::Int(1)).is_err());
    }

    // Mirrors `run_sim`'s body once: a `power` update copying `setpoint` reflects after a cycle.
    #[test]
    /// SC-R-028 — a script's register write lands in the module's in-memory state after a cycle.
    fn ut_sim_script_mirrors_register() {
        let memory = evse_memory();
        let bridge = RegisterBridge::new(memory.clone(), vstore(), Arc::new(evse_registers()));
        bridge
            .write("setpoint".to_string(), ValueType::Int(42))
            .expect("seed setpoint");

        let mut context = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(RegisterModule::init(bridge))
            .with_module(TimeModule::default())
            .with_script(
                "power".to_string(),
                "C_Register:Set(\"power\", C_Register:Get(\"setpoint\"))",
            )
            .build()
            .expect("build context");

        context.call_all().expect("run script");

        let power = memory
            .read()
            .read(
                Key {
                    id: SlaveKey {
                        slave_id: UnitId(1),
                        kind: Kind::HoldingRegister,
                    },
                },
                &CellType::Register,
                &Range::new(1, 1),
            )
            .expect("read power");
        assert_eq!(power, vec![42]);
    }

    // A failing `C_Test` assertion surfaces through the same `call_all` error path the sim loop
    // logs (and headless `--exit-on-error` keys off), mirroring the wiring in `run_sim`.
    #[test]
    /// SC-R-032, SC-R-051 — a failed C_Test assertion surfaces through the sim's collected-error path with its `assertion failed:` prefix intact.
    fn ut_sim_script_test_assertion_failure_surfaces() {
        let bridge = RegisterBridge::new(evse_memory(), vstore(), Arc::new(evse_registers()));
        let mut context = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(RegisterModule::init(bridge))
            .with_module(TimeModule::default())
            .with_module(TestModule)
            .with_script(
                "check".to_string(),
                "C_Test:Assert(C_Register:Get(\"setpoint\") == 1, \"setpoint must be 1\")",
            )
            .build()
            .expect("build context");

        let errors = context.call_all().expect_err("assertion should fail");
        assert!(
            errors.iter().any(|e| e
                .to_string()
                .contains("assertion failed: setpoint must be 1")),
            "expected assertion failure in {errors:?}"
        );
    }

    /// Polls `cond` up to `timeout`, sleeping in small steps (mirrors `session_sim`'s helper).
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

    fn script_log() -> ModuleLog {
        Arc::new(tokio::sync::RwLock::new(crate::app::LogRing::init()))
    }

    /// A sink with no file attached — the run-once path must log to the ring regardless.
    fn no_sink() -> FileSink {
        Arc::new(std::sync::Mutex::new(None))
    }

    fn log_lines(log: &ModuleLog) -> Vec<String> {
        log.blocking_read()
            .peek_n(crate::app::LOG_SIZE)
            .into_iter()
            .map(|(_, _, line)| line)
            .collect()
    }

    #[test]
    /// SC-R-058 — a one-shot run executes the script against the module's registers without a sim.
    fn ut_run_script_once_writes_register() {
        let memory = evse_memory();
        let log = script_log();
        run_script_once(
            memory.clone(),
            vstore(),
            evse_registers(),
            "once".to_string(),
            "C_Register:Set(\"power\", 7)".to_string(),
            log,
            no_sink(),
        );

        assert!(wait_for(Duration::from_secs(2), || {
            memory
                .read()
                .read(
                    Key {
                        id: SlaveKey {
                            slave_id: UnitId(1),
                            kind: Kind::HoldingRegister,
                        },
                    },
                    &CellType::Register,
                    &Range::new(1, 1),
                )
                .is_ok_and(|v| v == vec![7])
        }));
    }

    #[test]
    /// SC-R-049, SC-R-056 — a failing one-shot logs under `[run]`, never `[sim]`: headless `--exit-on-error`
    /// keys its exit code off `[sim]` (CL-R-031) and must not see an interactive test run.
    fn ut_run_script_once_error_logged_with_run_prefix() {
        let log = script_log();
        run_script_once(
            evse_memory(),
            vstore(),
            evse_registers(),
            "bad".to_string(),
            "C_Register:Set(\"nope\", 1)".to_string(),
            log.clone(),
            no_sink(),
        );

        assert!(wait_for(Duration::from_secs(2), || {
            log_lines(&log).iter().any(|l| l.contains("[run]"))
        }));
        assert!(!log_lines(&log).iter().any(|l| l.contains("[sim]")));
    }

    #[test]
    /// SC-R-041 — a sim script context has no `os` library and a working `C_Time`, in the same
    /// context: the existing coverage proves each half separately, in a different context.
    fn ut_sim_context_has_no_os_and_a_working_c_time() {
        let log = script_log();
        run_script_once(
            evse_memory(),
            vstore(),
            evse_registers(),
            "sandbox".to_string(),
            r#"print(os == nil and "os-nil" or "os-present")
               print("time-ok:" .. tostring(C_Time:Get() >= 0))"#
                .to_string(),
            log.clone(),
            no_sink(),
        );

        assert!(wait_for(Duration::from_secs(2), || {
            log_lines(&log).iter().any(|l| l.contains("time-ok:"))
        }));
        let lines = log_lines(&log);
        assert!(
            lines.iter().any(|l| l.contains("os-nil")),
            "os must not be reachable from a sim script: {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("time-ok:true")),
            "C_Time:Get() must work in the same context: {lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("[run]")),
            "the script must not error: {lines:?}"
        );
    }

    // --- Typed write path (no string round-trip) ---

    #[test]
    /// SC-R-027 — int/float/bool register writes round-trip as their natural host type.
    fn ut_bridge_typed_int_float_bool_roundtrip() {
        let bridge = RegisterBridge::new(evse_memory(), vstore(), Arc::new(evse_registers()));

        bridge
            .write("setpoint".to_string(), ValueType::Int(1234))
            .expect("int write");
        match bridge.read("setpoint".to_string()).expect("read") {
            ValueType::Int(v) => assert_eq!(v, 1234),
            _ => panic!("expected Int"),
        }

        bridge
            .write("power".to_string(), ValueType::Bool(true))
            .expect("bool write");
        match bridge.read("power".to_string()).expect("read") {
            ValueType::Int(v) => assert_eq!(v, 1),
            _ => panic!("expected Int"),
        }
    }

    #[test]
    fn ut_bridge_typed_float_whole_number_onto_int_format() {
        // SC-R-027 — a whole-number float writes onto an integer format (no coercion loss).
        let bridge = RegisterBridge::new(evse_memory(), vstore(), Arc::new(evse_registers()));
        bridge
            .write("setpoint".to_string(), ValueType::Float(42.0))
            .expect("whole float write");
        match bridge.read("setpoint".to_string()).expect("read") {
            ValueType::Int(v) => assert_eq!(v, 42),
            _ => panic!("expected Int"),
        }
    }

    #[test]
    /// SC-R-027 — a fractional float onto an integer format is a range mismatch that errors, not truncates.
    fn ut_bridge_typed_float_fractional_onto_int_format_errors() {
        let bridge = RegisterBridge::new(evse_memory(), vstore(), Arc::new(evse_registers()));
        assert!(
            bridge
                .write("setpoint".to_string(), ValueType::Float(3.5))
                .is_err()
        );
    }

    #[test]
    /// SC-R-027 — a string register write is applied via the encode path and reads back as its value.
    fn ut_bridge_typed_string_roundtrip() {
        let bridge = RegisterBridge::new(evse_memory(), vstore(), Arc::new(evse_registers()));
        bridge
            .write("setpoint".to_string(), ValueType::String("77".to_string()))
            .expect("string write");
        match bridge.read("setpoint".to_string()).expect("read") {
            ValueType::Int(v) => assert_eq!(v, 77),
            _ => panic!("expected Int"),
        }
    }

    #[test]
    /// SC-R-027 — a nil register write is rejected rather than silently coerced.
    fn ut_bridge_typed_nil_errors_cleanly() {
        let bridge = RegisterBridge::new(evse_memory(), vstore(), Arc::new(evse_registers()));
        let err = bridge
            .write("setpoint".to_string(), ValueType::Nil)
            .unwrap_err();
        assert!(err.to_string().contains("cannot Set nil value"));
    }

    #[test]
    fn ut_bridge_typed_int_overflow_errors_cleanly() {
        // SC-R-027 — "setpoint" is U16 (max 65535); a too-large Int must error, not silently truncate.
        let bridge = RegisterBridge::new(evse_memory(), vstore(), Arc::new(evse_registers()));
        assert!(
            bridge
                .write("setpoint".to_string(), ValueType::Int(100_000))
                .is_err()
        );
    }

    #[test]
    fn ut_bridge_virtual_typed_int_overflow_falls_back_to_float() {
        // Virtual registers ignore the declared format (mirrors `str_to_value`'s `Scalar`
        // fallback): an out-of-i64-range `Int` is stored as `F64` rather than erroring.
        let virtual_store = vstore();
        let mut registers = evse_registers();
        registers.insert(
            "calc".to_string(),
            RegisterBuilder::default()
                .slave_id(UnitId(1))
                .access(Access::ReadWrite)
                .kind(Kind::HoldingRegister)
                .address(ferrowl_codec::Address::Virtual)
                .format(Format::u16(
                    Endian::Big,
                    WordOrder::Normal,
                    Resolution(1.0),
                    BitField::default(),
                ))
                .build()
                .unwrap(),
        );
        let bridge = RegisterBridge::new(evse_memory(), virtual_store, Arc::new(registers));
        bridge
            .write("calc".to_string(), ValueType::Int(i128::MAX))
            .expect("virtual int overflow falls back to float");
        match bridge.read("calc".to_string()).expect("read") {
            ValueType::Float(_) => {}
            _ => panic!("expected Float fallback"),
        }
    }

    #[test]
    /// SC-R-027 — a nil write to a virtual register is rejected rather than coerced.
    fn ut_bridge_virtual_typed_nil_errors_cleanly() {
        let virtual_store = vstore();
        let mut registers = evse_registers();
        registers.insert(
            "calc".to_string(),
            RegisterBuilder::default()
                .slave_id(UnitId(1))
                .access(Access::ReadWrite)
                .kind(Kind::HoldingRegister)
                .address(ferrowl_codec::Address::Virtual)
                .format(Format::u16(
                    Endian::Big,
                    WordOrder::Normal,
                    Resolution(1.0),
                    BitField::default(),
                ))
                .build()
                .unwrap(),
        );
        let bridge = RegisterBridge::new(evse_memory(), virtual_store, Arc::new(registers));
        let err = bridge
            .write("calc".to_string(), ValueType::Nil)
            .unwrap_err();
        assert!(err.to_string().contains("cannot Set nil value"));
    }

    // --- Execution model: dedicated thread, stop control, responsive sleep, shared locking ---

    /// A log sink that records the OS thread on which Lua logging (and thus execution) occurred.
    #[derive(Clone)]
    struct ThreadIdSink(Arc<std::sync::Mutex<Option<std::thread::ThreadId>>>);
    impl LogSink for ThreadIdSink {
        fn log(&self, _level: LogLevel, _line: &str) {
            *self.0.lock().unwrap() = Some(std::thread::current().id());
        }
    }

    #[test]
    /// SC-R-010 — a sim owner runs its Lua context on a dedicated OS thread, building and executing
    /// the context there rather than on the caller (UI/async) thread.
    fn ut_sim_runs_lua_on_a_dedicated_thread() {
        let seen = Arc::new(std::sync::Mutex::new(None));
        let sink = ThreadIdSink(seen.clone());
        let caller = std::thread::current().id();

        // Mirror run_sim's model: the context is built and run inside its own thread (mlua::Lua is
        // !Send, so it cannot be built on one thread and moved to another).
        std::thread::spawn(move || {
            let mut ctx = ContextBuilder::<String>::default()
                .with_stdlib()
                .with_print_sink(sink)
                .with_script("t".to_string(), r#"print("ran")"#)
                .build()
                .unwrap();
            let _ = ctx.call_all();
        })
        .join()
        .unwrap();

        let ran_on = seen.lock().unwrap().expect("the script must have executed");
        assert_ne!(
            ran_on, caller,
            "Lua must execute on a dedicated thread, not the caller"
        );
    }

    #[test]
    /// SC-R-012 — setting the stop flag and joining stops the sim; the sim handle's destruction
    /// (Drop) also stops and joins its thread.
    fn ut_sim_stops_on_flag_and_on_drop() {
        let noop = || vec![("noop".to_string(), "local x = 1".to_string())];

        // Explicit stop(): once it returns, the thread has been joined and logged its exit.
        let log = script_log();
        let mut handle = run_sim(
            evse_memory(),
            vstore(),
            evse_registers(),
            noop(),
            Duration::from_millis(20),
            log.clone(),
            no_sink(),
        )
        .expect("a non-empty script set spawns a sim thread");
        handle.stop();
        assert!(
            log_lines(&log)
                .iter()
                .any(|l| l.contains("[sim] stopped completely"))
        );

        // Drop path: dropping the handle stops and joins without an explicit stop().
        let log2 = script_log();
        {
            let _h = run_sim(
                evse_memory(),
                vstore(),
                evse_registers(),
                noop(),
                Duration::from_millis(20),
                log2.clone(),
                no_sink(),
            )
            .expect("a non-empty script set spawns a sim thread");
        }
        assert!(
            log_lines(&log2)
                .iter()
                .any(|l| l.contains("[sim] stopped completely"))
        );
    }

    #[test]
    /// SC-R-012 — a stop request is observed *during* a runaway cycle via the execution hook
    /// (SC-R-034), not only between cycles: `stop()` returns promptly instead of blocking on the
    /// join forever.
    fn ut_sim_stop_interrupts_runaway_script_promptly() {
        let log = script_log();
        let mut handle = run_sim(
            evse_memory(),
            vstore(),
            evse_registers(),
            vec![("runaway".to_string(), "while true do end".to_string())],
            Duration::from_secs(10), // long interval: a between-cycle-only check would never fire
            log.clone(),
            no_sink(),
        )
        .expect("a non-empty script set spawns a sim thread");

        // Let the sim thread actually enter the runaway script's busy loop before requesting a
        // stop, so the assertion below exercises "interrupted mid-execution", not "never started".
        std::thread::sleep(Duration::from_millis(100));

        let start = std::time::Instant::now();
        handle.stop();
        // Tight enough to distinguish "the stop flag itself interrupted the runaway script" from
        // the unconditional ~1,000ms wall-clock cap alone (SC-R-034(b), which every context gets
        // for free regardless of whether a stop flag is attached) still bounding it eventually.
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "stop() blocked on the join instead of the hook interrupting the runaway script"
        );
    }

    #[test]
    /// SC-R-013 — the cycle sleep is chunked and re-checks the stop flag, so a stop set during the
    /// idle portion of a cycle is observed promptly instead of after the full interval.
    fn ut_sleep_responsive_observes_stop_promptly() {
        // Already-requested stop: the chunked sleep must bail out well before the 10s interval.
        let stopped = AtomicBool::new(true);
        let start = std::time::Instant::now();
        sleep_responsive(Duration::from_secs(10), &stopped);
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "a stopped sim must not sleep the full cycle interval"
        );

        // Without a stop, it does sleep roughly the (short) interval.
        let running = AtomicBool::new(false);
        let start = std::time::Instant::now();
        sleep_responsive(Duration::from_millis(80), &running);
        assert!(start.elapsed() >= Duration::from_millis(60));
    }

    #[test]
    /// SC-R-029 — host state reached from Lua is guarded by the same lock the network task uses: a
    /// bridge write blocks while that lock is held elsewhere and completes once it is released.
    fn ut_bridge_write_contends_on_the_host_lock() {
        let memory = evse_memory();
        let bridge = RegisterBridge::new(memory.clone(), vstore(), Arc::new(evse_registers()));
        let done = Arc::new(AtomicBool::new(false));

        // Hold the module memory's write lock, exactly as the network task would while updating.
        let guard = memory.write();

        let done_writer = done.clone();
        let writer = std::thread::spawn(move || {
            // Takes the same lock, so it cannot make progress until the guard above is released.
            bridge
                .write("setpoint".to_string(), ValueType::Int(5))
                .expect("write");
            done_writer.store(true, Ordering::Relaxed);
        });

        // While we hold the lock, the Lua-side write is blocked on it.
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            !done.load(Ordering::Relaxed),
            "the write proceeded without acquiring the shared host lock"
        );

        drop(guard); // release: the contended write now completes
        assert!(wait_for(Duration::from_secs(2), || done.load(Ordering::Relaxed)));
        writer.join().unwrap();

        // The value the Lua write applied is the one now in shared host memory.
        let setpoint = memory
            .read()
            .read(
                Key {
                    id: SlaveKey {
                        slave_id: UnitId(1),
                        kind: Kind::HoldingRegister,
                    },
                },
                &CellType::Register,
                &Range::new(0, 1),
            )
            .expect("read setpoint");
        assert_eq!(setpoint, vec![5]);
    }
}
