use crate::module::Module;
use mlua::{Result, UserData};

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
        methods.add_method("Get", Time::get);
        methods.add_method("GetMs", Time::get_ms);
    }
}

impl Time {
    fn get(_: &mlua::Lua, this: &Time, _: ()) -> Result<u64> {
        Ok(std::time::Instant::now()
            .duration_since(this.start)
            .as_secs())
    }

    fn get_ms(_: &mlua::Lua, this: &Time, _: ()) -> Result<u128> {
        Ok(std::time::Instant::now()
            .duration_since(this.start)
            .as_millis())
    }
}
