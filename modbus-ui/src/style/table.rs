use ratatui::style::palette::tailwind;
use ratatui::style::{Color, Style as UiStyle};

#[derive(Debug, Clone)]
pub struct TableStyle {
    pub focused: UiStyle,
    pub border: UiStyle,
    pub default: UiStyle,
    pub rows: [UiStyle; 2],
    pub header: UiStyle,
}

impl Default for TableStyle {
    fn default() -> Self {
        TableStyle {
            default: UiStyle::default().fg(tailwind::WHITE).bg(Color::default()),
            focused: UiStyle::default()
                .fg(tailwind::INDIGO.c400)
                .bg(tailwind::SLATE.c900),
            border: UiStyle::default().fg(tailwind::INDIGO.c400),
            rows: [
                UiStyle::default()
                    .fg(tailwind::SLATE.c200)
                    .bg(tailwind::SLATE.c950),
                UiStyle::default()
                    .fg(tailwind::SLATE.c200)
                    .bg(tailwind::SLATE.c800),
            ],
            header: UiStyle::default()
                .bg(tailwind::INDIGO.c400)
                .fg(tailwind::SLATE.c200)
                .bold(),
        }
    }
}
