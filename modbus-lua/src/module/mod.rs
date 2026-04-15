mod register;
mod statics;
mod time;

pub use register::Register as RegisterModule;
pub use statics::Statics as StaticsModule;
pub use time::Time as TimeModule;

pub enum ValueType {
    Int(i128),
    Float(f64),
    String(String),
    Bool(bool),
}

pub trait Module {
    fn module() -> &'static str;
}
