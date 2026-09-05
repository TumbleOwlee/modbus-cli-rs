use std::ops::RangeInclusive;

use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};

/// A parsed body line's classification (UI-R-208).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffKind {
    Context,
    Added,
    Removed,
    Meta,
}

/// One side's content of an aligned row: its text and the file line number counted from
/// the hunk header (UI-R-217).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiffEntry {
    pub(crate) text: String,
    pub(crate) line_no: usize,
}

/// One row of the parsed diff: either an aligned pair of optional old/new entries
/// (UI-R-209), or a meta line drawn on its own (UI-R-210). Modelled as an enum rather
/// than a struct with a `meta` flag and dependent optionals: a meta row has no sides and
/// no line numbers at all, so no field combination can express it wrongly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiffRow {
    Pair {
        kind: DiffKind,
        old: Option<DiffEntry>,
        new: Option<DiffEntry>,
    },
    Meta {
        text: String,
    },
}

/// The split/unified rendering layout of the diff widget (UI-R-211, UI-R-213).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLayout {
    Split,
    Unified,
}

/// Which pane holds input focus (UI-R-228).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Old,
    New,
}

/// The diff widget's two modes (UI-R-223): unlike [`VimMode`](super::vim::VimMode), there
/// is no `Insert` variant, since the diff widget is read-only (UI-R-222).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffMode {
    Normal,
    Visual,
}

/// A row's diff kind and its old/new file line numbers, each absent where that side holds
/// a filler (UI-R-227).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRowInfo {
    pub kind: DiffKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
}

/// State of a [`DiffView`](crate::widgets::DiffView) widget: the parsed, aligned rows of a
/// unified diff (UI-R-207, UI-R-209), and the cursor/selection/layout bookkeeping the
/// widget renders and navigates from.
#[derive(Builder, Debug, Clone, Getters, Setters, CopyGetters)]
pub struct DiffViewState {
    #[getset(skip)]
    #[builder(setter(skip), default)]
    rows: Vec<DiffRow>,
    #[getset(skip)]
    #[builder(setter(skip), default)]
    active_row: usize,
    #[getset(skip)]
    #[builder(setter(skip), default)]
    anchor: Option<usize>,
    #[getset(skip)]
    #[builder(setter(skip), default = "DiffMode::Normal")]
    mode: DiffMode,
    /// Vertical scroll offset in rows. Read and written by the widget's own navigation
    /// code, keeping the active row visible.
    #[getset(skip)]
    #[builder(setter(skip), default)]
    #[allow(dead_code)]
    scroll_offset: usize,
    /// Horizontal scroll offset in columns, one shared by every pane (UI-R-232). Read and
    /// written by the widget's own navigation code.
    #[getset(skip)]
    #[builder(setter(skip), default)]
    #[allow(dead_code)]
    h_scroll: usize,
    /// Per-row gutter labels for the old side (UI-R-218), settable when built and
    /// afterwards.
    #[getset(get = "pub", set = "pub")]
    #[builder(default = "None")]
    old_labels: Option<Vec<String>>,
    /// Per-row gutter labels for the new side (UI-R-218), settable when built and
    /// afterwards.
    #[getset(get = "pub", set = "pub")]
    #[builder(default = "None")]
    new_labels: Option<Vec<String>>,
    /// Syntax language highlighting the old side's text; `None` by default (UI-R-220,
    /// UI-R-221). Crate-private field, read directly by the widget that renders this
    /// state: no `api-contract.md` row needs a getter, only the builder setter.
    #[builder(default = "None")]
    #[allow(dead_code)]
    pub(crate) old_language: Option<ferrowl_syntax::Language>,
    /// Syntax language highlighting the new side's text; `None` by default (UI-R-220,
    /// UI-R-221). Crate-private field, read directly by the widget that renders this
    /// state: no `api-contract.md` row needs a getter, only the builder setter.
    #[builder(default = "None")]
    #[allow(dead_code)]
    pub(crate) new_language: Option<ferrowl_syntax::Language>,
    /// Split or unified rendering layout, defaulting to split (UI-R-214).
    #[getset(get_copy = "pub")]
    #[builder(default = "DiffLayout::Split")]
    layout: DiffLayout,
    /// The pane currently holding focus (UI-R-228). `set_focused_side` is the only way to
    /// change it.
    #[getset(get_copy = "pub", set = "pub")]
    #[builder(default = "Side::Old")]
    focused_side: Side,
    /// Visible height in rows of the last render; one row before the first render, the
    /// same pre-render convention as the code editor's (UI-R-173). Written by the widget
    /// that renders this state and read by its own paging logic.
    #[getset(skip)]
    #[builder(setter(skip), default = "1")]
    #[allow(dead_code)]
    visible_height: usize,
    /// Content width in columns of the last render; one column before the first render.
    /// Written by the widget that renders this state and read by its own horizontal-scroll
    /// logic.
    #[getset(skip)]
    #[builder(setter(skip), default = "1")]
    #[allow(dead_code)]
    content_width: usize,
}

/// Parses `text` into aligned rows (UI-R-207, UI-R-209): a `@@` line is a hunk header
/// supplying both sides' starting line numbers (UI-R-217) and is kept as a meta row
/// itself; inside a hunk, ` `/`+`/`-` classify context/added/removed body lines
/// (UI-R-208); a run of removed lines pairs positionwise with the added run that follows
/// it, a surplus line on either side getting a filler on the other (UI-R-209); anything
/// else — before any hunk header, or unrecognized inside one — is kept as a meta row
/// verbatim (UI-E-097, UI-E-098). Empty input yields no rows (UI-E-099). A single
/// trailing newline is stripped before splitting, so ordinary diff text (which almost
/// always ends in one) does not produce a phantom empty meta row after the last line.
fn parse(text: &str) -> Vec<DiffRow> {
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() {
        return Vec::new();
    }
    let mut rows = Vec::new();

    let mut in_hunk = false;
    let mut old_line = 1usize;
    let mut new_line = 1usize;
    let mut removed: Vec<DiffEntry> = Vec::new();
    let mut added: Vec<DiffEntry> = Vec::new();
    // The buffer length just after the most recent removed/added push: whichever line a
    // meta line directly follows, that line's own eventual row index is one less than
    // this — its side's own run length, not the longer of the two sides (UI-E-098: an
    // added line inside a longer removed run sits on an earlier row than the run's last,
    // filler-only row).
    let mut last_run_len = 0usize;
    // Meta lines seen mid-run (UI-E-098's "no newline" marker is the common case) do not
    // flush the pending removed/added buffers, so the run they interrupt still pairs
    // positionwise (UI-R-209). Each is held here with the row index it was seen after so
    // it lands right after that row once the run is finally flushed (UI-E-098: "following
    // the line it belongs to"), even when that row turns out to pair a removed line with
    // an added one.
    let mut pending_metas: Vec<(usize, String)> = Vec::new();

    fn flush(
        rows: &mut Vec<DiffRow>,
        removed: &mut Vec<DiffEntry>,
        added: &mut Vec<DiffEntry>,
        pending_metas: &mut Vec<(usize, String)>,
    ) {
        let count = removed.len().max(added.len());
        let mut metas = std::mem::take(pending_metas);
        metas.sort_by_key(|(after_row, _)| *after_row);
        let mut metas = metas.into_iter().peekable();
        for i in 0..count {
            let old = removed.get(i).cloned();
            let new = added.get(i).cloned();
            // `kind` names the row for the surplus (single-sided) case exactly: `Removed`
            // when only the old side survives, `Added` when only the new side does. A row
            // pairing a removed line with an added line (both sides present, from the same
            // positionwise pairing) also records `Removed`: the widget that renders this
            // row derives each side's own marker and style from which entry is present
            // (UI-R-216) rather than from this field, which for a `Pair` row exists to
            // distinguish `Context` from every other case.
            let kind = if old.is_some() {
                DiffKind::Removed
            } else {
                DiffKind::Added
            };
            rows.push(DiffRow::Pair { kind, old, new });
            while let Some((_, text)) = metas.next_if(|(after_row, _)| *after_row == i + 1) {
                rows.push(DiffRow::Meta { text });
            }
        }
        rows.extend(metas.map(|(_, text)| DiffRow::Meta { text }));
        removed.clear();
        added.clear();
    }

    // Scans only the segment between the header's two `@@` delimiters: a trailing
    // section-heading (`@@ ... @@ fn bar(&self) -> u8 {`) can itself contain a
    // `-`/`+`-prefixed token (`-> u8`) that must never be mistaken for a line-number.
    fn hunk_starts(header: &str) -> (usize, usize) {
        let mut old_start = 1usize;
        let mut new_start = 1usize;
        let Some(rest) = header.strip_prefix("@@") else {
            return (old_start, new_start);
        };
        let counters = rest.split("@@").next().unwrap_or(rest);
        for tok in counters.split_whitespace() {
            if let Some(rest) = tok.strip_prefix('-') {
                old_start = rest.split(',').next().unwrap_or("1").parse().unwrap_or(1);
            } else if let Some(rest) = tok.strip_prefix('+') {
                new_start = rest.split(',').next().unwrap_or("1").parse().unwrap_or(1);
            }
        }
        (old_start, new_start)
    }

    for line in text.split('\n') {
        if line.starts_with("@@") {
            flush(&mut rows, &mut removed, &mut added, &mut pending_metas);
            let (start_old, start_new) = hunk_starts(line);
            old_line = start_old;
            new_line = start_new;
            in_hunk = true;
            rows.push(DiffRow::Meta {
                text: line.to_string(),
            });
            continue;
        }

        let mut chars = line.chars();
        match (in_hunk, chars.next()) {
            (true, Some(' ')) => {
                flush(&mut rows, &mut removed, &mut added, &mut pending_metas);
                let text = chars.as_str().to_string();
                rows.push(DiffRow::Pair {
                    kind: DiffKind::Context,
                    old: Some(DiffEntry {
                        text: text.clone(),
                        line_no: old_line,
                    }),
                    new: Some(DiffEntry {
                        text,
                        line_no: new_line,
                    }),
                });
                old_line += 1;
                new_line += 1;
            }
            (true, Some('+')) => {
                added.push(DiffEntry {
                    text: chars.as_str().to_string(),
                    line_no: new_line,
                });
                new_line += 1;
                last_run_len = added.len();
            }
            (true, Some('-')) => {
                if !added.is_empty() {
                    flush(&mut rows, &mut removed, &mut added, &mut pending_metas);
                }
                removed.push(DiffEntry {
                    text: chars.as_str().to_string(),
                    line_no: old_line,
                });
                old_line += 1;
                last_run_len = removed.len();
            }
            _ if in_hunk && (!removed.is_empty() || !added.is_empty()) => {
                pending_metas.push((last_run_len, line.to_string()));
            }
            _ => {
                flush(&mut rows, &mut removed, &mut added, &mut pending_metas);
                rows.push(DiffRow::Meta {
                    text: line.to_string(),
                });
            }
        }
    }
    flush(&mut rows, &mut removed, &mut added, &mut pending_metas);
    rows
}

impl DiffViewState {
    /// Replaces the diff text, re-parsing it into rows (UI-R-207) and resetting the
    /// cursor, selection and scroll to their defaults. Crate-private: construction from a
    /// unified diff text is this widget's public surface, not a post-construction replace.
    pub(crate) fn set_diff(&mut self, text: &str) {
        self.rows = parse(text);
        self.active_row = 0;
        self.anchor = None;
        self.mode = DiffMode::Normal;
        self.scroll_offset = 0;
        self.h_scroll = 0;
    }

    /// The row and its old/new file line numbers (UI-R-227), reporting `None` for a side
    /// holding a filler (UI-E-100). A meta row answers [`DiffKind::Meta`] with both line
    /// numbers absent.
    pub fn row(&self, index: usize) -> Option<DiffRowInfo> {
        self.rows.get(index).map(|row| match row {
            DiffRow::Meta { .. } => DiffRowInfo {
                kind: DiffKind::Meta,
                old_line: None,
                new_line: None,
            },
            DiffRow::Pair { kind, old, new } => DiffRowInfo {
                kind: kind.clone(),
                old_line: old.as_ref().map(|e| e.line_no),
                new_line: new.as_ref().map(|e| e.line_no),
            },
        })
    }

    /// The active row alone in `Normal`, and the inclusive, ascending-ordered range
    /// between the selection anchor and the active row in `Visual` (UI-R-226). `None`
    /// only when the diff has no rows (UI-E-099).
    pub fn selected_rows(&self) -> Option<RangeInclusive<usize>> {
        if self.rows.is_empty() {
            return None;
        }
        match self.mode {
            DiffMode::Normal => Some(self.active_row..=self.active_row),
            DiffMode::Visual => {
                let anchor = self.anchor.unwrap_or(self.active_row);
                let (lo, hi) = if anchor <= self.active_row {
                    (anchor, self.active_row)
                } else {
                    (self.active_row, anchor)
                };
                Some(lo..=hi)
            }
        }
    }
}

impl DiffViewState {
    /// Read by the widget that renders this state, to know what to draw on each row.
    #[allow(dead_code)]
    pub(crate) fn rows(&self) -> &[DiffRow] {
        &self.rows
    }

    /// Read by the widget that renders this state, to know which row to highlight.
    #[allow(dead_code)]
    pub(crate) fn active_row(&self) -> usize {
        self.active_row
    }

    /// Written by the key-handling code that moves the active row.
    #[allow(dead_code)]
    pub(crate) fn set_active_row(&mut self, row: usize) {
        self.active_row = row;
    }

    /// Written by the key-handling code that opens and moves a Visual selection.
    #[allow(dead_code)]
    pub(crate) fn set_anchor(&mut self, anchor: Option<usize>) {
        self.anchor = anchor;
    }

    /// Read by the widget that renders this state, to know whether a selection is active.
    #[allow(dead_code)]
    pub(crate) fn mode(&self) -> DiffMode {
        self.mode
    }

    /// Written by the key-handling code that enters and leaves Visual mode.
    #[allow(dead_code)]
    pub(crate) fn set_mode(&mut self, mode: DiffMode) {
        self.mode = mode;
    }

    /// Written by the widget that renders this state, recording the visible height so its
    /// own paging can use it.
    #[allow(dead_code)]
    pub(crate) fn set_visible_height(&mut self, height: usize) {
        self.visible_height = height;
    }

    /// Written by the widget that renders this state, recording the content width so its
    /// own horizontal scrolling can use it.
    #[allow(dead_code)]
    pub(crate) fn set_content_width(&mut self, width: usize) {
        self.content_width = width;
    }
}

impl DiffViewStateBuilder {
    /// Builds the state and immediately parses `text` into rows (UI-R-207). The diff text
    /// is not itself a builder field: it is consumed once, here, so the built state never
    /// carries a second copy of it alongside the rows it parsed into.
    pub fn build_with_diff(&self, text: &str) -> Result<DiffViewState, DiffViewStateBuilderError> {
        let mut state = self.build()?;
        state.set_diff(text);
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows_of(text: &str) -> Vec<DiffRow> {
        parse(text)
    }

    #[test]
    /// UI-R-207 — a `-a,b +c,d` hunk header supplies both sides' starting line numbers.
    fn ut_hunk_header_supplies_both_starting_line_numbers() {
        let rows = rows_of("@@ -10,3 +20,4 @@\n context\n-removed\n+added\n");
        let DiffRow::Pair { old, .. } = &rows[1] else {
            panic!("expected a pair row")
        };
        assert_eq!(old.as_ref().unwrap().line_no, 10);
        let DiffRow::Pair { old, new, .. } = &rows[2] else {
            panic!("expected a pair row")
        };
        assert_eq!(old.as_ref().unwrap().line_no, 11);
        assert_eq!(new.as_ref().unwrap().line_no, 21);
    }

    #[test]
    /// UI-R-207 — the count-less `-<old> +<new>` header form is tolerated too.
    fn ut_hunk_header_tolerates_the_count_less_form() {
        let rows = rows_of("@@ -5 +8 @@\n context\n");
        let DiffRow::Pair { old, new, .. } = &rows[1] else {
            panic!("expected a pair row")
        };
        assert_eq!(old.as_ref().unwrap().line_no, 5);
        assert_eq!(new.as_ref().unwrap().line_no, 8);
    }

    #[test]
    /// UI-R-207, UI-R-217 — a hunk header's trailing section-heading text (after the
    /// closing `@@`) is never scanned for line-number tokens, even when it contains a
    /// `-`/`+`-prefixed word of its own (`-> u8`).
    fn ut_hunk_header_ignores_trailing_section_heading_text() {
        let rows = rows_of("@@ -10,7 +20,7 @@ fn bar(&self) -> u8 {\n context\n");
        let DiffRow::Pair { old, new, .. } = &rows[1] else {
            panic!("expected a pair row")
        };
        assert_eq!(old.as_ref().unwrap().line_no, 10);
        assert_eq!(new.as_ref().unwrap().line_no, 20);
    }

    #[test]
    /// UI-R-208 — body lines classify by first character, keeping the remainder as text.
    fn ut_body_lines_classify_by_first_character_and_keep_the_remainder() {
        let rows = rows_of("@@ -1 +1 @@\n unchanged\n-gone\n+new\n");
        let DiffRow::Pair { kind, old, new } = &rows[1] else {
            panic!()
        };
        assert_eq!(*kind, DiffKind::Context);
        assert_eq!(old.as_ref().unwrap().text, "unchanged");
        assert_eq!(new.as_ref().unwrap().text, "unchanged");
        let DiffRow::Pair { kind, old, new } = &rows[2] else {
            panic!()
        };
        assert_eq!(*kind, DiffKind::Removed);
        assert_eq!(old.as_ref().unwrap().text, "gone");
        assert_eq!(new.as_ref().unwrap().text, "new");
    }

    #[test]
    /// UI-R-209 — a removed run pairs positionwise with the added run that follows it.
    fn ut_removed_run_pairs_positionwise_with_the_following_added_run() {
        let rows = rows_of("@@ -1,2 +1,2 @@\n-a\n-b\n+x\n+y\n");
        let DiffRow::Pair { old, new, .. } = &rows[1] else {
            panic!()
        };
        assert_eq!(old.as_ref().unwrap().text, "a");
        assert_eq!(new.as_ref().unwrap().text, "x");
        let DiffRow::Pair { old, new, .. } = &rows[2] else {
            panic!()
        };
        assert_eq!(old.as_ref().unwrap().text, "b");
        assert_eq!(new.as_ref().unwrap().text, "y");
    }

    #[test]
    /// UI-R-209 — a surplus line on either side occupies a row whose other entry is a
    /// filler (`None`).
    fn ut_surplus_line_on_either_side_gets_a_filler_row() {
        let rows = rows_of("@@ -1,3 +1,1 @@\n-a\n-b\n-c\n+x\n");
        let DiffRow::Pair { old, new, .. } = &rows[2] else {
            panic!()
        };
        assert_eq!(old.as_ref().unwrap().text, "b");
        assert!(new.is_none());
        let DiffRow::Pair { old, new, .. } = &rows[3] else {
            panic!()
        };
        assert_eq!(old.as_ref().unwrap().text, "c");
        assert!(new.is_none());

        let rows = rows_of("@@ -1,1 +1,3 @@\n-a\n+x\n+y\n+z\n");
        let DiffRow::Pair { old, new, .. } = &rows[2] else {
            panic!()
        };
        assert!(old.is_none());
        assert_eq!(new.as_ref().unwrap().text, "y");
    }

    #[test]
    /// UI-R-214 — the layout builder option defaults to split.
    fn ut_layout_defaults_to_split() {
        let s = DiffViewStateBuilder::default().build().unwrap();
        assert_eq!(s.layout(), DiffLayout::Split);
    }

    #[test]
    /// UI-R-217 — line numbers count from the hunk header per side, including a context
    /// line reached after an unbalanced removed/added run.
    fn ut_line_numbers_count_from_the_hunk_header_per_side() {
        let rows = rows_of("@@ -10,3 +20,1 @@\n-a\n-b\n-c\n+x\n context\n");
        let DiffRow::Pair { old, new, .. } = rows.last().unwrap() else {
            panic!()
        };
        assert_eq!(old.as_ref().unwrap().line_no, 13);
        assert_eq!(new.as_ref().unwrap().line_no, 21);
    }

    #[test]
    /// UI-R-226 — selected rows is the active row alone in Normal, and the ordered
    /// inclusive anchor range in Visual, whether the anchor sits above or below it.
    fn ut_selected_rows_is_the_active_row_in_normal_and_the_ordered_anchor_range_in_visual() {
        let mut s = DiffViewStateBuilder::default().build().unwrap();
        s.set_diff("@@ -1,5 +1,5 @@\n a\n b\n c\n d\n e\n");
        s.set_active_row(2);
        assert_eq!(s.selected_rows(), Some(2..=2));

        s.set_mode(DiffMode::Visual);
        s.set_anchor(Some(0));
        assert_eq!(s.selected_rows(), Some(0..=2));

        s.set_anchor(Some(4));
        assert_eq!(s.selected_rows(), Some(2..=4));
    }

    #[test]
    /// UI-R-227, UI-E-100 — the row query reports kind and per-side line numbers, `None`
    /// for a filler side.
    fn ut_row_query_reports_kind_and_per_side_line_numbers_with_none_for_a_filler() {
        let mut s = DiffViewStateBuilder::default().build().unwrap();
        s.set_diff("@@ -1,1 +1,2 @@\n-a\n+x\n+y\n");
        assert_eq!(
            s.row(0),
            Some(DiffRowInfo {
                kind: DiffKind::Meta,
                old_line: None,
                new_line: None
            })
        );
        assert_eq!(
            s.row(1),
            Some(DiffRowInfo {
                kind: DiffKind::Removed,
                old_line: Some(1),
                new_line: Some(1)
            })
        );
        assert_eq!(
            s.row(2),
            Some(DiffRowInfo {
                kind: DiffKind::Added,
                old_line: None,
                new_line: Some(2)
            })
        );
        assert_eq!(s.row(99), None);
    }

    #[test]
    /// UI-E-097 — a line before any hunk header is kept as a meta row.
    fn ut_line_before_any_hunk_header_is_kept_as_a_meta_row() {
        let rows =
            rows_of("diff --git a/f b/f\nindex 0..1\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n context\n");
        assert!(matches!(rows[0], DiffRow::Meta { .. }));
        assert!(matches!(rows[3], DiffRow::Meta { .. }));
    }

    #[test]
    /// UI-E-097 — an unrecognized line inside a hunk is kept as a meta row, no error.
    fn ut_unrecognized_line_is_kept_as_a_meta_row_without_error() {
        let rows = rows_of("@@ -1 +1 @@\n context\n???unexpected\n");
        assert!(matches!(rows.last().unwrap(), DiffRow::Meta { text } if text == "???unexpected"));
    }

    #[test]
    /// UI-E-098 — the "no newline" marker is a meta row following the line it belongs to.
    fn ut_no_newline_marker_is_a_meta_row_after_the_line_it_belongs_to() {
        let rows = rows_of("@@ -1 +1 @@\n-a\n\\ No newline at end of file\n");
        let last = rows.last().unwrap();
        assert!(matches!(last, DiffRow::Meta { text } if text == "\\ No newline at end of file"));
    }

    #[test]
    /// UI-R-209, UI-E-098 — a meta line arriving between a removed run and the added run
    /// that follows it (the "no newline" marker's usual position) does not break the
    /// positionwise pairing between them.
    fn ut_meta_line_between_a_removed_and_added_run_does_not_break_their_pairing() {
        let rows = rows_of("@@ -1,1 +1,1 @@\n-old\n\\ No newline at end of file\n+new\n");
        let DiffRow::Pair { old, new, .. } = &rows[1] else {
            panic!("expected old and new paired on one row")
        };
        assert_eq!(old.as_ref().unwrap().text, "old");
        assert_eq!(new.as_ref().unwrap().text, "new");
        assert!(
            matches!(&rows[2], DiffRow::Meta { text } if text == "\\ No newline at end of file")
        );
    }

    #[test]
    /// UI-E-099 — an empty diff has no rows, and selection reports none.
    fn ut_empty_diff_has_no_rows_and_reports_no_selection() {
        let s = DiffViewStateBuilder::default().build().unwrap();
        assert!(s.rows().is_empty());
        assert_eq!(s.selected_rows(), None);
    }

    #[test]
    /// UI-E-099 — the parser itself yields no rows for both the truly empty string and a
    /// lone trailing newline, exercising the early return directly rather than only
    /// through a state that never called it.
    fn ut_parser_yields_no_rows_for_empty_text_or_a_lone_newline() {
        assert!(rows_of("").is_empty());
        assert!(rows_of("\n").is_empty());
    }

    #[test]
    /// UI-R-207 — `build_with_diff` is the widget's public construction path: it parses
    /// the given text into rows in one step, alongside whatever else the builder set.
    fn ut_build_with_diff_parses_the_given_text_into_rows() {
        let s = DiffViewStateBuilder::default()
            .build_with_diff("@@ -1,1 +1,1 @@\n context\n")
            .unwrap();
        assert_eq!(s.rows().len(), 2);
        assert!(matches!(s.rows()[0], DiffRow::Meta { .. }));
        assert!(matches!(
            s.rows()[1],
            DiffRow::Pair {
                kind: DiffKind::Context,
                ..
            }
        ));
    }

    #[test]
    /// UI-E-098 — a meta line following an added line inside a longer removed run is
    /// positioned after that added line's own row, not after the run's last (filler) row.
    fn ut_meta_after_an_added_line_lands_on_that_lines_own_row_not_the_runs_last_row() {
        let rows = rows_of("@@ -1,2 +1,1 @@\n-a\n-b\n+x\n\\ No newline at end of file\n");
        let DiffRow::Pair { old, new, .. } = &rows[1] else {
            panic!("expected the a/x pair row")
        };
        assert_eq!(old.as_ref().unwrap().text, "a");
        assert_eq!(new.as_ref().unwrap().text, "x");
        assert!(
            matches!(&rows[2], DiffRow::Meta { text } if text == "\\ No newline at end of file"),
            "the marker follows the row holding the added line it belongs to, not the \
             later filler-only row"
        );
    }
}
