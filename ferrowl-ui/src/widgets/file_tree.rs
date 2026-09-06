use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters, WithSetters};

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, StatefulWidget, Widget};

use crate::Border;
use crate::state::{FileStatus, FileTreeState};
use crate::style::{InputFieldStyle, MarkdownTheme, SyntaxTheme};
use crate::traits::{IsFocus, Margins};
use crate::widgets::Title;

/// A file tree rendered from a [`FileTreeState`]: each visible row indented by its depth
/// with an expansion marker on directories (UI-R-238), a change-status marker and style on
/// a file that carries one (UI-R-244), and the selected row painted in the theme's
/// highlighted-row style across the widget's full width (UI-R-252).
#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters, WithSetters)]
#[getset(set = "pub")]
pub struct FileTree {
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
}

impl Default for FileTree {
    fn default() -> Self {
        FileTreeBuilder::default()
            .build()
            .expect("FileTreeBuilder fields all default")
    }
}

impl Margins for FileTree {
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

fn status_marker(status: FileStatus) -> char {
    match status {
        FileStatus::Added => '+',
        FileStatus::Removed => '-',
        FileStatus::Modified => '~',
    }
}

impl FileTree {
    fn status_style(&self, status: FileStatus) -> Style {
        match status {
            FileStatus::Added => self.syntax_theme.added,
            FileStatus::Removed => self.syntax_theme.removed,
            FileStatus::Modified => self.syntax_theme.meta,
        }
    }
}

impl StatefulWidget for FileTree {
    type State = FileTreeState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self, area, buf, state);
    }
}

impl StatefulWidget for &FileTree {
    type State = FileTreeState;

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

            let (content_prefix, style) = match row.status {
                Some(status) => (status_marker(status).to_string(), self.status_style(status)),
                None => (String::new(), self.style.general),
            };

            let mut line = String::new();
            line.push_str(&indent);
            line.push(marker);
            line.push_str(&content_prefix);
            line.push_str(&row.name);

            let clipped: String = line.chars().take(area.width as usize).collect();
            let row_rect = Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            };
            buf.set_style(row_rect, self.style.general);
            buf.set_string(area.x, y, &clipped, style);

            if !rows.is_empty() && i == selected {
                buf.set_style(row_rect, self.highlighted_row);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::FileTreeStateBuilder;
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
            .paths(list.iter().map(|(p, s)| (p.to_string(), *s)).collect())
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
    /// UI-R-244 — a change-status marker and style on a file that carries one; a file
    /// with none takes the normal text style.
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
}
