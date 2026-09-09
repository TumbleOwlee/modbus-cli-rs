use crate::{Context, Result, module::LogSink, module::Module};
use mlua::UserData;
use std::hash::Hash;
use std::sync::{Arc, atomic::AtomicBool};

pub struct ContextBuilder<K>
where
    K: Hash + Eq + Default,
{
    context: Result<Context<K>>,
    stop_flag: Option<Arc<AtomicBool>>,
}

impl<K> Default for ContextBuilder<K>
where
    K: Hash + Eq + Default,
{
    fn default() -> Self {
        Self {
            context: Ok(Context::<K>::default()),
            stop_flag: None,
        }
    }
}

impl<K> ContextBuilder<K>
where
    K: Hash + Eq + Default,
{
    /// Create context builder from context result. Test-only: production builders start from
    /// [`ContextBuilder::default`] and chain `with_*`.
    #[cfg(test)]
    fn from(context: Result<Context<K>>) -> Self {
        Self {
            context,
            stop_flag: None,
        }
    }

    /// SC-R-034, SC-R-012 — attach a sim thread's stop flag so the execution hook installed in
    /// `build()` can unwind a runaway script promptly instead of only observing the flag between
    /// cycles. Only a sim-thread call site passes this; an on-demand run (SC-R-035) omits it.
    pub fn with_stop_flag(mut self, stop: Arc<AtomicBool>) -> Self {
        self.stop_flag = Some(stop);
        self
    }

    /// Run `f` against the context if it is still `Ok`, latching any error it returns. Once
    /// `self.context` is `Err`, later calls are no-ops that leave the stored error untouched.
    fn try_apply(mut self, f: impl FnOnce(&mut Context<K>) -> Result<()>) -> Self {
        if let Ok(ref mut ctx) = self.context
            && let Err(e) = f(ctx)
        {
            self.context = Err(e);
        }
        self
    }

    /// Add a new module to the lua context
    pub fn with_module<T>(self, value: T) -> Self
    where
        T: 'static + Module + UserData,
    {
        self.try_apply(|ctx| ctx.add_module(value))
    }

    /// Enable support of standard libraries in lua context
    pub fn with_stdlib(self) -> Self {
        self.try_apply(Context::enable_stdlib)
    }

    /// Redirect the global `print` to a host log sink. Order relative to `with_stdlib()` does not
    /// matter in practice (enabling the standard libraries does not reload `base`/`print`), but
    /// call this after `with_stdlib()` to keep the builder chain reading top-to-bottom as setup
    /// followed by overrides.
    pub fn with_print_sink<S>(self, sink: S) -> Self
    where
        S: LogSink + 'static,
    {
        self.try_apply(|ctx| ctx.redirect_print(sink))
    }

    /// Load a given script into the lua context and store it under the given key
    pub fn with_script(self, key: K, script: &str) -> Self {
        self.try_apply(|ctx| ctx.load_script(key, script))
    }

    /// Build the final context, installing the SC-R-034 execution hook (wall-clock cap always,
    /// stop-flag check when `with_stop_flag` was called) as the last step.
    pub fn build(self) -> Result<Context<K>> {
        let mut ctx = self.context?;
        ctx.install_execution_hook(self.stop_flag)?;
        Ok(ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::ContextBuilder;
    use crate::Context;

    #[test]
    fn ut_builder_from_ok_context() {
        let builder = ContextBuilder::<String>::from(Ok(Context::default()));
        assert!(builder.build().is_ok());
    }

    #[test]
    fn ut_builder_from_err_context_short_circuits() {
        // First error wins. The chained script is deliberately invalid Lua: a regression that ran
        // later steps against an already-`Err` context would latch that syntax error over the
        // original, so the surviving variant — not merely the presence of an error — is what
        // distinguishes correct short-circuiting from an overwrite.
        let builder = ContextBuilder::<String>::from(Err(mlua::Error::BindError))
            .with_stdlib()
            .with_script("k".to_string(), "this is not ! valid lua");
        assert!(matches!(builder.build(), Err(mlua::Error::BindError)));
    }
}
