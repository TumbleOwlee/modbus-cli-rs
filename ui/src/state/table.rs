use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};
use ratatui::widgets::{StatefulWidget, Widget};

use crate::EventResult;
use crate::traits::{HandleEvents, SetFocus};

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters)]
#[getset(set = "pub")]
pub struct TableState<ValueType>
where
    ValueType: Widget + StatefulWidget,
{
    #[getset(get_copy = "pub")]
    #[builder(default = "true")]
    focused: bool,
    #[getset(get_copy = "pub")]
    #[builder(setter(skip), default = "0")]
    selection: usize,
    #[getset(get_copy = "pub")]
    #[builder(setter(skip), default = "0")]
    horizontal_scroll: usize,
    #[getset(get = "pub")]
    values: Vec<ValueType>,
}

impl<ValueType> SetFocus for TableState<ValueType>
where
    ValueType: Widget + StatefulWidget,
{
    fn set_focused(&mut self, focus: bool) {
        self.focused = focus;
    }
}

impl<ValueType> TableState<ValueType>
where
    ValueType: Widget + StatefulWidget,
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

    pub fn move_to_bottom(&mut self) {
        self.selection = if self.values.is_empty() {
            0
        } else {
            self.values.len() - 1
        };
    }

    pub fn move_to_top(&mut self) {
        self.selection = 0;
    }

    pub fn move_right(&mut self) {
        self.horizontal_scroll += 1;
    }

    pub fn move_left(&mut self) {
        self.horizontal_scroll -= if self.horizontal_scroll > 0 { 1 } else { 0 };
    }
}

impl<ValueType> HandleEvents for TableState<ValueType>
where
    ValueType: Widget + StatefulWidget,
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
