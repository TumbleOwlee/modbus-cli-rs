mod register;
mod time;

pub use register::Register;
pub use time::Time;

pub trait Namespace {
    fn namespace() -> &'static str;
}
