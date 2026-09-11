//! Session-level module registry: the live `Arc<dyn ModuleHost>` set the `C_Module` Lua module
//! resolves against, plus the per-module-type [`ModuleHost`] implementations that bridge the
//! registry into each module's existing Lua glue (`RegisterBridge` for modbus, `ClientCsHandle`/
//! `ServerHost` for OCPP). `App` rebuilds the whole set from `Tab::name -> ModuleView::module_host`
//! whenever tabs are added/removed/renamed or a view is replaced (see `App::rebuild_registry`).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;

use ferrowl_codec::Register;
use ferrowl_lua::module::{ModuleDirectory, ModuleHost, OcppClient, OcppServer, RegisterModule};
use mlua::{AnyUserData, Lua, Result as LuaResult};

use crate::lua::RegisterBridge;
use crate::module::modbus::{ModuleMemory, VirtualStore};
use crate::module::ocpp::client::lua_sim::{ClientCsHandle, ClientFields, ScopedActionQueue};
use crate::module::ocpp::server::lua::{ServerActionQueue, ServerHost, SharedServerStates};
use crate::module::ocpp::server::view::ServerVersion;

/// Live directory of every open tab's module, keyed by tab name.
#[derive(Clone, Default)]
pub struct ModuleRegistry {
    modules: Arc<RwLock<HashMap<String, Arc<dyn ModuleHost>>>>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the whole module set atomically (the single writer — `App::rebuild_registry`).
    pub fn replace_all(&self, modules: HashMap<String, Arc<dyn ModuleHost>>) {
        *self.modules.write() = modules;
    }
}

impl ModuleDirectory for ModuleRegistry {
    fn list(&self) -> Vec<String> {
        self.modules.read().keys().cloned().collect()
    }

    fn resolve(&self, name: &str) -> Option<Arc<dyn ModuleHost>> {
        self.modules.read().get(name).cloned()
    }
}

// --- Modbus ------------------------------------------------------------------

/// `ModuleHost` for a modbus module: builds a fresh [`RegisterBridge`] accessor over the memory/
/// virtual-store/register-set snapshot the owning view handed to [`ModuleView::module_host`]
/// (`crate::module::view::ModuleView`).
pub struct ModbusHost {
    pub memory: ModuleMemory,
    pub virtual_store: VirtualStore,
    pub registers: Arc<HashMap<String, Register>>,
    pub role: &'static str,
}

impl ModuleHost for ModbusHost {
    fn kind(&self) -> &'static str {
        "modbus"
    }

    fn role(&self) -> &'static str {
        self.role
    }

    fn register_accessor(&self, lua: &Lua) -> LuaResult<Option<AnyUserData>> {
        let bridge = RegisterBridge::new(
            self.memory.clone(),
            self.virtual_store.clone(),
            self.registers.clone(),
        );
        Ok(Some(lua.create_userdata(RegisterModule::init(bridge))?))
    }

    fn ocpp_accessor(&self, _lua: &Lua) -> LuaResult<Option<AnyUserData>> {
        Ok(None)
    }
}

// --- OCPP client ---------------------------------------------------------------

/// `ModuleHost` for an OCPP client (charging-station) module: builds a fresh [`OcppClient`]
/// accessor over the shared CS state + scoped action queue, mirroring the running sim's
/// [`crate::module::ocpp::client::lua_sim::run_client_sim`] wiring.
pub struct OcppClientEntry<S: ClientFields + Send + Sync + 'static> {
    pub state: Arc<parking_lot::RwLock<S>>,
    pub queue: ScopedActionQueue,
}

impl<S: ClientFields + Send + Sync + 'static> ModuleHost for OcppClientEntry<S> {
    fn kind(&self) -> &'static str {
        "ocpp"
    }

    fn role(&self) -> &'static str {
        "client"
    }

    fn register_accessor(&self, _lua: &Lua) -> LuaResult<Option<AnyUserData>> {
        Ok(None)
    }

    fn ocpp_accessor(&self, lua: &Lua) -> LuaResult<Option<AnyUserData>> {
        let handle = ClientCsHandle::new(self.state.clone(), self.queue.clone());
        Ok(Some(lua.create_userdata(OcppClient::init(handle))?))
    }
}

// --- OCPP server ---------------------------------------------------------------

/// `ModuleHost` for an OCPP server (CSMS) module: builds a fresh [`OcppServer`] accessor over the
/// shared per-station state registry + action queue, mirroring the running sim's
/// [`crate::module::ocpp::server::lua::run_server_sim`] wiring.
pub struct OcppServerEntry<V: ServerVersion + Send + Sync + 'static> {
    pub states: SharedServerStates<V>,
    pub queue: ServerActionQueue,
}

impl<V: ServerVersion + Send + Sync + 'static> ModuleHost for OcppServerEntry<V> {
    fn kind(&self) -> &'static str {
        "ocpp"
    }

    fn role(&self) -> &'static str {
        "server"
    }

    fn register_accessor(&self, _lua: &Lua) -> LuaResult<Option<AnyUserData>> {
        Ok(None)
    }

    fn ocpp_accessor(&self, lua: &Lua) -> LuaResult<Option<AnyUserData>> {
        let host = ServerHost::new(self.states.clone(), self.queue.clone());
        Ok(Some(lua.create_userdata(OcppServer::init(host))?))
    }
}

/// Resolve name collisions for a batch of tab names built in order (session load): the first
/// occurrence of a name keeps it, later duplicates get ` (2)`, ` (3)`, ... suffixes, skipping any
/// candidate that (rarely) collides with a name earlier in the batch.
pub fn dedupe_names(names: &[String]) -> Vec<String> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        if seen.insert(name.clone()) {
            out.push(name.clone());
            continue;
        }
        let mut n = 2;
        loop {
            let candidate = format!("{name} ({n})");
            if seen.insert(candidate.clone()) {
                out.push(candidate);
                break;
            }
            n += 1;
        }
    }
    out
}

// Compile-time Send + Sync assertions for every host type the registry stores as
// `Arc<dyn ModuleHost>` (`ModuleHost: Send + Sync`).
#[allow(dead_code)]
fn _assert_send_sync<T: Send + Sync>() {}
#[allow(dead_code)]
fn _assert_host_types_send_sync() {
    _assert_send_sync::<ModbusHost>();
    _assert_send_sync::<OcppClientEntry<crate::module::ocpp::client::v1_6::state::CsState>>();
    _assert_send_sync::<OcppServerEntry<ferrowl_ocpp::V1_6>>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrowl_modbus::UnitId;

    struct StubHost {
        kind: &'static str,
        role: &'static str,
    }
    impl ModuleHost for StubHost {
        fn kind(&self) -> &'static str {
            self.kind
        }
        fn role(&self) -> &'static str {
            self.role
        }
        fn register_accessor(&self, _lua: &Lua) -> LuaResult<Option<AnyUserData>> {
            Ok(None)
        }
        fn ocpp_accessor(&self, _lua: &Lua) -> LuaResult<Option<AnyUserData>> {
            Ok(None)
        }
    }

    #[test]
    /// SC-R-020 — the module registry (C_Module) resolves and lists modules by name.
    fn ut_registry_resolve_and_list() {
        let registry = ModuleRegistry::new();
        assert!(registry.list().is_empty());
        assert!(registry.resolve("a").is_none());

        let mut modules: HashMap<String, Arc<dyn ModuleHost>> = HashMap::new();
        modules.insert(
            "a".to_string(),
            Arc::new(StubHost {
                kind: "modbus",
                role: "client",
            }),
        );
        modules.insert(
            "b".to_string(),
            Arc::new(StubHost {
                kind: "ocpp",
                role: "server",
            }),
        );
        registry.replace_all(modules);

        let mut names = registry.list();
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
        let a = registry.resolve("a").expect("a resolves");
        assert_eq!(a.kind(), "modbus");
        assert_eq!(a.role(), "client");
        assert!(registry.resolve("nope").is_none());
    }

    #[test]
    /// SC-R-020 — replacing the registry drops stale module entries.
    fn ut_registry_replace_all_drops_stale_entries() {
        let registry = ModuleRegistry::new();
        let mut first: HashMap<String, Arc<dyn ModuleHost>> = HashMap::new();
        first.insert(
            "a".to_string(),
            Arc::new(StubHost {
                kind: "modbus",
                role: "client",
            }),
        );
        registry.replace_all(first);
        assert!(registry.resolve("a").is_some());

        // A second `replace_all` without "a" makes it unresolvable — mirrors what happens after
        // its tab is closed.
        registry.replace_all(HashMap::new());
        assert!(registry.resolve("a").is_none());
    }

    #[test]
    /// CS-R-058, CS-E-024 — the first occurrence of a duplicated instance name is left unchanged.
    fn ut_dedupe_names_first_occurrence_unchanged() {
        assert_eq!(
            dedupe_names(&["a".to_string(), "b".to_string()]),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    /// CS-R-058, CS-E-024 — later duplicate names are suffixed ` (2)`, ` (3)`, … in creation order.
    fn ut_dedupe_names_suffixes_repeats_in_order() {
        let names = vec!["a".to_string(), "a".to_string(), "a".to_string()];
        assert_eq!(
            dedupe_names(&names),
            vec!["a".to_string(), "a (2)".to_string(), "a (3)".to_string()]
        );
    }

    #[test]
    /// CS-R-058, CS-E-024 — name de-duplication skips a suffix already taken by an earlier name.
    fn ut_dedupe_names_skips_candidate_colliding_with_earlier_name() {
        // "a (2)" is already taken by an earlier distinct name, so the second "a" must skip to
        // "a (3)".
        let names = vec!["a".to_string(), "a (2)".to_string(), "a".to_string()];
        assert_eq!(
            dedupe_names(&names),
            vec!["a".to_string(), "a (2)".to_string(), "a (3)".to_string()]
        );
    }

    #[test]
    /// UI-R-004 — tab display names are unique at all times: repeats of a name are auto-suffixed
    /// so no two tabs ever collide.
    fn ut_tab_names_are_unique_after_dedupe() {
        let names = vec![
            "Modbus".to_string(),
            "Modbus".to_string(),
            "Modbus".to_string(),
        ];
        let out = dedupe_names(&names);
        let unique: std::collections::HashSet<_> = out.iter().collect();
        assert_eq!(unique.len(), out.len(), "every tab name must be distinct");
        assert_eq!(out, vec!["Modbus", "Modbus (2)", "Modbus (3)"]);
    }

    // --- End-to-end through a real Lua context ----------------------------------

    use ferrowl_codec::format::{BitField, Endian, Resolution, WordOrder};
    use ferrowl_codec::{Access, Format, Kind, RegisterBuilder};
    use ferrowl_lua::ContextBuilder;
    use ferrowl_lua::module::{ModuleDirModule, ValueType};
    use ferrowl_modbus::{Key, SlaveKey};
    use ferrowl_store::{CellKind, Memory, Range};

    fn holding(addr: u16) -> Register {
        RegisterBuilder::default()
            .slave_id(UnitId(1))
            .access(Access::ReadWrite)
            .kind(Kind::HoldingRegister)
            .address(ferrowl_codec::Address::Fixed(addr))
            .format(Format::u16(
                Endian::Big,
                WordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ))
            .build()
            .unwrap()
    }

    /// Memory holding one U16 register ("setpoint" @ 0), read/write, mirroring `lua.rs`'s fixture
    /// style.
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
            &CellKind::read_write(ferrowl_store::CellType::Register),
            &[Range::new(0, 1)],
        );
        Arc::new(RwLock::new(memory))
    }

    #[test]
    /// SC-R-020 — a modbus module's register state is reachable from the session sim via the C_Module directory.
    fn ut_modbus_host_roundtrip_through_directory() {
        let mut registers = HashMap::new();
        registers.insert("setpoint".to_string(), holding(0));
        let memory = evse_memory();
        let host = ModbusHost {
            memory: memory.clone(),
            virtual_store: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            registers: Arc::new(registers),
            role: "client",
        };

        let registry = ModuleRegistry::new();
        let mut modules: HashMap<String, Arc<dyn ModuleHost>> = HashMap::new();
        modules.insert("evse".to_string(), Arc::new(host));
        registry.replace_all(modules);

        let module = ModuleDirModule::init(Arc::new(registry) as Arc<dyn ModuleDirectory>);
        let mut ctx = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(module)
            .with_script(
                "s".to_string(),
                r#"
                local m = C_Module:Get("evse")
                kind = m:Type()
                role = m:Role()
                m:Register():Set("setpoint", 42)
                v = m:Register():Get("setpoint")
                "#,
            )
            .build()
            .expect("build context");
        ctx.call_all().expect("run");

        let key = Key {
            id: SlaveKey {
                slave_id: UnitId(1),
                kind: Kind::HoldingRegister,
            },
        };
        let raw = memory
            .read()
            .read_unchecked(key, &Range::new(0, 1))
            .expect("register readable");
        assert_eq!(raw, vec![42]);
    }

    #[test]
    /// SC-R-020 — an ocpp client module's state and actions are reachable from the session sim via C_Module.
    fn ut_ocpp_client_entry_roundtrip_through_directory() {
        use crate::module::ocpp::client::lua_sim::ClientFields;
        use crate::module::ocpp::client::v1_6::state::CsState as Cs16;
        use crate::module::ocpp::scope::Scope;

        let state: Arc<RwLock<Cs16>> = Arc::new(RwLock::new(Cs16::default()));
        let queue: ScopedActionQueue = Arc::new(parking_lot::Mutex::new(Default::default()));
        let entry = OcppClientEntry {
            state: state.clone(),
            queue: queue.clone(),
        };

        let registry = ModuleRegistry::new();
        let mut modules: HashMap<String, Arc<dyn ModuleHost>> = HashMap::new();
        modules.insert("cs1".to_string(), Arc::new(entry));
        registry.replace_all(modules);

        let module = ModuleDirModule::init(Arc::new(registry) as Arc<dyn ModuleDirectory>);
        let mut ctx = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(module)
            .with_script(
                "s".to_string(),
                r#"
                local m = C_Module:Get("cs1")
                kind = m:Type()
                role = m:Role()
                m:OCPP():Set("Model", "X")
                m:OCPP():Connector(1):Set("Power", 5.0)
                m:OCPP():BootNotification()
                "#,
            )
            .build()
            .expect("build context");
        ctx.call_all().expect("run");

        assert!(matches!(
            state.read().cs_get("Model"),
            Some(ValueType::String(ref s)) if s == "X"
        ));
        assert!(matches!(
            state.read().conn_get(1, "Power"),
            Some(ValueType::Float(v)) if v == 5.0
        ));
        let (scope, action, _) = queue.lock().pop_front().expect("dispatched action");
        assert_eq!(scope, Scope::CS);
        assert_eq!(action, "BootNotification");
    }

    #[test]
    /// SC-R-020 — an ocpp server module's stations are reachable from the session sim via C_Module.
    fn ut_ocpp_server_entry_roundtrip_through_directory() {
        use crate::module::ocpp::client::lua_sim::OcppFields;
        use crate::module::ocpp::lock::with_state_mut;
        use crate::module::ocpp::scope::Scope;
        use crate::module::ocpp::server::lua::ServerStates;
        use ferrowl_ocpp::V1_6;

        type Cs = <V1_6 as ServerVersion>::Cs;
        type Conn = <V1_6 as ServerVersion>::Conn;

        let states: SharedServerStates<V1_6> = Arc::new(RwLock::new(ServerStates::default()));
        with_state_mut(&states, |reg| {
            let st = reg.stations.entry("CP1".to_string()).or_default();
            st.cs = Some(Arc::new(RwLock::new(Cs::default())));
            st.conns
                .push((Scope::connector(1), Arc::new(RwLock::new(Conn::default()))));
        });
        let queue: ServerActionQueue = Arc::new(parking_lot::Mutex::new(Default::default()));
        let entry = OcppServerEntry {
            states: states.clone(),
            queue: queue.clone(),
        };

        let registry = ModuleRegistry::new();
        let mut modules: HashMap<String, Arc<dyn ModuleHost>> = HashMap::new();
        modules.insert("csms".to_string(), Arc::new(entry));
        registry.replace_all(modules);

        let module = ModuleDirModule::init(Arc::new(registry) as Arc<dyn ModuleDirectory>);
        let mut ctx = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(module)
            .with_script(
                "s".to_string(),
                r#"
                local m = C_Module:Get("csms")
                kind = m:Type()
                role = m:Role()
                stations = m:OCPP():GetChargingStations()
                conns = m:OCPP():GetConnectors("CP1")
                m:OCPP():ChargingStation("CP1"):Set("Model", "X")
                m:OCPP():Connector("CP1", 1):RemoteStartTransaction()
                "#,
            )
            .build()
            .expect("build context");
        ctx.call_all().expect("run");

        assert!(matches!(
            with_state_mut(&states, |reg| reg
                .stations
                .get("CP1")
                .and_then(|st| st.cs.clone()))
            .map(|cs| cs.read().get_field("Model")),
            Some(Some(ValueType::String(ref s))) if s == "X"
        ));
        let (identity, scope, action, _) = queue.lock().pop_front().expect("dispatched action");
        assert_eq!(identity, "CP1");
        assert_eq!(scope, Scope::connector(1));
        assert_eq!(action, "RemoteStartTransaction");
    }

    #[test]
    /// SC-R-020 — a handle resolved before a registry replace goes stale and errors.
    fn ut_resolve_stale_after_replace_all_errors() {
        let registry = ModuleRegistry::new();
        let mut modules: HashMap<String, Arc<dyn ModuleHost>> = HashMap::new();
        modules.insert(
            "a".to_string(),
            Arc::new(StubHost {
                kind: "modbus",
                role: "client",
            }),
        );
        registry.replace_all(modules);

        let module = ModuleDirModule::init(Arc::new(registry.clone()) as Arc<dyn ModuleDirectory>);
        let mut ctx = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(module)
            .with_script("get".to_string(), r#"m = C_Module:Get("a")"#)
            .with_script("probe".to_string(), r#"kind = m:Type()"#)
            .build()
            .expect("build context");
        ctx.call(&"get".to_string()).expect("get succeeds");
        ctx.call(&"probe".to_string())
            .expect("probe succeeds before removal");

        registry.replace_all(HashMap::new());
        let err = ctx.call(&"probe".to_string()).unwrap_err();
        assert!(err.to_string().contains("unknown module"));
    }
}
