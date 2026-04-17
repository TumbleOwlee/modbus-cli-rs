use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};
use ratatui::widgets::{ScrollbarState, TableState as UiTableState};

use crate::EventResult;
use crate::traits::{HandleEvents, IsFocus, SetFocus};
use crate::widgets::{GetValue, TableEntry};

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters)]
#[getset(set = "pub")]
pub struct TableState<V, const N: usize>
where
    V: TableEntry<N>,
{
    #[getset(get_copy = "pub")]
    #[builder(default = "true")]
    focused: bool,
    #[getset(get_copy = "pub")]
    #[builder(setter(skip), default = "ScrollbarState::default()")]
    vertical_scroll: ScrollbarState,
    #[getset(get_copy = "pub")]
    #[builder(setter(skip), default = "0")]
    horizontal_scroll: u16,
    #[getset(get = "pub")]
    values: Vec<V>,
    #[getset(get_copy = "pub")]
    #[builder(setter(skip), default = "0")]
    visible_width: u16,
    #[getset(get_copy = "pub")]
    #[builder(setter(skip), default = "0")]
    total_width: u16,
    #[getset(get_copy = "pub")]
    #[builder(setter(skip), default = "UiTableState::default().with_selected(0)")]
    table_state: UiTableState,
}

impl<V, const N: usize> GetValue for TableState<V, N>
where
    V: TableEntry<N> + Clone + Default,
{
    type ValueType = V;

    fn get_value(&self) -> Self::ValueType {
        self.values
            .get(self.table_state.selected().unwrap_or(0))
            .map(|v| (*v).clone())
            .unwrap_or_default()
    }
}

impl<V, const N: usize> SetFocus for TableState<V, N>
where
    V: TableEntry<N>,
{
    fn set_focused(&mut self, focus: bool) {
        self.focused = focus;
    }
}

impl<V, const N: usize> IsFocus for TableState<V, N>
where
    V: TableEntry<N>,
{
    fn is_focused(&self) -> bool {
        self.focused
    }
}

impl<V, const N: usize> TableState<V, N>
where
    V: TableEntry<N>,
{
    pub fn move_down(&mut self) {
        if self.values.is_empty() {
            self.table_state.select(None);
            self.vertical_scroll = self.vertical_scroll.position(0);
        } else {
            let i = self
                .table_state
                .selected()
                .map(|i| std::cmp::min(i + 1, std::cmp::max(self.values.len(), 1) - 1))
                .unwrap_or(0);
            self.table_state.select(Some(i));
            self.vertical_scroll = self.vertical_scroll.position(i);
        }
    }

    pub fn move_up(&mut self) {
        if self.values.is_empty() {
            self.table_state.select(None);
            self.vertical_scroll = self.vertical_scroll.position(0);
        } else {
            let i = self
                .table_state
                .selected()
                .map(|i| std::cmp::max(i, 1) - 1)
                .unwrap_or(0);
            self.table_state.select(Some(i));
            self.vertical_scroll = self.vertical_scroll.position(i);
        }
    }

    pub fn move_to_bottom(&mut self) {
        if self.values.is_empty() {
            self.table_state.select(None);
            self.vertical_scroll = self.vertical_scroll.position(0);
        } else {
            self.table_state.select(Some(self.values.len() - 1));
            self.vertical_scroll = self.vertical_scroll.position(self.values.len() - 1);
        }
    }

    pub fn move_to_top(&mut self) {
        if self.values.is_empty() {
            self.table_state.select(None);
            self.vertical_scroll = self.vertical_scroll.position(0);
        } else {
            self.table_state.select(Some(0));
            self.vertical_scroll = self.vertical_scroll.position(0);
        }
    }

    pub fn move_right(&mut self) {
        self.horizontal_scroll = std::cmp::min(
            std::cmp::max(self.total_width, self.visible_width) - self.visible_width,
            self.horizontal_scroll + 3,
        );
    }

    pub fn move_to_right(&mut self) {
        self.horizontal_scroll =
            std::cmp::max(self.total_width, self.visible_width) - self.visible_width;
    }

    pub fn move_left(&mut self) {
        self.horizontal_scroll = std::cmp::max(3, self.horizontal_scroll) - 3;
    }

    pub fn move_to_left(&mut self) {
        self.horizontal_scroll = 0;
    }
}

impl<V, const N: usize> HandleEvents for TableState<V, N>
where
    V: TableEntry<N>,
{
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        match (modifiers, code) {
            (KeyModifiers::NONE, KeyCode::Char('j')) | (KeyModifiers::NONE, KeyCode::Down) => {
                self.move_down();
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Char('k')) | (KeyModifiers::NONE, KeyCode::Up) => {
                self.move_up();
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Char('h')) | (KeyModifiers::NONE, KeyCode::Left) => {
                self.move_left();
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Char('l')) | (KeyModifiers::NONE, KeyCode::Right) => {
                self.move_right();
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::End) => {
                self.move_to_right();
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Home) => {
                self.move_to_left();
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Char('G')) => {
                self.move_to_bottom();
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Char('g')) => {
                self.move_to_top();
                EventResult::Consumed
            }
            _ => EventResult::Unhandled(modifiers, code),
        }
    }
}
