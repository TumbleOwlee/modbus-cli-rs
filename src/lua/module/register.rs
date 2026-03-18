use crate::lua::module::Module;
use crate::mem::memory::Range;
use crate::{mem::memory::Memory, msg::LogMsg, AppConfig};
use mlua::{Result as LuaResult, UserData};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::Sender;

pub struct Register {
    memory: Arc<Mutex<Memory>>,
    config: Arc<Mutex<AppConfig>>,
    logger: Sender<LogMsg>,
}

impl Register {
    pub fn init(
        memory: Arc<Mutex<Memory>>,
        config: Arc<Mutex<AppConfig>>,
        logger: Sender<LogMsg>,
    ) -> Self {
        Self {
            memory,
            config,
            logger,
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
        methods.add_method("GetInt", Self::get_int);
        methods.add_method("GetFloat", Self::get_float);
        methods.add_method("GetString", Self::get_string);
        methods.add_method("GetBool", Self::get_bool);
        methods.add_method("Set", Self::set);
    }
}

impl Register {
    fn get_int(_: &mlua::Lua, this: &Register, name: String) -> LuaResult<i128> {
        let config = this
            .config
            .lock()
            .map_err(|_| mlua::Error::UserDataBorrowError)?;
        let (def_by_name, def_by_id): (Vec<_>, Vec<_>) = (
            config.definitions.iter().filter(|r| *r.0 == name).collect(),
            config
                .definitions
                .iter()
                .filter(|r| {
                    if let Some(ref id) = r.1.get_id() {
                        *id == name
                    } else {
                        false
                    }
                })
                .collect(),
        );
        match (def_by_name.len(), def_by_id.len()) {
            (1, 0) => {
                let bytes: Vec<u16> = this
                    .memory
                    .lock()
                    .expect("Unable to lock memory")
                    .read(
                        def_by_name[0].1.get_slave_id().unwrap_or(0),
                        &def_by_name[0].1.get_range(),
                    )
                    .unwrap_or(vec![&0, &0, &0, &0, &0, &0, &0, &0])
                    .into_iter()
                    .copied()
                    .collect();
                let value = def_by_name[0]
                    .1
                    .get_type()
                    .as_plain_str(&bytes)
                    .map_err(|_| mlua::Error::UserDataTypeMismatch)?;

                value
                    .parse::<i128>()
                    .map_err(|_| mlua::Error::UserDataTypeMismatch)
            }
            (0, 1) => {
                let bytes: Vec<u16> = this
                    .memory
                    .lock()
                    .expect("Unable to lock memory")
                    .read(
                        def_by_id[0].1.get_slave_id().unwrap_or(0),
                        &def_by_id[0].1.get_range(),
                    )
                    .unwrap_or(vec![&0, &0, &0, &0, &0, &0, &0, &0])
                    .into_iter()
                    .copied()
                    .collect();
                let value = def_by_id[0]
                    .1
                    .get_type()
                    .as_plain_str(&bytes)
                    .map_err(|_| mlua::Error::UserDataTypeMismatch)?;

                value
                    .parse::<i128>()
                    .map_err(|_| mlua::Error::UserDataTypeMismatch)
            }
            _ => return Err(mlua::Error::RuntimeError(String::new())),
        }
    }

    fn get_float(_: &mlua::Lua, this: &Register, name: String) -> LuaResult<f64> {
        let rg = this
            .config
            .lock()
            .map_err(|_| mlua::Error::UserDataBorrowError)?;
        let regs: Vec<_> = rg.definitions.iter().filter(|r| *r.0 == name).collect();
        if regs.len() == 1 {
            let bytes: Vec<u16> = this
                .memory
                .lock()
                .expect("Unable to lock memory")
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
    }

    fn get_string(_: &mlua::Lua, this: &Register, name: String) -> LuaResult<String> {
        let config = this
            .config
            .lock()
            .map_err(|_| mlua::Error::UserDataBorrowError)?;
        let regs: Vec<_> = config.definitions.iter().filter(|r| *r.0 == name).collect();
        if regs.len() == 1 {
            let bytes: Vec<u16> = this
                .memory
                .lock()
                .expect("Unable to lock memory")
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
    }

    fn get_bool(_: &mlua::Lua, this: &Register, name: String) -> LuaResult<bool> {
        let config = this
            .config
            .lock()
            .map_err(|_| mlua::Error::UserDataBorrowError)?;
        let regs: Vec<_> = config.definitions.iter().filter(|r| *r.0 == name).collect();
        if regs.len() == 1 {
            let bytes: Vec<u16> = this
                .memory
                .lock()
                .expect("Unable to lock memory")
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
    }

    fn set(_: &mlua::Lua, this: &Register, (name, value): (String, String)) -> LuaResult<()> {
        if let Some(register) = this
            .config
            .lock()
            .map_err(|_| mlua::Error::UserDataBorrowError)?
            .definitions
            .get(&name)
        {
            match register.get_type().encode(&value) {
                Ok(values) => {
                    if values.len() > register.length() as usize {
                        let _ = this.logger.try_send(LogMsg::err(
                            "Provided input requires a longer register as available.",
                        ));
                    } else {
                        let mut memory = this.memory.lock().expect("Unable to lock memory");
                        let addr = register.get_address();
                        let slave = register.get_slave_id().unwrap_or(0);

                        if let Err(e) = memory
                            .write(slave, Range::new(addr, addr + values.len() as u16), &values)
                            .map(|_| ())
                        {
                            let _ = this
                                .logger
                                .try_send(LogMsg::err(&format!("{} = {}", value, e)));
                        }
                    }
                }
                Err(e) => {
                    let _ = this
                        .logger
                        .try_send(LogMsg::err(&format!("{} = {}", value, e)));
                }
            }
        }
        Ok(())
    }
}
