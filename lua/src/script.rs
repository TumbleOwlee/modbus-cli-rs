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
