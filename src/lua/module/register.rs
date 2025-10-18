use crate::lua::module::Module;
use crate::mem::memory::Range;
use crate::{mem::memory::Memory, msg::LogMsg, AppConfig};
use mlua::{Result as LuaResult, UserData};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::Sender;

pub struct Register {
    memory: Arc<Mutex<Memory>>,
    app_config: Arc<Mutex<AppConfig>>,
    log_sender: Sender<LogMsg>,
}

impl Register {
    pub fn init(
        memory: Arc<Mutex<Memory>>,
        app_config: Arc<Mutex<AppConfig>>,
        log_sender: Sender<LogMsg>,
    ) -> Self {
        Self {
            memory,
            app_config,
            log_sender,
        }
    }
}

impl Module for Register {
    fn module() -> &'static str {
        "C_Register"
    }
}

impl UserData for Register {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("GetInt", |_, this, name: String| -> LuaResult<i128> {
            let config = this
                .app_config
                .lock()
                .map_err(|_| mlua::Error::UserDataBorrowError)?;
            let definitions: Vec<_> = config.definitions.iter().filter(|r| *r.0 == name).collect();
            if definitions.len() == 1 {
                let bytes: Vec<u16> = this
                    .memory
                    .lock()
                    .unwrap()
                    .read(
                        definitions[0].1.get_slave_id().unwrap_or(0),
                        &definitions[0].1.get_range(),
                    )
                    .unwrap_or(vec![&0, &0, &0, &0, &0, &0, &0, &0])
                    .into_iter()
                    .copied()
                    .collect();
                let value = definitions[0]
                    .1
                    .get_type()
                    .as_plain_str(&bytes)
                    .map_err(|_| mlua::Error::UserDataTypeMismatch)?;

                value
                    .parse::<i128>()
                    .map_err(|_| mlua::Error::UserDataTypeMismatch)
            } else {
                Err(mlua::Error::RuntimeError(String::new()))
            }
        });

        methods.add_method("GetFloat", |_, this, name: String| -> LuaResult<f64> {
            let rg = this
                .app_config
                .lock()
                .map_err(|_| mlua::Error::UserDataBorrowError)?;
            let regs: Vec<_> = rg.definitions.iter().filter(|r| *r.0 == name).collect();
            if regs.len() == 1 {
                let bytes: Vec<u16> = this
                    .memory
                    .lock()
                    .unwrap()
                    .read(
                        regs[0].1.get_slave_id().unwrap_or(0),
                        &regs[0].1.get_range(),
                    )
                    .unwrap_or(vec![&0, &0, &0, &0, &0, &0, &0, &0])
                    .into_iter()
                    .copied()
                    .collect();
                let value = regs[0]
                    .1
                    .get_type()
                    .as_plain_str(&bytes)
                    .map_err(|_| mlua::Error::UserDataTypeMismatch)?;

                value
                    .parse::<f64>()
                    .map_err(|_| mlua::Error::UserDataTypeMismatch)
            } else {
                Err(mlua::Error::RuntimeError(String::new()))
            }
        });

        methods.add_method("GetString", |_, this, name: String| -> LuaResult<String> {
            let app_config = this
                .app_config
                .lock()
                .map_err(|_| mlua::Error::UserDataBorrowError)?;
            let regs: Vec<_> = app_config
                .definitions
                .iter()
                .filter(|r| *r.0 == name)
                .collect();
            if regs.len() == 1 {
                let bytes: Vec<u16> = this
                    .memory
                    .lock()
                    .unwrap()
                    .read(
                        regs[0].1.get_slave_id().unwrap_or(0),
                        &regs[0].1.get_range(),
                    )
                    .unwrap_or(vec![&0, &0, &0, &0, &0, &0, &0, &0])
                    .into_iter()
                    .copied()
                    .collect();
                regs[0]
                    .1
                    .get_type()
                    .as_plain_str(&bytes)
                    .map_err(|_| mlua::Error::UserDataTypeMismatch)
            } else {
                Err(mlua::Error::RuntimeError(String::new()))
            }
        });

        methods.add_method("GetBool", |_, this, name: String| -> LuaResult<bool> {
            let app_config = this
                .app_config
                .lock()
                .map_err(|_| mlua::Error::UserDataBorrowError)?;
            let regs: Vec<_> = app_config
                .definitions
                .iter()
                .filter(|r| *r.0 == name)
                .collect();
            if regs.len() == 1 {
                let bytes: Vec<u16> = this
                    .memory
                    .lock()
                    .unwrap()
                    .read(
                        regs[0].1.get_slave_id().unwrap_or(0),
                        &regs[0].1.get_range(),
                    )
                    .unwrap_or(vec![&0, &0, &0, &0, &0, &0, &0, &0])
                    .into_iter()
                    .copied()
                    .collect();
                let value = regs[0]
                    .1
                    .get_type()
                    .as_plain_str(&bytes)
                    .map_err(|_| mlua::Error::UserDataTypeMismatch)?;

                value
                    .parse::<bool>()
                    .map_err(|_| mlua::Error::UserDataTypeMismatch)
            } else {
                Err(mlua::Error::RuntimeError(String::new()))
            }
        });

        methods.add_method(
            "Set",
            |_, this, (name, value): (String, String)| -> LuaResult<()> {
                if let Some(register) = this
                    .app_config
                    .lock()
                    .map_err(|_| mlua::Error::UserDataBorrowError)?
                    .definitions
                    .get(&name)
                {
                    match register.get_type().encode(&value) {
                        Ok(values) => {
                            if values.len() > register.length() as usize {
                                let _ = this.log_sender.try_send(LogMsg::err(
                                    "Provided input requires a longer register as available.",
                                ));
                            } else {
                                let mut memory = this.memory.lock().unwrap();
                                let addr = register.get_address();
                                let slave = register.get_slave_id().unwrap_or(0);

                                if let Err(e) = memory
                                    .write(
                                        slave,
                                        Range::new(addr, addr + values.len() as u16),
                                        &values,
                                    )
                                    .map(|_| ())
                                {
                                    let _ = this
                                        .log_sender
                                        .try_send(LogMsg::err(&format!("{} = {}", value, e)));
                                }
                            }
                        }
                        Err(e) => {
                            let _ = this
                                .log_sender
                                .try_send(LogMsg::err(&format!("{} = {}", value, e)));
                        }
                    }
                }
                Ok(())
            },
        );
    }
}
