mod color;
mod input_field;
mod selection;

pub use color::{ColorPair, PALETTES, TableColors};
pub use input_field::InputField;
pub use selection::Selection;

use ratatui::style::palette::tailwind;
use ratatui::style::{Color, Style as UiStyle};

pub enum Action {
    InputTaken,
    FocusNext,
    FocusPrevious,
    InputConfirm,
    InputIgnored,
}

#[derive(Clone)]
pub struct Style {
    pub default: UiStyle,
    pub focused: UiStyle,
    pub cursor: UiStyle,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            default: UiStyle::default()
                .fg(tailwind::WHITE)
                .bg(tailwind::SLATE.c950),
            focused: UiStyle::default()
                .fg(tailwind::INDIGO.c400)
                .bg(tailwind::SLATE.c950),
            cursor: UiStyle::default()
                .fg(tailwind::WHITE)
                .bg(tailwind::INDIGO.c600),
        }
    }
}
