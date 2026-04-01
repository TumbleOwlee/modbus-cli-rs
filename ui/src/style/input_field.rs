use ratatui::style::palette::tailwind;
use ratatui::style::{Color, Style as UiStyle};

#[derive(Debug, Clone)]
pub struct InputFieldStyle {
    pub default: UiStyle,
    pub focused: UiStyle,
    pub cursor: UiStyle,
}

impl Default for InputFieldStyle {
    fn default() -> Self {
        InputFieldStyle {
            default: UiStyle::default().fg(tailwind::WHITE).bg(Color::default()),
            focused: UiStyle::default()
                .fg(tailwind::INDIGO.c400)
                .bg(tailwind::SLATE.c950),
            cursor: UiStyle::default()
                .fg(tailwind::WHITE)
                .bg(tailwind::INDIGO.c600),
        }
    }
}
