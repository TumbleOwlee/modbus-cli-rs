use crate::COLOR_SCHEME;
use crate::style::SyntaxTheme;
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};
use ratatui::style::{Color, Style};

/// Styles for [`DiffView`](crate::widgets::DiffView) rendering, including the added,
/// removed and meta row styles of UI-R-219: the widget's own, not the syntax theme's, so
/// a Diff-language code field elsewhere can keep the syntax theme's foreground-only
/// styles untouched (UI-R-220) while a diff view gets its own full row styles. `meta`
/// defaults to `SyntaxTheme::default().meta` specifically, not whatever `syntax_theme` a
/// caller supplies the widget: `DiffViewStyle` is built separately from `syntax_theme`,
/// so a caller supplying a custom theme and no explicit row styles here still gets the
/// default theme's meta, not their own.
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
    /// UI-R-276 — an added row's style, defaulting to white on `COLOR_SCHEME.success`.
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(Color::White).bg(COLOR_SCHEME.success)")]
    pub added: Style,
    /// UI-R-276 — a removed row's style, defaulting to white on `COLOR_SCHEME.error`.
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(Color::White).bg(COLOR_SCHEME.error)")]
    pub removed: Style,
    /// UI-R-276 — a meta row's style, defaulting to `SyntaxTheme::default().meta`, frozen
    /// at that value regardless of the widget's own `syntax_theme` (see the struct doc).
    #[getset(get = "pub")]
    #[builder(default = "SyntaxTheme::default().meta")]
    pub meta: Style,
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

    #[test]
    /// UI-R-276 — added/removed default to white on the scheme's success/error colors,
    /// and meta defaults to the syntax theme's own meta style, frozen at that value.
    fn ut_row_styles_default_to_white_on_green_white_on_red_and_the_theme_meta() {
        let style = DiffViewStyle::default();
        assert_eq!(
            style.added,
            Style::default().fg(Color::White).bg(COLOR_SCHEME.success)
        );
        assert_eq!(
            style.removed,
            Style::default().fg(Color::White).bg(COLOR_SCHEME.error)
        );
        assert_eq!(style.meta, SyntaxTheme::default().meta);
    }
}
