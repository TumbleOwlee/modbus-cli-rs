pub mod traits;

use crate::module::{Module, ValueType};
use mlua::{Result, UserData};
use traits::{Read, Write};

pub struct Register<T>
where
    T: Write + Read + 'static,
{
    handle: T,
}

impl<T> Register<T>
where
    T: Write + Read + 'static,
{
    pub fn init(handle: T) -> Self {
        Self { handle }
    }
}

impl<T> Module for Register<T>
where
    T: Write + Read + 'static,
{
    fn module() -> &'static str {
        "C_Register"
    }
}

impl<T> UserData for Register<T>
where
    T: Write + Read + 'static,
{
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("GetInt", Self::get_int);
        methods.add_method("GetFloat", Self::get_float);
        methods.add_method("GetString", Self::get_string);
        methods.add_method("GetBool", Self::get_bool);
        methods.add_method("Set", Self::set);
    }
}

impl<T> Register<T>
where
    T: Write + Read + 'static,
{
    fn get_int(_: &mlua::Lua, this: &Register<T>, name: String) -> Result<i128> {
        match this.handle.read(name) {
            Ok(ValueType::Int(v)) => Ok(v),
            Err(e) => Err(e),
            Ok(_) => Err(mlua::Error::UserDataTypeMismatch),
        }
    }

    fn get_float(_: &mlua::Lua, this: &Register<T>, name: String) -> Result<f64> {
        match this.handle.read(name) {
            Ok(ValueType::Float(v)) => Ok(v),
            Err(e) => Err(e),
            Ok(_) => Err(mlua::Error::UserDataTypeMismatch),
        }
    }

    fn get_string(_: &mlua::Lua, this: &Register<T>, name: String) -> Result<String> {
        match this.handle.read(name) {
            Ok(ValueType::String(v)) => Ok(v),
            Err(e) => Err(e),
            Ok(_) => Err(mlua::Error::UserDataTypeMismatch),
        }
    }

    fn get_bool(_: &mlua::Lua, this: &Register<T>, name: String) -> Result<bool> {
        match this.handle.read(name) {
            Ok(ValueType::Bool(v)) => Ok(v),
            Err(e) => Err(e),
            Ok(_) => Err(mlua::Error::UserDataTypeMismatch),
        }
    }

    fn set(_: &mlua::Lua, this: &Register<T>, (name, value): (String, String)) -> Result<()> {
        this.handle.write(name, value)
    }
}
