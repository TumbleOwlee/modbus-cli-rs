use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};

use crate::traits::HandleEvents;
use crate::{EventResult, Transition};

#[derive(Builder, Debug, Default, Clone, Getters, Setters, CopyGetters)]
#[getset(set = "pub")]
pub struct InputFieldState {
    #[getset(get = "pub")]
    #[builder(setter(skip))]
    input: String,
    #[getset(get_copy = "pub")]
    #[builder(setter(skip))]
    cursor: usize,
    #[getset(get_copy = "pub")]
    #[builder(default = "true")]
    focused: bool,
    #[getset(get_copy = "pub")]
    #[builder(default = "false")]
    disabled: bool,
    #[getset(get = "pub")]
    #[builder(default = "None")]
    placeholder: Option<String>,
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
