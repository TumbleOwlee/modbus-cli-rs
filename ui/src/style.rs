use ratatui::style::palette::tailwind;
use ratatui::style::{Color, Style as UiStyle};

#[derive(Clone)]
pub struct Style {
    pub default: UiStyle,
    pub focused: UiStyle,
    pub cursor: UiStyle,
}

impl Default for Style {
    fn default() -> Self {
        Style {
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
