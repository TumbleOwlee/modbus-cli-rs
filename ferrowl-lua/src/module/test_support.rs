//! Shared in-memory host fixtures for the module unit tests.

use crate::module::{Has, OcppActions, OcppClientHost, Read, ValueType, Write};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// A key/value state store shared between a mock handle and the test asserting on it.
pub(crate) type Store = Arc<Mutex<HashMap<String, ValueType>>>;

/// One recorded `dispatch` call: the scope label of the handle it was called on, the action
/// name, and the override table flattened to name/value pairs.
#[derive(Clone, Debug)]
pub(crate) struct Dispatch {
    pub(crate) scope: String,
    pub(crate) action: String,
    pub(crate) args: Vec<(String, ValueType)>,
}

/// Shared dispatch log: every handle cloned or derived from one mock appends to the same log.
pub(crate) type DispatchLog = Arc<Mutex<Vec<Dispatch>>>;

/// A dispatch log's entries as `(scope, action)`.
pub(crate) fn pairs(log: &DispatchLog) -> Vec<(String, String)> {
    log.lock()
        .unwrap()
        .iter()
        .map(|d| (d.scope.clone(), d.action.clone()))
        .collect()
}

/// Shared `Send + Sync` in-memory host, implementing every trait the module unit tests mock:
/// `Read + Write + Has` for a modbus-shaped host, `Read + Write + OcppActions + OcppClientHost`
/// for an ocpp-shaped one.
#[derive(Clone, Default)]
pub(crate) struct MockHost {
    pub(crate) scope: String,
    pub(crate) store: Store,
    pub(crate) conns: Arc<Mutex<HashMap<i64, Store>>>,
    pub(crate) dispatched: DispatchLog,
}

impl MockHost {
    /// A handle over an existing store, sharing an existing dispatch log, with an empty
    /// connector map.
    pub(crate) fn scoped(scope: &str, store: Store, dispatched: DispatchLog) -> Self {
        Self {
            scope: scope.to_string(),
            store,
            conns: Arc::new(Mutex::new(HashMap::new())),
            dispatched,
        }
    }

    /// Snapshot of a stored value by name.
    pub(crate) fn get(&self, name: &str) -> Option<ValueType> {
        self.store.lock().unwrap().get(name).cloned()
    }

    /// The state store of the connector `id`. Panics if `id` has never been touched.
    pub(crate) fn conn_store(&self, id: i64) -> Store {
        self.conns.lock().unwrap()[&id].clone()
    }

    /// Snapshot of every recorded dispatch, in call order.
    pub(crate) fn dispatched(&self) -> Vec<Dispatch> {
        self.dispatched.lock().unwrap().clone()
    }

    /// Every recorded dispatch as `(scope, action)`.
    pub(crate) fn dispatched_pairs(&self) -> Vec<(String, String)> {
        pairs(&self.dispatched)
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
    fn dispatch(&self, action: &str, args: Vec<(String, ValueType)>) -> bool {
        self.dispatched.lock().unwrap().push(Dispatch {
            scope: self.scope.clone(),
            action: action.to_string(),
            args,
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
