use crossterm::event::{KeyCode, KeyModifiers};
use std::io::{Stderr, Stdout, stderr, stdout};

use crate::EventResult;

pub trait HandleEvents {
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult;
}

pub trait Init {
    fn init() -> Self;
}

impl Init for Stdout {
    fn init() -> Self {
        stdout()
    }
}

impl Init for Stderr {
    fn init() -> Self {
        stderr()
    }
}

pub trait ToLabel {
    fn to_label(&self) -> String;
}

impl ToLabel for String {
    fn to_label(&self) -> String {
        self.clone()
    }
}

pub trait SetFocus {
    fn set_focused(&mut self, focus: bool);
}
