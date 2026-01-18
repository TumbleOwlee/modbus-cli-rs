use crate::Error;

/// State of a lua execution
#[allow(dead_code)]
enum ExecState {
    Err(Error),
    Ok,
}

/// Meta state of a lua execution
#[allow(dead_code)]
pub struct State {
    state: ExecState,
    time_since: std::time::Instant,
}

#[allow(dead_code)]
impl State {
    /// Create a new error state
    pub fn err(err: Error) -> Self {
        Self {
            state: ExecState::Err(err),
            time_since: std::time::Instant::now(),
        }
    }

    /// Create a new success state
    pub fn ok() -> Self {
        Self {
            state: ExecState::Ok,
            time_since: std::time::Instant::now(),
        }
    }

    /// Retrieve duration passed since last execution
    pub fn time_since(&self) -> std::time::Instant {
        self.time_since
    }

    /// Retrieve error if present
    pub fn error(&self) -> Option<Error> {
        match self.state {
            ExecState::Err(ref e) => Some(e.clone()),
            ExecState::Ok => None,
        }
    }

    /// Check whether the execution is in `Ok` state
    pub fn is_ok(&self) -> bool {
        match self.state {
            ExecState::Ok => true,
            ExecState::Err(_) => false,
        }
    }

    /// Check whether the execution is in `Err` state
    pub fn is_err(&self) -> bool {
        !self.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::{Error, State};

    #[test]
    fn ut_state_error() {
        let error = Error::SyntaxError {
            message: "Syntax Error".to_string(),
            incomplete_input: true,
        };

        let state = State::err(error.clone());
        assert_eq!(false, state.is_ok());
        assert_eq!(true, state.is_err());
        assert_eq!(true, state.error().is_some());
    }

    #[test]
    fn ut_state_ok() {
        let state = State::ok();
        assert_eq!(true, state.is_ok());
        assert_eq!(false, state.is_err());
        assert_eq!(false, state.error().is_some());
    }
}
