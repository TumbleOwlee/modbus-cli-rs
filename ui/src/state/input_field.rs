use crossterm::event::{KeyCode, KeyModifiers};

use crate::traits::HandleEvents;
use crate::{EventResult, Transition};

#[derive(Debug, Default)]
pub struct InputFieldState {
    input: Option<String>,
    cursor: usize,
    pub focused: bool,
    pub disabled: bool,
}

impl InputFieldState {
    pub fn set_input(&mut self, value: &str) {
        let len = value.len();
        self.input = Some(value.to_string());
        self.cursor = std::cmp::min(self.cursor, len);
    }

    pub fn clear(&mut self) {
        self.input = None;
        self.cursor = 0;
    }

    pub fn get_input(&self) -> Option<String> {
        self.input.clone()
    }

    pub fn set_cursor(&mut self, cursor: usize) {
        let len = self.input.as_ref().map(|s| s.len()).unwrap_or(0);
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

    pub fn focus(self) -> Self {
        Self {
            focused: true,
            ..self
        }
    }

    pub fn disabled(self) -> Self {
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
                if let Some(input) = &self.input {
                    self.cursor = input.len();
                } else {
                    self.cursor = 0;
                }
                EventResult::Consumed
            }
            (_, KeyCode::Char(c)) => {
                if let Some(input) = &mut self.input {
                    input.insert(self.cursor, c);
                } else {
                    self.input = Some(String::from(c));
                }
                self.cursor += 1;
                EventResult::Consumed
            }
            (_, KeyCode::Backspace) => {
                if self.cursor > 0 {
                    if let Some(input) = &mut self.input {
                        if input.len() > 0 {
                            input.remove(self.cursor - 1);
                        } else {
                            self.input = None;
                        }
                        self.cursor -= 1;
                    }
                }
                EventResult::Consumed
            }
            (_, KeyCode::Delete) => {
                if let Some(input) = &mut self.input {
                    if input.len() > self.cursor {
                        input.remove(self.cursor);
                    }
                    if input.len() == 0 {
                        self.input = None;
                    }
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
                if let Some(input) = &self.input {
                    self.cursor = std::cmp::min(self.cursor + 1, input.len());
                } else {
                    self.cursor = 0;
                }
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
