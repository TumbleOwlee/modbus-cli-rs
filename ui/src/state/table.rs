use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};
use ratatui::widgets::{ScrollbarState as UiScrollbarState, TableState as UiTableState};

use crate::EventResult;
use crate::traits::HandleEvents;

pub trait ToRow<const N: usize> {
    fn to_row(&self) -> [String; N];
}

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters)]
#[getset(set = "pub")]
pub struct TableState<ValueType, const N: usize>
where
    ValueType: ToRow<N> + Clone,
{
    #[getset(get_copy = "pub")]
    #[builder(default = "true")]
    focused: bool,
    #[getset(get_copy = "pub")]
    #[builder(setter(skip))]
    selection: usize,
    #[getset(get = "pub")]
    table_state: UiTableState,
    #[getset(get = "pub")]
    scroll_state: UiScrollbarState,
    #[getset(get = "pub")]
    values: Vec<ValueType>,
}

impl<ValueType, const N: usize> TableState<ValueType, N>
where
    ValueType: ToRow<N> + Clone,
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

impl<ValueType, const N: usize> HandleEvents for TableState<ValueType, N>
where
    ValueType: ToRow<N> + Clone,
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
