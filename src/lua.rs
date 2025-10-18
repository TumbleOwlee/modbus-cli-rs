use mlua::{Lua, StdLib};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::Sender;

use crate::{
    mem::memory::{Memory, Range},
    msg::LogMsg,
    AppConfig,
};

pub struct LuaRuntime {
    lua: Lua,
    app_config: Arc<Mutex<AppConfig>>,
    log_sender: Sender<LogMsg>,
}

impl LuaRuntime {
    pub fn init(
        memory: Arc<Mutex<Memory>>,
        app_config: Arc<Mutex<AppConfig>>,
        log_sender: Sender<LogMsg>,
    ) -> Result<Self, mlua::Error> {
        let lua = Lua::new();
        lua.load_std_libs(StdLib::STRING)?;
        lua.load_std_libs(StdLib::MATH)?;
        lua.load_std_libs(StdLib::TABLE)?;
        lua.load_std_libs(StdLib::ALL_SAFE)?;
        let globals = lua.globals();

        let start = std::time::Instant::now();
        let get_time = lua.create_function(move |_, _: ()| -> Result<u128, mlua::Error> {
            let now = std::time::Instant::now();
            Ok(now.duration_since(start).as_secs() as u128)
        })?;
        globals.set("GetTime", get_time)?;

        let get_time_ms = lua.create_function(move |_, _: ()| -> Result<u128, mlua::Error> {
            let now = std::time::Instant::now();
            Ok(now.duration_since(start).as_millis())
        })?;
        globals.set("GetTimeMs", get_time_ms)?;

        let table = lua.create_table()?;

        let _app_config = app_config.clone();
        let _memory = memory.clone();
        let get_value_as_int =
            lua.create_function(move |_, name: String| -> Result<i128, mlua::Error> {
                let rg = _app_config
                    .lock()
                    .map_err(|_| mlua::Error::UserDataBorrowError)?;
                let regs: Vec<_> = rg.definitions.iter().filter(|r| *r.0 == name).collect();
                if regs.len() == 1 {
                    let bytes: Vec<u16> = _memory
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
                        .parse::<i128>()
                        .map_err(|_| mlua::Error::UserDataTypeMismatch)
                } else {
                    Err(mlua::Error::RuntimeError(String::new()))
                }
            })?;
        table.set("GetInt", get_value_as_int)?;

        let _app_config = app_config.clone();
        let _memory = memory.clone();
        let get_value_as_float =
            lua.create_function(move |_, name: String| -> Result<f64, mlua::Error> {
                let rg = _app_config
                    .lock()
                    .map_err(|_| mlua::Error::UserDataBorrowError)?;
                let regs: Vec<_> = rg.definitions.iter().filter(|r| *r.0 == name).collect();
                if regs.len() == 1 {
                    let bytes: Vec<u16> = _memory
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
            })?;
        table.set("GetFloat", get_value_as_float)?;

        let _app_config = app_config.clone();
        let _memory = memory.clone();
        let get_value_as_str =
            lua.create_function(move |_, name: String| -> Result<String, mlua::Error> {
                let app_config = _app_config
                    .lock()
                    .map_err(|_| mlua::Error::UserDataBorrowError)?;
                let regs: Vec<_> = app_config
                    .definitions
                    .iter()
                    .filter(|r| *r.0 == name)
                    .collect();
                if regs.len() == 1 {
                    let bytes: Vec<u16> = _memory
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
            })?;
        table.set("GetString", get_value_as_str)?;

        let _app_config = app_config.clone();
        let _memory = memory.clone();
        let get_value_as_bool =
            lua.create_function(move |_, name: String| -> Result<bool, mlua::Error> {
                let app_config = _app_config
                    .lock()
                    .map_err(|_| mlua::Error::UserDataBorrowError)?;
                let regs: Vec<_> = app_config
                    .definitions
                    .iter()
                    .filter(|r| *r.0 == name)
                    .collect();
                if regs.len() == 1 {
                    let bytes: Vec<u16> = _memory
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
            })?;
        table.set("GetBool", get_value_as_bool)?;

        let _app_config = app_config.clone();
        let _memory = memory.clone();
        let _log_sender = log_sender.clone();
        let set_value = lua.create_function(
            move |_, (name, value): (String, String)| -> Result<(), mlua::Error> {
                if let Some(register) = _app_config
                    .lock()
                    .map_err(|_| mlua::Error::UserDataBorrowError)?
                    .definitions
                    .get(&name)
                {
                    match register.get_type().encode(&value) {
                        Ok(values) => {
                            if values.len() > register.length() as usize {
                                let _ = _log_sender.try_send(LogMsg::err(
                                    "Provided input requires a longer register as available.",
                                ));
                            } else {
                                let mut memory = _memory.lock().unwrap();
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
                                    let _ = _log_sender
                                        .try_send(LogMsg::err(&format!("{} = {}", value, e)));
                                }
                            }
                        }
                        Err(e) => {
                            let _ =
                                _log_sender.try_send(LogMsg::err(&format!("{} = {}", value, e)));
                        }
                    }
                }
                Ok(())
            },
        )?;
        table.set("Set", set_value)?;
        globals.set("Register", table)?;

        Ok(Self {
            lua,
            app_config,
            log_sender,
        })
    }

    pub fn execute(&mut self) {
        let app_config = self.app_config.lock().unwrap();
        let definitions = app_config.definitions.clone();
        drop(app_config);

        for (name, def) in definitions.iter() {
            if let Some(code) = def.on_update() {
                if let Err(e) = self.lua.load(code).exec() {
                    let _ = self
                        .log_sender
                        .try_send(LogMsg::err(&format!("Lua Error for {}: {}", name, e)));

                    let mut app_config = self.app_config.lock().unwrap();
                    app_config.definitions.remove(name);
                }
            }
        }
    }
}
