//! Per-widget style bundles, defaulting to [`COLOR_SCHEME`](crate::COLOR_SCHEME).

mod button;
mod input_field;
mod markdown;
mod selection;
mod suggest_input;
mod syntax;
mod tab_bar;
mod table;
mod text;

pub use button::*;
pub use input_field::*;
pub use markdown::*;
pub use selection::*;
pub use suggest_input::*;
pub use syntax::*;
pub use tab_bar::*;
pub use table::*;
pub use text::*;
