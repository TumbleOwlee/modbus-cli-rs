use crate::EventResult;
use crate::traits::HandleEvents;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::widgets::{ScrollbarState as UiScrollbarState, TableState as UiTableState};

pub trait ToRow<const N: usize> {
    fn to_row(&self) -> [String; N];
}

#[derive(Debug)]
pub struct TableState<ValueType, const N: usize>
where
    ValueType: ToRow<N> + Clone,
{
    focused: bool,
    selection: usize,
    table_state: UiTableState,
    scroll_state: UiScrollbarState,
    values: Vec<ValueType>,
}

impl<ValueType, const N: usize> Default for TableState<ValueType, N>
where
    ValueType: ToRow<N> + Clone,
{
    fn default() -> Self {
        Self {
            focused: false,
            selection: 0,
            table_state: UiTableState::default(),
            scroll_state: UiScrollbarState::default(),
            values: Vec::new(),
        }
    }
}

impl<ValueType, const N: usize> TableState<ValueType, N>
where
    ValueType: ToRow<N> + Clone,
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

    pub fn set_focus(&mut self, focus: bool) {
        self.focused = focus;
    }

    pub fn in_focus(&self) -> bool {
        self.focused
    }

    pub fn focus(self) -> Self {
        Self {
            focused: true,
            ..self
        }
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
