use crossterm::event::{KeyCode, KeyModifiers};

use crate::traits::HandleEvents;
use crate::{EventResult, Transition};

#[derive(Debug, Default, Clone)]
pub struct InputFieldState {
    input: String,
    cursor: usize,
    focused: bool,
    disabled: bool,
}

impl InputFieldState {
    pub fn set_input(&mut self, value: &str) {
        let len = value.len();
        self.input = value.to_string();
        self.cursor = std::cmp::min(self.cursor, len);
    }

    pub fn clear(&mut self) {
        self.input.clear();
        self.cursor = 0;
    }

    pub fn get_input(&self) -> Option<String> {
        if self.input.is_empty() {
            None
        } else {
            Some(self.input.clone())
        }
    }

    pub fn set_cursor(&mut self, cursor: usize) {
        let len = self.input.len();
        self.cursor = std::cmp::min(cursor, len);
    }

    pub fn get_cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_focus(&mut self) {
        if self.disabled {
            panic!("Tried to select disabled input field.");
        }
        self.focused = true;
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

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn disable(self) -> Self {
        Self {
            disabled: true,
            ..self
        }
    }
}

impl HandleEvents for InputFieldState {
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        if self.disabled {
            return EventResult::Consumed;
        }

        match (modifiers, code) {
            (_, KeyCode::Home) => {
                self.cursor = 0;
                EventResult::Consumed
            }
            (_, KeyCode::End) => {
                self.cursor = self.input.len();
                EventResult::Consumed
            }
            (_, KeyCode::Char(c)) => {
                self.input.insert(self.cursor, c);
                self.cursor += 1;
                EventResult::Consumed
            }
            (_, KeyCode::Backspace) => {
                if self.cursor > 0 {
                    if self.input.len() >= self.cursor {
                        self.input.remove(self.cursor - 1);
                    }
                    self.cursor -= 1;
                }
                EventResult::Consumed
            }
            (_, KeyCode::Delete) => {
                if self.input.len() > self.cursor {
                    self.input.remove(self.cursor);
                }
                EventResult::Consumed
            }
            (_, KeyCode::Left) => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
                EventResult::Consumed
            }
            (_, KeyCode::Right) => {
                self.cursor = std::cmp::min(self.cursor + 1, self.input.len());
                EventResult::Consumed
            }
            (KeyModifiers::SHIFT, KeyCode::Tab) => {
                self.focused = false;
                EventResult::Transition(Transition::FocusPrevious)
            }
            (_, KeyCode::Tab) => {
                self.focused = false;
                EventResult::Transition(Transition::FocusNext)
            }
            (m, c) => EventResult::Unhandled(m, c),
        }
    }
}
