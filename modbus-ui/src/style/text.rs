use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};
use ratatui::style::palette::tailwind;
use ratatui::style::{Color, Style};

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters)]
#[getset(set = "pub")]
pub struct TextStyle {
    #[builder(default = "Style::default().fg(tailwind::WHITE).bg(Color::default())")]
    pub general: Style,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyleBuilder::default().build().unwrap()
    }
}
