use crate::lua::namespace::Namespace;
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

impl Namespace for Time {
    fn namespace() -> &'static str {
        "C_Time"
    }
}

impl UserData for Time {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("GetTime", |_, this, ()| -> LuaResult<u64> {
            Ok(std::time::Instant::now()
                .duration_since(this.start)
                .as_secs())
        });
        methods.add_method("GetTimeMs", |_, this, ()| -> LuaResult<u128> {
            Ok(std::time::Instant::now()
                .duration_since(this.start)
                .as_millis())
        })
    }
}
