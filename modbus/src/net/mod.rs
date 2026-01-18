use serde::{Deserialize, Serialize};

pub mod rtu;
pub mod tcp;

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Config {
    pub interval_ms: usize,
    pub delay_after_connect_ms: usize,
    pub timeout_ms: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_ms: 500,
            timeout_ms: 3000,
            delay_after_connect_ms: 500,
        }
    }
}
