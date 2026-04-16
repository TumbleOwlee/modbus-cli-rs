use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};

use crate::EventResult;
use crate::traits::HandleEvents;
use crate::traits::SetFocus;
use crate::traits::ToLabel;

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters)]
#[getset(set = "pub")]
pub struct SelectionState<ValueType>
where
    ValueType: ToLabel + Clone,
{
    #[getset(get_copy = "pub")]
    #[builder(default = "true")]
    focused: bool,
    #[getset(get_copy = "pub")]
    #[builder(setter(skip))]
    selection: usize,
    #[getset(get_copy = "pub")]
    #[builder(setter(skip))]
    horizontal_offset: usize,
    #[getset(get = "pub")]
    values: Vec<ValueType>,
}

impl<ValueType> SetFocus for SelectionState<ValueType>
where
    ValueType: ToLabel + Clone,
{
    fn set_focused(&mut self, focus: bool) {
        self.focused = focus;
    }

    fn is_focused(&self) -> bool {
        self.focused
    }
}

impl<ValueType> SelectionState<ValueType>
where
    ValueType: ToLabel + Clone,
{
    pub fn move_down(&mut self) {
        self.selection = if self.selection >= self.values.len() - 1 {
            0
        } else {
            self.selection + 1
        };
    }

    pub fn move_up(&mut self) {
        self.selection = if self.selection == 0 {
            self.values.len() - 1
        } else {
            self.selection - 1
        };
    }

    pub fn move_right(&mut self) {
        self.horizontal_offset += 1;
    }

    pub fn move_left(&mut self) {
        self.horizontal_offset -= if self.horizontal_offset > 0 { 1 } else { 0 };
    }
}

impl<ValueType> HandleEvents for SelectionState<ValueType>
where
    ValueType: ToLabel + Clone,
{
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        match (modifiers, code) {
            (_, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                self.move_down();
                EventResult::Consumed
            }
            (_, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                self.move_up();
                EventResult::Consumed
            }
            (_, KeyCode::Char('h')) | (_, KeyCode::Left) => {
                self.move_left();
                EventResult::Consumed
            }
            (_, KeyCode::Char('l')) | (_, KeyCode::Right) => {
                self.move_right();
                EventResult::Consumed
            }
            _ => EventResult::Unhandled(modifiers, code),
        }
    }
}
