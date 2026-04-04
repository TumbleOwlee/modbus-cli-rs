use ratatui::style::palette::tailwind;
use ratatui::style::{Color, Style as UiStyle};

#[derive(Debug, Clone)]
pub struct TableStyle<const N: usize> {
    pub focused: UiStyle,
    pub border: UiStyle,
    pub default: UiStyle,
    pub rows: [UiStyle; 2],
    pub header: [UiStyle; N],
}

impl<const N: usize> Default for TableStyle<N> {
    fn default() -> Self {
        TableStyle {
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
            header: [UiStyle::default()
                .fg(tailwind::INDIGO.c400)
                .bg(tailwind::SLATE.c950); N],
        }
    }
}
