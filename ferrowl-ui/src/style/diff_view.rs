use crate::COLOR_SCHEME;
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};
use ratatui::style::Style;

/// Styles for [`DiffView`](crate::widgets::DiffView) rendering. The added, removed and
/// meta text styles are not duplicated here: UI-R-219 names the syntax theme's
/// (`SyntaxTheme::added`, `::removed`, `::meta`), which the widget reads directly.
#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters)]
#[getset(set = "pub")]
pub struct DiffViewStyle {
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(COLOR_SCHEME.text).bg(COLOR_SCHEME.bg)")]
    pub general: Style,
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(COLOR_SCHEME.hi).bg(COLOR_SCHEME.bg)")]
    pub focused: Style,
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(COLOR_SCHEME.text).bg(COLOR_SCHEME.bg)")]
    pub border: Style,
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(COLOR_SCHEME.text_hi).bg(COLOR_SCHEME.hi_bg)")]
    pub selection: Style,
    /// The read-only highlighted-row style of UI-R-224. Defaulted to the same value as
    /// `MarkdownTheme::highlighted_row` (UI-R-138), so the two widgets' read-only active-row
    /// highlight agree.
    #[getset(get = "pub")]
    #[builder(default = "Style::default().bg(COLOR_SCHEME.hi_bg)")]
    pub highlighted_row: Style,
}

impl Default for DiffViewStyle {
    fn default() -> Self {
        DiffViewStyleBuilder::default()
            .build()
            .expect("DiffViewStyleBuilder fields all default")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::MarkdownTheme;

    #[test]
    /// UI-R-224 — the highlighted-row default agrees with `MarkdownTheme`'s (UI-R-138), so
    /// the two widgets' read-only active-row highlight look the same.
    fn ut_highlighted_row_default_matches_markdown_theme() {
        let diff = DiffViewStyle::default();
        let markdown = MarkdownTheme::default();
        assert_eq!(diff.highlighted_row, *markdown.highlighted_row());
    }
}
