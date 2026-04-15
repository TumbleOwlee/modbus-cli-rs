use crate::module::register::ValueType;
use mlua::Result;

pub trait Write {
    fn write(&self, name: String, value: String) -> Result<()>;
}

pub trait Read {
    fn read(&self, name: String) -> Result<ValueType>;
}
