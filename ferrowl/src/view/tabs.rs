//! Module tab bar across the top.

use ferrowl_ui::{
    state::TabBarState,
    widgets::{TabBarBuilder, TabBarBuilderError},
};
use ratatui::{buffer::Buffer, layout::Rect, widgets::StatefulWidget};

fn build_widget() -> Result<ferrowl_ui::widgets::TabBar<String>, TabBarBuilderError> {
    TabBarBuilder::<String>::default().build()
}

/// The tab bar's rendered height under its default padding (UI-R-002), the
/// same value the widget itself computes so the layout row and the render
/// agree by construction.
pub fn tab_bar_height() -> u16 {
    build_widget()
        .expect("all required builder fields are set")
        .rendered_extent()
}

/// Render the tab bar with `names`, scrolling as needed to keep `active`
/// visible. `offset` is the widget-owned scroll offset (UI-R-119): the
/// widget only ever updates it, never resets it itself between frames, so
/// the caller must persist it across renders.
pub fn render_tabs(
    names: &[String],
    active: usize,
    area: Rect,
    buf: &mut Buffer,
    offset: &mut usize,
) {
    let widget = build_widget().expect("all required builder fields are set");
    let mut state = TabBarState {
        titles: names.to_vec(),
        active,
        offset: *offset,
    };
    StatefulWidget::render(&widget, area, buf, &mut state);
    *offset = state.offset;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UI-R-002 — the tab bar's layout row height equals the widget's own
    /// rendered extent under the default padding, one row.
    #[test]
    fn ut_tab_bar_height_matches_layout_row() {
        assert_eq!(tab_bar_height(), 1);
        assert_eq!(tab_bar_height(), build_widget().unwrap().rendered_extent());
    }

    /// UI-E-050 — many tabs in a narrow area scroll to keep the active
    /// tab's characters visible in the buffer.
    #[test]
    fn ut_render_tabs_scrolls_active_into_view() {
        let names: Vec<String> = (0..10).map(|i| format!(" [{i}] Module{i} ")).collect();
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        let mut offset = 0usize;
        render_tabs(&names, 9, area, &mut buf, &mut offset);
        let rendered: String = (0..20).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(rendered.contains("Module9"));
    }

    /// UI-R-175 — the stored offset is an output of the render: seeding it
    /// with a bogus value yields the same freshly computed window a render
    /// starting from offset 0 produces, not the seeded value carried through.
    #[test]
    fn ut_render_tabs_overwrites_a_seeded_offset() {
        let names: Vec<String> = (0..10).map(|i| format!(" [{i}] Module{i} ")).collect();
        let area = Rect::new(0, 0, 20, 1);

        let mut seeded_buf = Buffer::empty(area);
        let mut seeded_offset = 999usize;
        render_tabs(&names, 5, area, &mut seeded_buf, &mut seeded_offset);

        let mut fresh_buf = Buffer::empty(area);
        let mut fresh_offset = 0usize;
        render_tabs(&names, 5, area, &mut fresh_buf, &mut fresh_offset);

        assert_eq!(seeded_offset, fresh_offset);
        for x in 0..20 {
            assert_eq!(seeded_buf[(x, 0)].symbol(), fresh_buf[(x, 0)].symbol());
        }
    }

    /// UI-R-114 — the tab names are written adjacent across the bar's one
    /// row, and an empty tab list does not panic.
    #[test]
    fn ut_render_tabs() {
        let area = Rect::new(0, 0, 9, 1);
        let mut buf = Buffer::empty(area);
        let mut offset = 0usize;
        render_tabs(
            &["alpha".to_string(), "beta".to_string()],
            1,
            area,
            &mut buf,
            &mut offset,
        );
        let rendered: String = (0..9).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert_eq!(rendered, "alphabeta");
        assert_eq!(offset, 0);

        render_tabs(&[], 0, area, &mut buf, &mut offset);
    }
}
