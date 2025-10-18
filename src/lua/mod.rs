mod context;
mod module;

use crate::lua::context::Context;
use crate::{mem::memory::Memory, msg::LogMsg, AppConfig};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::Sender;

pub struct Runtime {
    context: Context,
    log_sender: Sender<LogMsg>,
}

impl Runtime {
    pub fn init(
        memory: Arc<Mutex<Memory>>,
        app_config: Arc<Mutex<AppConfig>>,
        log_sender: Sender<LogMsg>,
    ) -> Result<Self, anyhow::Error> {
        let mut lua = Context::default();
        lua.enable_stdlib()?;

        lua.add_module(module::Time::default())?;
        lua.add_module(module::Register::init(
            memory,
            app_config.clone(),
            log_sender.clone(),
        ))?;

        let config = app_config.lock().unwrap();
        for (id, definition) in config.definitions.iter() {
            if let Some(code) = definition.on_update() {
                lua.load(id, code)?;
            }
        }
        drop(config);

        Ok(Self {
            context: lua,
            log_sender,
        })
    }

    pub fn execute(&mut self) {
        if let Err(v) = self.context.exec_all() {
            for err in v.into_iter() {
                let _ = self.log_sender.try_send(LogMsg::err(&format!("{}", err)));
            }
        }
    }
}
