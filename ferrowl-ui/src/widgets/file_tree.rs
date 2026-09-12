use std::marker::PhantomData;

use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, StatefulWidget, Widget};

use crate::Border;
use crate::state::{FileStatus, FileTreeState, FileTreeStatus};
use crate::style::{InputFieldStyle, MarkdownTheme, SyntaxTheme};
use crate::traits::{IsFocus, Margins};
use crate::widgets::Title;

/// A file tree rendered from a [`FileTreeState`]: each visible row indented by its depth
/// with an expansion marker on directories (UI-R-238), a status marker and style on a file
/// that carries one (UI-R-244), and the selected row painted in the theme's highlighted-row
/// style across the widget's full width (UI-R-252).
#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct FileTree<S = FileStatus> {
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
}

impl Default for FileTree<FileStatus> {
    fn default() -> Self {
        FileTreeBuilder::default()
            .build()
            .expect("FileTreeBuilder fields all default")
    }
}

impl<S> Margins for FileTree<S> {
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

impl<S: FileTreeStatus> StatefulWidget for FileTree<S> {
    type State = FileTreeState<S>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

impl<S: FileTreeStatus> StatefulWidget for &FileTree<S> {
    type State = FileTreeState<S>;

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

            let mut segments: Vec<(String, Style)> = vec![(prefix, style)];
            if let Some(badge) = &row.badge
                && !badge.marker.is_empty()
            {
                segments.push((badge.marker.clone(), badge.style));
                segments.push((" ".to_string(), style));
            }
            segments.push((row.name.clone(), style));

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
    use crate::state::{FileTreeBadge, FileTreeEntry, FileTreeStateBuilder};
    use crate::traits::SetFocus;
    use ratatui::layout::Rect as RRect;

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
        list: &[(&str, Option<FileStatus>, Option<FileTreeBadge>)],
    ) -> crate::state::FileTreeState {
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

    #[test]
    /// UI-R-314 — a badge's marker text and style are drawn as the caller supplied them.
    fn ut_badge_marker_and_style_are_drawn_as_given() {
        let badge = FileTreeBadge::new("*", Style::default().fg(ratatui::style::Color::Cyan));
        let mut s = badged_tree(&[("a.rs", None, Some(badge))]);
        let w = FileTree::default();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with(" * a.rs"));
        assert_eq!(b[(1, 0)].fg, ratatui::style::Color::Cyan);
    }

    #[test]
    /// UI-R-315 — a badge draws after the status marker and immediately before the name,
    /// separated from the name by one space.
    fn ut_badge_draws_after_the_status_marker_and_one_space_before_the_name() {
        let badge = FileTreeBadge::new("*", Style::default());
        let mut s = badged_tree(&[("a.rs", Some(FileStatus::Added), Some(badge))]);
        let w = FileTree::default();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert_eq!(row_text(&b, 0, 8), " +* a.rs");
    }

    #[test]
    /// UI-R-316 — the badge marker's cells carry the badge's own style, while the
    /// separating space and the name cells carry the row's status style.
    fn ut_badge_keeps_its_own_style_while_the_row_keeps_the_status_style() {
        let badge = FileTreeBadge::new("*", Style::default().fg(ratatui::style::Color::Cyan));
        let mut s = badged_tree(&[("a.rs", Some(FileStatus::Added), Some(badge))]);
        let w = FileTree::default();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        let added_fg = w.syntax_theme.added.fg.expect("style sets a color");
        // ` +* a.rs`: 0=' ' 1='+' 2='*' 3=' ' 4='a'
        assert_eq!(b[(1, 0)].fg, added_fg);
        assert_eq!(b[(2, 0)].fg, ratatui::style::Color::Cyan);
        assert_eq!(b[(3, 0)].fg, added_fg);
        assert_eq!(b[(4, 0)].fg, added_fg);
    }

    #[test]
    /// UI-R-316, UI-R-252 — the badge keeps its own foreground even on the selected row,
    /// where the row background is patched to the highlighted-row style across the full
    /// width.
    fn ut_badge_keeps_its_foreground_over_the_highlighted_row_background() {
        let badge = FileTreeBadge::new("*", Style::default().fg(ratatui::style::Color::Cyan));
        let mut s = badged_tree(&[("a.rs", None, Some(badge))]);
        let w = FileTree::default();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        for x in 0..20 {
            assert_eq!(
                b[(x, 0)].bg,
                w.highlighted_row.bg.expect("style sets a color")
            );
        }
        assert_eq!(b[(1, 0)].fg, ratatui::style::Color::Cyan);
    }

    #[test]
    /// UI-R-317 — `set_badge` is reflected by the next render.
    fn ut_set_badge_is_reflected_by_the_next_render() {
        let mut s = tree(&[("a.rs", None)]);
        let w = FileTree::default();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with(" a.rs"));

        s.set_badge("a.rs", Some(FileTreeBadge::new("*", Style::default())));
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with(" * a.rs"));
    }

    #[test]
    /// UI-R-318 — an unbadged row draws the identical cells whether or not a sibling
    /// carries a badge.
    fn ut_unbadged_rows_render_identically_whether_or_not_a_sibling_is_badged() {
        let mut plain = tree(&[("a.rs", None), ("b.rs", None)]);
        let w = FileTree::default();
        let mut b_plain = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b_plain, &mut plain);

        let badge = FileTreeBadge::new("*", Style::default());
        let mut with_badge = badged_tree(&[("a.rs", None, Some(badge)), ("b.rs", None, None)]);
        let mut b_badged = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b_badged, &mut with_badge);

        assert_eq!(row_text(&b_plain, 1, 20), row_text(&b_badged, 1, 20));
        for x in 0..20 {
            assert_eq!(b_plain[(x, 1)].fg, b_badged[(x, 1)].fg);
        }
    }

    #[test]
    /// UI-E-147 — a badge on a status-free file draws before the name with no status
    /// marker preceding it.
    fn ut_badge_on_a_status_free_file_draws_before_the_name_with_no_status_marker() {
        let badge = FileTreeBadge::new("*", Style::default());
        let mut s = badged_tree(&[("a.rs", None, Some(badge))]);
        let w = FileTree::default();
        let mut b = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b, &mut s);
        assert_eq!(row_text(&b, 0, 8), " * a.rs ");
    }

    #[test]
    /// UI-E-149 — a badge widening a row clips the name at the area width without
    /// wrapping onto the next row.
    fn ut_badge_widening_a_row_clips_the_name_at_the_area_width() {
        let badge = FileTreeBadge::new("*", Style::default());
        let mut s = badged_tree(&[("a-very-long-file-name-indeed.rs", None, Some(badge))]);
        let w = FileTree::default();
        let mut b = buffer(10, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 10, 2), &mut b, &mut s);
        let line = row_text(&b, 0, 10);
        assert_eq!(line.chars().count(), 10);
        assert_eq!(line, " * a-very-");
        assert!(row_text(&b, 1, 10).trim().is_empty());
    }

    #[test]
    /// UI-E-150 — an empty badge marker renders as an unbadged row: no badge cells and no
    /// separating space.
    fn ut_empty_badge_marker_renders_as_an_unbadged_row() {
        let empty_badge = FileTreeBadge::new("", Style::default().fg(ratatui::style::Color::Cyan));
        let mut with_empty = badged_tree(&[("a.rs", None, Some(empty_badge))]);
        let mut plain = tree(&[("a.rs", None)]);
        let w = FileTree::default();

        let mut b_empty = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b_empty, &mut with_empty);
        let mut b_plain = buffer(20, 1);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 1), &mut b_plain, &mut plain);

        assert_eq!(row_text(&b_empty, 0, 20), row_text(&b_plain, 0, 20));
        for x in 0..20 {
            assert_eq!(b_empty[(x, 0)].fg, b_plain[(x, 0)].fg);
        }
    }

    #[test]
    /// UI-E-148 — a badge set for a directory path is stored but never drawn: the
    /// directory row and its children's ancestor cells carry no badge marker.
    fn ut_badge_for_a_directory_path_draws_no_badge_on_that_row_or_its_children() {
        let mut s = tree(&[("a/b.rs", None)]);
        s.set_badge("a", Some(FileTreeBadge::new("*", Style::default())));
        s.expand_all();
        let w = FileTree::default();
        let mut b = buffer(20, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 20, 2), &mut b, &mut s);
        assert!(row_text(&b, 0, 20).starts_with("▾a"));
        assert!(!row_text(&b, 0, 20).contains('*'));
        assert!(row_text(&b, 1, 20).starts_with("   b.rs"));
    }
}
