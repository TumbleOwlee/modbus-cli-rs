use crate::{module::Module, Error, Result, Script};
use mlua::{Lua, StdLib, UserData};
use std::{collections::HashMap, hash::Hash};

/// Lua context handling module and script loading
#[derive(Default)]
pub struct Context<K>
where
    K: Hash + Eq + Default,
{
    /// Native lua context
    lua: Lua,
    /// Collection of all loaded lua scripts
    scripts: HashMap<K, Script>,
}

#[allow(dead_code)]
impl<K> Context<K>
where
    K: Hash + Eq + Default,
{
    /// Add a new module to the lua context
    pub fn add_module<T>(&mut self, value: T) -> Result<()>
    where
        T: 'static + Module + UserData,
    {
        let globals = self.lua.globals();
        globals.set(T::module(), value)
    }

    /// Enable support of standard libraries in lua context
    pub fn enable_stdlib(&mut self) -> Result<()> {
        self.lua.load_std_libs(StdLib::STRING)?;
        self.lua.load_std_libs(StdLib::MATH)?;
        self.lua.load_std_libs(StdLib::TABLE)?;
        self.lua.load_std_libs(StdLib::ALL_SAFE)?;
        Ok(())
    }

    /// Retrieve iterator over all loaded scripts
    pub fn iter<'a>(&'a self) -> std::collections::hash_map::Iter<'a, K, Script> {
        self.scripts.iter()
    }

    /// Retrieve mutable iterator over all loaded scripts
    pub fn iter_mut<'a>(&'a mut self) -> std::collections::hash_map::IterMut<'a, K, Script> {
        self.scripts.iter_mut()
    }

    /// Execute a loaded script specified by specific key
    pub fn call(&mut self, key: &K) -> Result<()> {
        self.iter_mut()
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v.exec())
            .find(|r| r.is_err())
            .unwrap_or(Ok(()))
    }

    /// Execute a loaded script specified by specific key while skipping it if it has been executed
    /// in the last timeframe of given duration
    pub fn refresh(&mut self, key: &K, since: std::time::Duration) -> Result<()> {
        self.iter_mut()
            .filter(|(k, v)| *k == key && v.since_last_execution() >= since)
            .map(|(_, v)| v.exec())
            .find(|r| r.is_err())
            .unwrap_or(Ok(()))
    }

    /// Execute all loaded scripts
    pub fn call_all(&mut self, since: std::time::Duration) -> std::result::Result<(), Vec<Error>> {
        let errors: Vec<_> = self
            .iter_mut()
            .filter(|(_, v)| v.since_last_execution() >= since)
            .map(|(_, v)| v.exec())
            .filter(|r| r.is_err())
            .map(|e| e.err().unwrap())
            .collect();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Execute all loaded scripts while skipping all scrips executed in the last timeframe of
    /// given duration
    pub fn refresh_all(
        &mut self,
        since: std::time::Duration,
    ) -> std::result::Result<(), Vec<Error>> {
        let errors: Vec<_> = self
            .iter_mut()
            .filter(|(_, v)| v.since_last_execution() >= since)
            .map(|(_, v)| v.exec())
            .filter(|r| r.is_err())
            .map(|e| e.err().unwrap())
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
