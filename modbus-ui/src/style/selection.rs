use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};
use ratatui::style::palette::tailwind;
use ratatui::style::{Color, Style};

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters)]
#[getset(set = "pub")]
pub struct SelectionStyle {
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(tailwind::INDIGO.c400).bg(tailwind::SLATE.c950)")]
    pub focused: Style,
    #[getset(get = "pub")]
    #[builder(default = "Style::default().bg(tailwind::INDIGO.c400).fg(tailwind::SLATE.c950)")]
    pub border: Style,
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(tailwind::WHITE).bg(Color::default())")]
    pub general: Style,
    #[getset(get = "pub")]
    #[builder(
        default = "[Style::default().fg(tailwind::WHITE).bg(tailwind::SLATE.c950),Style::default().fg(tailwind::WHITE).bg(tailwind::SLATE.c900),]"
    )]
    pub rows: [Style; 2],
}

impl Default for SelectionStyle {
    fn default() -> Self {
        SelectionStyleBuilder::default().build().unwrap()
    }
}
