use crossterm::event::{KeyCode, KeyModifiers};

#[derive(Debug)]
pub enum Transition {
    FocusPrevious,
    FocusNext,
}

#[derive(Debug)]
pub enum EventResult {
    Transition(Transition),
    Consumed,
    Unhandled(KeyModifiers, KeyCode),
}
