use crate::lua::module::Module;
use mlua::{Function as LuaFunction, Lua, StdLib, UserData};
use std::collections::HashMap;

#[allow(dead_code)]
struct Error {
    error: mlua::Error,
    time: std::time::Instant,
}

impl Error {
    fn new(error: mlua::Error) -> Self {
        Self {
            error,
            time: std::time::Instant::now(),
        }
    }
}

enum State {
    Err(Error),
    Ok,
}

struct Function {
    state: State,
    func: LuaFunction,
}

impl Function {
    pub fn init(func: LuaFunction) -> Self {
        Self {
            state: State::Ok,
            func,
        }
    }
}

pub struct Context {
    lua: Lua,
    funcs: HashMap<String, Function>,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            lua: Lua::new(),
            funcs: HashMap::new(),
        }
    }
}

impl Context {
    pub fn add_module<T>(&mut self, value: T) -> Result<(), mlua::Error>
    where
        T: 'static + Module + UserData,
    {
        let globals = self.lua.globals();
        globals.set(T::module(), value)
    }
    pub fn enable_stdlib(&mut self) -> Result<(), mlua::Error> {
        self.lua.load_std_libs(StdLib::STRING)?;
        self.lua.load_std_libs(StdLib::MATH)?;
        self.lua.load_std_libs(StdLib::TABLE)?;
        self.lua.load_std_libs(StdLib::ALL_SAFE)?;
        Ok(())
    }

    pub fn exec_all(&mut self) -> Result<(), Vec<anyhow::Error>> {
        let now = std::time::Instant::now();
        let mut res: Result<(), Vec<anyhow::Error>> = Ok(());
        for (_, ctx) in self.funcs.iter_mut() {
            if let State::Err(error) = &ctx.state {
                if now.duration_since(error.time).as_secs() < 5 {
                    continue;
                }
            }

            if let Err(e) = ctx.func.call::<()>(()) {
                ctx.state = State::Err(Error::new(e.clone()));
                if let Err(ref mut v) = res {
                    v.push(e.into());
                } else {
                    res = Err(vec![e.into()]);
                }
            }
        }
        res
    }

    pub fn load(&mut self, id: &str, code: &str) -> Result<(), mlua::Error> {
        let func = self.lua.load(code).into_function()?;
        let _ = self.funcs.insert(id.to_string(), Function::init(func));
        Ok(())
    }
}
