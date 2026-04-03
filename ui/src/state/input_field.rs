use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};

use crate::EventResult;
use crate::traits::{HandleEvents, SetFocus};

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

impl SetFocus for InputFieldState {
    fn set_focused(&mut self, focus: bool) {
        self.focused = focus;
    }
}

impl HandleEvents for InputFieldState {
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        match (modifiers, code) {
            (_, KeyCode::Home) => {
                self.cursor = 0;
                EventResult::Consumed
            }
            (_, KeyCode::End) => {
                self.cursor = self.input.chars().count();
                EventResult::Consumed
            }
            (_, KeyCode::Char(c)) => {
                if !self.disabled {
                    if self.input.is_empty() || self.input.chars().count() == self.cursor {
                        self.input.push(c);
                    } else {
                        self.input = self.input.chars().enumerate().fold(
                            String::with_capacity(self.input.capacity() + 1),
                            |mut s, (i, v)| {
                                if i == self.cursor {
                                    s.push(c);
                                }
                                s.push(v);
                                s
                            },
                        );
                    }
                    self.cursor += 1;
                }
                EventResult::Consumed
            }
            (_, KeyCode::Backspace) => {
                if !self.disabled {
                    if self.cursor > 0 {
                        if self.input.chars().count() >= self.cursor {
                            self.input = self.input.chars().enumerate().fold(
                                String::with_capacity(self.input.capacity() + 1),
                                |mut s, (i, v)| {
                                    if i != self.cursor - 1 {
                                        s.push(v);
                                    }
                                    s
                                },
                            );
                        }
                        self.cursor -= 1;
                    }
                }
                EventResult::Consumed
            }
            (_, KeyCode::Delete) => {
                if !self.disabled {
                    if self.input.chars().count() > self.cursor {
                        self.input = self.input.chars().enumerate().fold(
                            String::with_capacity(self.input.capacity() + 1),
                            |mut s, (i, v)| {
                                if i != self.cursor {
                                    s.push(v);
                                }
                                s
                            },
                        );
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
                self.cursor = std::cmp::min(self.cursor + 1, self.input.chars().count());
                EventResult::Consumed
            }
            (m, c) => EventResult::Unhandled(m, c),
        }
    }
}
