use crate::COLOR_SCHEME;
use crate::style::SyntaxTheme;
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};
use ratatui::style::{Color, Style};

/// The added and removed row backgrounds of UI-R-276: the color scheme's success or error
/// color darkened toward black, the shade chosen so a full-row band (UI-R-278) reads as a
/// band rather than a wash and still carries white text. Every shipped color scheme states
/// its colors as `Color::Rgb`; any other variant has no component to darken and is returned
/// unchanged.
fn band_bg(color: Color) -> Color {
    const BAND_DARKEN: f32 = 0.45;
    match color {
        Color::Rgb(r, g, b) => {
            let scale = |c: u8| (c as f32 * (1.0 - BAND_DARKEN)).round() as u8;
            Color::Rgb(scale(r), scale(g), scale(b))
        }
        other => other,
    }
}

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
    /// UI-R-276 — an added row's style, defaulting to white on `COLOR_SCHEME.success`
    /// darkened toward black (see `band_bg`).
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(Color::White).bg(band_bg(COLOR_SCHEME.success))")]
    pub added: Style,
    /// UI-R-276 — a removed row's style, defaulting to white on `COLOR_SCHEME.error`
    /// darkened toward black (see `band_bg`).
    #[getset(get = "pub")]
    #[builder(default = "Style::default().fg(Color::White).bg(band_bg(COLOR_SCHEME.error))")]
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
    /// UI-R-276 — added/removed default to white on the scheme's success/error
    /// colors darkened toward black, and meta defaults to the syntax theme's own meta
    /// style, frozen at that value.
    fn ut_row_styles_default_to_darkened_success_error_and_theme_meta() {
        let style = DiffViewStyle::default();
        assert_eq!(style.added.fg, Some(Color::White));
        assert_eq!(style.removed.fg, Some(Color::White));
        let darker = |base: Color, got: Option<Color>| {
            let Color::Rgb(br, bg, bb) = base else {
                panic!("scheme colors are Color::Rgb")
            };
            let Some(Color::Rgb(gr, gg, gb)) = got else {
                panic!("row style background is Color::Rgb")
            };
            assert!(gr <= br && gg <= bg && gb <= bb, "darkened, not brighter");
            assert!(
                gr < br || gg < bg || gb < bb,
                "at least one component moved"
            );
            assert!(gr > 0 || gg > 0 || gb > 0, "not black");
        };
        darker(COLOR_SCHEME.success, style.added.bg);
        darker(COLOR_SCHEME.error, style.removed.bg);
        assert_eq!(style.meta, SyntaxTheme::default().meta);
    }
}
