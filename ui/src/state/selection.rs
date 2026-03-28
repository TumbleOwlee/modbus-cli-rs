use crossterm::event::{KeyCode, KeyModifiers};

use super::super::traits::ToLabel;
use crate::traits::HandleEvents;
use crate::{EventResult, Transition};

#[derive(Debug)]
pub struct SelectionState<ValueType>
where
    ValueType: ToLabel + Clone,
{
    selection: usize,
    values: Vec<ValueType>,
}

impl<ValueType> Default for SelectionState<ValueType>
where
    ValueType: ToLabel + Clone,
{
    fn default() -> Self {
        Self {
            selection: 0,
            values: Vec::new(),
        }
    }
}

impl<ValueType> SelectionState<ValueType>
where
    ValueType: ToLabel + Clone,
{
    pub fn set_values(&mut self, values: Vec<ValueType>) {
        self.selection = 0;
        self.values = values;
    }

    pub fn get_values(&self) -> &Vec<ValueType> {
        &self.values
    }

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

    pub fn get_label(&self) -> Option<String> {
        if self.values.len() > self.selection {
            self.values.get(self.selection).map(ToLabel::to_label)
        } else {
            self.values.first().map(ToLabel::to_label)
        }
    }

    pub fn get_selection(&self) -> Option<ValueType> {
        if self.values.len() > self.selection {
            self.values.get(self.selection).cloned()
        } else {
            self.values.first().cloned()
        }
    }

    pub fn get_selection_index(&self) -> usize {
        self.selection
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
