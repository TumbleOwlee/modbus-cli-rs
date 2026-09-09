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
    /// UI-R-276 — an added row's style, defaulting to white on the scheme's
    /// `diff_added` color.
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(Color::White).bg(COLOR_SCHEME.diff_added)")]
    pub added: Style,
    /// UI-R-276 — a removed row's style, defaulting to white on the scheme's
    /// `diff_removed` color.
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(Color::White).bg(COLOR_SCHEME.diff_removed)")]
    pub removed: Style,
    /// UI-R-276 — a meta row's style, defaulting to `SyntaxTheme::default().meta`, frozen
    /// at that value regardless of the widget's own `syntax_theme` (see the struct doc).
    #[getset(get = "pub")]
    #[builder(default = "SyntaxTheme::default().meta")]
    pub meta: Style,
    /// UI-R-282, UI-R-284 — the word-diff emphasis style for an added row's changed
    /// words, defaulting to the scheme's `diff_added_word` color (UI-R-290 keeps it
    /// lighter than `diff_added`). A caller setting one style is setting it against a
    /// band they may also have replaced. Painting applies the background only: the
    /// style sets no foreground.
    #[getset(get = "pub")]
    #[builder(default = "Style::default().bg(COLOR_SCHEME.diff_added_word)")]
    pub added_word: Style,
    /// UI-R-282, UI-R-284 — the word-diff emphasis style for a removed row's changed
    /// words, defaulting to the scheme's `diff_removed_word` color. See `added_word`
    /// for the caller note.
    #[getset(get = "pub")]
    #[builder(default = "Style::default().bg(COLOR_SCHEME.diff_removed_word)")]
    pub removed_word: Style,
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
    /// UI-R-276 — added/removed default to white on the scheme's `diff_added`/
    /// `diff_removed` colors, and meta defaults to the syntax theme's own meta style,
    /// frozen at that value.
    fn ut_row_styles_default_to_the_schemes_diff_colors_and_theme_meta() {
        let style = DiffViewStyle::default();
        assert_eq!(style.added.fg, Some(Color::White));
        assert_eq!(style.removed.fg, Some(Color::White));
        assert_eq!(style.added.bg, Some(COLOR_SCHEME.diff_added));
        assert_eq!(style.removed.bg, Some(COLOR_SCHEME.diff_removed));
        assert_eq!(style.meta, SyntaxTheme::default().meta);
    }

    #[test]
    /// UI-R-282 — the word-emphasis backgrounds default to the scheme's
    /// `diff_added_word`/`diff_removed_word` colors and set no foreground.
    fn ut_word_emphasis_styles_default_to_the_schemes_word_colors() {
        let style = DiffViewStyle::default();
        assert_eq!(style.added_word.bg, Some(COLOR_SCHEME.diff_added_word));
        assert_eq!(style.removed_word.bg, Some(COLOR_SCHEME.diff_removed_word));
        assert_eq!(style.added_word.fg, None);
        assert_eq!(style.removed_word.fg, None);
    }

    #[test]
    /// UI-R-282 — the word-emphasis styles are builder-settable, not just defaulted.
    fn ut_word_emphasis_styles_are_builder_settable() {
        let added_word = Style::default().bg(Color::Rgb(1, 2, 3));
        let removed_word = Style::default().bg(Color::Rgb(4, 5, 6));
        let style = DiffViewStyleBuilder::default()
            .added_word(added_word)
            .removed_word(removed_word)
            .build()
            .expect("all other fields default");
        assert_eq!(*style.added_word(), added_word);
        assert_eq!(*style.removed_word(), removed_word);
    }
}
