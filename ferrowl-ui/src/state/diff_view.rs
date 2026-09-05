use std::ops::RangeInclusive;

use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};

use super::vim::emit_osc52;
use crate::EventResult;
use crate::traits::HandleEvents;

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
    /// The first key of a `gg`, `yy`, `]c` or `[c` chord, awaiting its second.
    #[getset(skip)]
    #[builder(setter(skip), default)]
    pending: Option<char>,
    /// The count prefix accumulated ahead of `j`/`k` (UI-R-230).
    #[getset(skip)]
    #[builder(setter(skip), default)]
    pending_count: Option<usize>,
    /// The text a yank (`yy`/`y`) last copied (UI-R-229), for tests to assert without
    /// adding public surface no `api-contract.md` row names.
    #[getset(skip)]
    #[builder(setter(skip), default)]
    register: Option<String>,
    /// Vertical scroll offset in rows. Read and written by the widget's own navigation
    /// code, keeping the active row visible.
    #[getset(skip)]
    #[builder(setter(skip), default)]
    scroll_offset: usize,
    /// Horizontal scroll offset in columns, one shared by every pane (UI-R-232). Read and
    /// written by the widget's own navigation code.
    #[getset(skip)]
    #[builder(setter(skip), default)]
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
    /// Syntax language highlighting both sides' text; `None` by default (UI-R-220,
    /// UI-R-221). A diff is never between two languages — a file has one — so this is one
    /// field, not a pair. Crate-private field, read directly by the widget that renders
    /// this state: no `api-contract.md` row needs a getter, only the builder setter.
    #[builder(default = "None")]
    pub(crate) language: Option<ferrowl_syntax::Language>,
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
        self.pending = None;
        self.pending_count = None;
        self.register = None;
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

    /// Read by the widget that renders this state, to know which aligned row to draw
    /// first (UI-R-230).
    pub(crate) fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Written by the key-handling code that moves the vertical scroll offset. No
    /// non-test caller exists until that key handling lands, so a render test can place a
    /// nonzero offset before it does.
    #[allow(dead_code)]
    pub(crate) fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset;
    }

    /// Read by the widget that renders this state, to know how many leading columns of
    /// each entry's text to drop (UI-R-232).
    pub(crate) fn h_scroll(&self) -> usize {
        self.h_scroll
    }

    /// Written by the key-handling code that moves the horizontal scroll offset. No
    /// non-test caller exists until that key handling lands, so a render test can place a
    /// nonzero offset before it does.
    #[allow(dead_code)]
    pub(crate) fn set_h_scroll(&mut self, offset: usize) {
        self.h_scroll = offset;
    }

    /// The text the last yank copied (UI-R-229), for tests: no `api-contract.md` row
    /// names a register query, since the clipboard (OSC 52) is the yank's real output.
    /// No non-test caller exists, since the register has no reader but the clipboard.
    #[allow(dead_code)]
    pub(crate) fn register(&self) -> Option<&str> {
        self.register.as_deref()
    }
}

impl DiffViewState {
    fn clamp_active_row(&mut self) {
        self.active_row = if self.rows.is_empty() {
            0
        } else {
            self.active_row.min(self.rows.len() - 1)
        };
    }

    /// Whether aligned row `index` draws as two screen rows in the unified layout: a
    /// `Pair` with both sides present and differing text (the same test
    /// `widgets/diff_view.rs`'s unified renderer uses); everything else draws as one.
    fn row_screen_height(&self, index: usize) -> usize {
        match self.rows.get(index) {
            Some(DiffRow::Pair {
                old: Some(o),
                new: Some(n),
                ..
            }) if self.layout == DiffLayout::Unified && o.text != n.text => 2,
            _ => 1,
        }
    }

    /// The last aligned row index still inside `visible_height` screen rows starting at
    /// `from`: in split layout one aligned row is one screen row, but in unified layout a
    /// changed pair draws as two, so counting aligned rows alone can place the window a
    /// row short of where the renderer actually stops.
    fn last_visible_row_from(&self, from: usize) -> usize {
        let mut used = 0usize;
        let mut last = from;
        for i in from..self.rows.len() {
            let height = self.row_screen_height(i);
            if used + height > self.visible_height.max(1) {
                break;
            }
            used += height;
            last = i;
        }
        last
    }

    /// Keeps `active_row` inside the last-rendered visible window (UI-R-230), the same
    /// remembered-height paging scheme the code editor's `page_move`/`handle_readonly_nav`
    /// use, adjusted for the unified layout's two-screen-row entries.
    fn ensure_visible(&mut self) {
        if self.active_row < self.scroll_offset {
            self.scroll_offset = self.active_row;
            return;
        }
        while self.active_row > self.last_visible_row_from(self.scroll_offset) {
            self.scroll_offset += 1;
        }
    }

    fn move_active_row_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() as isize - 1;
        self.active_row = (self.active_row as isize + delta).clamp(0, last) as usize;
        self.ensure_visible();
    }

    fn move_active_row_to(&mut self, row: usize) {
        self.active_row = row;
        self.clamp_active_row();
        self.ensure_visible();
    }

    /// Mirrors `code_input_field.rs`'s `page_move`: moves by at least one row, clamped at
    /// the first and last row.
    fn page_move(&mut self, down: bool, rows: usize) {
        let rows = rows.max(1) as isize;
        self.move_active_row_by(if down { rows } else { -rows });
    }

    /// The last column of the widest rendered text across every row and both sides
    /// (UI-R-232), the horizontal-scroll clamp for `h`/`l`/`Left`/`Right`. Mirrors
    /// `code_input_field.rs`'s `max_h_scroll`, widened to both sides and meta rows.
    fn max_h_scroll(&self) -> usize {
        self.rows
            .iter()
            .map(|row| match row {
                DiffRow::Meta { text } => text.chars().count(),
                DiffRow::Pair { old, new, .. } => {
                    let o = old.as_ref().map_or(0, |e| e.text.chars().count());
                    let n = new.as_ref().map_or(0, |e| e.text.chars().count());
                    o.max(n)
                }
            })
            .max()
            .unwrap_or(0)
            .saturating_sub(1)
    }

    /// The active row's own text length on `side`, `0` for a meta row or a filler.
    fn active_row_text_len(&self, side: Side) -> usize {
        match self.rows.get(self.active_row) {
            Some(DiffRow::Pair { old, new, .. }) => {
                let entry = match side {
                    Side::Old => old,
                    Side::New => new,
                };
                entry.as_ref().map_or(0, |e| e.text.chars().count())
            }
            _ => 0,
        }
    }

    /// The row indices of every hunk header (a `DiffRow::Meta` starting `@@`), a hunk
    /// boundary (UI-R-233).
    fn hunk_header_rows(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, DiffRow::Meta { text } if text.starts_with("@@")))
            .map(|(i, _)| i)
            .collect()
    }

    /// `]c` (UI-R-233): the first row of the next hunk, clamping at the last hunk.
    fn jump_to_next_hunk(&mut self) {
        let headers = self.hunk_header_rows();
        if let Some(&h) = headers.iter().find(|&&h| h + 1 > self.active_row) {
            self.move_active_row_to(h + 1);
        } else if let Some(&last) = headers.last() {
            self.move_active_row_to(last + 1);
        }
    }

    /// `[c` (UI-R-233): the first row of the previous hunk, clamping at the first hunk.
    fn jump_to_prev_hunk(&mut self) {
        let headers = self.hunk_header_rows();
        if let Some(&h) = headers.iter().rev().find(|&&h| h + 1 < self.active_row) {
            self.move_active_row_to(h + 1);
        } else if let Some(&first) = headers.first() {
            self.move_active_row_to(first + 1);
        }
    }

    /// `yy`/`y` (UI-R-229): the focused side's text of the selected rows, skipping a row
    /// whose focused side holds a filler, joined and copied to `register` and the system
    /// clipboard via the code editor's own best-effort OSC 52 path.
    fn yank(&mut self) {
        let Some(range) = self.selected_rows() else {
            return;
        };
        let side = self.focused_side;
        let mut lines = Vec::new();
        for i in range {
            if let Some(DiffRow::Pair { old, new, .. }) = self.rows.get(i) {
                let entry = match side {
                    Side::Old => old,
                    Side::New => new,
                };
                if let Some(e) = entry {
                    lines.push(e.text.clone());
                }
            }
        }
        let text = lines.join("\n");
        emit_osc52(&text);
        self.register = Some(text);
    }
}

impl HandleEvents for DiffViewState {
    /// The diff widget holds no editable buffer (UI-R-222): every mutating and
    /// Insert-entering key — `i`, `a`, `o`, `x`, `p`, `d` and any printable character —
    /// simply falls through to the catch-all below and is reported unhandled by
    /// construction, with no per-key ignore arm written for any of them.
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        if modifiers == KeyModifiers::NONE
            && code == KeyCode::Char('c')
            && matches!(self.pending, Some('[') | Some(']'))
        {
            let next = self.pending == Some(']');
            self.pending = None;
            if next {
                self.jump_to_next_hunk();
            } else {
                self.jump_to_prev_hunk();
            }
            return EventResult::Consumed;
        }
        if modifiers == KeyModifiers::NONE
            && matches!(code, KeyCode::Char('[') | KeyCode::Char(']'))
        {
            self.pending = Some(if code == KeyCode::Char(']') { ']' } else { '[' });
            return EventResult::Consumed;
        }

        if modifiers == KeyModifiers::NONE && code == KeyCode::Char('g') {
            if self.pending == Some('g') {
                self.pending = None;
                self.pending_count = None;
                self.move_active_row_to(0);
            } else {
                self.pending = Some('g');
            }
            return EventResult::Consumed;
        }

        if modifiers == KeyModifiers::NONE && code == KeyCode::Char('y') {
            if self.mode == DiffMode::Visual {
                self.yank();
                self.mode = DiffMode::Normal;
                self.anchor = None;
            } else if self.pending == Some('y') {
                self.pending = None;
                self.yank();
            } else {
                self.pending = Some('y');
            }
            return EventResult::Consumed;
        }

        self.pending = None;

        if modifiers == KeyModifiers::NONE
            && let KeyCode::Char(c @ '1'..='9') = code
        {
            let digit = c as usize - '0' as usize;
            self.pending_count = Some(self.pending_count.unwrap_or(0) * 10 + digit);
            return EventResult::Consumed;
        }
        match (modifiers, code) {
            (KeyModifiers::NONE, KeyCode::Char('j')) => {
                let count = self.pending_count.take().unwrap_or(1).max(1) as isize;
                self.move_active_row_by(count);
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Char('k')) => {
                let count = self.pending_count.take().unwrap_or(1).max(1) as isize;
                self.move_active_row_by(-count);
                EventResult::Consumed
            }
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('G')) => {
                self.pending_count = None;
                let last = self.rows.len().saturating_sub(1);
                self.move_active_row_to(last);
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::PageDown) => {
                self.page_move(true, self.visible_height);
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::PageUp) => {
                self.page_move(false, self.visible_height);
                EventResult::Consumed
            }
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
                self.page_move(true, (self.visible_height / 2).max(1));
                EventResult::Consumed
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                self.page_move(false, (self.visible_height / 2).max(1));
                EventResult::Consumed
            }
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('h')) => {
                self.h_scroll = self.h_scroll.saturating_sub(1);
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Left) => {
                self.h_scroll = self.h_scroll.saturating_sub(1);
                EventResult::Consumed
            }
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('l')) => {
                self.h_scroll = (self.h_scroll + 1).min(self.max_h_scroll());
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Right) => {
                self.h_scroll = (self.h_scroll + 1).min(self.max_h_scroll());
                EventResult::Consumed
            }
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('0')) => {
                self.h_scroll = 0;
                EventResult::Consumed
            }
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('$')) => {
                let len = self.active_row_text_len(self.focused_side);
                let last = len.saturating_sub(1);
                self.h_scroll = (last + 1).saturating_sub(self.content_width);
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Char('v')) if self.mode == DiffMode::Normal => {
                self.mode = DiffMode::Visual;
                self.anchor = Some(self.active_row);
                EventResult::Consumed
            }
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('V'))
                if self.mode == DiffMode::Normal =>
            {
                self.mode = DiffMode::Visual;
                self.anchor = Some(self.active_row);
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Esc) if self.mode == DiffMode::Visual => {
                self.mode = DiffMode::Normal;
                self.anchor = None;
                EventResult::Consumed
            }
            (KeyModifiers::CONTROL, KeyCode::Char('t')) => {
                self.layout = match self.layout {
                    DiffLayout::Split => DiffLayout::Unified,
                    DiffLayout::Unified => DiffLayout::Split,
                };
                EventResult::Consumed
            }
            _ => EventResult::Unhandled(modifiers, code),
        }
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

    /// Two hunks, an unbalanced first one (filler on the old side), a context row in
    /// each: row 0 header, row 1 "a" (context), row 2 old "b"/new "x" (a change), row 3
    /// filler-old/new "y" (a surplus add), row 4 the second header, row 5 "c" (context).
    fn nav_fixture() -> DiffViewState {
        let mut s = DiffViewStateBuilder::default()
            .build_with_diff("@@ -1,2 +1,3 @@\n a\n-b\n+x\n+y\n@@ -5,1 +7,1 @@\n c\n")
            .unwrap();
        s.set_visible_height(3);
        s.set_content_width(10);
        s
    }

    #[test]
    /// UI-R-215, UI-E-101 — `Ctrl+T` toggles between split and unified, leaving the
    /// active row and any Visual selection exactly where they were.
    fn ut_ctrl_t_toggles_layout_leaving_the_active_row_and_selection_untouched() {
        let mut s = nav_fixture();
        s.set_active_row(2);
        s.set_mode(DiffMode::Visual);
        s.set_anchor(Some(1));
        assert_eq!(s.layout(), DiffLayout::Split);

        assert!(matches!(
            s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('t')),
            EventResult::Consumed
        ));
        assert_eq!(s.layout(), DiffLayout::Unified);
        assert_eq!(s.active_row(), 2);
        assert_eq!(s.selected_rows(), Some(1..=2));

        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('t'));
        assert_eq!(s.layout(), DiffLayout::Split);
        assert_eq!(s.active_row(), 2);
        assert_eq!(s.selected_rows(), Some(1..=2));
    }

    #[test]
    /// UI-R-222 — the diff widget holds no buffer, so every mutating and Insert-entering
    /// key is reported unhandled: `i`, `a`, `o`, `x`, `p`, `dd` (each press) and a
    /// printable character that maps to none of the widget's own keys.
    fn ut_mutating_and_insert_entering_keys_are_reported_unhandled() {
        for code in [
            KeyCode::Char('i'),
            KeyCode::Char('a'),
            KeyCode::Char('o'),
            KeyCode::Char('x'),
            KeyCode::Char('p'),
            KeyCode::Char('d'),
            KeyCode::Char('z'),
        ] {
            let mut s = nav_fixture();
            assert!(
                matches!(
                    s.handle_events(KeyModifiers::NONE, code),
                    EventResult::Unhandled(KeyModifiers::NONE, c) if c == code
                ),
                "{code:?} should be unhandled"
            );
        }
        let mut s = nav_fixture();
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('d'));
        assert!(matches!(
            s.handle_events(KeyModifiers::NONE, KeyCode::Char('d')),
            EventResult::Unhandled(KeyModifiers::NONE, KeyCode::Char('d'))
        ));
    }

    #[test]
    /// UI-R-223 — `v`/`V` from Normal enter Visual, `Esc` in Visual returns to Normal,
    /// and `Esc` in Normal is unhandled so it reaches the enclosing layer.
    fn ut_v_and_shift_v_enter_visual_esc_returns_to_normal_and_esc_in_normal_is_unhandled() {
        let mut s = nav_fixture();
        assert!(matches!(
            s.handle_events(KeyModifiers::NONE, KeyCode::Esc),
            EventResult::Unhandled(KeyModifiers::NONE, KeyCode::Esc)
        ));

        s.handle_events(KeyModifiers::NONE, KeyCode::Char('v'));
        assert_eq!(s.mode(), DiffMode::Visual);
        assert!(matches!(
            s.handle_events(KeyModifiers::NONE, KeyCode::Esc),
            EventResult::Consumed
        ));
        assert_eq!(s.mode(), DiffMode::Normal);

        s.handle_events(KeyModifiers::SHIFT, KeyCode::Char('V'));
        assert_eq!(s.mode(), DiffMode::Visual);
        s.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert_eq!(s.mode(), DiffMode::Normal);
    }

    #[test]
    /// UI-R-229 — `yy`/`y` copy the focused side's text of the selected rows into the
    /// register, skipping a row whose focused side is a filler.
    fn ut_yank_copies_the_focused_sides_selected_text_skipping_filler_rows() {
        let mut s = nav_fixture();
        s.set_focused_side(Side::New);
        s.set_mode(DiffMode::Visual);
        s.set_active_row(1);
        s.set_anchor(Some(3));
        // Rows 1..=3: "a" (context, both sides), "x" (change, new side), "y" (surplus
        // add, old side a filler — but the focused side is New here, so nothing skips).
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('y'));
        assert_eq!(s.register(), Some("a\nx\ny"));
        assert_eq!(s.mode(), DiffMode::Normal);

        // Normal-mode `yy` on a single row whose focused (New) side holds a filler
        // skips it, yielding an empty register content for that row alone.
        let mut s = nav_fixture();
        s.set_focused_side(Side::Old);
        s.set_active_row(3);
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('y'));
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('y'));
        assert_eq!(s.register(), Some(""));
    }

    #[test]
    /// UI-R-230 (movement half) — `j`/`k`, their count prefixes, `gg` and `G` move the
    /// active row and keep it inside the visible window.
    fn ut_j_k_counts_gg_and_g_move_the_active_row_and_keep_it_visible() {
        let mut s = nav_fixture();
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.active_row(), 1);

        s.handle_events(KeyModifiers::NONE, KeyCode::Char('2'));
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.active_row(), 3);

        s.handle_events(KeyModifiers::NONE, KeyCode::Char('k'));
        assert_eq!(s.active_row(), 2);

        s.handle_events(KeyModifiers::SHIFT, KeyCode::Char('G'));
        assert_eq!(s.active_row(), 5);
        assert!(
            s.scroll_offset() + 3 > s.active_row(),
            "active row must stay visible"
        );

        s.handle_events(KeyModifiers::NONE, KeyCode::Char('g'));
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('g'));
        assert_eq!(s.active_row(), 0);
        assert_eq!(s.scroll_offset(), 0);

        // Clamped at the last row: moving past it stays put.
        s.set_active_row(5);
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.active_row(), 5);
    }

    #[test]
    /// UI-R-231 — paging moves by the visible height and half of it, clamping at the
    /// first and last row.
    fn ut_paging_moves_by_the_visible_height_and_half_of_it_and_clamps() {
        let mut s = nav_fixture();
        s.handle_events(KeyModifiers::NONE, KeyCode::PageDown);
        assert_eq!(s.active_row(), 3);
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('u'));
        assert_eq!(s.active_row(), 2);
        s.handle_events(KeyModifiers::NONE, KeyCode::PageDown);
        assert_eq!(s.active_row(), 5, "clamped at the last row");
        s.handle_events(KeyModifiers::NONE, KeyCode::PageUp);
        assert_eq!(s.active_row(), 2);
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('d'));
        assert_eq!(s.active_row(), 3, "Ctrl+D moves by half the visible height");
        for _ in 0..10 {
            s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('u'));
        }
        assert_eq!(s.active_row(), 0, "clamped at the first row");
    }

    #[test]
    /// UI-R-232 (movement half) — `h`/`l`/`Left`/`Right`/`0` move one shared horizontal
    /// offset, applying to every pane alike (asserted here by reading `h_scroll` itself).
    fn ut_horizontal_keys_move_one_shared_offset_for_every_pane() {
        let mut s = nav_fixture();
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('l'));
        assert_eq!(s.h_scroll(), 1);
        s.handle_events(KeyModifiers::NONE, KeyCode::Right);
        assert_eq!(s.h_scroll(), 2);
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('h'));
        assert_eq!(s.h_scroll(), 1);
        s.handle_events(KeyModifiers::NONE, KeyCode::Left);
        assert_eq!(s.h_scroll(), 0);
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('h'));
        assert_eq!(s.h_scroll(), 0, "clamped at zero");

        for _ in 0..50 {
            s.handle_events(KeyModifiers::NONE, KeyCode::Char('l'));
        }
        let max = s.h_scroll();
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('l'));
        assert_eq!(s.h_scroll(), max, "clamped at the widest rendered text");

        s.handle_events(KeyModifiers::NONE, KeyCode::Char('0'));
        assert_eq!(s.h_scroll(), 0);
    }

    #[test]
    /// UI-R-232 (movement half) — `$` brings the active row's focused-side last column
    /// into view, computed from the remembered content width so it works even unfocused
    /// (the code editor's UI-R-179 exception, mirrored here).
    fn ut_dollar_brings_the_last_column_into_view_unfocused_too() {
        // A row wider than the content width, so `$` must produce a nonzero offset: an
        // implementation with the `$` arm deleted (or a no-op) would otherwise still pass
        // an assertion built only from rows shorter than the content width.
        let mut s = DiffViewStateBuilder::default()
            .build_with_diff("@@ -1,1 +1,1 @@\n-short\n+a much longer line of text\n")
            .unwrap();
        s.set_content_width(5);
        s.set_focused_side(Side::Old);
        s.set_active_row(1); // old text "short", length 5
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('$'));
        assert_eq!(
            s.h_scroll(),
            0,
            "text no longer than the content width needs no scroll"
        );

        s.set_focused_side(Side::New);
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('$'));
        // new text "a much longer line of text" is 26 characters; last_col = 25, so
        // h_scroll = (25 + 1) - 5 = 21.
        assert_eq!(s.h_scroll(), 21);

        // Computed from the remembered content width (UI-R-179's mechanism), not a live
        // render: changing it directly still moves `$`'s result.
        s.set_content_width(10);
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('$'));
        assert_eq!(s.h_scroll(), 16);
    }

    #[test]
    /// UI-R-233 — `]c`/`[c` move to the first row of the next/previous hunk, clamping at
    /// the last and first hunk.
    fn ut_bracket_c_moves_to_the_next_and_previous_hunk_and_clamps_at_both_ends() {
        let mut s = nav_fixture();
        s.handle_events(KeyModifiers::NONE, KeyCode::Char(']'));
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('c'));
        assert_eq!(s.active_row(), 1, "the row after the first hunk's header");

        s.handle_events(KeyModifiers::NONE, KeyCode::Char(']'));
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('c'));
        assert_eq!(s.active_row(), 5, "the row after the second hunk's header");

        s.handle_events(KeyModifiers::NONE, KeyCode::Char(']'));
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('c'));
        assert_eq!(s.active_row(), 5, "clamped at the last hunk");

        s.handle_events(KeyModifiers::NONE, KeyCode::Char('['));
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('c'));
        assert_eq!(s.active_row(), 1, "the previous hunk's first row");

        s.handle_events(KeyModifiers::NONE, KeyCode::Char('['));
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('c'));
        assert_eq!(s.active_row(), 1, "clamped at the first hunk");
    }

    #[test]
    /// UI-R-230, UI-R-231 — in unified layout a changed pair (both sides present,
    /// differing text) draws as two screen rows, so keeping the active row visible must
    /// count screen rows, not aligned rows: three screen rows of visible height can hold
    /// row 0 and row 1 alone, or rows 1 and 2 (row 2 costing two), but never all three.
    fn ut_unified_paging_accounts_for_two_screen_row_entries() {
        let mut s = DiffViewStateBuilder::default()
            .layout(DiffLayout::Unified)
            .build_with_diff("@@ -1,2 +1,3 @@\n a\n-b\n+x\n+y\n@@ -5,1 +7,1 @@\n c\n")
            .unwrap();
        s.set_visible_height(3);
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('j'));
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.active_row(), 2);
        assert_eq!(
            s.scroll_offset(),
            1,
            "row 2 costs two screen rows, so row 0 must scroll off to fit rows 1 and 2 \
             within a 3-screen-row window"
        );
    }
}
