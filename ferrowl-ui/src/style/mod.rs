//! Per-widget style bundles, defaulting to [`COLOR_SCHEME`](crate::COLOR_SCHEME).

mod button;
mod diff_view;
mod input_field;
mod markdown;
mod scrolling_tabs;
mod selection;
mod suggest_input;
mod syntax;
mod table;
mod text;

pub use button::*;
pub use diff_view::*;
pub use input_field::*;
pub use markdown::*;
pub use scrolling_tabs::*;
pub use selection::*;
pub use suggest_input::*;
pub use syntax::*;
pub use table::*;
pub use text::*;
