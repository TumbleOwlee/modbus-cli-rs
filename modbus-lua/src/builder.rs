use crate::{module::Module, Context, Result};
use mlua::UserData;
use std::hash::Hash;

/// Lua context builder
pub struct ContextBuilder<K>
where
    K: Hash + Eq + Default,
{
    context: Result<Context<K>>,
}

impl<K> Default for ContextBuilder<K>
where
    K: Hash + Eq + Default,
{
    /// Create new context builder
    fn default() -> Self {
        Self {
            context: Ok(Context::<K>::default()),
        }
    }
}

#[allow(dead_code)]
impl<K> ContextBuilder<K>
where
    K: Hash + Eq + Default,
{
    /// Create context builder from context result
    fn from(context: Result<Context<K>>) -> Self {
        Self { context }
    }

    /// Add a new module to the lua context
    pub fn with_module<T>(mut self, value: T) -> Self
    where
        T: 'static + Module + UserData,
    {
        if let Ok(ref mut ctx) = self.context {
            if let Err(e) = ctx.add_module(value) {
                self.context = Err(e);
            };
        }
        self
    }

    /// Enable support of standard libraries in lua context
    pub fn with_stdlib(mut self) -> Self {
        if let Ok(ref mut ctx) = self.context {
            if let Err(e) = ctx.enable_stdlib() {
                self.context = Err(e);
            };
        }
        self
    }

    ///  Load a given script into the lua context and store it under the given key
    pub fn with_script(mut self, key: K, script: &str) -> Self {
        if let Ok(ref mut ctx) = self.context {
            if let Err(e) = ctx.load_script(key, script) {
                self.context = Err(e);
            };
        }
        self
    }

    /// Build the final context
    pub fn build(self) -> Result<Context<K>> {
        self.context
    }
}
