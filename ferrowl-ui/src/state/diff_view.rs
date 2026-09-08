use std::ops::RangeInclusive;

use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};
use ratatui::style::{Color, Style};

use super::vim::emit_osc52;
use crate::EventResult;
use crate::traits::HandleEvents;
use crate::widgets::markdown_render::word_wrap;

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

/// A colour-filled span of one side's file line numbers (UI-R-266): named by file line, not
/// row index, so a consumer that knows only the file — not this widget's row layout — can
/// mark it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkedRange {
    pub side: Side,
    pub lines: RangeInclusive<usize>,
    pub color: Color,
}

/// A block of markdown text anchored to one side's file line range (UI-R-269): named by
/// file line, not row index, for the same reason as [`MarkedRange`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    pub side: Side,
    pub lines: RangeInclusive<usize>,
    pub text: String,
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
    /// logic. The maximum of `content_width_old` and `content_width_new`, kept for the
    /// horizontal-scroll arithmetic (UI-R-232) which applies one offset to both panes.
    #[getset(skip)]
    #[builder(setter(skip), default = "1")]
    #[allow(dead_code)]
    content_width: usize,
    /// The old side's own content width, one column before the first render. Read by
    /// `display_rows()` so each side wraps at its own edge (UI-R-262) rather than the
    /// wider pane's.
    #[getset(skip)]
    #[builder(setter(skip), default = "1")]
    content_width_old: usize,
    /// The new side's own content width, one column before the first render; same reason
    /// as `content_width_old`.
    #[getset(skip)]
    #[builder(setter(skip), default = "1")]
    content_width_new: usize,
    /// A meta row's own available width, one column before the first render: a meta row
    /// carries no gutter or marker, so its wrapping width is the full row rect rather than
    /// either side's post-gutter `content_width_old`/`content_width_new`.
    #[getset(skip)]
    #[builder(setter(skip), default = "1")]
    content_width_meta: usize,
    /// Line-wrap option (UI-R-260), defaulting to off.
    #[getset(get_copy = "pub")]
    #[builder(default = "false")]
    wrap: bool,
    /// Full-file display mode (UI-R-257), defaulting to hunk-only.
    #[getset(get_copy = "pub")]
    #[builder(default = "DiffDisplay::HunkOnly")]
    display: DiffDisplay,
    /// Row-index spans outside every hunk, unreachable and undrawn while `display` is
    /// `HunkOnly` (UI-R-255, UI-R-256); empty when built from a diff alone (UI-R-259).
    #[getset(skip)]
    #[builder(setter(skip), default)]
    folds: Vec<RangeInclusive<usize>>,
    /// Whether this state was built with the full new-side text (UI-R-207): `false` means
    /// there is no full-file mode to switch to, so `Ctrl+F` is consumed and ignored
    /// (UI-E-113, UI-R-259) regardless of `display`'s value.
    #[getset(skip)]
    #[builder(setter(skip), default)]
    has_full_file: bool,
    /// Colour-filled file line ranges painted over the gutter (UI-R-266), settable when
    /// built and afterwards.
    #[getset(get = "pub", set = "pub")]
    #[builder(default)]
    marked_ranges: Vec<MarkedRange>,
    /// Markdown blocks anchored to a file line range (UI-R-269), settable when built and
    /// afterwards.
    #[getset(get = "pub", set = "pub")]
    #[builder(default)]
    annotations: Vec<Annotation>,
    /// Whether annotations contribute display rows (UI-R-274, UI-R-275); shown by default,
    /// toggled only by `Ctrl+A`, never through the builder.
    #[getset(skip)]
    #[builder(setter(skip), default = "true")]
    annotations_shown: bool,
    /// Each annotation's own measured display-row count at its last render, indexed like
    /// `annotations`; empty before the first render, the same pre-render convention
    /// `visible_height` uses (UI-E-084's rule), so `display_rows()` adds no annotation rows
    /// until the widget has measured them.
    #[getset(skip)]
    #[builder(setter(skip), default)]
    annotation_heights: Vec<usize>,
}

/// The diff widget's hunk-only or full-file display mode (UI-R-257).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffDisplay {
    HunkOnly,
    FullFile,
}

/// One screen line the diff widget draws: a logical row
/// ([`DiffRow`]) may span several of these when wrapped (UI-R-260) or, in the unified
/// layout, when it is a changed pair drawn as two entries (UI-R-213).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DisplayRow {
    pub(crate) logical: usize,
    pub(crate) part: RowPart,
}

/// Which part of a logical row a [`DisplayRow`] draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowPart {
    /// A meta row's screen line: `sub_row` indexes its own wrapped chunk list, a meta row
    /// wrapping like any other row (UI-R-260 exempts none).
    Meta { sub_row: usize },
    /// A pair row's screen line: `old_sub`/`new_sub` index into that side's own wrapped
    /// chunk list (`None` when that side draws nothing on this display row — the unified
    /// layout draws one side's lines, then the other's, sequentially, and the split
    /// layout pads the shorter side's remaining rows blank, UI-R-262). A side's `Some(0)`
    /// carries the gutter; any later sub-row is a wrapped continuation (UI-R-261).
    Pair {
        old_sub: Option<usize>,
        new_sub: Option<usize>,
    },
    /// One screen line of an annotation block anchored beneath the last row of its range
    /// (UI-R-270): `index` names the annotation in `annotations()`, `sub_row` its own
    /// border/text row. Carries no logical row of its own — the `DisplayRow` it rides on
    /// reuses the anchor row's logical index, which is what keeps an annotation out of
    /// `active_row_display_span` (UI-R-273) while still shifting the scroll bookkeeping
    /// that counts display rows (UI-R-265).
    Annotation { index: usize, sub_row: usize },
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

/// Parses a hunk header's declared old/new starting line and line count (defaulting an
/// omitted count to 1, the unified-diff convention for a one-line range).
fn hunk_header(header: &str) -> (usize, usize, usize, usize) {
    let mut old_start = 1usize;
    let mut old_count = 1usize;
    let mut new_start = 1usize;
    let mut new_count = 1usize;
    let Some(rest) = header.strip_prefix("@@") else {
        return (old_start, old_count, new_start, new_count);
    };
    let counters = rest.split("@@").next().unwrap_or(rest);
    for tok in counters.split_whitespace() {
        if let Some(rest) = tok.strip_prefix('-') {
            let mut parts = rest.splitn(2, ',');
            old_start = parts.next().unwrap_or("1").parse().unwrap_or(1);
            old_count = parts.next().map_or(1, |c| c.parse().unwrap_or(1));
        } else if let Some(rest) = tok.strip_prefix('+') {
            let mut parts = rest.splitn(2, ',');
            new_start = parts.next().unwrap_or("1").parse().unwrap_or(1);
            new_count = parts.next().map_or(1, |c| c.parse().unwrap_or(1));
        }
    }
    (old_start, old_count, new_start, new_count)
}

/// Builds the full-file row superset (amended UI-R-209, UI-R-253): the hunk-only rows
/// `parse(diff)` already produces, plus a further row for every line of `new_text` that no
/// hunk covers, its old-side number offset by the cumulative line-count delta of the hunks
/// before it. Runs no diff algorithm and no similarity heuristic: it only walks `new_text`
/// line by line, consulting each hunk's own declared line range to know which of those
/// lines the patch already classifies. Returns the row list together with the row-index
/// spans of the rows this walk inserted (the folds of UI-R-255).
fn parse_full_file(diff: &str, new_text: &str) -> (Vec<DiffRow>, Vec<RangeInclusive<usize>>) {
    let hunk_rows = parse(diff);
    let new_lines: Vec<&str> = {
        let text = new_text.strip_suffix('\n').unwrap_or(new_text);
        if text.is_empty() {
            Vec::new()
        } else {
            text.split('\n').collect()
        }
    };

    // Splits `hunk_rows` into hunks: each a `(old_start, old_count, new_start, new_count,
    // rows)`, `rows` holding that hunk's header meta, its pairs and any interleaved meta
    // lines (UI-E-098), in original order.
    let mut hunks: Vec<(usize, usize, usize, usize, Vec<DiffRow>)> = Vec::new();
    // Rows preceding the first `@@` header (`diff --git`, `---`, `+++`): UI-R-254 wants
    // the full-file build a superset of the hunk-only rows, so these are kept rather than
    // dropped just because no hunk has been opened yet to hold them.
    let mut leading: Vec<DiffRow> = Vec::new();
    for row in hunk_rows {
        if let DiffRow::Meta { text } = &row
            && text.starts_with("@@")
        {
            let (old_start, old_count, new_start, new_count) = hunk_header(text);
            hunks.push((old_start, old_count, new_start, new_count, vec![row]));
            continue;
        }
        if let Some((.., rows)) = hunks.last_mut() {
            rows.push(row);
        } else {
            leading.push(row);
        }
    }

    fn unchanged_row(new_lines: &[&str], line_no: usize, delta: isize) -> DiffRow {
        let text = new_lines[line_no - 1].to_string();
        let old_no = (line_no as isize - delta).max(1) as usize;
        DiffRow::Pair {
            kind: DiffKind::Context,
            old: Some(DiffEntry {
                text: text.clone(),
                line_no: old_no,
            }),
            new: Some(DiffEntry { text, line_no }),
        }
    }

    let mut out = leading;
    let mut folds = Vec::new();
    let mut next_line = 1usize;
    let mut delta = 0isize;

    for (_old_start, old_count, new_start, new_count, rows) in hunks {
        // A hunk header naming a line past the supplied text (mismatched patch/file) must
        // not panic: the walk stops at `new_lines.len()`, leaving the rest to whatever the
        // patch itself supplies.
        // A `+n,0` header (a zero-context deletion) names the new-side line the removal
        // follows, not one it owns (UI-R-253): that line belongs to the pre-hunk fold, at
        // the pre-hunk delta, so it is included here rather than left for the post-hunk walk.
        let hunk_boundary = if new_count == 0 {
            new_start + 1
        } else {
            new_start
        };
        let fold_end = hunk_boundary.min(new_lines.len() + 1);
        if next_line < fold_end {
            let fold_start = out.len();
            while next_line < fold_end {
                out.push(unchanged_row(&new_lines, next_line, delta));
                next_line += 1;
            }
            folds.push(fold_start..=out.len() - 1);
        }
        // UI-E-114: where the supplied text disagrees with the patch's own context/added
        // text, the supplied text supplies the new-side content and the patch keeps the
        // row's classification and line numbers — a removed entry has no new-side
        // counterpart, so it is untouched.
        out.extend(rows.into_iter().map(|row| {
            match row {
                DiffRow::Pair {
                    kind,
                    old,
                    new: Some(new),
                } => DiffRow::Pair {
                    kind,
                    old,
                    new: Some(DiffEntry {
                        text: new
                            .line_no
                            .checked_sub(1)
                            .and_then(|i| new_lines.get(i))
                            .map_or(new.text.clone(), |t| t.to_string()),
                        ..new
                    }),
                },
                other => other,
            }
        }));
        // A `+0,0` header (a deleted file's only hunk) would otherwise leave `next_line`
        // at 0, and the trailing walk below treats line 0 as one past the end rather than
        // "nothing follows" and indexes `new_lines[usize::MAX]`: clamped to 1, since there
        // is no line 0 to resume from either way. A `+n,0` header's boundary line was
        // already folded in above through `hunk_boundary`, so resuming there (not
        // `new_start`) avoids reprocessing it.
        next_line = if new_count == 0 {
            hunk_boundary
        } else {
            new_start + new_count
        }
        .max(1);
        delta += new_count as isize - old_count as isize;
    }
    if next_line <= new_lines.len() {
        let fold_start = out.len();
        while next_line <= new_lines.len() {
            out.push(unchanged_row(&new_lines, next_line, delta));
            next_line += 1;
        }
        folds.push(fold_start..=out.len() - 1);
    }

    (out, folds)
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
        self.display = DiffDisplay::HunkOnly;
        self.folds = Vec::new();
        self.has_full_file = false;
    }

    /// Replaces the diff and full new-side file text (UI-R-207, UI-R-253), rebuilding the
    /// full-file row superset and its folds, and resetting the same navigation state
    /// `set_diff` does. Unlike `set_diff`, this leaves `self.display` untouched: it was
    /// already set from the builder's own field (default hunk-only, UI-R-257) by `build()`
    /// before this runs, and a full-file construction is the one path where a caller can
    /// legitimately ask to start already in full-file display.
    pub(crate) fn set_diff_and_file(&mut self, diff: &str, new_text: &str) {
        let (rows, folds) = parse_full_file(diff, new_text);
        self.rows = rows;
        self.folds = folds;
        self.has_full_file = true;
        self.active_row = 0;
        self.anchor = None;
        self.mode = DiffMode::Normal;
        self.scroll_offset = 0;
        self.h_scroll = 0;
        self.pending = None;
        self.pending_count = None;
        self.register = None;
    }

    /// Whether row `index` is inside a fold (UI-R-255, UI-R-256): always `false` in
    /// full-file display, since folds are cleared there rather than removed.
    fn is_folded(&self, index: usize) -> bool {
        self.display == DiffDisplay::HunkOnly && self.folds.iter().any(|f| f.contains(&index))
    }

    /// The nearest row to `from` that is not folded, searching forward then back
    /// (UI-R-256); `from` itself if there are no rows or no fold covers it. Not a pinned
    /// requirement's choice — the toggle back to hunk-only just needs the active row to
    /// land somewhere reachable.
    fn nearest_unfolded(&self, from: usize) -> usize {
        if self.rows.is_empty() || !self.is_folded(from) {
            return from.min(self.rows.len().saturating_sub(1));
        }
        for i in from..self.rows.len() {
            if !self.is_folded(i) {
                return i;
            }
        }
        for i in (0..from).rev() {
            if !self.is_folded(i) {
                return i;
            }
        }
        from
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
    /// own horizontal scrolling can use it. Equivalent to `set_content_widths(width,
    /// width)`, for a test that has only one width to give and does not care which side
    /// it applies to; every render caller now calls `set_content_widths` directly.
    #[allow(dead_code)]
    pub(crate) fn set_content_width(&mut self, width: usize) {
        self.set_content_widths(width, width);
    }

    /// Written by both layouts' renderers, recording each pane's own text width
    /// (UI-R-262) alongside their maximum, which the horizontal-scroll arithmetic
    /// (UI-R-232) still reads as one shared `content_width`.
    pub(crate) fn set_content_widths(&mut self, old: usize, new: usize) {
        self.content_width_old = old.max(1);
        self.content_width_new = new.max(1);
        self.content_width = self.content_width_old.max(self.content_width_new);
    }

    /// Written by the widget that renders this state, recording a meta row's own
    /// available width (UI-R-260) so `display_rows()` wraps it at the width it is
    /// actually drawn into.
    pub(crate) fn set_meta_width(&mut self, width: usize) {
        self.content_width_meta = width.max(1);
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

    /// Whether annotations currently contribute display rows (UI-R-274, UI-R-275), read
    /// through `display_rows()` and `Ctrl+A`'s own toggle rather than by the widget
    /// directly; kept for tests to assert without adding public surface no
    /// `api-contract.md` row names.
    #[allow(dead_code)]
    pub(crate) fn annotations_shown(&self) -> bool {
        self.annotations_shown
    }

    /// Written by the widget that renders this state, recording each visible annotation's
    /// own measured height (UI-R-272) so `display_rows()` can add its border rows.
    pub(crate) fn set_annotation_heights(&mut self, heights: Vec<usize>) {
        self.annotation_heights = heights;
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

    /// The number of display rows side `text` at `width` occupies: one,
    /// unwrapped; with wrapping on, [`word_wrap`]'s row count, style irrelevant to where it
    /// breaks so `Style::default()` stands in for the real per-character styles the widget
    /// applies at draw time.
    fn side_sub_rows(&self, text: &str, width: usize) -> usize {
        if !self.wrap {
            return 1;
        }
        let width = width.max(1);
        let chars: Vec<(char, Style)> = text.chars().map(|c| (c, Style::default())).collect();
        word_wrap(&chars, width, 0).len().max(1)
    }

    /// The display rows one logical row expands to: a `Meta` row is always
    /// one; a `Pair` is `old_sub.max(new_sub)` sub-rows wide in the split layout, both
    /// sides drawn side by side and the shorter padded blank (UI-R-262), but
    /// `old_sub + new_sub` in the unified layout, whose two sides draw as separate,
    /// sequential screen lines only when both are present and differ — the same rule
    /// `widgets/diff_view.rs`'s unified renderer already applies, folded in here so
    /// display rows and logical rows already differ before wrapping exists.
    fn row_display_parts(&self, row: &DiffRow) -> Vec<RowPart> {
        match row {
            DiffRow::Meta { text } => {
                let n = self.side_sub_rows(text, self.content_width_meta);
                (0..n).map(|sub_row| RowPart::Meta { sub_row }).collect()
            }
            DiffRow::Pair { old, new, .. } => {
                let old_text = old.as_ref().map_or("", |e| e.text.as_str());
                let new_text = new.as_ref().map_or("", |e| e.text.as_str());
                match self.layout {
                    DiffLayout::Split => {
                        let o = self.side_sub_rows(old_text, self.content_width_old);
                        let n = self.side_sub_rows(new_text, self.content_width_new);
                        (0..o.max(n))
                            .map(|i| RowPart::Pair {
                                old_sub: (i < o).then_some(i),
                                new_sub: (i < n).then_some(i),
                            })
                            .collect()
                    }
                    DiffLayout::Unified => {
                        let differ = matches!((old, new), (Some(o), Some(n)) if o.text != n.text);
                        if differ {
                            let o = self.side_sub_rows(old_text, self.content_width);
                            let n = self.side_sub_rows(new_text, self.content_width);
                            (0..o)
                                .map(|i| RowPart::Pair {
                                    old_sub: Some(i),
                                    new_sub: None,
                                })
                                .chain((0..n).map(|i| RowPart::Pair {
                                    old_sub: None,
                                    new_sub: Some(i),
                                }))
                                .collect()
                        } else {
                            let text = if old.is_some() { old_text } else { new_text };
                            let n = self.side_sub_rows(text, self.content_width);
                            (0..n)
                                .map(|i| {
                                    if old.is_some() {
                                        RowPart::Pair {
                                            old_sub: Some(i),
                                            new_sub: None,
                                        }
                                    } else {
                                        RowPart::Pair {
                                            old_sub: None,
                                            new_sub: Some(i),
                                        }
                                    }
                                })
                                .collect()
                        }
                    }
                }
            }
        }
    }

    /// The last row (highest index) whose named side holds an entry with a file line
    /// inside `ann.lines` (UI-R-270): the last row of the range, scanning back from the
    /// end since file lines only increase with row index. `None` when no row covers the
    /// range at all (UI-E-115), silently dropping the annotation rather than erroring.
    fn annotation_anchor_row(&self, ann: &Annotation) -> Option<usize> {
        self.rows.iter().enumerate().rev().find_map(|(i, row)| {
            let entry = match row {
                DiffRow::Meta { .. } => None,
                DiffRow::Pair { old, new, .. } => match ann.side {
                    Side::Old => old.as_ref(),
                    Side::New => new.as_ref(),
                },
            };
            entry.filter(|e| ann.lines.contains(&e.line_no)).map(|_| i)
        })
    }

    /// Every shown annotation's own anchor row, resolved once rather than rescanned per
    /// row: `annotation_anchor_row` itself walks the row list, so calling it from inside
    /// `display_rows()`'s own per-row loop turned one `display_rows()` call — run every
    /// keystroke and render — into a scan of the row list for every row for every
    /// annotation. Grouped by anchor row and kept in `annotations()`'s order within each
    /// group (UI-E-116), a hidden or out-of-range annotation contributing no entry.
    fn annotation_anchors_by_row(&self) -> std::collections::HashMap<usize, Vec<usize>> {
        let mut by_row = std::collections::HashMap::new();
        if !self.annotations_shown {
            return by_row;
        }
        for (index, ann) in self.annotations.iter().enumerate() {
            if let Some(row) = self.annotation_anchor_row(ann) {
                by_row.entry(row).or_insert_with(Vec::new).push(index);
            }
        }
        by_row
    }

    /// The display-row layer: one entry per screen line, mapping back to the
    /// logical row it belongs to. Built fresh from `self.rows`, the wrap flag and the
    /// remembered per-side widths, never cached — the aligned rows this walks are
    /// themselves already the full parsed list (UI-R-254), so there is no second, larger
    /// structure being rebuilt here. Annotation rows (UI-R-269) are appended right after
    /// the anchor row's own parts, in `annotations()`'s order (UI-E-116), and only while
    /// `annotations_shown` (UI-R-275); a hidden or out-of-range annotation contributes
    /// none.
    pub(crate) fn display_rows(&self) -> Vec<DisplayRow> {
        let anchors = self.annotation_anchors_by_row();
        let mut out = Vec::new();
        for (logical, row) in self.rows.iter().enumerate() {
            if self.is_folded(logical) {
                continue;
            }
            for part in self.row_display_parts(row) {
                out.push(DisplayRow { logical, part });
            }
            for &index in anchors.get(&logical).into_iter().flatten() {
                // UI-R-272: the block's own height is its measured text rows plus its
                // border rows (top and bottom); zero (no block at all) before the
                // first render has measured it (UI-E-084's rule).
                let measured = self.annotation_heights.get(index).copied().unwrap_or(0);
                let total = if measured == 0 { 0 } else { measured + 2 };
                for sub_row in 0..total {
                    out.push(DisplayRow {
                        logical,
                        part: RowPart::Annotation { index, sub_row },
                    });
                }
            }
        }
        out
    }

    /// The display-row index range `[start, end]` (inclusive) the active logical row
    /// occupies, `(0, 0)` when there are no rows. Excludes any `RowPart::Annotation`
    /// riding on the active row's own logical index (UI-R-273): an annotation is never
    /// part of the active row, only drawn beneath it.
    fn active_row_display_span(&self) -> (usize, usize) {
        let display = self.display_rows();
        let mut start = None;
        let mut end = 0;
        for (i, d) in display.iter().enumerate() {
            if d.logical == self.active_row && !matches!(d.part, RowPart::Annotation { .. }) {
                start.get_or_insert(i);
                end = i;
            }
        }
        (start.unwrap_or(0), end)
    }

    /// Keeps the active row's display-row span inside the last-rendered visible window
    /// (UI-R-230, UI-R-265), settling `scroll_offset` — now counting display rows
    /// (amended UI-R-231) — to the least distance that puts that whole span in view.
    fn ensure_visible(&mut self) {
        let (start, end) = self.active_row_display_span();
        if start < self.scroll_offset {
            self.scroll_offset = start;
            return;
        }
        let visible = self.visible_height.max(1);
        // A span wider than the visible window can never fit whole: settling at its own
        // start, rather than bottom-anchoring it (the loop below), keeps its lead
        // reachable (UI-E-120 then scrolls display row by display row from here to reach
        // its tail).
        if end - start + 1 > visible {
            self.scroll_offset = start;
            return;
        }
        while end >= self.scroll_offset + visible {
            self.scroll_offset += 1;
        }
    }

    /// `j`/`Down` (`down`) or `k`/`Up` (`!down`): if the active row's display-row span
    /// extends past the visible window in the direction of travel, scrolls one display row
    /// toward that edge and leaves the active row where it is (UI-E-120); only once that
    /// edge is in view does it move to the next/previous logical row, the ordinary case
    /// for a row that is not taller than the viewport.
    fn step_display_or_logical(&mut self, down: bool) {
        if self.rows.is_empty() {
            return;
        }
        let (start, end) = self.active_row_display_span();
        let visible = self.visible_height.max(1);
        if down {
            if end >= self.scroll_offset + visible {
                self.scroll_offset += 1;
                return;
            }
        } else if start < self.scroll_offset {
            self.scroll_offset -= 1;
            return;
        }
        let last = self.rows.len() - 1;
        let mut next = if down {
            (self.active_row + 1).min(last)
        } else {
            self.active_row.saturating_sub(1)
        };
        // UI-R-256: a folded row is unreachable, so a step in either direction lands on
        // the next unfolded row, not just the next row.
        while self.is_folded(next) {
            let stepped = if down {
                (next + 1).min(last)
            } else {
                next.saturating_sub(1)
            };
            if stepped == next {
                break;
            }
            next = stepped;
        }
        // A leading/trailing fold can run to the very first or last row, leaving no
        // unfolded row further in the travel direction; the loop above then stops still
        // folded, so falls back to the nearest unfolded row in either direction rather
        // than land the active row inside a fold (UI-R-256).
        if self.is_folded(next) {
            next = self.nearest_unfolded(next);
        }
        self.active_row = next;
        self.clamp_active_row();
        self.ensure_visible();
    }

    fn move_active_row_to(&mut self, row: usize) {
        self.active_row = row;
        self.clamp_active_row();
        if self.is_folded(self.active_row) {
            self.active_row = self.nearest_unfolded(self.active_row);
        }
        self.ensure_visible();
    }

    /// Moves by `rows` display rows (amended UI-R-231) from the active row's own display
    /// span, then lands on the logical row holding the display row reached — mirrors
    /// `code_input_field.rs`'s `page_move`, expressed over `display_rows()`
    /// instead of logical rows.
    fn page_move(&mut self, down: bool, rows: usize) {
        let display = self.display_rows();
        if display.is_empty() {
            return;
        }
        let (start, _) = self.active_row_display_span();
        let rows = rows.max(1) as isize;
        let target = (start as isize + if down { rows } else { -rows })
            .clamp(0, display.len() as isize - 1) as usize;
        let mut logical = display[target].logical;
        // A row taller than the page (UI-R-260's wrapping, or an unwrapped unified changed
        // pair) can leave the target display row inside the row paging started from: land
        // on the next logical row instead, so a page never stalls in place.
        if logical == self.active_row {
            let last = self.rows.len().saturating_sub(1);
            logical = if down {
                (logical + 1).min(last)
            } else {
                logical.saturating_sub(1)
            };
        }
        self.move_active_row_to(logical);
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
            (KeyModifiers::NONE, KeyCode::Char('j') | KeyCode::Down) => {
                let count = self.pending_count.take().unwrap_or(1).max(1);
                for _ in 0..count {
                    self.step_display_or_logical(true);
                }
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Char('k') | KeyCode::Up) => {
                let count = self.pending_count.take().unwrap_or(1).max(1);
                for _ in 0..count {
                    self.step_display_or_logical(false);
                }
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
                if !self.wrap {
                    self.h_scroll = self.h_scroll.saturating_sub(1);
                }
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Left) => {
                if !self.wrap {
                    self.h_scroll = self.h_scroll.saturating_sub(1);
                }
                EventResult::Consumed
            }
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('l')) => {
                if !self.wrap {
                    self.h_scroll = (self.h_scroll + 1).min(self.max_h_scroll());
                }
                EventResult::Consumed
            }
            (KeyModifiers::NONE, KeyCode::Right) => {
                if !self.wrap {
                    self.h_scroll = (self.h_scroll + 1).min(self.max_h_scroll());
                }
                EventResult::Consumed
            }
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('0')) => {
                if !self.wrap {
                    self.h_scroll = 0;
                }
                EventResult::Consumed
            }
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('$')) => {
                if !self.wrap {
                    let len = self.active_row_text_len(self.focused_side);
                    let last = len.saturating_sub(1);
                    self.h_scroll = (last + 1).saturating_sub(self.content_width);
                }
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
            (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
                // UI-R-275: toggling annotation visibility changes how many display rows
                // precede the active row without moving it (UI-E-118), so the scroll must
                // re-settle in display rows the same way Ctrl+F's fold toggle does.
                self.annotations_shown = !self.annotations_shown;
                self.ensure_visible();
                EventResult::Consumed
            }
            (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
                // Built from a diff alone, there is no full-file mode to switch to
                // (UI-R-259): consumed and ignored (UI-E-113).
                if self.has_full_file {
                    self.display = match self.display {
                        DiffDisplay::HunkOnly => DiffDisplay::FullFile,
                        DiffDisplay::FullFile => DiffDisplay::HunkOnly,
                    };
                    if self.display == DiffDisplay::HunkOnly {
                        // Folding back in can fold the span the active row (and any
                        // Visual anchor) sits in (UI-R-256): land both on the nearest
                        // unfolded row before re-settling the scroll.
                        self.active_row = self.nearest_unfolded(self.active_row);
                        if let Some(anchor) = self.anchor {
                            self.anchor = Some(self.nearest_unfolded(anchor));
                        }
                    }
                    // Either direction changes which display rows precede the active row
                    // (folded spans appear or disappear), which can leave it off the
                    // currently settled window even though it did not itself move
                    // (amended UI-R-231): re-settle the scroll every time.
                    self.scroll_offset = 0;
                    self.ensure_visible();
                }
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

    /// Builds the state from a unified diff text plus the full new-side file text
    /// (UI-R-207, UI-R-253): the row list is the full-file superset (UI-R-254), with fold
    /// ranges over the spans outside every hunk so hunk-only display (the default,
    /// UI-R-257) draws the same rows `build_with_diff` alone would have.
    pub fn build_with_diff_and_file(
        &self,
        diff: &str,
        new_text: &str,
    ) -> Result<DiffViewState, DiffViewStateBuilderError> {
        let mut state = self.build()?;
        state.set_diff_and_file(diff, new_text);
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
    /// UI-R-266 — `marked_ranges` is settable through the builder and, separately, through
    /// the post-build setter.
    fn ut_marked_ranges_are_settable_at_build_time_and_after() {
        let built = vec![MarkedRange {
            side: Side::Old,
            lines: 1..=2,
            color: Color::Yellow,
        }];
        let mut s = DiffViewStateBuilder::default()
            .marked_ranges(built.clone())
            .build()
            .unwrap();
        assert_eq!(s.marked_ranges(), &built);

        let after = vec![MarkedRange {
            side: Side::New,
            lines: 3..=4,
            color: Color::Red,
        }];
        s.set_marked_ranges(after.clone());
        assert_eq!(s.marked_ranges(), &after);
    }

    #[test]
    /// UI-R-269, UI-R-274 — `annotations` is settable through the builder and, separately,
    /// through the post-build setter, and annotations start shown.
    fn ut_annotations_are_settable_at_build_time_and_after_and_start_shown() {
        let built = vec![Annotation {
            side: Side::Old,
            lines: 1..=1,
            text: "one".into(),
        }];
        let mut s = DiffViewStateBuilder::default()
            .annotations(built.clone())
            .build_with_diff("@@ -1,1 +1,1 @@\n a\n")
            .unwrap();
        assert_eq!(s.annotations(), &built);
        assert!(s.annotations_shown());

        let after = vec![Annotation {
            side: Side::New,
            lines: 1..=1,
            text: "two".into(),
        }];
        s.set_annotations(after.clone());
        assert_eq!(s.annotations(), &after);
    }

    #[test]
    /// UI-R-273 — annotations add no logical row, so the active row can never land on
    /// one, and the selected-row query never reports one either.
    fn ut_annotations_are_never_active_never_navigable_and_never_selected() {
        let mut s = DiffViewStateBuilder::default()
            .annotations(vec![Annotation {
                side: Side::Old,
                lines: 1..=1,
                text: "note".into(),
            }])
            .build_with_diff("@@ -1,2 +1,2 @@\n a\n b\n")
            .unwrap();
        s.set_annotation_heights(vec![3]);
        s.set_visible_height(3);
        s.set_active_row(1);
        let display = s.display_rows();
        let annotation_rows = display
            .iter()
            .filter(|d| matches!(d.part, RowPart::Annotation { index: 0, .. }))
            .count();
        assert_eq!(
            annotation_rows, 5,
            "the annotation contributes its measured 3 rows plus its 2 border rows"
        );
        let anchor_display_count = display
            .iter()
            .filter(|d| d.logical == 1 && !matches!(d.part, RowPart::Annotation { .. }))
            .count();
        assert_eq!(
            anchor_display_count, 1,
            "the annotation's rows never widen row 1's own span"
        );
        assert_eq!(s.selected_rows(), Some(1..=1));

        s.handle_events(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(
            s.active_row(),
            2,
            "j steps straight to the next logical row, never landing inside the block"
        );
        assert_eq!(s.selected_rows(), Some(2..=2));
    }

    #[test]
    /// UI-R-275, UI-E-118 — `Ctrl+A` toggles every annotation's display rows at once,
    /// leaving the active row unchanged and re-settling the scroll offset in display
    /// rows, with the active row sitting below the block so the toggle actually shifts
    /// how many display rows precede it.
    fn ut_ctrl_a_hides_and_shows_every_annotation_keeping_the_active_row() {
        let mut s = DiffViewStateBuilder::default()
            .annotations(vec![Annotation {
                side: Side::Old,
                lines: 1..=1,
                text: "note".into(),
            }])
            .build_with_diff("@@ -1,3 +1,3 @@\n a\n b\n c\n")
            .unwrap();
        // Rows: 0 = meta, 1 = "a" (the annotation's anchor), 2 = "b", 3 = "c" (active,
        // below the block). Shown, the block's 5 rows (3 measured + 2 border) sit
        // between rows 1 and 2, so the active row's own display index is 8 of 9; hidden,
        // it drops to 3 of 4.
        s.set_annotation_heights(vec![3]);
        s.set_visible_height(3);
        s.set_active_row(3);
        s.set_scroll_offset(6);
        assert!(
            s.display_rows()
                .iter()
                .any(|d| matches!(d.part, RowPart::Annotation { .. }))
        );

        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('a'));
        assert!(!s.annotations_shown());
        assert!(
            !s.display_rows()
                .iter()
                .any(|d| matches!(d.part, RowPart::Annotation { .. }))
        );
        assert_eq!(s.active_row(), 3);
        assert_eq!(
            s.scroll_offset(),
            3,
            "hiding the block frees the 5 display rows that used to precede row 3"
        );

        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('a'));
        assert!(s.annotations_shown());
        assert!(
            s.display_rows()
                .iter()
                .any(|d| matches!(d.part, RowPart::Annotation { .. }))
        );
        assert_eq!(s.active_row(), 3);
        assert_eq!(
            s.scroll_offset(),
            6,
            "showing it again re-settles the scroll to keep row 3 in view"
        );
    }

    #[test]
    /// UI-E-115 — an annotation naming a side and file line range no row covers is
    /// silently dropped: no error, no display row.
    fn ut_out_of_range_annotation_is_silently_not_rendered() {
        let mut s = DiffViewStateBuilder::default()
            .annotations(vec![Annotation {
                side: Side::New,
                lines: 999..=999,
                text: "note".into(),
            }])
            .build_with_diff("@@ -1,1 +1,1 @@\n a\n")
            .unwrap();
        s.set_annotation_heights(vec![3]);
        assert!(
            !s.display_rows()
                .iter()
                .any(|d| matches!(d.part, RowPart::Annotation { .. }))
        );
    }

    #[test]
    /// UI-E-116 — several annotations anchored to the same row draw in the order they
    /// were supplied.
    fn ut_several_annotations_on_one_row_stack_in_supplied_order() {
        let mut s = DiffViewStateBuilder::default()
            .annotations(vec![
                Annotation {
                    side: Side::Old,
                    lines: 1..=1,
                    text: "first".into(),
                },
                Annotation {
                    side: Side::Old,
                    lines: 1..=1,
                    text: "second".into(),
                },
            ])
            .build_with_diff("@@ -1,1 +1,1 @@\n a\n")
            .unwrap();
        s.set_annotation_heights(vec![2, 2]);
        let indices: Vec<usize> = s
            .display_rows()
            .iter()
            .filter_map(|d| match d.part {
                RowPart::Annotation { index, sub_row: 0 } => Some(index),
                _ => None,
            })
            .collect();
        assert_eq!(indices, vec![0, 1]);
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

    #[test]
    /// UI-R-260 — wrapping defaults off, so a row too wide for the pane draws as one
    /// display row; with it on, the row breaks at a whitespace boundary instead.
    fn ut_wrap_breaks_a_long_row_at_whitespace_and_defaults_off() {
        let mut unwrapped = DiffViewStateBuilder::default()
            .build_with_diff("@@ -1,1 +1,1 @@\n-hello world foo\n")
            .unwrap();
        assert!(!unwrapped.wrap(), "wrapping defaults off");
        unwrapped.set_content_widths(5, 5);
        assert_eq!(
            unwrapped.display_rows().len(),
            2,
            "unwrapped: header row plus the one pair row"
        );

        let mut wrapped = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@ -1,1 +1,1 @@\n-hello world foo\n")
            .unwrap();
        wrapped.set_content_widths(5, 5);
        wrapped.set_meta_width(20);
        // "hello world foo" at width 5 word-wraps to "hello", "world", "foo": 3 rows.
        assert_eq!(wrapped.display_rows().len(), 1 + 3);
    }

    #[test]
    /// UI-R-260 — a meta row wraps like any other row; UI-R-260 exempts none.
    fn ut_meta_row_wraps_like_any_other_row() {
        let mut s = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@ header text that is long @@\n context\n")
            .unwrap();
        s.set_content_widths(20, 20);
        s.set_meta_width(10);
        // "@@ header text that is long @@" (31 chars) word-wraps at width 10 into more
        // than one row.
        let meta_subs = s.display_rows().iter().filter(|d| d.logical == 0).count();
        assert!(
            meta_subs > 1,
            "the meta row itself wrapped into several rows"
        );
    }

    #[test]
    /// UI-R-262 — in the split layout each side wraps at its own width, so a narrow old
    /// side and a wide new side do not force each other's break points.
    fn ut_each_split_side_wraps_at_its_own_width() {
        let mut s = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@ -1,1 +1,1 @@\n-a bb ccc dddd\n+short\n")
            .unwrap();
        s.set_content_widths(5, 20);
        s.set_meta_width(20);
        let display = s.display_rows();
        // Row 0 is the "@@" header (one display row); row 1 is the pair.
        let pair_parts: Vec<_> = display.iter().filter(|d| d.logical == 1).collect();
        let old_subs: Vec<usize> = pair_parts
            .iter()
            .filter_map(|d| match d.part {
                RowPart::Pair {
                    old_sub: Some(i), ..
                } => Some(i),
                _ => None,
            })
            .collect();
        let new_subs: Vec<usize> = pair_parts
            .iter()
            .filter_map(|d| match d.part {
                RowPart::Pair {
                    new_sub: Some(i), ..
                } => Some(i),
                _ => None,
            })
            .collect();
        assert!(
            old_subs.len() > 1,
            "the narrow old side wraps at its own width"
        );
        assert_eq!(new_subs.len(), 1, "the wide new side does not wrap");
        assert_eq!(
            pair_parts.len(),
            old_subs.len(),
            "the shorter side pads to the taller side's row count"
        );
    }

    #[test]
    /// UI-E-111 — a word longer than the available width breaks at a character boundary
    /// instead of overflowing or being truncated.
    fn ut_word_longer_than_the_width_breaks_at_a_character_boundary() {
        let mut s = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@ -1,1 +1,1 @@\n-abcdefghijklm\n")
            .unwrap();
        s.set_content_widths(3, 3);
        s.set_meta_width(20);
        let display = s.display_rows();
        let old_subs = display
            .iter()
            .filter(|d| d.logical == 1)
            .filter(|d| {
                matches!(
                    d.part,
                    RowPart::Pair {
                        old_sub: Some(_),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(
            old_subs, 5,
            "13 characters at width 3 is ceil(13 / 3) = 5 rows"
        );
    }

    #[test]
    /// UI-E-112 — a pane too narrow for the gutter treats the available text
    /// width as one column, wrapping one character per display row.
    fn ut_pane_too_narrow_for_the_gutter_wraps_one_character_per_row() {
        let mut s = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@ -1,1 +1,1 @@\n-abcd\n")
            .unwrap();
        s.set_content_widths(1, 1);
        s.set_meta_width(20);
        let display = s.display_rows();
        let old_subs = display
            .iter()
            .filter(|d| d.logical == 1)
            .filter(|d| {
                matches!(
                    d.part,
                    RowPart::Pair {
                        old_sub: Some(_),
                        ..
                    }
                )
            })
            .count();
        assert_eq!(old_subs, 4, "one character per display row");
    }

    #[test]
    /// UI-R-263, amended UI-R-232 — with wrapping on, the horizontal keys are consumed
    /// but move nothing; with it off, they behave exactly as before.
    fn ut_horizontal_keys_are_consumed_and_do_nothing_while_wrapping() {
        let mut s = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@ -1,1 +1,1 @@\n-a much longer line of text\n")
            .unwrap();
        s.set_content_widths(5, 5);
        s.set_meta_width(20);
        for code in [
            KeyCode::Char('l'),
            KeyCode::Right,
            KeyCode::Char('$'),
            KeyCode::Char('h'),
            KeyCode::Left,
            KeyCode::Char('0'),
        ] {
            let result = s.handle_events(KeyModifiers::NONE, code);
            assert!(matches!(result, EventResult::Consumed));
            assert_eq!(
                s.h_scroll(),
                0,
                "wrapped content never scrolls horizontally"
            );
        }
    }

    #[test]
    /// Amended UI-R-232 — with wrapping off the horizontal keys still scroll.
    fn ut_horizontal_keys_still_scroll_with_wrapping_off() {
        let mut s = nav_fixture();
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('l'));
        assert_eq!(s.h_scroll(), 1);
    }

    #[test]
    /// UI-R-264 — the active row and the selected-row query address logical rows even
    /// when the active row spans several display rows once wrapped.
    fn ut_active_row_and_selection_stay_logical_across_a_wrapped_row() {
        let mut s = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff("@@ -1,2 +1,2 @@\n-hello world foo bar\n context\n")
            .unwrap();
        s.set_content_widths(5, 5);
        s.set_meta_width(20);
        s.set_visible_height(10);
        // `j` from the header lands on row 1, wrapped into several display rows; the
        // active row is still the one logical index, not a display sub-row.
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.active_row(), 1);
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('v'));
        assert_eq!(
            s.selected_rows(),
            Some(1..=1),
            "one wrapped row selects whole, as a single logical index"
        );
        // Extending the selection onto the next logical row still reports logical indices.
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.selected_rows(), Some(1..=2));
    }

    #[test]
    /// UI-R-265, amended UI-R-231 — scrolling and paging count display rows and land on
    /// the logical row holding the display row reached.
    fn ut_scroll_and_paging_count_display_rows_and_land_on_a_logical_row() {
        let mut s = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff(
                "@@ -1,3 +1,3 @@\n-hello world foo bar\n context\n another context line\n",
            )
            .unwrap();
        s.set_content_widths(5, 5);
        s.set_meta_width(20);
        s.set_visible_height(3);
        // Row 0 is the header (1 display row, index 0); row 1 wraps to 4 display rows
        // ("hello", "world", "foo", "bar", indices 1..=4); rows 2 and 3 are one display
        // row each (indices 5, 6). From the header, paging 3 display rows down lands on
        // display index 3, which is row 1's third wrapped line.
        s.handle_events(KeyModifiers::NONE, KeyCode::PageDown);
        assert_eq!(
            s.active_row(),
            1,
            "page moved 3 display rows, landing on row 1"
        );
        assert_eq!(
            s.scroll_offset(),
            1,
            "row 1's span top-anchors, wider than the window"
        );

        // `Ctrl+D`/`Ctrl+U` halve the page (1 display row here) and count display rows the
        // same way.
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('d'));
        assert_eq!(s.active_row(), 2, "half-page moved on to row 2");
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('u'));
        assert_eq!(s.active_row(), 1, "half-page back lands on row 1 again");
    }

    #[test]
    /// Amended UI-R-231 — paging never stalls on a row wider than the visible window: if
    /// moving by the page's display-row count would still land inside the active row,
    /// the page advances to the next logical row instead of doing nothing.
    fn ut_paging_advances_past_a_row_taller_than_the_visible_window() {
        let mut s = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff(
                "@@ -1,3 +1,3 @@\n-hello world foo bar\n context\n another context line\n",
            )
            .unwrap();
        s.set_content_widths(5, 5);
        s.set_meta_width(20);
        s.set_visible_height(3);
        // Row 1 spans display rows 1..=4, four rows, wider than the 3-row visible window:
        // paging 3 display rows from its own start (1) reaches display index 4, whose
        // logical row is still 1 — a naive implementation would leave `active_row`
        // unchanged here.
        s.set_active_row(1);
        s.handle_events(KeyModifiers::NONE, KeyCode::PageDown);
        assert_eq!(
            s.active_row(),
            2,
            "paging advances to the next logical row rather than stalling"
        );
    }

    #[test]
    /// UI-E-120 — `j`/`Down` on a row taller than the viewport scrolls one display row at
    /// a time within that row until its last display row is visible, only then moving to
    /// the next logical row; `k`/`Up` mirrors it toward the first display row.
    fn ut_j_and_k_step_through_a_row_taller_than_the_viewport_before_changing_logical_row() {
        let mut s = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff(
                "@@ -1,3 +1,3 @@\n-hello world foo bar\n context\n another context line\n",
            )
            .unwrap();
        s.set_content_widths(5, 5);
        s.set_meta_width(20);
        s.set_visible_height(3);
        // `j` from the header (display row 0) lands on row 1 (display rows 1..=4), which
        // top-anchors: scroll_offset == 1.
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.active_row(), 1);
        assert_eq!(s.scroll_offset(), 1);

        // Row 1's span (1..=4) does not fit the 3-row window (rows 1..=3 visible, row 4
        // hidden): `j`/`Down` scrolls within it first, leaving `active_row` unchanged,
        // until its last display row (4) is visible (window becomes rows 2..=4).
        s.handle_events(KeyModifiers::NONE, KeyCode::Down);
        assert_eq!(s.active_row(), 1, "still row 1: only scrolled, not moved");
        assert_eq!(
            s.scroll_offset(),
            2,
            "row 1's last display row is now visible"
        );

        // Now that display row 4 (row 1's last) is in view, the next `j` moves on.
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.active_row(), 2, "row 1's tail was visible, so j advances");

        // `k`/`Up` mirrors it: scrolling back up within row 1 before re-entering it moves
        // active_row.
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('k'));
        assert_eq!(s.active_row(), 1);
        assert_eq!(
            s.scroll_offset(),
            1,
            "k re-enters row 1 already at its own start"
        );
        s.handle_events(KeyModifiers::NONE, KeyCode::Up);
        assert_eq!(
            s.active_row(),
            0,
            "row 1's start was already visible, so k moves on"
        );
    }

    #[test]
    /// UI-R-230 (re-tested), UI-R-265 — the keep-visible rule settles on the whole
    /// display-row span of a wrapped active row, not just its first display row.
    fn ut_keep_visible_settles_on_the_active_rows_display_span() {
        let mut s = DiffViewStateBuilder::default()
            .wrap(true)
            .build_with_diff(
                "@@ -1,3 +1,3 @@\n-hello world foo bar\n context\n another context line\n",
            )
            .unwrap();
        s.set_content_widths(5, 5);
        s.set_meta_width(20);
        s.set_visible_height(3);
        // `j` from row 0 moves to logical row 1 and runs `ensure_visible` (unlike the
        // test-only `set_active_row`, which does not).
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.active_row(), 1);
        // Row 1 alone spans display rows 1..=4 (4 rows), wider than visible_height (3):
        // its span does not fit at all, so `ensure_visible` must not bottom-anchor (which
        // would permanently hide the span's own leading display rows) but settle at the
        // span's own first display row instead.
        assert_eq!(
            s.scroll_offset(),
            1,
            "settles at the span's own start rather than hiding its leading rows"
        );
    }

    #[test]
    /// UI-R-207, UI-R-253 — with the full new-side text supplied, a row outside every
    /// hunk holds that text on both sides, the old-side number reconstructed from the
    /// cumulative delta of the hunks before it.
    fn ut_full_file_rows_take_new_text_and_reconstruct_the_old_side_numbers() {
        let s = DiffViewStateBuilder::default()
            .build_with_diff_and_file(
                "@@ -2,1 +2,1 @@\n-old line\n+new line\n",
                "line1\nnew line\nline3\n",
            )
            .unwrap();
        // Row 0: "line1", outside the hunk, no delta yet: old == new == 1.
        let row0 = s.row(0).unwrap();
        assert_eq!(row0.old_line, Some(1));
        assert_eq!(row0.new_line, Some(1));
        // Row 1: the header meta; row 2: the hunk's own removed/added pair at line 2.
        let row2 = s.row(2).unwrap();
        assert_eq!(row2.old_line, Some(2));
        assert_eq!(row2.new_line, Some(2));
        // Row 3: "line3", outside the hunk; the hunk removed and added one line each, so
        // no delta: old == new == 3.
        let row3 = s.row(3).unwrap();
        assert_eq!(row3.old_line, Some(3));
        assert_eq!(row3.new_line, Some(3));
    }

    #[test]
    /// UI-E-114 — full new-side text disagreeing with the patch's context line: the
    /// supplied text supplies the content, the patch the classification; nothing dropped.
    fn ut_text_disagreeing_with_a_context_line_keeps_the_text_and_the_patch_classification() {
        let s = DiffViewStateBuilder::default()
            .build_with_diff_and_file(
                "@@ -1,2 +1,2 @@\n context line\n-old\n+added\n",
                "edited context line\nadded\n",
            )
            .unwrap();
        let DiffRow::Pair { kind, new, .. } = &s.rows()[1] else {
            panic!("expected the context row")
        };
        assert_eq!(
            *kind,
            DiffKind::Context,
            "the patch still classifies it Context"
        );
        assert_eq!(
            new.as_ref().unwrap().text,
            "edited context line",
            "the supplied text supplies the content"
        );
    }

    #[test]
    /// UI-R-209 — lines between and around hunks become rows on both sides.
    fn ut_lines_between_and_around_hunks_become_rows_on_both_sides() {
        let s = DiffViewStateBuilder::default()
            .build_with_diff_and_file("@@ -2,1 +2,1 @@\n-b\n+B\n", "a\nB\nc\n")
            .unwrap();
        assert_eq!(s.rows().len(), 4, "a (before), header, the pair, c (after)");
        let DiffRow::Pair { old, new, .. } = &s.rows()[0] else {
            panic!("expected a pair row for the leading unchanged line")
        };
        assert_eq!(old.as_ref().unwrap().text, "a");
        assert_eq!(new.as_ref().unwrap().text, "a");
        let DiffRow::Pair { old, new, .. } = &s.rows()[3] else {
            panic!("expected a pair row for the trailing unchanged line")
        };
        assert_eq!(old.as_ref().unwrap().text, "c");
        assert_eq!(new.as_ref().unwrap().text, "c");
    }

    #[test]
    /// UI-R-254, UI-R-255 — the row list and every row's index are the same across a
    /// display-mode toggle; only the folds change.
    fn ut_row_indices_are_stable_across_a_display_mode_toggle() {
        let mut s = DiffViewStateBuilder::default()
            .build_with_diff_and_file("@@ -2,1 +2,1 @@\n-b\n+B\n", "a\nB\nc\n")
            .unwrap();
        assert_eq!(s.display(), DiffDisplay::HunkOnly);
        let before = s.rows().to_vec();
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('f'));
        assert_eq!(s.display(), DiffDisplay::FullFile);
        assert_eq!(s.rows().to_vec(), before, "same list, same indices");
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('f'));
        assert_eq!(s.display(), DiffDisplay::HunkOnly);
        assert_eq!(s.rows().to_vec(), before);
    }

    #[test]
    /// UI-R-256 — a folded row is neither drawn nor reachable by navigation, but keeps
    /// its index.
    fn ut_folded_rows_are_neither_drawn_nor_reachable_but_keep_their_indices() {
        let mut s = DiffViewStateBuilder::default()
            .build_with_diff_and_file("@@ -2,1 +2,1 @@\n-b\n+B\n", "a\nB\nc\n")
            .unwrap();
        // Rows: 0 "a" (folded), 1 header, 2 pair, 3 "c" (folded).
        assert_eq!(s.rows().len(), 4);
        let display = s.display_rows();
        assert!(
            display.iter().all(|d| d.logical != 0 && d.logical != 3),
            "the folded rows contribute no display rows"
        );
        // `gg` from the pair row must not land inside the leading fold.
        s.set_active_row(2);
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('g'));
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('g'));
        assert_eq!(
            s.active_row(),
            1,
            "row 0 is folded; gg lands on row 1 instead"
        );
        // `G` must not land inside the trailing fold.
        s.handle_events(KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('G'));
        assert_eq!(
            s.active_row(),
            2,
            "row 3 is folded; G lands on row 2 instead"
        );
        // `k` from row 1 (the header, the first unfolded row) must not step into the
        // leading fold at row 0 even though row 0 is the only row left in that direction.
        s.set_active_row(1);
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('k'));
        assert_eq!(s.active_row(), 1, "row 0 is folded; k has nowhere to go");
        // `j` from row 2 (the pair, the last unfolded row) must not step into the
        // trailing fold at row 3 even though row 3 is the only row left in that direction.
        s.set_active_row(2);
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('j'));
        assert_eq!(s.active_row(), 2, "row 3 is folded; j has nowhere to go");
    }

    #[test]
    /// UI-R-257 — the display mode defaults to hunk-only.
    fn ut_display_mode_defaults_to_hunk_only() {
        let s = DiffViewStateBuilder::default()
            .build_with_diff_and_file("@@ -1,1 +1,1 @@\n-a\n+A\n", "A\n")
            .unwrap();
        assert_eq!(s.display(), DiffDisplay::HunkOnly);
    }

    #[test]
    /// UI-R-254 — rows preceding the first `@@` header (`diff --git`, `---`, `+++`) are
    /// kept as leading meta rows in the full-file build, not dropped.
    fn ut_leading_diff_header_lines_are_kept_in_the_full_file_build() {
        let s = DiffViewStateBuilder::default()
            .build_with_diff_and_file(
                "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,1 +1,1 @@\n-old\n+new\n",
                "new\n",
            )
            .unwrap();
        let DiffRow::Meta { text } = &s.rows()[0] else {
            panic!("expected the leading diff --git line to survive")
        };
        assert_eq!(text, "diff --git a/f b/f");
        let DiffRow::Meta { text } = &s.rows()[1] else {
            panic!("expected the leading --- line to survive")
        };
        assert_eq!(text, "--- a/f");
        let DiffRow::Meta { text } = &s.rows()[2] else {
            panic!("expected the leading +++ line to survive")
        };
        assert_eq!(text, "+++ b/f");
    }

    #[test]
    /// A hunk header naming a line past the supplied new-side text must not panic;
    /// external input (a mismatched patch/file pair) is handled, not trusted.
    fn ut_hunk_header_past_the_end_of_the_supplied_text_does_not_panic() {
        let s = DiffViewStateBuilder::default()
            .build_with_diff_and_file("@@ -1,1 +5,1 @@\n-old\n+new\n", "only one line\n")
            .unwrap();
        assert!(!s.rows().is_empty());
    }

    #[test]
    /// UI-R-258 — `Ctrl+F` toggles hunk-only and full-file display.
    fn ut_ctrl_f_toggles_hunk_only_and_full_file() {
        let mut s = DiffViewStateBuilder::default()
            .build_with_diff_and_file("@@ -1,1 +1,1 @@\n-a\n+A\n", "A\n")
            .unwrap();
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('f'));
        assert_eq!(s.display(), DiffDisplay::FullFile);
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('f'));
        assert_eq!(s.display(), DiffDisplay::HunkOnly);
    }

    #[test]
    /// UI-R-256, UI-R-258 — toggling back to hunk-only moves an active row (and any
    /// Visual anchor) that a fold would otherwise swallow onto a row that is not folded.
    fn ut_toggling_back_to_hunk_only_moves_an_active_row_out_of_a_folded_span() {
        let mut s = DiffViewStateBuilder::default()
            .build_with_diff_and_file("@@ -2,1 +2,1 @@\n-b\n+B\n", "a\nB\nc\n")
            .unwrap();
        let visible_height = 5;
        s.set_visible_height(visible_height);
        // Row 3 ("c") folds in hunk-only mode; select it while in full-file mode.
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('f'));
        s.set_active_row(3);
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('f'));
        assert_eq!(s.display(), DiffDisplay::HunkOnly);
        assert!(
            !s.display_rows().is_empty() && s.active_row() != 3,
            "the active row is no longer the folded one"
        );
        let (start, end) = {
            let display = s.display_rows();
            let idx: Vec<usize> = display
                .iter()
                .enumerate()
                .filter(|(_, d)| d.logical == s.active_row())
                .map(|(i, _)| i)
                .collect();
            (*idx.first().unwrap(), *idx.last().unwrap())
        };
        assert!(
            s.scroll_offset() <= start && end < s.scroll_offset() + visible_height,
            "the scroll settles on the active row's display span"
        );
    }

    #[test]
    /// A `+0,0` hunk header (a deleted-file diff's only hunk) must not leave `next_line`
    /// at 0: the trailing-rows walk indexes `new_lines[line_no - 1]` and would panic on
    /// `line_no == 0`, which is external input, not a bug to trust away.
    fn ut_deleted_file_hunk_with_a_zero_new_side_does_not_panic() {
        let s = DiffViewStateBuilder::default()
            .build_with_diff_and_file("@@ -1,1 +0,0 @@\n-a\n", "")
            .unwrap();
        assert!(!s.rows().is_empty());
    }

    #[test]
    /// UI-R-253 — a zero-context deletion's `+n,0` header names the new-side line the
    /// removal follows, not one it owns: the new line at that number precedes the hunk
    /// and must keep the pre-hunk delta, not the hunk's own.
    fn ut_zero_context_deletion_leaves_the_preceding_new_line_unaffected() {
        let s = DiffViewStateBuilder::default()
            .build_with_diff_and_file("@@ -3,1 +2,0 @@\n-x\n", "one\ntwo\nfour\n")
            .unwrap();
        let row = s
            .rows()
            .iter()
            .find_map(|r| match r {
                DiffRow::Pair { new: Some(new), .. } if new.line_no == 2 => Some(r.clone()),
                _ => None,
            })
            .expect("new line 2 present");
        let DiffRow::Pair { old: Some(old), .. } = row else {
            panic!("expected a pair row")
        };
        assert_eq!(
            old.line_no, 2,
            "new line 2 precedes the hunk, unaffected by it"
        );
    }

    #[test]
    /// A `+0,n` hunk header (a new file's added-lines-only hunk) puts a new-side entry's
    /// `line_no` at 0; the text lookup must not underflow computing `line_no - 1`.
    fn ut_new_side_line_no_zero_does_not_underflow_the_text_lookup() {
        let s = DiffViewStateBuilder::default()
            .build_with_diff_and_file("@@ -0,0 +0,1 @@\n+added\n", "added\n")
            .unwrap();
        assert!(!s.rows().is_empty());
    }

    #[test]
    /// UI-R-257 — the display option is a builder field like any other and survives a
    /// full-file construction, not just a hunk-only one.
    fn ut_display_builder_option_survives_full_file_construction() {
        let s = DiffViewStateBuilder::default()
            .display(DiffDisplay::FullFile)
            .build_with_diff_and_file("@@ -1,1 +1,1 @@\n-a\n+A\n", "A\n")
            .unwrap();
        assert_eq!(
            s.display(),
            DiffDisplay::FullFile,
            "the builder's own display option must not be forced back to hunk-only"
        );
    }

    #[test]
    /// Amended UI-R-231 — toggling into full-file mode inserts the folded spans' rows
    /// ahead of rows that were already visible, which can push the active row off the
    /// window even though the active row itself did not move; the scroll must re-settle.
    fn ut_toggling_into_full_file_re_settles_the_scroll_on_the_active_row() {
        let mut s = DiffViewStateBuilder::default()
            .build_with_diff_and_file("@@ -2,1 +2,1 @@\n-b\n+B\n", "a\nB\nc\n")
            .unwrap();
        s.set_visible_height(2);
        // Hunk-only: rows 0 and 3 are folded, so display rows are logical 1 (header) then
        // 2 (pair) — both fit in the 2-row window at scroll 0.
        s.set_active_row(2);
        assert_eq!(s.scroll_offset(), 0);
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('f'));
        assert_eq!(s.display(), DiffDisplay::FullFile);
        assert_eq!(s.active_row(), 2, "the active row itself does not move");
        let (start, end) = {
            let display = s.display_rows();
            let idx: Vec<usize> = display
                .iter()
                .enumerate()
                .filter(|(_, d)| d.logical == 2)
                .map(|(i, _)| i)
                .collect();
            (*idx.first().unwrap(), *idx.last().unwrap())
        };
        assert!(
            s.scroll_offset() <= start && end < s.scroll_offset() + 2,
            "full file mode now shows row 0 ahead of it, pushing row 2 off-window \
             unless the scroll re-settles"
        );
    }

    #[test]
    /// UI-R-259, UI-E-113 — construction from a unified diff text alone stays hunk-only,
    /// with no full-file mode to switch to, so `Ctrl+F` is consumed and ignored.
    fn ut_diff_only_construction_stays_hunk_only_and_ctrl_f_is_consumed_and_ignored() {
        let mut s = DiffViewStateBuilder::default()
            .build_with_diff("@@ -1,1 +1,1 @@\n-a\n+A\n")
            .unwrap();
        assert_eq!(s.display(), DiffDisplay::HunkOnly);
        let result = s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('f'));
        assert!(matches!(result, EventResult::Consumed));
        assert_eq!(
            s.display(),
            DiffDisplay::HunkOnly,
            "no full-file mode to switch to"
        );
    }
}
