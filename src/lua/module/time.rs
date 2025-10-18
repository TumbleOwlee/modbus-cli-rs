use crate::lua::module::Module;
use mlua::{Result as LuaResult, UserData};

pub struct Time {
    start: std::time::Instant,
}

impl Default for Time {
    fn default() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }
}

impl Module for Time {
    fn module() -> &'static str {
        "C_Time"
    }
}

impl UserData for Time {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("Get", |_, this, ()| -> LuaResult<u64> {
            Ok(std::time::Instant::now()
                .duration_since(this.start)
                .as_secs())
        });
        methods.add_method("GetMs", |_, this, ()| -> LuaResult<u128> {
            Ok(std::time::Instant::now()
                .duration_since(this.start)
                .as_millis())
        })
    }
}
