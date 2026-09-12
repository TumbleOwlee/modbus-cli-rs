//! Shared in-memory host fixtures for the module unit tests.

use crate::module::{Has, OcppActions, OcppClientHost, Read, ValueType, Write};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A key/value state store shared between a mock handle and the test asserting on it.
pub(crate) type Store = Arc<Mutex<HashMap<String, ValueType>>>;

/// One recorded `dispatch` call: the scope label of the handle it was called on and the action
/// name.
#[derive(Clone, Debug)]
pub(crate) struct Dispatch {
    pub(crate) scope: String,
    pub(crate) action: String,
}

/// Shared `Send + Sync` in-memory host, implementing every trait the module unit tests mock:
/// `Read + Write + Has` for a modbus-shaped host, `Read + Write + OcppActions + OcppClientHost`
/// for an ocpp-shaped one.
#[derive(Clone, Default)]
pub(crate) struct MockHost {
    pub(crate) scope: String,
    pub(crate) store: Store,
    pub(crate) conns: Arc<Mutex<HashMap<i64, Store>>>,
    pub(crate) dispatched: Arc<Mutex<Vec<Dispatch>>>,
}

impl MockHost {
    /// Snapshot of a stored value by name.
    pub(crate) fn get(&self, name: &str) -> Option<ValueType> {
        self.store.lock().unwrap().get(name).cloned()
    }

    /// The state store of the connector `id`. Panics if `id` has never been touched.
    pub(crate) fn conn_store(&self, id: i64) -> Store {
        self.conns.lock().unwrap()[&id].clone()
    }

    /// Every recorded dispatch as `(scope, action)`.
    pub(crate) fn dispatched_pairs(&self) -> Vec<(String, String)> {
        self.dispatched
            .lock()
            .unwrap()
            .iter()
            .map(|d| (d.scope.clone(), d.action.clone()))
            .collect()
    }
}

impl Read for MockHost {
    fn read(&self, name: String) -> mlua::Result<ValueType> {
        self.store
            .lock()
            .unwrap()
            .get(&name)
            .cloned()
            .ok_or_else(|| mlua::Error::RuntimeError(format!("unknown '{name}'")))
    }
}

impl Write for MockHost {
    fn write(&self, name: String, value: ValueType) -> mlua::Result<()> {
        self.store.lock().unwrap().insert(name, value);
        Ok(())
    }
}

impl Has for MockHost {
    fn has(&self, name: String) -> mlua::Result<bool> {
        Ok(self.store.lock().unwrap().get(&name).is_some())
    }
}

impl OcppActions for MockHost {
    fn actions() -> Vec<&'static str> {
        vec!["BootNotification", "StartTransaction"]
    }
    fn dispatch(&self, action: &str, _args: Vec<(String, ValueType)>) -> bool {
        self.dispatched.lock().unwrap().push(Dispatch {
            scope: self.scope.clone(),
            action: action.to_string(),
        });
        true
    }
}

impl OcppClientHost for MockHost {
    type Conn = MockHost;
    fn connector(&self, id: i64) -> MockHost {
        let store = self.conns.lock().unwrap().entry(id).or_default().clone();
        MockHost {
            scope: format!("c{id}"),
            store,
            conns: self.conns.clone(),
            dispatched: self.dispatched.clone(),
        }
    }
    fn connectors(&self) -> Vec<i64> {
        self.conns.lock().unwrap().keys().copied().collect()
    }
}
