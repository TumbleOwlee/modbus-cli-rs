use crate::util::str;
use chrono::Local;
use tokio_modbus::prelude::SlaveId;

pub enum Status {
    String(String),
}

pub enum Command {
    Connect,
    Disconnect,
    WriteMultipleRegisters((SlaveId, u16, Vec<u16>)),
}

#[derive(Clone, Debug)]
pub struct Message {
    pub timestamp: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub enum LogMsg {
    Err(Message),
    Ok(Message),
    Info(Message),
}

impl LogMsg {
    pub fn info(msg: &str) -> LogMsg {
        Self::Info(Message {
            timestamp: format!("{}", Local::now().format("[ %d:%m:%Y | %H:%M:%S ]")),
            message: str!(msg),
        })
    }

    pub fn err(msg: &str) -> LogMsg {
        Self::Err(Message {
            timestamp: format!("{}", Local::now().format("[ %d:%m:%Y | %H:%M:%S ]")),
            message: str!(msg),
        })
    }

    pub fn ok(msg: &str) -> LogMsg {
        Self::Ok(Message {
            timestamp: format!("{}", Local::now().format("[ %d:%m:%Y | %H:%M:%S ]")),
            message: str!(msg),
        })
    }

    pub fn timestamp(&self) -> String {
        match self {
            Self::Ok(v) => v.timestamp.clone(),
            Self::Info(v) => v.timestamp.clone(),
            Self::Err(v) => v.timestamp.clone(),
        }
    }
}
