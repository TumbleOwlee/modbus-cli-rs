use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};
use ratatui::style::palette::tailwind;
use ratatui::style::{Color, Style};

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters)]
#[getset(set = "pub")]
pub struct InputFieldStyle {
    #[builder(default = "Style::default().fg(tailwind::WHITE).bg(Color::default())")]
    pub general: Style,
    #[builder(default = "Style::default().fg(tailwind::INDIGO.c950).bg(tailwind::SLATE.c950)")]
    pub focused: Style,
    #[builder(default = "Style::default().fg(tailwind::NEUTRAL.c500)")]
    pub placeholder: Style,
    #[builder(default = "Style::default().fg(tailwind::WHITE).bg(tailwind::INDIGO.c950)")]
    pub cursor: Style,
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(tailwind::RED.c500)")]
    pub error: Style,
}

impl Default for InputFieldStyle {
    fn default() -> Self {
        InputFieldStyleBuilder::default().build().unwrap()
    }
}
