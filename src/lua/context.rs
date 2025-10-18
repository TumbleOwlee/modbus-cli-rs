use crate::lua::namespace::Namespace;
use anyhow::anyhow;
use mlua::{Function, Lua, StdLib, UserData};
use std::collections::HashMap;

enum State {
    Err((mlua::Error, std::time::Instant)),
    Ok,
}

struct FnContext {
    state: State,
    func: Function,
}

impl FnContext {
    pub fn init(func: Function) -> Self {
        Self {
            state: State::Ok,
            func,
        }
    }
}

pub struct Context {
    lua: Lua,
    funcs: HashMap<String, FnContext>,
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
    pub fn add_namespace<T>(&mut self, value: T) -> Result<(), mlua::Error>
    where
        T: 'static + Namespace + UserData,
    {
        let globals = self.lua.globals();
        globals.set(T::namespace(), value)
    }
    pub fn enable_stdlib(&mut self) -> Result<(), mlua::Error> {
        self.lua.load_std_libs(StdLib::STRING)?;
        self.lua.load_std_libs(StdLib::MATH)?;
        self.lua.load_std_libs(StdLib::TABLE)?;
        self.lua.load_std_libs(StdLib::ALL_SAFE)?;
        Ok(())
    }

    pub fn exec(&mut self, id: &str) -> Result<(), anyhow::Error> {
        let now = std::time::Instant::now();
        if let Some((_, ctx)) = self.funcs.iter_mut().find(|(i, _)| *i == id) {
            if let State::Err((_, t)) = &ctx.state {
                if now.duration_since(*t).as_secs() < 5 {
                    return Err(anyhow!("Execute blocked because function is failing."));
                }
            }

            if let Err(e) = ctx.func.call::<()>(()) {
                ctx.state = State::Err((e.clone(), now));
                Err(e.into())
            } else {
                Ok(())
            }
        } else {
            Err(anyhow!("Chunk of given id not loaded"))
        }
    }

    pub fn exec_all(&mut self) -> Result<(), Vec<anyhow::Error>> {
        let now = std::time::Instant::now();
        let mut res: Result<(), Vec<anyhow::Error>> = Ok(());
        for (_, ctx) in self.funcs.iter_mut() {
            if let State::Err((_, t)) = &ctx.state {
                if now.duration_since(*t).as_secs() < 5 {
                    continue;
                }
            }

            if let Err(e) = ctx.func.call::<()>(()) {
                ctx.state = State::Err((e.clone(), std::time::Instant::now()));
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
        let _ = self.funcs.insert(id.to_string(), FnContext::init(func));
        Ok(())
    }
}
