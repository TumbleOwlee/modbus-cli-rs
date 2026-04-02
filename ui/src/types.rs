use crossterm::event::{KeyCode, KeyModifiers};

#[derive(Debug)]
pub enum EventResult {
    Consumed,
    Unhandled(KeyModifiers, KeyCode),
}
