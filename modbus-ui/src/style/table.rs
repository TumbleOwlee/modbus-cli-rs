use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};
use ratatui::style::palette::tailwind;
use ratatui::style::{Color, Style};

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters)]
#[getset(set = "pub")]
pub struct TableStyle {
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(tailwind::INDIGO.c950).bg(tailwind::SLATE.c900)")]
    pub focused: Style,
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(tailwind::WHITE).bg(Color::default())")]
    pub border: Style,
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(tailwind::INDIGO.c950)")]
    pub general: Style,
    #[getset(get = "pub")]
    #[builder(
        default = "[Style::default().fg(tailwind::SLATE.c200).bg(tailwind::SLATE.c950), Style::default().fg(tailwind::SLATE.c200).bg(tailwind::SLATE.c800)]"
    )]
    pub rows: [Style; 2],
    #[getset(get = "pub")]
    #[builder(
        default = "Style::default().bg(tailwind::INDIGO.c950).fg(tailwind::SLATE.c200).bold()"
    )]
    pub header: Style,
}

impl Default for TableStyle {
    fn default() -> Self {
        TableStyleBuilder::default().build().unwrap()
    }
}
