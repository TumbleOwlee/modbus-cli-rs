use std::marker::PhantomData;

use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, StatefulWidget, Widget};

use crate::Border;
use crate::state::{FileStatus, FileTreeBadge, FileTreeState, FileTreeStatus, NoBadge};
use crate::style::{InputFieldStyle, MarkdownTheme, SyntaxTheme};
use crate::traits::{IsFocus, Margins};
use crate::widgets::Title;

/// A file tree rendered from a [`FileTreeState`]: each visible row indented by its depth
/// with an expansion marker on directories (UI-R-238), a status marker and style on a file
/// that carries one (UI-R-244), and the selected row painted in the theme's highlighted-row
/// style across the widget's full width (UI-R-252).
#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct FileTree<S = FileStatus, B = NoBadge> {
    #[getset(get = "pub")]
    #[builder(default = "Border::None")]
    border: Border,
    #[getset(get = "pub")]
    #[builder(default = "None")]
    title: Option<Title>,
    #[getset(get = "pub")]
    #[builder(default = "Margin::default()")]
    margin: Margin,
    #[getset(get = "pub")]
    #[builder(default = "InputFieldStyle::default()")]
    style: InputFieldStyle,
    #[getset(get = "pub")]
    #[builder(default = "SyntaxTheme::default()")]
    syntax_theme: SyntaxTheme,
    /// UI-R-252 — defaulted from the single place this value lives, `MarkdownTheme`'s
    /// (UI-R-138), so the file tree's and the markdown field's read-only active-row
    /// highlight agree; not a literal colour and not a copy of `DiffViewStyle`'s field.
    #[getset(get = "pub")]
    #[builder(default = "*MarkdownTheme::default().highlighted_row()")]
    highlighted_row: Style,
    #[getset(skip)]
    #[builder(setter(skip), default = "PhantomData")]
    status: PhantomData<S>,
    #[getset(skip)]
    #[builder(setter(skip), default = "PhantomData")]
    badge: PhantomData<B>,
}

impl Default for FileTree<FileStatus, NoBadge> {
    fn default() -> Self {
        FileTreeBuilder::default()
            .build()
            .expect("FileTreeBuilder fields all default")
    }
}

impl<S, B> Margins for FileTree<S, B> {
    fn margins(&self) -> Margin {
        let horizontal = if let Border::Full(m) = &self.border {
            4 + m.horizontal * 2
        } else {
            0
        } + 2 * self.margin.horizontal
            + 1;
        let vertical = if let Border::Full(m) = &self.border {
            2 + m.vertical * 2
        } else if self.title.is_some() {
            1
        } else {
            0
        } + self.margin.vertical;
        Margin {
            horizontal,
            vertical,
        }
    }
}

impl<S: FileTreeStatus, B: FileTreeBadge> StatefulWidget for FileTree<S, B> {
    type State = FileTreeState<S, B>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

impl<S: FileTreeStatus, B: FileTreeBadge> StatefulWidget for &FileTree<S, B> {
    type State = FileTreeState<S, B>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        buf.set_style(area, self.style.general);

        let area = Layout::vertical([
            Constraint::Length(self.margin.vertical),
            Constraint::Min(1),
            Constraint::Length(self.margin.vertical),
        ])
        .split(area)[1];

        let mut area = Layout::horizontal([
            Constraint::Length(self.margin.horizontal),
            Constraint::Min(1),
            Constraint::Length(self.margin.horizontal),
        ])
        .split(area)[1];

        if let Border::Full(m) = &self.border {
            let border_style = if state.is_focused() {
                *self.style.focused()
            } else {
                *self.style.border()
            };
            let mut block = Block::bordered().style(border_style);
            if let Some(t) = &self.title {
                block = block.title(t.name.as_str()).title_alignment(t.alignment);
            }
            let inner = block.inner(area);
            block.render(area, buf);
            area = inner.inner(*m);
        }

        if area.height == 0 {
            return;
        }
        state.set_visible_height(area.height as usize);

        let rows = state.visible_rows();
        let scroll = state.scroll_offset();
        let selected = state.selected();

        for (i, row) in rows
            .iter()
            .enumerate()
            .skip(scroll)
            .take(area.height as usize)
        {
            let y = area.y + (i - scroll) as u16;
            let indent: String = "  ".repeat(row.depth);
            let marker = if row.is_dir {
                if row.expanded { '▾' } else { '▸' }
            } else {
                ' '
            };

            let (content_prefix, style) = match &row.status {
                Some(status) => (status.marker(), status.style(&self.syntax_theme)),
                None => (String::new(), self.style.general),
            };

            let mut prefix = String::new();
            prefix.push_str(&indent);
            prefix.push(marker);
            prefix.push_str(&content_prefix);

            let mut segments: Vec<(String, Style)> =
                vec![(prefix, style), (row.name.clone(), style)];
            if let Some(badge) = &row.badge {
                let text = badge.text();
                if !text.is_empty() {
                    segments.push((" ".to_string(), style));
                    segments.push((text, badge.style().unwrap_or(style)));
                }
            }

            let row_rect = Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            };
            buf.set_style(row_rect, self.style.general);

            let mut x = area.x;
            let end = area.x + area.width;
            for (chunk, seg_style) in segments {
                if x >= end {
                    break;
                }
                let budget = (end - x) as usize;
                (x, _) = buf.set_stringn(x, y, &chunk, budget, seg_style);
            }

            if !rows.is_empty() && i == selected {
                buf.set_style(row_rect, self.highlighted_row);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{FileTreeEntry, FileTreeStateBuilder};
    use crate::traits::SetFocus;
    use ratatui::layout::Rect as RRect;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Marker(&'static str, Option<ratatui::style::Color>);

    impl crate::state::FileTreeBadge for Marker {
        fn text(&self) -> String {
            self.0.to_string()
        }

        fn style(&self) -> Option<Style> {
            self.1.map(|c| Style::default().fg(c))
        }
    }

    fn buffer(w: u16, h: u16) -> Buffer {
        Buffer::empty(RRect::new(0, 0, w, h))
    }

    fn row_text(b: &Buffer, y: u16, w: u16) -> String {
        (0..w)
            .map(|x| b[(x, y)].symbol().chars().next().unwrap_or(' '))
            .collect()
    }

    fn tree(list: &[(&str, Option<FileStatus>)]) -> crate::state::FileTreeState {
        FileTreeStateBuilder::default()
            .paths(
                list.iter()
                    .map(|(p, s)| {
                        let mut e = FileTreeEntry::new(*p);
                        if let Some(s) = s {
                            e = e.with_status(*s);
                        }
                        e
                    })
                    .collect(),
            )
            .build()
            .unwrap()
    }

    fn badged_tree(
        list: &[(&str, Option<FileStatus>, Option<Marker>)],
    ) -> crate::state::FileTreeState<FileStatus, Marker> {
        FileTreeStateBuilder::default()
            .paths(
                list.iter()
                    .map(|(p, s, b)| {
                        let mut e = FileTreeEntry::new(*p);
                        if let Some(s) = s {
                            e = e.with_status(*s);
                        }
                        if let Some(b) = b {
                            e = e.with_badge(b.clone());
                        }
                        e
                    })
                    .collect(),
            )
            .build()
            .unwrap()
    }

    #[test]
    /// UI-R-238 — rows are indented by depth, a directory carries an expansion marker
    /// (`▾` expanded, `▸` collapsed), a file carries none.
    fn ut_rows_are_indented_by_depth_with_expansion_markers_on_directories() {
        let mut s = tree(&[("a/b.rs", None)]);
        s.expand_all();
        let w = FileTree::default();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut s);
        assert!(row_text(&b, 0, 40).starts_with("▾a"));
        assert!(row_text(&b, 1, 40).starts_with("   b.rs"));

        s.collapse_all();
        let mut b = buffer(40, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 2), &mut b, &mut s);
        assert!(row_text(&b, 0, 40).starts_with("▸a"));
    }

    #[test]
    /// UI-R-244, UI-R-319 — the shipped status type's marker and style on a file that
    /// carries one; a file with none takes the normal text style.
    fn ut_status_markers_and_styles_follow_the_change_status() {
        let mut s = tree(&[
            ("added.rs", Some(FileStatus::Added)),
            ("removed.rs", Some(FileStatus::Removed)),
            ("modified.rs", Some(FileStatus::Modified)),
            ("plain.rs", None),
        ]);
        s.set_visible_height(4);
        let w = FileTree::default();
        let mut b = buffer(40, 4);
        StatefulWidget::render(&w, Rect::new(0, 0, 40, 4), &mut b, &mut s);

        // Sibling files sort by name (UI-R-237): added.rs, modified.rs, plain.rs, removed.rs.
        assert!(row_text(&b, 0, 40).starts_with(" +added.rs"));
        assert_eq!(
            b[(1, 0)].fg,
            w.syntax_theme.added.fg.expect("style sets a color")
        );
        assert!(row_text(&b, 1, 40).starts_with(" ~modified.rs"));
        assert_eq!(
            b[(1, 1)].fg,
            w.syntax_theme.meta.fg.expect("style sets a color")
        );
        assert!(row_text(&b, 2, 40).starts_with(" plain.rs"));
        assert_eq!(
            b[(1, 2)].fg,
            w.style.general().fg.expect("style sets a color")
        );
        assert!(row_text(&b, 3, 40).starts_with(" -removed.rs"));
        assert_eq!(
            b[(1, 3)].fg,
            w.syntax_theme.removed.fg.expect("style sets a color")
        );
    }

    #[test]
    /// UI-E-106 — a row wider than the area is clipped at the area width; the file tree
    /// never scrolls horizontally or wraps.
    fn ut_row_wider_than_the_area_is_clipped_and_never_wraps() {
        let mut s = tree(&[("a-very-long-file-name-indeed.rs", None)]);
        let w = FileTree::default();
        let mut b = buffer(10, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 10, 2), &mut b, &mut s);
        let line = row_text(&b, 0, 10);
        assert_eq!(line.chars().count(), 10);
        assert!(line.starts_with(" a-very-lo"));
        assert!(row_text(&b, 1, 10).trim().is_empty());
    }

    #[test]
    /// UI-R-246 — the focused border style while focused, the normal border otherwise.
    fn ut_border_style_follows_focus() {
        let mut s = tree(&[("a.rs", None)]);
        let w = FileTreeBuilder::default()
            .border(Border::Full(Margin::new(0, 0)))
            .build()
            .unwrap();

        SetFocus::set_focused(&mut s, false);
        let mut b = buffer(20, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 5), &mut b, &mut s);
        assert_eq!(
            b[(0, 0)].fg,
            w.style.border().fg.expect("style sets a color")
        );

        SetFocus::set_focused(&mut s, true);
        let mut b = buffer(20, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 5), &mut b, &mut s);
        assert_eq!(
            b[(0, 0)].fg,
            w.style.focused().fg.expect("style sets a color")
        );
    }

    #[test]
    /// UI-R-252 — the selected row takes the highlighted-row style across the full
    /// width, keeping a status-carrying file's foreground under the highlight.
    fn ut_selected_row_takes_the_highlighted_row_style_across_the_full_width() {
        let mut s = tree(&[("added.rs", Some(FileStatus::Added)), ("b.rs", None)]);
        let w = FileTree::default();
        let mut b = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b, &mut s);
        for x in 0..20 {
            assert_eq!(
                b[(x, 0)].bg,
                w.highlighted_row.bg.expect("style sets a color"),
                "column {x} not in the highlighted-row style"
            );
        }
        assert_eq!(
            b[(1, 0)].fg,
            w.syntax_theme.added.fg.expect("style sets a color")
        );
        assert_ne!(
            b[(0, 1)].bg,
            w.highlighted_row.bg.expect("style sets a color")
        );
    }

    #[test]
    /// UI-E-110, UI-E-103 — an empty tree draws its border around an empty interior:
    /// no row carries the highlighted-row style, since there is no selected node.
    fn ut_empty_tree_draws_its_border_around_an_empty_interior_and_highlights_no_row() {
        let mut s = tree(&[]);
        let w = FileTreeBuilder::default()
            .border(Border::Full(Margin::new(0, 0)))
            .build()
            .unwrap();
        let mut b = buffer(20, 5);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 5), &mut b, &mut s);

        assert_ne!(b[(0, 0)].symbol(), " ");
        for y in 1..4 {
            for x in 1..19 {
                assert_eq!(b[(x, y)].symbol(), " ");
                assert_ne!(
                    b[(x, y)].bg,
                    w.highlighted_row.bg.expect("style sets a color")
                );
            }
        }
    }

    #[test]
    /// UI-R-252 — the highlighted-row default agrees with `MarkdownTheme`'s (UI-R-138).
    fn ut_highlighted_row_default_matches_markdown_theme() {
        let w = FileTree::default();
        let markdown = MarkdownTheme::default();
        assert_eq!(w.highlighted_row, *markdown.highlighted_row());
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Severity {
        Info,
        Silent,
        Wide,
    }

    impl crate::state::FileTreeStatus for Severity {
        fn marker(&self) -> String {
            match self {
                Severity::Info => "!".to_string(),
                Severity::Silent => String::new(),
                Severity::Wide => ">>".to_string(),
            }
        }

        fn style(&self, _theme: &SyntaxTheme) -> Style {
            Style::default().fg(ratatui::style::Color::Magenta)
        }
    }

    fn severity_tree(list: &[(&str, Option<Severity>)]) -> crate::state::FileTreeState<Severity> {
        FileTreeStateBuilder::<Severity>::default()
            .paths(
                list.iter()
                    .map(|(p, s)| {
                        let mut e = FileTreeEntry::new(*p);
                        if let Some(s) = s {
                            e = e.with_status(*s);
                        }
                        e
                    })
                    .collect(),
            )
            .build()
            .unwrap()
    }

    #[test]
    /// UI-R-244 — a caller status type supplies the row's marker and style.
    fn ut_caller_status_type_supplies_the_marker_and_style() {
        let mut s = severity_tree(&[("a.rs", Some(Severity::Info))]);
        let w: FileTree<Severity> = FileTreeBuilder::default().build().unwrap();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with(" !a.rs"));
        assert_eq!(b[(1, 0)].fg, ratatui::style::Color::Magenta);
    }

    #[test]
    /// UI-R-320 — a tree built and rendered without naming a status type draws the
    /// shipped literal markers and syntax-theme foregrounds.
    fn ut_default_status_type_is_the_shipped_one() {
        let mut s = tree(&[("added.rs", Some(FileStatus::Added))]);
        let w = FileTree::default();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with(" +added.rs"));
        assert_eq!(
            b[(1, 0)].fg,
            w.syntax_theme.added.fg.expect("style sets a color")
        );
    }

    #[test]
    /// UI-R-321 — a file with no status draws no leading marker and takes the normal
    /// text style, under both the shipped status type and a caller-defined one.
    fn ut_file_without_status_draws_no_marker_and_takes_the_normal_style() {
        let mut s = tree(&[("plain.rs", None)]);
        let w = FileTree::default();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with(" plain.rs"));
        assert_eq!(
            b[(1, 0)].fg,
            w.style.general().fg.expect("style sets a color")
        );

        let mut s = severity_tree(&[("plain.rs", None)]);
        let w: FileTree<Severity> = FileTreeBuilder::default().build().unwrap();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with(" plain.rs"));
        assert_eq!(
            b[(1, 0)].fg,
            w.style.general().fg.expect("style sets a color")
        );
    }

    #[test]
    /// UI-E-151 — a caller status type reporting an empty marker draws no leading
    /// marker and no separating space, while its style still applies.
    fn ut_empty_status_marker_draws_no_marker_but_keeps_the_style() {
        let mut s = severity_tree(&[("silent.rs", Some(Severity::Silent))]);
        let w: FileTree<Severity> = FileTreeBuilder::default().build().unwrap();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with(" silent.rs"));
        assert_eq!(b[(1, 0)].fg, ratatui::style::Color::Magenta);
    }

    #[test]
    /// UI-E-152 — a caller status type reporting a marker of more than one cell is
    /// drawn as reported, shifting the name and clipping at the area width.
    fn ut_multi_cell_status_marker_shifts_the_name_and_clips_at_the_area() {
        let mut s = severity_tree(&[
            ("silent.rs", Some(Severity::Silent)),
            ("wide.rs", Some(Severity::Wide)),
        ]);
        s.set_visible_height(2);
        let w: FileTree<Severity> = FileTreeBuilder::default().build().unwrap();
        let mut b = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with(" silent.rs"));
        assert!(row_text(&b, 1, 20).starts_with(" >>wide.rs"));

        let mut s = severity_tree(&[("a-very-long-file-name-indeed.rs", Some(Severity::Wide))]);
        let w: FileTree<Severity> = FileTreeBuilder::default().build().unwrap();
        let mut b = buffer(10, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 10, 1), &mut b, &mut s);
        assert_eq!(row_text(&b, 0, 10), " >>a-very-");
    }

    fn marker_tree() -> FileTree<FileStatus, Marker> {
        FileTreeBuilder::default().build().unwrap()
    }

    #[test]
    /// UI-R-314 — a badge's text and style are drawn as its type reports them.
    fn ut_badge_marker_and_style_are_drawn_as_given() {
        let badge = Marker("*", Some(ratatui::style::Color::Cyan));
        let mut s = badged_tree(&[("a.rs", None, Some(badge))]);
        let w = marker_tree();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with(" a.rs *"));
        assert_eq!(b[(6, 0)].fg, ratatui::style::Color::Cyan);
    }

    #[test]
    /// UI-R-315 — a badge draws after the node's name, separated from it by one space,
    /// while the status marker stays leading.
    fn ut_badge_draws_after_the_name_and_one_space_after_it() {
        let badge = Marker("*", None);
        let mut s = badged_tree(&[("a.rs", Some(FileStatus::Added), Some(badge))]);
        let w = marker_tree();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert_eq!(row_text(&b, 0, 8), " +a.rs *");
    }

    #[test]
    /// UI-R-316 — the badge's cells carry its own style, while the leading status
    /// marker, separating space and name cells carry the row's status style.
    fn ut_badge_keeps_its_own_style_while_the_row_keeps_the_status_style() {
        let badge = Marker("*", Some(ratatui::style::Color::Cyan));
        let mut s = badged_tree(&[("a.rs", Some(FileStatus::Added), Some(badge))]);
        let w = marker_tree();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        let added_fg = w.syntax_theme.added.fg.expect("style sets a color");
        // ` +a.rs *`: 0=' ' 1='+' 2='a' ... 5='s' 6=' ' 7='*'
        assert_eq!(b[(1, 0)].fg, added_fg);
        assert_eq!(b[(2, 0)].fg, added_fg);
        assert_eq!(b[(6, 0)].fg, added_fg);
        assert_eq!(b[(7, 0)].fg, ratatui::style::Color::Cyan);
    }

    #[test]
    /// UI-R-316, UI-R-252 — the badge keeps its own foreground even on the selected row,
    /// where the row background is patched to the highlighted-row style across the full
    /// width.
    fn ut_badge_keeps_its_foreground_over_the_highlighted_row_background() {
        let badge = Marker("*", Some(ratatui::style::Color::Cyan));
        let mut s = badged_tree(&[("a.rs", None, Some(badge))]);
        let w = marker_tree();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        for x in 0..20 {
            assert_eq!(
                b[(x, 0)].bg,
                w.highlighted_row.bg.expect("style sets a color")
            );
        }
        assert_eq!(b[(6, 0)].fg, ratatui::style::Color::Cyan);
    }

    #[test]
    /// UI-R-317 — `set_badge` is reflected by the next render.
    fn ut_set_badge_is_reflected_by_the_next_render() {
        let mut s = badged_tree(&[("a.rs", None, None)]);
        let w = marker_tree();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with(" a.rs"));

        s.set_badge("a.rs", Some(Marker("*", None)));
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with(" a.rs *"));
    }

    #[test]
    /// UI-R-318 — an unbadged row draws the identical cells whether or not a sibling
    /// carries a badge.
    fn ut_unbadged_rows_render_identically_whether_or_not_a_sibling_is_badged() {
        let mut plain = tree(&[("a.rs", None), ("b.rs", None)]);
        let w = FileTree::default();
        let mut b_plain = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b_plain, &mut plain);

        let badge = Marker("*", None);
        let mut with_badge = badged_tree(&[("a.rs", None, Some(badge)), ("b.rs", None, None)]);
        let w = marker_tree();
        let mut b_badged = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b_badged, &mut with_badge);

        assert_eq!(row_text(&b_plain, 1, 20), row_text(&b_badged, 1, 20));
        for x in 0..20 {
            assert_eq!(b_plain[(x, 1)].fg, b_badged[(x, 1)].fg);
        }
    }

    #[test]
    /// UI-E-147 — a badge on a status-free file draws with no leading status marker; the
    /// badge still follows the name one space later.
    fn ut_badge_on_a_status_free_file_draws_with_no_status_marker() {
        let badge = Marker("*", None);
        let mut s = badged_tree(&[("a.rs", None, Some(badge))]);
        let w = marker_tree();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert_eq!(row_text(&b, 0, 8), " a.rs * ");
    }

    #[test]
    /// UI-E-149 — a badge widening a row past the area is clipped away entirely; the name
    /// is drawn in full and the row never wraps onto the next row.
    fn ut_badge_widening_a_row_is_clipped_away_at_the_area_width() {
        let badge = Marker("*", None);
        let mut s = badged_tree(&[("a-very-long-file-name-indeed.rs", None, Some(badge))]);
        let w = marker_tree();
        let mut b = buffer(10, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 10, 2), &mut b, &mut s);
        let line = row_text(&b, 0, 10);
        assert_eq!(line.chars().count(), 10);
        assert_eq!(line, " a-very-lo");
        assert!(row_text(&b, 1, 10).trim().is_empty());
    }

    #[test]
    /// UI-E-150 — a badge type reporting empty text renders as an unbadged row: no badge
    /// cells and no separating space, whatever style it reports.
    fn ut_empty_badge_marker_renders_as_an_unbadged_row() {
        let empty_badge = Marker("", Some(ratatui::style::Color::Cyan));
        let mut with_empty = badged_tree(&[("a.rs", None, Some(empty_badge))]);
        let mut plain = tree(&[("a.rs", None)]);
        let w = marker_tree();
        let plain_w = FileTree::default();

        let mut b_empty = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b_empty, &mut with_empty);
        let mut b_plain = buffer(20, 1);
        StatefulWidget::render(&plain_w, Rect::new(0, 0, 20, 1), &mut b_plain, &mut plain);

        assert_eq!(row_text(&b_empty, 0, 20), row_text(&b_plain, 0, 20));
        for x in 0..20 {
            assert_eq!(b_empty[(x, 0)].fg, b_plain[(x, 0)].fg);
        }
    }

    #[test]
    /// UI-E-148 — a badge set for a directory path is stored but never drawn: the
    /// directory row and its children's ancestor cells carry no badge marker.
    fn ut_badge_for_a_directory_path_draws_no_badge_on_that_row_or_its_children() {
        let mut s = badged_tree(&[("a/b.rs", None, None)]);
        s.set_badge("a", Some(Marker("*", None)));
        s.expand_all();
        let w = marker_tree();
        let mut b = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with("▾a"));
        assert!(!row_text(&b, 0, 20).contains('*'));
        assert!(row_text(&b, 1, 20).starts_with("   b.rs"));
    }

    #[test]
    /// UI-R-322 — a badge whose type reports no style is drawn in the row's own
    /// styling rather than a widget-chosen fallback.
    fn ut_badge_with_no_style_takes_the_row_style() {
        let badge = Marker("*", None);
        let mut s = badged_tree(&[("a.rs", Some(FileStatus::Added), Some(badge))]);
        let w = marker_tree();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        let added_fg = w.syntax_theme.added.fg.expect("style sets a color");
        assert_eq!(b[(7, 0)].fg, added_fg);
    }

    #[test]
    /// UI-R-323 — a tree built and rendered without naming a badge type draws no badge
    /// on any row, exactly as it did before badges existed.
    fn ut_default_badge_type_draws_no_badge() {
        let mut s = tree(&[("a.rs", None)]);
        let w = FileTree::default();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert_eq!(row_text(&b, 0, 20).trim_end(), " a.rs");
    }
}
