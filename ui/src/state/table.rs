use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};
use ratatui::widgets::{ScrollbarState, TableState as UiTableState};

use crate::EventResult;
use crate::traits::{HandleEvents, SetFocus};
use crate::widgets::TableEntry;

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

impl<V, const N: usize> SetFocus for TableState<V, N>
where
    V: TableEntry<N>,
{
    fn set_focused(&mut self, focus: bool) {
        self.focused = focus;
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

    pub fn move_left(&mut self) {
        self.horizontal_scroll = std::cmp::max(3, self.horizontal_scroll) - 3;
    }
}

impl<V, const N: usize> HandleEvents for TableState<V, N>
where
    V: TableEntry<N>,
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
