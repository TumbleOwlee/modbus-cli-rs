use crate::State;

/// Loaded lua script
pub struct Script {
    state: State,
    func: mlua::Function,
}

impl Script {
    /// Create lua script state from native function handle
    pub fn init(func: mlua::Function) -> Self {
        Self {
            state: State::ok(),
            func,
        }
    }

    /// Retrieve duration since last execution
    pub fn since_last_execution(&self) -> std::time::Duration {
        let now = std::time::Instant::now();
        now.duration_since(self.state.time_since())
    }

    /// Execute the loaded script
    pub fn exec(&mut self) -> crate::Result<()> {
        match self.func.call::<()>(()) {
            Ok(_) => {
                self.state = State::ok();
                Ok(())
            }
            Err(e) => {
                self.state = State::err(e.clone());
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
    fn ut_script() {
        let lua = Lua::new();

        let func = lua.load("local test = 1").into_function().unwrap();
        let mut script = Script::init(func);
        let result = script.exec();
        assert_eq!(result.is_ok(), true);

        let func = lua.load("func()").into_function().unwrap();
        let mut script = Script::init(func);
        let result = script.exec();
        assert_eq!(result.is_err(), true);
    }
}
