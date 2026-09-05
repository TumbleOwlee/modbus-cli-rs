use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};
use ratatui::style::Style;

use crate::COLOR_SCHEME;

#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters)]
pub struct TabBarStyle {
    #[builder(default = "Style::default().fg(COLOR_SCHEME.hi).bg(COLOR_SCHEME.bg).bold()")]
    pub general: Style,
    #[builder(default = "Style::default().bg(COLOR_SCHEME.hi_bg).fg(COLOR_SCHEME.text).bold()")]
    pub selected: Style,
}

impl Default for TabBarStyle {
    fn default() -> Self {
        TabBarStyleBuilder::default()
            .build()
            .expect("TabBarStyleBuilder fields all default")
    }
}
