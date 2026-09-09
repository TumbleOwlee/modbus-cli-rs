use crate::Error;

/// ScriptState of a lua execution
enum ExecState {
    Err(Error),
    Ok,
}

/// Meta state of a lua execution
pub struct ScriptState {
    state: ExecState,
    time_since: std::time::Instant,
}

impl ScriptState {
    pub fn err(err: Error) -> Self {
        Self {
            state: ExecState::Err(err),
            time_since: std::time::Instant::now(),
        }
    }

    pub fn ok() -> Self {
        Self {
            state: ExecState::Ok,
            time_since: std::time::Instant::now(),
        }
    }

    pub fn time_since(&self) -> std::time::Instant {
        self.time_since
    }

    pub fn error(&self) -> Option<Error> {
        match self.state {
            ExecState::Err(ref e) => Some(e.clone()),
            ExecState::Ok => None,
        }
    }

    pub fn is_ok(&self) -> bool {
        match self.state {
            ExecState::Ok => true,
            ExecState::Err(_) => false,
        }
    }

    pub fn is_err(&self) -> bool {
        !self.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, ScriptState};

    #[test]
    fn ut_state_error() {
        let error = Error::SyntaxError {
            message: "Syntax Error".to_string(),
            incomplete_input: true,
        };

        let state = ScriptState::err(error.clone());
        assert!(!state.is_ok());
        assert!(state.is_err());
        assert!(state.error().is_some());
    }

    #[test]
    fn ut_state_ok() {
        let state = ScriptState::ok();
        assert!(state.is_ok());
        assert!(!state.is_err());
        assert!(state.error().is_none());
    }
}
