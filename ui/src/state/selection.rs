use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};

use crate::EventResult;
use crate::traits::HandleEvents;
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
    #[getset(get = "pub")]
    values: Vec<ValueType>,
}

impl<ValueType> SelectionState<ValueType>
where
    ValueType: ToLabel + Clone,
{
    pub fn next_row(&mut self) {
        self.selection = if self.selection >= self.values.len() - 1 {
            0
        } else {
            self.selection + 1
        };
    }

    pub fn previous_row(&mut self) {
        self.selection = if self.selection == 0 {
            self.values.len() - 1
        } else {
            self.selection - 1
        };
    }
}

impl<ValueType> HandleEvents for SelectionState<ValueType>
where
    ValueType: ToLabel + Clone,
{
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        match (modifiers, code) {
            (_, KeyCode::Char('j')) | (_, KeyCode::Down) => {
                self.next_row();
                EventResult::Consumed
            }
            (_, KeyCode::Char('k')) | (_, KeyCode::Up) => {
                self.previous_row();
                EventResult::Consumed
            }
            _ => EventResult::Unhandled(modifiers, code),
        }
    }
}
