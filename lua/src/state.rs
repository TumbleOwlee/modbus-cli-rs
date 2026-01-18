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

    /// Check whether the execution is in `ok` state
    pub fn is_ok(&self) -> bool {
        match self.state {
            ExecState::Ok => true,
            ExecState::Err(_) => false,
        }
    }
}
