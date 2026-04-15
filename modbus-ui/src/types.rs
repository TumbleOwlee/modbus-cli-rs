use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Margin;

#[derive(Debug)]
pub enum EventResult {
    Consumed,
    Unhandled(KeyModifiers, KeyCode),
}

#[derive(Debug, Clone)]
pub enum Border {
    None,
    Full(Margin),
}
