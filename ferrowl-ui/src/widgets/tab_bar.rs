use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};
use ratatui::{
    buffer::Buffer,
    layout::{Direction, Margin, Rect},
    widgets::StatefulWidget,
};
use std::marker::PhantomData;

use crate::state::TabBarState;
use crate::style::TabBarStyle;
use crate::traits::ToLabel;

/// A tab's placement, `Start`, `Center` or `End`, along a [`TabBar`]'s
/// layout direction once it has gained cells from filling the area
/// (UI-R-126).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabAlignment {
    Start,
    #[default]
    Center,
    End,
}

/// A tab bar that lays its tabs out one after another along its layout
/// direction, writing each title one character per cell in that direction
/// and scrolling to keep the active tab's block of cells visible. An
/// optional padding of a horizontal count H and a vertical count V frames
/// every tab, making the widget's rendered extent across the layout
/// direction `1 + 2c` cells, where `c` is H under `Vertical` and V under
/// `Horizontal`. When the tabs' natural extent along the layout direction
/// is less than the area, the spare cells stretch the tabs to fill it and
/// `alignment` places each tab's character and padding cells at the start,
/// middle or end of the resulting extent.
///
/// Style lives in [`TabBarStyle`]; tab data and the active index live in
/// [`TabBarState`], which the widget only ever reads, aside from the
/// scroll offset it maintains itself.
#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct TabBar<T: ToLabel + Clone> {
    #[getset(get = "pub")]
    #[builder(default = "TabBarStyle::default()")]
    style: TabBarStyle,
    #[getset(get_copy = "pub")]
    #[builder(default = "Margin::new(0, 0)")]
    padding: Margin,
    #[getset(get_copy = "pub")]
    #[builder(default = "TabAlignment::Center")]
    alignment: TabAlignment,
    #[getset(get_copy = "pub")]
    #[builder(default = "Direction::Horizontal")]
    direction: Direction,
    #[builder(setter(skip))]
    #[builder(default = "PhantomData")]
    marker: PhantomData<T>,
}

impl<T: ToLabel + Clone> TabBar<T> {
    /// The padding count running along the layout direction: `padding.vertical`
    /// under `Vertical`, `padding.horizontal` under `Horizontal` (UI-R-120).
    fn along_padding(&self) -> usize {
        match self.direction {
            Direction::Vertical => self.padding.vertical as usize,
            Direction::Horizontal => self.padding.horizontal as usize,
        }
    }

    /// The padding count running across the layout direction: `padding.horizontal`
    /// under `Vertical`, `padding.vertical` under `Horizontal` (UI-R-121).
    fn across_padding(&self) -> usize {
        match self.direction {
            Direction::Vertical => self.padding.horizontal as usize,
            Direction::Horizontal => self.padding.vertical as usize,
        }
    }

    /// The widget's rendered extent across the layout direction, `1 + 2c`
    /// cells for the across-direction padding count `c` (UI-R-121).
    pub fn rendered_extent(&self) -> u16 {
        1 + 2 * self.across_padding() as u16
    }
}

impl<T: ToLabel + Clone> StatefulWidget for TabBar<T> {
    type State = TabBarState<T>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

/// Resolves slot `slot` (counted across the whole stacked tab list, in cells
/// along the layout direction) to the owning tab's index and, if the slot is
/// a character cell rather than an along-direction padding or gained cell,
/// the character it carries. `labels` holds each tab's title, pre-split into
/// characters so a slot lookup never re-renders a label; `heights` holds
/// each tab's rendered extent along the direction, natural or stretched;
/// `leads` holds each tab's gained cells placed before its padding, per its
/// alignment.
fn resolve_slot(
    labels: &[Vec<char>],
    heights: &[usize],
    leads: &[usize],
    along_padding: usize,
    slot: usize,
) -> Option<(usize, Option<char>)> {
    let mut start = 0usize;
    for (idx, label) in labels.iter().enumerate() {
        let chars_count = label.len();
        let height = heights[idx];
        if slot < start + height {
            let local = slot - start;
            let lead = leads[idx];
            if local < lead + along_padding || local >= lead + along_padding + chars_count {
                return Some((idx, None));
            }
            return Some((idx, label.get(local - lead - along_padding).copied()));
        }
        start += height;
    }
    None
}

impl<T: ToLabel + Clone> StatefulWidget for &TabBar<T> {
    type State = TabBarState<T>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        if state.titles.is_empty() {
            state.offset = 0;
            return;
        }
        let active = state.selected_index();

        let along_padding = self.along_padding();
        let across_padding = self.across_padding();
        let h = match self.direction {
            Direction::Vertical => area.height as usize,
            Direction::Horizontal => area.width as usize,
        };

        let labels: Vec<Vec<char>> = state
            .titles
            .iter()
            .map(|t| t.to_label().chars().collect())
            .collect();
        let natural: Vec<usize> = labels
            .iter()
            .map(|label| 2 * along_padding + label.len())
            .collect();
        let total: usize = natural.iter().sum();
        let heights: Vec<usize> = if total < h {
            let s = h - total;
            let n = natural.len();
            natural
                .iter()
                .enumerate()
                .map(|(i, nat)| nat + s / n + usize::from(i < s % n))
                .collect()
        } else {
            natural.clone()
        };
        let leads: Vec<usize> = heights
            .iter()
            .zip(natural.iter())
            .map(|(height, nat)| {
                let g = height - nat;
                match self.alignment {
                    TabAlignment::Start => 0,
                    TabAlignment::Center => g / 2,
                    TabAlignment::End => g,
                }
            })
            .collect();

        if total <= h {
            state.offset = 0;
        }

        let mut start = 0usize;
        let mut active_block = (0, heights[0]);
        for (idx, height) in heights.iter().enumerate() {
            if idx == active {
                active_block = (start, *height);
            }
            start += height;
        }
        let total_slots = start;

        let mut end = (state.offset + h).min(total_slots);
        if total > h {
            let (block_start, block_height) = active_block;
            let leftover = h.saturating_sub(block_height);
            let near_ideal = leftover / 2;
            let far_ideal = leftover - near_ideal;
            let far_available = total_slots - block_start - block_height;
            let near = near_ideal.min(block_start);
            let far = far_ideal.min(far_available);
            state.offset = block_start - near;
            end = (block_start + block_height + far).min(state.offset + h);
        }
        for slot in state.offset..end {
            let (tab_idx, ch) = resolve_slot(&labels, &heights, &leads, along_padding, slot)
                .expect("slot is bounded by total_slots, the same sum resolve_slot walks");
            let a = match self.direction {
                Direction::Vertical => area.y,
                Direction::Horizontal => area.x,
            } + (slot - state.offset) as u16;
            let style = if tab_idx == active {
                self.style.selected
            } else {
                self.style.general
            };
            for across_index in 0..=(2 * across_padding) {
                let b = match self.direction {
                    Direction::Vertical => area.x,
                    Direction::Horizontal => area.y,
                } + across_index as u16;
                let (x, y) = match self.direction {
                    Direction::Vertical => (b, a),
                    Direction::Horizontal => (a, b),
                };
                let clipped = match self.direction {
                    Direction::Vertical => x >= area.x + area.width,
                    Direction::Horizontal => y >= area.y + area.height,
                };
                if clipped {
                    break;
                }
                let sym = if across_index == across_padding {
                    ch.map_or_else(|| " ".to_string(), String::from)
                } else {
                    " ".to_string()
                };
                buf[(x, y)].set_symbol(&sym).set_style(style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(w: u16, h: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, w, h))
    }

    fn titles(labels: &[&str]) -> Vec<String> {
        labels.iter().map(|s| s.to_string()).collect()
    }

    fn cell_has_style(cell: &ratatui::buffer::Cell, style: ratatui::style::Style) -> bool {
        cell.fg == style.fg.unwrap_or(ratatui::style::Color::Reset)
            && cell.bg == style.bg.unwrap_or(ratatui::style::Color::Reset)
            && cell.modifier == style.add_modifier
    }

    /// UI-R-120, UI-R-121 — horizontal padding alone widens the render.
    #[test]
    fn ut_horizontal_padding_widens_render() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(2, 0))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(7, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 7, 3), &mut b, &mut st);
        assert_eq!(b[(2, 0)].symbol(), "T");
        assert_eq!(b[(2, 1)].symbol(), "a");
        assert_eq!(b[(2, 2)].symbol(), "b");
        let style = TabBarStyle::default();
        for y in 0..3 {
            for x in [0usize, 1, 3, 4] {
                assert_eq!(b[(x as u16, y)].symbol(), " ");
                assert!(cell_has_style(&b[(x as u16, y)], style.selected));
            }
        }
        for y in 0..3 {
            for x in 5..7 {
                assert_eq!(b[(x, y)].symbol(), " ");
                assert!(cell_has_style(&b[(x, y)], ratatui::style::Style::default()));
            }
        }
    }

    /// UI-R-120 — vertical padding alone frames the title with blank rows,
    /// without changing the rendered width.
    #[test]
    fn ut_vertical_padding_frames_rows() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(0, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 5), &mut b, &mut st);
        assert_eq!(b[(0, 0)].symbol(), " ");
        assert_eq!(b[(0, 1)].symbol(), "T");
        assert_eq!(b[(0, 2)].symbol(), "a");
        assert_eq!(b[(0, 3)].symbol(), "b");
        assert_eq!(b[(0, 4)].symbol(), " ");
    }

    /// UI-R-114, UI-R-115 — a title is written one character per row, top-down.
    #[test]
    fn ut_renders_title_one_char_per_row() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab", "Two"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 6);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 6), &mut b, &mut st);
        assert_eq!(b[(0, 0)].symbol(), "T");
        assert_eq!(b[(0, 1)].symbol(), "a");
        assert_eq!(b[(0, 2)].symbol(), "b");
        assert_eq!(b[(0, 3)].symbol(), "T");
        assert_eq!(b[(0, 4)].symbol(), "w");
        assert_eq!(b[(0, 5)].symbol(), "o");
    }

    /// UI-R-116, UI-R-121 — every cell of the active tab's block takes the
    /// selected style, including its padding rows and columns.
    #[test]
    fn ut_active_block_cells_use_selected_style() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(1, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab", "Two"]),
            active: 1,
            offset: 0,
        };
        let mut b = buffer(3, 10);
        StatefulWidget::render(&w, Rect::new(0, 0, 3, 10), &mut b, &mut st);
        let style = TabBarStyle::default();
        for y in 0..5 {
            for x in 0..3 {
                assert!(cell_has_style(&b[(x, y)], style.general));
            }
        }
        for y in 5..10 {
            for x in 0..3 {
                assert!(cell_has_style(&b[(x, y)], style.selected));
            }
        }
    }

    /// UI-R-120, UI-R-121 — default padding is zero, rendering exactly one column.
    #[test]
    fn ut_default_padding_is_zero() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(3, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 3, 3), &mut b, &mut st);
        assert_eq!(b[(0, 0)].symbol(), "T");
        assert_eq!(b[(0, 1)].symbol(), "a");
        assert_eq!(b[(0, 2)].symbol(), "b");
        for y in 0..3 {
            assert_eq!(b[(1, y)].symbol(), " ");
            assert_eq!(b[(2, y)].symbol(), " ");
            assert!(cell_has_style(&b[(1, y)], ratatui::style::Style::default()));
            assert!(cell_has_style(&b[(2, y)], ratatui::style::Style::default()));
        }
    }

    /// UI-R-120, UI-R-121 — H and V together frame the title with blank rows and columns.
    #[test]
    fn ut_padding_frames_each_title() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(1, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(3, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 3, 5), &mut b, &mut st);
        let row = |y: u16| {
            format!(
                "{}{}{}",
                b[(0, y)].symbol(),
                b[(1, y)].symbol(),
                b[(2, y)].symbol()
            )
        };
        assert_eq!(row(0), "   ");
        assert_eq!(row(1), " T ");
        assert_eq!(row(2), " a ");
        assert_eq!(row(3), " b ");
        assert_eq!(row(4), "   ");
    }

    /// UI-R-114, UI-R-115, UI-R-117 — fewer rows than the tabs' total height
    /// scrolls to keep the active tab's block visible.
    #[test]
    fn ut_scrolls_to_keep_active_visible() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab", "Two"]),
            active: 1,
            offset: 0,
        };
        let mut b = buffer(1, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 3), &mut b, &mut st);
        assert_eq!(st.offset, 3);
        assert_eq!(b[(0, 0)].symbol(), "T");
        assert_eq!(b[(0, 1)].symbol(), "w");
        assert_eq!(b[(0, 2)].symbol(), "o");
    }

    /// UI-R-114, UI-R-174, UI-R-117 — fewer columns than the tabs' total
    /// width scrolls to keep the active tab's block visible under
    /// `Horizontal` layout too.
    #[test]
    fn ut_horizontal_scrolls_to_keep_active_visible() {
        let w = TabBarBuilder::<String>::default().build().unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab", "Two"]),
            active: 1,
            offset: 0,
        };
        let mut b = buffer(3, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 3, 1), &mut b, &mut st);
        assert_eq!(st.offset, 3);
        assert_eq!(b[(0, 0)].symbol(), "T");
        assert_eq!(b[(1, 0)].symbol(), "w");
        assert_eq!(b[(2, 0)].symbol(), "o");
    }

    /// UI-R-118 — the active block is centred: with leftover `l` cells the
    /// block sits `l / 2` cells from the near edge, in both directions.
    #[test]
    fn ut_scroll_centres_the_active_block() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A", "B", "C", "D", "E"]),
            active: 2,
            offset: 0,
        };
        let mut b = buffer(1, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 3), &mut b, &mut st);
        assert_eq!(st.offset, 1);
        assert_eq!(b[(0, 0)].symbol(), "B");
        assert_eq!(b[(0, 1)].symbol(), "C");
        assert_eq!(b[(0, 2)].symbol(), "D");

        let w = TabBarBuilder::<String>::default().build().unwrap();
        let mut st = TabBarState {
            titles: titles(&["A", "B", "C", "D", "E"]),
            active: 2,
            offset: 0,
        };
        let mut b = buffer(3, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 3, 1), &mut b, &mut st);
        assert_eq!(st.offset, 1);
        assert_eq!(b[(0, 0)].symbol(), "B");
        assert_eq!(b[(1, 0)].symbol(), "C");
        assert_eq!(b[(2, 0)].symbol(), "D");
    }

    /// UI-R-118 — an active block flush against the start of the list gets
    /// no cells before it, no underflow, even though the ideal half of the
    /// leftover would ask for more; the cells the near side could not use
    /// are never handed to the far side, so the window stays short and the
    /// far edge of the area is left blank rather than showing an extra cell.
    #[test]
    fn ut_scroll_does_not_roll_leftover_over_to_the_far_side() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A", "B", "C", "D", "E"]),
            active: 0,
            offset: 5,
        };
        let mut b = buffer(1, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 3), &mut b, &mut st);
        assert_eq!(st.offset, 0);
        assert_eq!(b[(0, 0)].symbol(), "A");
        assert_eq!(b[(0, 1)].symbol(), "B");
        assert_eq!(b[(0, 2)].symbol(), " ");
        assert!(cell_has_style(&b[(0, 2)], ratatui::style::Style::default()));
    }

    /// UI-R-118, UI-R-117 — an active block near the far end still opens its
    /// window `leftover / 2` cells before it; when that runs the window past
    /// the last tab the trailing area cells stay untouched rather than the
    /// window being pulled back to consume them.
    #[test]
    fn ut_scroll_at_the_far_end_leaves_trailing_cells_blank() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A", "B", "C", "D", "E"]),
            active: 3,
            offset: 0,
        };
        let mut b = buffer(1, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 4), &mut b, &mut st);
        assert_eq!(st.offset, 2);
        assert_eq!(b[(0, 0)].symbol(), "C");
        assert_eq!(b[(0, 1)].symbol(), "D");
        assert_eq!(b[(0, 2)].symbol(), "E");
        assert_eq!(b[(0, 3)].symbol(), " ");
        assert!(cell_has_style(&b[(0, 3)], ratatui::style::Style::default()));
    }

    /// UI-R-117, UI-R-122 — tabs that already fit the area are stretched to
    /// cover it and never scroll, clearing any retained offset.
    #[test]
    fn ut_no_scroll_while_the_tabs_fit() {
        let w = TabBarBuilder::<String>::default().build().unwrap();
        let mut st = TabBarState {
            titles: titles(&["Abc", "Def"]),
            active: 1,
            offset: 99,
        };
        let mut b = buffer(10, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 10, 1), &mut b, &mut st);
        assert_eq!(st.offset, 0);
        let style = TabBarStyle::default();
        for x in 0..10 {
            assert!(
                cell_has_style(&b[(x, 0)], style.general)
                    || cell_has_style(&b[(x, 0)], style.selected)
            );
        }
    }

    /// UI-R-175 — the stored offset is an output of the render, overwritten
    /// every time regardless of what the caller set it to beforehand.
    #[test]
    fn ut_offset_is_recomputed_every_render() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A", "B", "C", "D", "E"]),
            active: 2,
            offset: 99,
        };
        let mut b = buffer(1, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 3), &mut b, &mut st);
        assert_eq!(st.offset, 1);
    }

    /// UI-R-119 — the render writes no field but the offset.
    #[test]
    fn ut_render_leaves_titles_and_active_untouched() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let input = titles(&["alpha", "beta", "gamma", "delta", "epsilon"]);
        let mut st = TabBarState {
            titles: input.clone(),
            active: 4,
            offset: 0,
        };
        let mut b = buffer(1, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 2), &mut b, &mut st);
        assert_eq!(st.titles, input);
        assert_eq!(st.active, 4);
        assert_eq!(st.offset, 19);
    }

    /// UI-E-063, UI-R-175 — zero-sized area skips drawing, offset unchanged.
    #[test]
    fn ut_zero_sized_area_skips_drawing() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["alpha", "beta", "gamma"]),
            active: 0,
            offset: 2,
        };
        let mut b = buffer(3, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 0, 3), &mut b, &mut st);
        assert_eq!(st.offset, 2);
        assert_eq!(b[(0, 0)].symbol(), " ");
        assert!(cell_has_style(&b[(0, 0)], ratatui::style::Style::default()));

        let mut b = buffer(3, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 3, 0), &mut b, &mut st);
        assert_eq!(st.offset, 2);
        assert_eq!(b[(0, 0)].symbol(), " ");
        assert!(cell_has_style(&b[(0, 0)], ratatui::style::Style::default()));
    }

    /// UI-E-064 — an area wider than the rendered width draws into the
    /// leftmost `1 + 2H` columns only.
    #[test]
    fn ut_wider_area_uses_rendered_width_only() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(1, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(6, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 6, 5), &mut b, &mut st);
        assert_eq!(b[(1, 1)].symbol(), "T");
        assert_eq!(b[(1, 2)].symbol(), "a");
        assert_eq!(b[(1, 3)].symbol(), "b");
        for x in 3..6 {
            for y in 0..5 {
                assert_eq!(b[(x, y)].symbol(), " ");
                assert!(cell_has_style(&b[(x, y)], ratatui::style::Style::default()));
            }
        }
    }

    /// UI-E-069 — an area narrower than the rendered width clips at the
    /// right edge, no reflow, no panic.
    #[test]
    fn ut_narrower_area_clips_rendered_columns() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(1, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(2, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 2, 5), &mut b, &mut st);
        assert_eq!(b[(1, 1)].symbol(), "T");
        assert_eq!(b[(1, 2)].symbol(), "a");
        assert_eq!(b[(1, 3)].symbol(), "b");
    }

    /// UI-E-065 — empty tab list draws nothing and resets the offset.
    #[test]
    fn ut_empty_titles_reset_offset() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: vec![],
            active: 0,
            offset: 3,
        };
        let mut b = buffer(1, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 3), &mut b, &mut st);
        assert_eq!(st.offset, 0);
    }

    /// UI-E-066, UI-R-325, UI-R-116 — an active index out of range reads as
    /// `0`: the first tab takes the active style and the offset resets to
    /// `0` rather than keeping the stored value.
    #[test]
    fn ut_active_out_of_range_renders_as_first_tab() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(1, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["alpha", "beta", "gamma"]),
            active: 9,
            offset: 1,
        };
        let mut b = buffer(3, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 3, 3), &mut b, &mut st);
        assert_eq!(st.offset, 0);
        let style = TabBarStyle::default();
        for y in 0..3 {
            for x in 0..3 {
                assert!(cell_has_style(&b[(x, y)], style.selected));
            }
        }
    }

    /// UI-R-120, UI-E-067 — an empty title occupies only its padding rows.
    #[test]
    fn ut_empty_title_occupies_padding_only() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(1, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["", "Two"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(3, 7);
        StatefulWidget::render(&w, Rect::new(0, 0, 3, 7), &mut b, &mut st);
        let style = TabBarStyle::default();
        assert!(cell_has_style(&b[(1, 0)], style.selected));
        assert!(cell_has_style(&b[(1, 1)], style.selected));
        assert_eq!(b[(1, 3)].symbol(), "T");

        let w0 = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st0 = TabBarState {
            titles: titles(&["", "Two"]),
            active: 0,
            offset: 0,
        };
        let mut b0 = buffer(1, 3);
        StatefulWidget::render(&w0, Rect::new(0, 0, 1, 3), &mut b0, &mut st0);
        assert_eq!(b0[(0, 0)].symbol(), "T");
    }

    /// UI-E-068 — double-width title characters written as-is, no fallback glyph.
    #[test]
    fn ut_wide_title_char_is_written_as_is() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["日本"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 2), &mut b, &mut st);
        assert_eq!(b[(0, 0)].symbol(), "日");
        assert_eq!(b[(0, 1)].symbol(), "本");
    }

    /// UI-E-070 — an active block longer than the area places its first
    /// cell at the near edge; the rest of the block is clipped. Holds in
    /// both layout directions.
    #[test]
    fn ut_block_longer_than_area_starts_at_near_edge() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(1, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Ab", "Longtitle"]),
            active: 1,
            offset: 0,
        };
        let mut b = buffer(3, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 3, 3), &mut b, &mut st);
        assert_eq!(st.offset, 4);
        assert_eq!(b[(1, 0)].symbol(), " ");
        assert_eq!(b[(1, 1)].symbol(), "L");
        assert_eq!(b[(1, 2)].symbol(), "o");

        let w = TabBarBuilder::<String>::default()
            .padding(Margin::new(1, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Ab", "Longtitle"]),
            active: 1,
            offset: 0,
        };
        let mut b = buffer(3, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 3, 3), &mut b, &mut st);
        assert_eq!(st.offset, 4);
        assert_eq!(b[(0, 1)].symbol(), " ");
        assert_eq!(b[(1, 1)].symbol(), "L");
        assert_eq!(b[(2, 1)].symbol(), "o");
    }

    /// UI-R-122 — spare rows below the tabs' natural height are divided
    /// among the tabs so they together cover the whole area.
    #[test]
    fn ut_spare_rows_stretch_tabs_to_cover_area() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A", "B"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 6);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 6), &mut b, &mut st);
        let style = TabBarStyle::default();
        for y in 0..6 {
            assert!(
                cell_has_style(&b[(0, y)], style.general)
                    || cell_has_style(&b[(0, y)], style.selected)
            );
        }
    }

    /// UI-R-123, UI-R-116 — spare rows are divided evenly, remainder to the
    /// topmost tabs, and a gained row of the active tab is styled selected.
    #[test]
    fn ut_spare_rows_divided_evenly_remainder_to_top() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A", "B", "C"]),
            active: 1,
            offset: 0,
        };
        let mut b = buffer(1, 8);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 8), &mut b, &mut st);
        let style = TabBarStyle::default();
        for y in 0..3 {
            assert!(cell_has_style(&b[(0, y)], style.general));
        }
        for y in 3..6 {
            assert!(cell_has_style(&b[(0, y)], style.selected));
        }
        for y in 6..8 {
            assert!(cell_has_style(&b[(0, y)], style.general));
        }
    }

    /// UI-R-124, UI-R-126, UI-R-116 — under `Start` alignment gained rows sit
    /// outside the padding, below it, and the whole extent stays selected.
    #[test]
    fn ut_gained_rows_sit_outside_the_padding() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(0, 1))
            .alignment(TabAlignment::Start)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 5), &mut b, &mut st);
        let style = TabBarStyle::default();
        assert_eq!(b[(0, 0)].symbol(), " ");
        assert_eq!(b[(0, 1)].symbol(), "A");
        assert_eq!(b[(0, 2)].symbol(), " ");
        assert_eq!(b[(0, 3)].symbol(), " ");
        assert_eq!(b[(0, 4)].symbol(), " ");
        for y in 0..5 {
            assert!(cell_has_style(&b[(0, y)], style.selected));
        }
    }

    /// UI-R-125, UI-R-118, UI-R-117 — no tab stretches once the natural
    /// height already fills or exceeds the area; the scroll rules govern
    /// instead, and an exact fit clears any retained offset without
    /// scrolling.
    #[test]
    fn ut_no_stretch_when_natural_height_fills_area() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();

        let mut st = TabBarState {
            titles: titles(&["Ab", "Cd"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 4), &mut b, &mut st);
        assert_eq!(b[(0, 0)].symbol(), "A");
        assert_eq!(b[(0, 1)].symbol(), "b");
        assert_eq!(b[(0, 2)].symbol(), "C");
        assert_eq!(b[(0, 3)].symbol(), "d");

        let mut st = TabBarState {
            titles: titles(&["Ab", "Cd"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 3), &mut b, &mut st);
        assert_eq!(b[(0, 0)].symbol(), "A");
        assert_eq!(b[(0, 1)].symbol(), "b");
        assert_eq!(b[(0, 2)].symbol(), "C");

        let mut st = TabBarState {
            titles: titles(&["Abc", "Def"]),
            active: 1,
            offset: 3,
        };
        let mut b = buffer(1, 6);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 6), &mut b, &mut st);
        // Natural height exactly fills the area: no stretch, but also no
        // overflow, so the retained offset is cleared and the whole list is
        // visible rather than a stale window leaving rows blank.
        assert_eq!(st.offset, 0);
        assert_eq!(b[(0, 0)].symbol(), "A");
        assert_eq!(b[(0, 1)].symbol(), "b");
        assert_eq!(b[(0, 2)].symbol(), "c");
        assert_eq!(b[(0, 3)].symbol(), "D");
        assert_eq!(b[(0, 4)].symbol(), "e");
        assert_eq!(b[(0, 5)].symbol(), "f");
    }

    /// UI-E-071, UI-R-116 — a single tab with spare height takes every
    /// gained row and covers the whole area.
    #[test]
    fn ut_single_tab_covers_whole_area() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 5), &mut b, &mut st);
        let style = TabBarStyle::default();
        for y in 0..5 {
            assert!(cell_has_style(&b[(0, y)], style.selected));
        }
    }

    /// UI-E-072, UI-R-123 — with fewer spare rows than tabs, the topmost
    /// tabs gain one row each and the area is still covered.
    #[test]
    fn ut_fewer_spare_rows_than_tabs() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A", "B", "C"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 5), &mut b, &mut st);
        let style = TabBarStyle::default();
        for y in 0..2 {
            assert!(cell_has_style(&b[(0, y)], style.selected));
        }
        for y in 2..5 {
            assert!(cell_has_style(&b[(0, y)], style.general));
        }
    }

    /// UI-E-066, UI-R-325 — an out-of-range active index reads as `0`: under the
    /// stretch path the offset resets to `0` and the first tab's cells carry
    /// the active style.
    #[test]
    fn ut_out_of_range_active_stretches_onto_first_tab() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Abc", "Def"]),
            active: 9,
            offset: 2,
        };
        let mut b = buffer(1, 10);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 10), &mut b, &mut st);
        assert_eq!(st.offset, 0);
        let style = TabBarStyle::default();
        for y in 0..5 {
            assert!(cell_has_style(&b[(0, y)], style.selected));
        }
        for y in 5..10 {
            assert!(cell_has_style(&b[(0, y)], style.general));
        }
    }

    /// UI-E-085, UI-R-175 — a stale offset past the last occupied cell, with
    /// an out-of-range active index, resets to `0` and the first tab takes
    /// the active style (UI-E-066), not an unstyled render.
    #[test]
    fn ut_stale_offset_past_end_is_clamped_to_show_tabs() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Abc"]),
            active: 9,
            offset: 10,
        };
        let mut b = buffer(1, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 3), &mut b, &mut st);
        assert_eq!(st.offset, 0);
        let rendered: String = (0..3).map(|y| b[(0, y)].symbol().to_string()).collect();
        assert_eq!(rendered, "Abc");
        let style = TabBarStyle::default();
        for y in 0..3 {
            assert!(cell_has_style(&b[(0, y)], style.selected));
        }
    }

    /// UI-E-073 — an empty title with zero vertical padding still takes
    /// part in the spare-row division and so becomes visible.
    #[test]
    fn ut_zero_height_tab_becomes_visible_via_stretch() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["", "A"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 4), &mut b, &mut st);
        let style = TabBarStyle::default();
        assert!(cell_has_style(&b[(0, 0)], style.selected));
        assert!(cell_has_style(&b[(0, 1)], style.selected));
    }

    /// UI-R-117, UI-R-122 — a scroll offset retained from a smaller area is
    /// cleared once a later render finds spare height to stretch into.
    #[test]
    fn ut_stretch_clears_a_retained_offset() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Abc", "Def"]),
            active: 1,
            offset: 0,
        };
        let mut small = buffer(1, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 2), &mut small, &mut st);
        assert_eq!(st.offset, 3);

        let mut big = buffer(1, 10);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 10), &mut big, &mut st);
        assert_eq!(st.offset, 0);
        let style = TabBarStyle::default();
        for y in 0..10 {
            assert!(
                cell_has_style(&big[(0, y)], style.general)
                    || cell_has_style(&big[(0, y)], style.selected)
            );
        }
    }

    /// UI-R-126, UI-R-127, UI-R-124 — `Center` splits the gained rows with
    /// the smaller half above the top padding row.
    #[test]
    fn ut_alignment_center_splits_the_gained_rows() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(0, 1))
            .alignment(TabAlignment::Center)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 5), &mut b, &mut st);
        assert_eq!(b[(0, 0)].symbol(), " ");
        assert_eq!(b[(0, 1)].symbol(), " ");
        assert_eq!(b[(0, 2)].symbol(), "A");
        assert_eq!(b[(0, 3)].symbol(), " ");
        assert_eq!(b[(0, 4)].symbol(), " ");
    }

    /// UI-R-126, UI-R-124 — `End` puts every gained row above the
    /// padding, outside it.
    #[test]
    fn ut_alignment_end_puts_gained_rows_first() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(0, 1))
            .alignment(TabAlignment::End)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 5), &mut b, &mut st);
        assert_eq!(b[(0, 0)].symbol(), " ");
        assert_eq!(b[(0, 1)].symbol(), " ");
        assert_eq!(b[(0, 2)].symbol(), " ");
        assert_eq!(b[(0, 3)].symbol(), "A");
        assert_eq!(b[(0, 4)].symbol(), " ");
    }

    /// UI-R-126 — with no `.alignment` call the builder defaults to `Center`.
    #[test]
    fn ut_alignment_defaults_to_center() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(0, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 5), &mut b, &mut st);
        assert_eq!(b[(0, 0)].symbol(), " ");
        assert_eq!(b[(0, 1)].symbol(), " ");
        assert_eq!(b[(0, 2)].symbol(), "A");
        assert_eq!(b[(0, 3)].symbol(), " ");
        assert_eq!(b[(0, 4)].symbol(), " ");
    }

    /// UI-R-127 — an odd `g` splits with the smaller half above.
    #[test]
    fn ut_center_puts_the_smaller_half_above() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .alignment(TabAlignment::Center)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 4), &mut b, &mut st);
        assert_eq!(b[(0, 0)].symbol(), " ");
        assert_eq!(b[(0, 1)].symbol(), "A");
        assert_eq!(b[(0, 2)].symbol(), " ");
        assert_eq!(b[(0, 3)].symbol(), " ");
    }

    /// UI-E-074, UI-R-127 — gaining exactly one row under `Center` puts it
    /// below the bottom padding row; the title sits one row above centre.
    #[test]
    fn ut_center_single_gained_row_goes_below() {
        let w = TabBarBuilder::<String>::default()
            .direction(Direction::Vertical)
            .padding(Margin::new(0, 1))
            .alignment(TabAlignment::Center)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(1, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 1, 4), &mut b, &mut st);
        assert_eq!(b[(0, 0)].symbol(), " ");
        assert_eq!(b[(0, 1)].symbol(), "A");
        assert_eq!(b[(0, 2)].symbol(), " ");
        assert_eq!(b[(0, 3)].symbol(), " ");
    }

    /// UI-R-128, UI-R-125 — alignment changes nothing once the natural
    /// height already fills or exceeds the area.
    #[test]
    fn ut_alignment_is_inert_without_stretching() {
        for alignment in [TabAlignment::Start, TabAlignment::Center, TabAlignment::End] {
            let w = TabBarBuilder::<String>::default()
                .direction(Direction::Vertical)
                .alignment(alignment)
                .build()
                .unwrap();

            let mut st = TabBarState {
                titles: titles(&["Ab", "Cd"]),
                active: 0,
                offset: 0,
            };
            let mut b = buffer(1, 4);
            StatefulWidget::render(&w, Rect::new(0, 0, 1, 4), &mut b, &mut st);
            assert_eq!(b[(0, 0)].symbol(), "A");
            assert_eq!(b[(0, 1)].symbol(), "b");
            assert_eq!(b[(0, 2)].symbol(), "C");
            assert_eq!(b[(0, 3)].symbol(), "d");

            let mut st = TabBarState {
                titles: titles(&["Ab", "Cd"]),
                active: 0,
                offset: 0,
            };
            let mut b = buffer(1, 3);
            StatefulWidget::render(&w, Rect::new(0, 0, 1, 3), &mut b, &mut st);
            assert_eq!(b[(0, 0)].symbol(), "A");
            assert_eq!(b[(0, 1)].symbol(), "b");
            assert_eq!(b[(0, 2)].symbol(), "C");
        }
    }

    /// UI-E-075, UI-R-123 — a tab that gains no rows renders alike under
    /// every alignment; only the tabs that gain rows move.
    #[test]
    fn ut_unstretched_tab_renders_alike_under_every_alignment() {
        for (alignment, first, second) in [
            (TabAlignment::Start, 0u16, 2u16),
            (TabAlignment::Center, 0, 2),
            (TabAlignment::End, 1, 3),
        ] {
            let w = TabBarBuilder::<String>::default()
                .direction(Direction::Vertical)
                .alignment(alignment)
                .build()
                .unwrap();
            let mut st = TabBarState {
                titles: titles(&["A", "B", "C"]),
                active: 0,
                offset: 0,
            };
            let mut b = buffer(1, 5);
            StatefulWidget::render(&w, Rect::new(0, 0, 1, 5), &mut b, &mut st);
            assert_eq!(b[(0, first)].symbol(), "A");
            assert_eq!(b[(0, second)].symbol(), "B");
            assert_eq!(b[(0, 4)].symbol(), "C");
        }
    }

    /// UI-R-116 — moving a title inside its extent moves no styling: the
    /// active tab's whole extent stays selected under every alignment.
    #[test]
    fn ut_active_extent_is_styled_under_every_alignment() {
        let style = TabBarStyle::default();
        for alignment in [TabAlignment::Start, TabAlignment::Center, TabAlignment::End] {
            let w = TabBarBuilder::<String>::default()
                .direction(Direction::Vertical)
                .padding(Margin::new(0, 1))
                .alignment(alignment)
                .build()
                .unwrap();
            let mut st = TabBarState {
                titles: titles(&["A"]),
                active: 0,
                offset: 0,
            };
            let mut b = buffer(1, 5);
            StatefulWidget::render(&w, Rect::new(0, 0, 1, 5), &mut b, &mut st);
            for y in 0..5 {
                assert!(cell_has_style(&b[(0, y)], style.selected));
            }
        }
    }

    fn row(b: &Buffer, y: u16, w: u16) -> String {
        (0..w).map(|x| b[(x, y)].symbol().to_string()).collect()
    }

    /// UI-R-173 — with no `.direction` call the builder defaults to
    /// `Horizontal`, laying tabs left to right on the first row.
    #[test]
    fn ut_direction_defaults_to_horizontal() {
        let w = TabBarBuilder::<String>::default().build().unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab", "Two"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(6, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 6, 1), &mut b, &mut st);
        assert_eq!(row(&b, 0, 6), "TabTwo");
    }

    /// UI-R-114, UI-R-174 — titles are written adjacent, one character per
    /// column, with no separator cell between them.
    #[test]
    fn ut_horizontal_writes_one_char_per_column() {
        let w = TabBarBuilder::<String>::default().build().unwrap();
        let mut st = TabBarState {
            titles: titles(&["A", "B"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(2, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 2, 1), &mut b, &mut st);
        assert_eq!(row(&b, 0, 2), "AB");
    }

    /// UI-R-120, UI-R-121 — H=1, V=1 frames the title with blank rows and
    /// columns, the literal example from UI-R-121.
    #[test]
    fn ut_horizontal_padding_frames_title() {
        let w = TabBarBuilder::<String>::default()
            .padding(Margin::new(1, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(5, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 5, 3), &mut b, &mut st);
        assert_eq!(row(&b, 0, 5), "     ");
        assert_eq!(row(&b, 1, 5), " Tab ");
        assert_eq!(row(&b, 2, 5), "     ");
    }

    /// UI-R-121 — `rendered_extent` is `1 + 2c` for the across-direction
    /// padding count `c`: V under `Horizontal`, H under `Vertical`.
    #[test]
    fn ut_rendered_extent_matches_padding() {
        let default = TabBarBuilder::<String>::default().build().unwrap();
        assert_eq!(default.rendered_extent(), 1);

        let horizontal = TabBarBuilder::<String>::default()
            .padding(Margin::new(2, 3))
            .build()
            .unwrap();
        assert_eq!(horizontal.rendered_extent(), 1 + 2 * 3);

        let vertical = TabBarBuilder::<String>::default()
            .padding(Margin::new(2, 3))
            .direction(Direction::Vertical)
            .build()
            .unwrap();
        assert_eq!(vertical.rendered_extent(), 1 + 2 * 2);
    }

    /// UI-R-116 — every cell of the active tab, its padding cells included,
    /// carries the selected style under `Horizontal` layout too.
    #[test]
    fn ut_horizontal_active_tab_cells_use_selected_style() {
        let w = TabBarBuilder::<String>::default()
            .padding(Margin::new(1, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab", "Two"]),
            active: 1,
            offset: 0,
        };
        let mut b = buffer(10, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 10, 3), &mut b, &mut st);
        let style = TabBarStyle::default();
        for y in 0..3 {
            for x in 0..5 {
                assert!(cell_has_style(&b[(x, y)], style.general));
            }
            for x in 5..10 {
                assert!(cell_has_style(&b[(x, y)], style.selected));
            }
        }
    }

    /// UI-R-122, UI-R-123 — spare columns stretch the tabs so together they
    /// cover every column of the area, the remainder going to the first
    /// tabs from the start of the layout direction.
    #[test]
    fn ut_horizontal_spare_columns_stretch_tabs_to_cover_area() {
        let w = TabBarBuilder::<String>::default().build().unwrap();
        let mut st = TabBarState {
            titles: titles(&["A", "B", "C"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(8, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 8, 1), &mut b, &mut st);
        let style = TabBarStyle::default();
        // natural 1 each over 3 tabs, 5 spare columns: s / n = 1 to every
        // tab, remainder 5 % 3 = 2 to the first two tabs from the start —
        // extents 3, 3, 2, covering the area exactly.
        for x in 0..3 {
            assert!(cell_has_style(&b[(x, 0)], style.selected));
        }
        for x in 3..8 {
            assert!(cell_has_style(&b[(x, 0)], style.general));
        }
    }

    /// UI-R-124, UI-R-126 — under `Start` alignment the cells a tab gains
    /// sit outside its padding, after it.
    #[test]
    fn ut_horizontal_gained_columns_sit_outside_the_padding() {
        let w = TabBarBuilder::<String>::default()
            .padding(Margin::new(1, 0))
            .alignment(TabAlignment::Start)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(5, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 5, 1), &mut b, &mut st);
        assert_eq!(row(&b, 0, 5), " A   ");
        let style = TabBarStyle::default();
        for x in 0..5 {
            assert!(cell_has_style(&b[(x, 0)], style.selected));
        }
    }

    /// UI-R-127, UI-E-074 — an odd `g` splits with the smaller half before
    /// the tab; gaining exactly one cell puts it after the trailing padding.
    #[test]
    fn ut_horizontal_center_puts_the_smaller_half_before() {
        let w = TabBarBuilder::<String>::default()
            .alignment(TabAlignment::Center)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(4, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 4, 1), &mut b, &mut st);
        assert_eq!(row(&b, 0, 4), " A  ");

        let w = TabBarBuilder::<String>::default()
            .padding(Margin::new(1, 0))
            .alignment(TabAlignment::Center)
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["A"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(4, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 4, 1), &mut b, &mut st);
        assert_eq!(row(&b, 0, 4), " A  ");
    }

    /// UI-R-125, UI-R-128 — alignment changes nothing once the natural
    /// extent already fills or exceeds the area.
    #[test]
    fn ut_horizontal_alignment_is_inert_without_stretching() {
        for alignment in [TabAlignment::Start, TabAlignment::Center, TabAlignment::End] {
            let w = TabBarBuilder::<String>::default()
                .alignment(alignment)
                .build()
                .unwrap();

            let mut st = TabBarState {
                titles: titles(&["Ab", "Cd"]),
                active: 0,
                offset: 0,
            };
            let mut b = buffer(4, 1);
            StatefulWidget::render(&w, Rect::new(0, 0, 4, 1), &mut b, &mut st);
            assert_eq!(row(&b, 0, 4), "AbCd");

            let mut st = TabBarState {
                titles: titles(&["Ab", "Cd"]),
                active: 0,
                offset: 0,
            };
            let mut b = buffer(3, 1);
            StatefulWidget::render(&w, Rect::new(0, 0, 3, 1), &mut b, &mut st);
            assert_eq!(row(&b, 0, 3), "AbC");
        }
    }

    /// UI-E-064 — a taller area than the rendered extent draws into the
    /// first `1 + 2c` rows only, leaving the rest untouched.
    #[test]
    fn ut_horizontal_taller_area_uses_rendered_extent_only() {
        let w = TabBarBuilder::<String>::default()
            .padding(Margin::new(1, 0))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(5, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 5, 3), &mut b, &mut st);
        assert_eq!(row(&b, 0, 5), " Tab ");
        for y in 1..3 {
            for x in 0..5 {
                assert_eq!(b[(x, y)].symbol(), " ");
                assert!(cell_has_style(&b[(x, y)], ratatui::style::Style::default()));
            }
        }
    }

    /// UI-E-069 — an area shorter across the layout direction than the
    /// rendered extent clips the far padding row, no reflow, no panic.
    #[test]
    fn ut_horizontal_shorter_area_clips_rows() {
        let w = TabBarBuilder::<String>::default()
            .padding(Margin::new(1, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["Tab"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(5, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 5, 2), &mut b, &mut st);
        assert_eq!(row(&b, 0, 5), "     ");
        assert_eq!(row(&b, 1, 5), " Tab ");
    }

    /// UI-E-067, UI-E-073 — an empty title occupies only its along-direction
    /// padding cells, and with that padding 0 it becomes visible again
    /// through its share of spare columns.
    #[test]
    fn ut_horizontal_empty_title_occupies_padding_only() {
        let w = TabBarBuilder::<String>::default()
            .padding(Margin::new(1, 1))
            .build()
            .unwrap();
        let mut st = TabBarState {
            titles: titles(&["", "Two"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(7, 3);
        StatefulWidget::render(&w, Rect::new(0, 0, 7, 3), &mut b, &mut st);
        let style = TabBarStyle::default();
        assert!(cell_has_style(&b[(0, 1)], style.selected));
        assert!(cell_has_style(&b[(1, 1)], style.selected));
        assert_eq!(b[(3, 1)].symbol(), "T");

        let w0 = TabBarBuilder::<String>::default().build().unwrap();
        let mut st0 = TabBarState {
            titles: titles(&["", "Two"]),
            active: 0,
            offset: 0,
        };
        let mut b0 = buffer(5, 1);
        StatefulWidget::render(&w0, Rect::new(0, 0, 5, 1), &mut b0, &mut st0);
        let style0 = TabBarStyle::default();
        assert!(cell_has_style(&b0[(0, 0)], style0.selected));
        assert_eq!(b0[(1, 0)].symbol(), "T");
    }

    /// UI-E-071 — a single tab with spare extent takes every gained cell
    /// and covers the whole area.
    #[test]
    fn ut_horizontal_single_tab_covers_whole_area() {
        let w = TabBarBuilder::<String>::default().build().unwrap();
        let mut st = TabBarState {
            titles: titles(&["A"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(5, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 5, 1), &mut b, &mut st);
        let style = TabBarStyle::default();
        for x in 0..5 {
            assert!(cell_has_style(&b[(x, 0)], style.selected));
        }
    }

    /// UI-E-072 — with fewer spare columns than tabs, the first tabs from
    /// the start of the layout direction gain one cell each and the area is
    /// still covered to its last column.
    #[test]
    fn ut_horizontal_fewer_spare_columns_than_tabs() {
        let w = TabBarBuilder::<String>::default().build().unwrap();
        let mut st = TabBarState {
            titles: titles(&["A", "B", "C"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(5, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 5, 1), &mut b, &mut st);
        let style = TabBarStyle::default();
        for x in 0..2 {
            assert!(cell_has_style(&b[(x, 0)], style.selected));
        }
        for x in 2..5 {
            assert!(cell_has_style(&b[(x, 0)], style.general));
        }
    }

    /// UI-E-075 — a tab that gains no cells renders alike under every
    /// alignment; only the tabs that gain cells move.
    #[test]
    fn ut_horizontal_unstretched_tab_renders_alike_under_every_alignment() {
        for (alignment, first, second) in [
            (TabAlignment::Start, 0u16, 2u16),
            (TabAlignment::Center, 0, 2),
            (TabAlignment::End, 1, 3),
        ] {
            let w = TabBarBuilder::<String>::default()
                .alignment(alignment)
                .build()
                .unwrap();
            let mut st = TabBarState {
                titles: titles(&["A", "B", "C"]),
                active: 0,
                offset: 0,
            };
            let mut b = buffer(5, 1);
            StatefulWidget::render(&w, Rect::new(0, 0, 5, 1), &mut b, &mut st);
            assert_eq!(b[(first, 0)].symbol(), "A");
            assert_eq!(b[(second, 0)].symbol(), "B");
            assert_eq!(b[(4, 0)].symbol(), "C");
        }
    }

    /// UI-E-084 — a double-width title character counts as one cell in the
    /// extent computation and is written unchanged. A natural extent of 4
    /// (miscounting each wide character as its 2-column display width)
    /// instead of 2 would center the title one cell later than a correct
    /// count of 2 does, so the gap before and after the title discriminates
    /// the count as well as the character glyphs themselves.
    #[test]
    fn ut_horizontal_wide_title_char_is_written_as_is() {
        let w = TabBarBuilder::<String>::default().build().unwrap();
        let mut st = TabBarState {
            titles: titles(&["日本"]),
            active: 0,
            offset: 0,
        };
        let mut b = buffer(5, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 5, 1), &mut b, &mut st);
        assert_eq!(b[(0, 0)].symbol(), " ");
        assert_eq!(b[(1, 0)].symbol(), "日");
        assert_eq!(b[(2, 0)].symbol(), "本");
        assert_eq!(b[(3, 0)].symbol(), " ");
        assert_eq!(b[(4, 0)].symbol(), " ");
    }
}
