mod register;
mod time;

pub use register::Register;
pub use time::Time;

pub trait Module {
    fn module() -> &'static str;
}
