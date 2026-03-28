use ratatui::style::palette::tailwind;
use ratatui::style::{Color, Style as UiStyle};

#[derive(Clone)]
pub struct InputStyle {
    pub default: UiStyle,
    pub focused: UiStyle,
    pub cursor: UiStyle,
}

impl Default for InputStyle {
    fn default() -> Self {
        InputStyle {
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

#[derive(Clone)]
pub struct SelectionStyle {
    pub focused: UiStyle,
    pub border: UiStyle,
    pub default: UiStyle,
    pub rows: [UiStyle; 2],
}

impl Default for SelectionStyle {
    fn default() -> Self {
        SelectionStyle {
            default: UiStyle::default().fg(tailwind::WHITE).bg(Color::default()),
            focused: UiStyle::default()
                .fg(tailwind::INDIGO.c400)
                .bg(tailwind::SLATE.c950),
            border: UiStyle::default()
                .bg(tailwind::INDIGO.c400)
                .fg(tailwind::SLATE.c950),
            rows: [
                UiStyle::default()
                    .fg(tailwind::WHITE)
                    .bg(tailwind::SLATE.c950),
                UiStyle::default()
                    .fg(tailwind::WHITE)
                    .bg(tailwind::SLATE.c900),
            ],
        }
    }
}
