//! Host modules exposed to Lua scripts as global userdata objects.

mod log;
mod module_dir;
mod ocpp;
mod register;
mod statics;
mod test;
#[cfg(test)]
pub(crate) mod test_support;
mod time;
mod value_type;

pub use log::{Log as LogModule, LogLevel, LogSink};
pub use module_dir::{ModuleDir as ModuleDirModule, ModuleDirectory, ModuleHandle, ModuleHost};
pub use ocpp::traits::{OcppActions, OcppClientHost, OcppHandle, OcppServerHost};
pub use ocpp::{Accessor, Ocpp as OcppModule, OcppClient, OcppServer};
pub use register::Register as RegisterModule;
pub use register::traits::{Has, Read, Write};
pub use statics::Statics as StaticsModule;
pub use test::Test as TestModule;
pub use time::Time as TimeModule;
pub use value_type::ValueType;

/// A host module that can be registered in a Lua context.
pub trait Module {
    /// The Lua global name the module is registered under (e.g. `C_Time`).
    fn module() -> &'static str;
}
