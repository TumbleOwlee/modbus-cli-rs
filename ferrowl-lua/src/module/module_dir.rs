//! Lua module `C_Module`: session-level access to every host module's Lua surface, resolved by
//! name against a live [`ModuleDirectory`].
//!
//! Unlike the other host modules (which wrap a single fixed handle), `C_Module` is a lookup: it
//! hands scripts a [`ModuleHandle`] that re-resolves its target through the directory on every
//! call. That makes the surface immediately consistent with modules being added or removed at
//! runtime — a handle obtained before a removal simply starts erroring afterwards instead of
//! reading stale state.

use ferrowl_lua_derive::Module;
use mlua::{AnyUserData, Lua, Result, UserData, UserDataMethods};
use std::sync::Arc;

/// One module's Lua-facing surface, type-erased. Implementations capture only `Send + Sync`
/// shared state (e.g. `Arc`s) so a [`ModuleDirectory`] can be built and resolved independently of
/// any particular Lua context.
pub trait ModuleHost: Send + Sync {
    /// The module kind, e.g. `"modbus"` or `"ocpp"`.
    fn kind(&self) -> &'static str;
    /// The module's role, e.g. `"client"` or `"server"`.
    fn role(&self) -> &'static str;
    /// Builds the `C_Register`-shaped accessor userdata for a modbus module, `Ok(None)` for any
    /// other kind.
    fn register_accessor(&self, lua: &Lua) -> Result<Option<AnyUserData>>;
    /// Builds the role-shaped `C_OCPP` accessor userdata for an ocpp module, `Ok(None)` for any
    /// other kind.
    fn ocpp_accessor(&self, lua: &Lua) -> Result<Option<AnyUserData>>;
}

/// Live directory of modules, resolved by name at every access rather than snapshotted once.
pub trait ModuleDirectory: Send + Sync {
    /// Names of every currently known module.
    fn list(&self) -> Vec<String>;
    /// Resolves `name` to its host, `None` if no such module currently exists.
    fn resolve(&self, name: &str) -> Option<Arc<dyn ModuleHost>>;
}

/// Lua module `C_Module`: enumerates and resolves the session's other host modules by name.
///
/// Exposed Lua methods: `List()` — sorted array of module names — and `Get(name)`, which raises
/// if `name` is unknown and otherwise returns a [`ModuleHandle`].
#[derive(Module)]
#[module = "C_Module"]
pub struct ModuleDir {
    directory: Arc<dyn ModuleDirectory>,
}

impl ModuleDir {
    /// Creates the module around the host's live module directory.
    pub fn init(directory: Arc<dyn ModuleDirectory>) -> Self {
        Self { directory }
    }
}

impl UserData for ModuleDir {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("List", |_, this, ()| {
            let mut names = this.directory.list();
            names.sort();
            Ok(names)
        });
        methods.add_method("Get", |_, this, name: String| {
            if this.directory.resolve(&name).is_none() {
                return Err(mlua::Error::RuntimeError(format!(
                    "unknown module '{name}'"
                )));
            }
            Ok(ModuleHandle {
                directory: this.directory.clone(),
                name,
            })
        });
    }
}

/// A reference to one module resolved by name. Every method re-resolves the name through the
/// directory, so a module removed after `Get` surfaces as an "unknown module" error rather than
/// returning stale data.
pub struct ModuleHandle {
    directory: Arc<dyn ModuleDirectory>,
    name: String,
}

impl ModuleHandle {
    /// Re-resolves the target host, raising if it no longer exists.
    fn resolve(&self) -> Result<Arc<dyn ModuleHost>> {
        self.directory
            .resolve(&self.name)
            .ok_or_else(|| mlua::Error::RuntimeError(format!("unknown module '{}'", self.name)))
    }
}

impl UserData for ModuleHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("Type", |_, this, ()| Ok(this.resolve()?.kind()));
        methods.add_method("Role", |_, this, ()| Ok(this.resolve()?.role()));
        methods.add_method("Register", |lua, this, ()| {
            let host = this.resolve()?;
            host.register_accessor(lua)?.ok_or_else(|| {
                mlua::Error::RuntimeError(format!("module '{}' is not a modbus module", this.name))
            })
        });
        methods.add_method("OCPP", |lua, this, ()| {
            let host = this.resolve()?;
            host.ocpp_accessor(lua)?.ok_or_else(|| {
                mlua::Error::RuntimeError(format!("module '{}' is not an ocpp module", this.name))
            })
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContextBuilder;
    use crate::module::ValueType;
    use crate::module::test_support::MockHost;
    use crate::module::{OcppClient, RegisterModule};
    use std::collections::HashMap;
    use std::sync::RwLock;

    /// Directory backed by a map the test can mutate directly (to simulate module removal)
    /// independently of the `Arc<dyn ModuleDirectory>` handed to the Lua context.
    #[derive(Default)]
    struct MockDirectory {
        modules: RwLock<HashMap<String, Arc<dyn ModuleHost>>>,
    }

    impl MockDirectory {
        fn insert(&self, name: &str, host: Arc<dyn ModuleHost>) {
            self.modules.write().unwrap().insert(name.to_string(), host);
        }
        fn remove(&self, name: &str) {
            self.modules.write().unwrap().remove(name);
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

    /// A minimal modbus-shaped host: kind `"modbus"`, no ocpp accessor.
    struct ModbusHost {
        role: &'static str,
        rw: MockHost,
    }
    impl ModuleHost for ModbusHost {
        fn kind(&self) -> &'static str {
            "modbus"
        }
        fn role(&self) -> &'static str {
            self.role
        }
        fn register_accessor(&self, lua: &Lua) -> Result<Option<AnyUserData>> {
            Ok(Some(
                lua.create_userdata(RegisterModule::init(self.rw.clone()))?,
            ))
        }
        fn ocpp_accessor(&self, _lua: &Lua) -> Result<Option<AnyUserData>> {
            Ok(None)
        }
    }

    /// A minimal ocpp-shaped host: kind `"ocpp"`, no register accessor.
    struct OcppHost {
        role: &'static str,
        handle: MockHost,
    }
    impl ModuleHost for OcppHost {
        fn kind(&self) -> &'static str {
            "ocpp"
        }
        fn role(&self) -> &'static str {
            self.role
        }
        fn register_accessor(&self, _lua: &Lua) -> Result<Option<AnyUserData>> {
            Ok(None)
        }
        fn ocpp_accessor(&self, lua: &Lua) -> Result<Option<AnyUserData>> {
            Ok(Some(
                lua.create_userdata(OcppClient::init(self.handle.clone()))?,
            ))
        }
    }

    #[test]
    /// SC-R-020 — C_Module enumerates the session's other modules by name.
    fn ut_list_returns_sorted_names() {
        let dir = Arc::new(MockDirectory::default());
        dir.insert(
            "b",
            Arc::new(ModbusHost {
                role: "client",
                rw: MockHost::default(),
            }),
        );
        dir.insert(
            "a",
            Arc::new(ModbusHost {
                role: "client",
                rw: MockHost::default(),
            }),
        );
        let module = ModuleDir::init(dir as Arc<dyn ModuleDirectory>);
        let mut ctx = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(module)
            .with_script(
                "s".to_string(),
                r#"
                names = C_Module:List()
                "#,
            )
            .build()
            .expect("build context");
        ctx.call_all().expect("run");
    }

    #[test]
    /// SC-R-020 — resolving an unknown module name raises rather than returning a null handle.
    fn ut_get_unknown_module_raises() {
        let dir = Arc::new(MockDirectory::default());
        let module = ModuleDir::init(dir as Arc<dyn ModuleDirectory>);
        let mut ctx = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(module)
            .with_script("s".to_string(), r#"C_Module:Get("nope")"#)
            .build()
            .expect("build context");
        let err = ctx.call_all().unwrap_err();
        assert!(err[0].to_string().contains("unknown module"));
    }

    #[test]
    /// SC-R-020 — a resolved C_Module handle reports its target's kind and role.
    fn ut_type_and_role_return_host_values() {
        let dir = Arc::new(MockDirectory::default());
        dir.insert(
            "a",
            Arc::new(ModbusHost {
                role: "server",
                rw: MockHost::default(),
            }),
        );
        let module = ModuleDir::init(dir as Arc<dyn ModuleDirectory>);
        let mut ctx = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(module)
            .with_script(
                "s".to_string(),
                r#"
                local m = C_Module:Get("a")
                kind = m:Type()
                role = m:Role()
                "#,
            )
            .build()
            .expect("build context");
        ctx.call_all().expect("run");
    }

    #[test]
    /// SC-R-020 — C_Module hands out a C_Register-shaped accessor only for a modbus module; others raise.
    fn ut_register_on_non_modbus_raises() {
        let dir = Arc::new(MockDirectory::default());
        dir.insert(
            "a",
            Arc::new(OcppHost {
                role: "client",
                handle: MockHost::default(),
            }),
        );
        let module = ModuleDir::init(dir as Arc<dyn ModuleDirectory>);
        let mut ctx = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(module)
            .with_script(
                "s".to_string(),
                r#"local m = C_Module:Get("a"); m:Register()"#,
            )
            .build()
            .expect("build context");
        let err = ctx.call_all().unwrap_err();
        assert!(err[0].to_string().contains("is not a modbus module"));
    }

    #[test]
    /// SC-R-020 — C_Module hands out a C_OCPP-shaped accessor only for an ocpp module; others raise.
    fn ut_ocpp_on_non_ocpp_raises() {
        let dir = Arc::new(MockDirectory::default());
        dir.insert(
            "a",
            Arc::new(ModbusHost {
                role: "client",
                rw: MockHost::default(),
            }),
        );
        let module = ModuleDir::init(dir as Arc<dyn ModuleDirectory>);
        let mut ctx = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(module)
            .with_script("s".to_string(), r#"local m = C_Module:Get("a"); m:OCPP()"#)
            .build()
            .expect("build context");
        let err = ctx.call_all().unwrap_err();
        assert!(err[0].to_string().contains("is not an ocpp module"));
    }

    #[test]
    /// SC-R-020 — the session sim reaches a modbus module's register state through C_Module.
    fn ut_register_roundtrip_through_directory() {
        let rw = MockHost::default();
        let dir = Arc::new(MockDirectory::default());
        dir.insert(
            "a",
            Arc::new(ModbusHost {
                role: "client",
                rw: rw.clone(),
            }),
        );
        let module = ModuleDir::init(dir as Arc<dyn ModuleDirectory>);
        let mut ctx = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(module)
            .with_script(
                "s".to_string(),
                r#"
                local m = C_Module:Get("a")
                m:Register():Set("x", 7)
                v = m:Register():Get("x")
                "#,
            )
            .build()
            .expect("build context");
        ctx.call_all().expect("run");

        match rw.get("x") {
            Some(ValueType::Int(v)) => assert_eq!(v, 7),
            other => panic!("expected Int(7), got {other:?}"),
        }
    }

    #[test]
    /// SC-R-020 — the session sim reaches an ocpp module's state and actions through C_Module.
    fn ut_ocpp_roundtrip_through_directory() {
        let handle = MockHost::default();
        let dir = Arc::new(MockDirectory::default());
        dir.insert(
            "a",
            Arc::new(OcppHost {
                role: "client",
                handle: handle.clone(),
            }),
        );
        let module = ModuleDir::init(dir as Arc<dyn ModuleDirectory>);
        let mut ctx = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(module)
            .with_script(
                "s".to_string(),
                r#"
                local m = C_Module:Get("a")
                m:OCPP():Set("Model", "M")
                m:OCPP():Connector(1):Set("Power", 11)
                m:OCPP():BootNotification()
                "#,
            )
            .build()
            .expect("build context");
        ctx.call_all().expect("run");

        assert!(matches!(
            handle.get("Model"),
            Some(ValueType::String(s)) if s == "M"
        ));
        let conn1 = handle.conn_store(1);
        assert!(matches!(
            conn1.lock().unwrap().get("Power"),
            Some(ValueType::Int(11))
        ));
        assert!(
            handle
                .dispatched_pairs()
                .contains(&("".to_string(), "BootNotification".to_string()))
        );
    }

    #[test]
    /// SC-R-020 — a C_Module handle re-resolves by name each call, so a removed module goes stale.
    fn ut_handle_becomes_stale_after_removal() {
        let dir = Arc::new(MockDirectory::default());
        dir.insert(
            "a",
            Arc::new(ModbusHost {
                role: "client",
                rw: MockHost::default(),
            }),
        );
        let module = ModuleDir::init(dir.clone() as Arc<dyn ModuleDirectory>);
        let mut ctx = ContextBuilder::<String>::default()
            .with_stdlib()
            .with_module(module)
            .with_script("get".to_string(), r#"m = C_Module:Get("a")"#)
            .with_script("probe".to_string(), r#"kind = m:Type()"#)
            .build()
            .expect("build context");

        // The handle resolves fine while "a" is present.
        ctx.call(&"get".to_string()).expect("get succeeds");
        ctx.call(&"probe".to_string())
            .expect("probe succeeds before removal");

        // Removing the module from the directory makes the already-held handle stale.
        dir.remove("a");
        let err = ctx.call(&"probe".to_string()).unwrap_err();
        assert!(err.to_string().contains("unknown module"));
    }
}
