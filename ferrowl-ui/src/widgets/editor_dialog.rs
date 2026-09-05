use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Clear, StatefulWidget, Widget};

use crate::state::EditorDialogState;
use crate::style::InputFieldStyle;
use crate::traits::IsFocus;
use crate::widgets::{MarkdownInputField, MarkdownInputFieldBuilder};

/// A centered, bordered dialog box holding one [`MarkdownInputField`], rendered from an
/// [`EditorDialogState`]. The box takes a percentage of the frame (UI-R-200), never
/// smaller than a builder-settable minimum, though the minimum itself is not enforced
/// against a frame smaller than it (UI-E-094).
#[derive(Builder, Debug, Clone, Getters, CopyGetters, Setters, WithSetters)]
#[getset(set = "pub")]
pub struct EditorDialog {
    #[getset(get = "pub")]
    #[builder(default = "String::new()")]
    title: String,
    #[getset(get_copy = "pub")]
    #[builder(default = "60")]
    width_pct: u16,
    #[getset(get_copy = "pub")]
    #[builder(default = "50")]
    height_pct: u16,
    #[getset(get_copy = "pub")]
    #[builder(default = "40")]
    min_width: u16,
    #[getset(get_copy = "pub")]
    #[builder(default = "8")]
    min_height: u16,
    #[getset(get = "pub")]
    #[builder(default = "InputFieldStyle::default()")]
    style: InputFieldStyle,
    #[getset(get = "pub")]
    #[builder(
        default = "MarkdownInputFieldBuilder::default().build().expect(\"MarkdownInputFieldBuilder fields all default\")"
    )]
    field: MarkdownInputField,
}

impl Default for EditorDialog {
    fn default() -> Self {
        EditorDialogBuilder::default()
            .build()
            .expect("EditorDialogBuilder fields all default")
    }
}

impl EditorDialog {
    /// UI-R-200, UI-E-094 — a percentage of `area`, never below the minimum, but the
    /// minimum is never enforced against `area` itself: the trailing `.min` always wins.
    fn box_rect(&self, area: Rect) -> Rect {
        let w = ((area.width as u32 * self.width_pct as u32) / 100) as u16;
        let w = w.max(self.min_width).min(area.width);
        let h = ((area.height as u32 * self.height_pct as u32) / 100) as u16;
        let h = h.max(self.min_height).min(area.height);
        Rect {
            x: area.x + (area.width.saturating_sub(w)) / 2,
            y: area.y + (area.height.saturating_sub(h)) / 2,
            width: w,
            height: h,
        }
    }
}

impl StatefulWidget for EditorDialog {
    type State = EditorDialogState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

impl StatefulWidget for &EditorDialog {
    type State = EditorDialogState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let popup = self.box_rect(area);
        Widget::render(Clear, popup, buf);

        let border_style = if state.field().is_focused() {
            *self.style.focused()
        } else {
            *self.style.border()
        };
        let mut block = Block::bordered().style(border_style);
        block = match (self.title.is_empty(), state.field().mode_label()) {
            (false, Some(label)) => block.title(format!("{} [{}]", self.title, label)),
            (false, None) => block.title(self.title.as_str()),
            (true, Some(label)) => block.title(format!("[{label}]")),
            (true, None) => block,
        };
        let inner = block.inner(popup);
        Widget::render(block, popup, buf);

        StatefulWidget::render(&self.field, inner, buf, state.field_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect as RRect;

    fn buffer(w: u16, h: u16) -> Buffer {
        Buffer::empty(RRect::new(0, 0, w, h))
    }

    #[test]
    /// UI-R-199, UI-R-200 — on a 60x20 frame with the default 60%/50% box and 40x8
    /// minimum, the box is exactly `Rect::new(10, 5, 40, 10)` (centered), and it clears
    /// the marker char from all four of its interior corners.
    fn ut_dialog_is_centered_and_clears_the_cells_beneath_it() {
        let mut b = buffer(60, 20);
        for y in 0..20 {
            for x in 0..60 {
                b[(x, y)].set_symbol("#");
            }
        }
        let w = EditorDialog::default();
        let mut state = EditorDialogState::default();
        state.open();
        StatefulWidget::render(&w, Rect::new(0, 0, 60, 20), &mut b, &mut state);

        let popup = Rect::new(10, 5, 40, 10);
        assert_eq!(w.box_rect(Rect::new(0, 0, 60, 20)), popup);
        let corners = [
            (popup.x + 1, popup.y + 1),
            (popup.x + popup.width - 2, popup.y + 1),
            (popup.x + 1, popup.y + popup.height - 2),
            (popup.x + popup.width - 2, popup.y + popup.height - 2),
        ];
        for (x, y) in corners {
            assert_ne!(b[(x, y)].symbol(), "#", "corner ({x}, {y}) still marked");
        }
    }

    #[test]
    /// UI-R-200 — the box takes the configured percentages, and never shrinks below the
    /// minimum on a frame that has room for it.
    fn ut_box_takes_the_configured_percentages_and_never_shrinks_below_the_minimum() {
        let w = EditorDialog::default();
        let area = Rect::new(0, 0, 100, 40);
        let popup = w.box_rect(area);
        assert_eq!(popup.width, 60);
        assert_eq!(popup.height, 20);
        assert!(popup.width >= w.min_width());
        assert!(popup.height >= w.min_height());
    }

    #[test]
    /// UI-E-094 — a frame smaller than the minimum gives the box the whole frame; the
    /// minimum is not enforced against the terminal.
    fn ut_frame_smaller_than_the_minimum_gives_the_box_the_whole_frame() {
        let w = EditorDialog::default();
        let area = Rect::new(0, 0, 20, 5);
        let popup = w.box_rect(area);
        assert_eq!(popup.width, 20);
        assert_eq!(popup.height, 5);
    }

    #[test]
    /// UI-R-206 — the border title shows the caller-supplied title and the field's current
    /// mode label.
    fn ut_border_title_shows_the_title_and_the_current_mode_label() {
        let w = EditorDialogBuilder::default()
            .title("Notes".to_string())
            .build()
            .unwrap();
        let mut state = EditorDialogState::default();
        state.open();
        let mut b = buffer(60, 20);
        StatefulWidget::render(&w, Rect::new(0, 0, 60, 20), &mut b, &mut state);

        let popup = w.box_rect(Rect::new(0, 0, 60, 20));
        let title_row: String = (popup.x..popup.x + popup.width)
            .map(|x| b[(x, popup.y)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(title_row.contains("Notes"));
        assert!(title_row.contains("NORMAL"));
    }
}
