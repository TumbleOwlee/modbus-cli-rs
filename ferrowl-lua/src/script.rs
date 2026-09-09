use crate::ScriptState;

pub struct Script {
    state: ScriptState,
    func: mlua::Function,
}

impl Script {
    /// Create lua script state from native function handle
    pub fn init(func: mlua::Function) -> Self {
        Self {
            state: ScriptState::ok(),
            func,
        }
    }

    pub fn since_last_execution(&self) -> std::time::Duration {
        let now = std::time::Instant::now();
        now.duration_since(self.state.time_since())
    }

    pub fn exec(&mut self) -> crate::Result<()> {
        match self.func.call::<()>(()) {
            Ok(_) => {
                self.state = ScriptState::ok();
                Ok(())
            }
            Err(e) => {
                self.state = ScriptState::err(e.clone());
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Script;
    use mlua::Lua;

    #[test]
    /// SC-R-003 — a compiled script is invoked with no args; a successful call is Ok, a failing one Err.
    fn ut_script() {
        let lua = Lua::new();

        let func = lua.load("local test = 1").into_function().unwrap();
        let mut script = Script::init(func);
        let result = script.exec();
        assert!(result.is_ok());

        let func = lua.load("func()").into_function().unwrap();
        let mut script = Script::init(func);
        let result = script.exec();
        assert!(result.is_err());
    }
}
