use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use getset::{CopyGetters, Getters, Setters};

use super::code_input_field::{CodeInputFieldState, CodeInputFieldStateBuilder};
use super::vim::VimMode;
use crate::EventResult;
use crate::traits::{HandleEvents, IsFocus, SetFocus};

/// Markdown input field state: a vim-modal editor (UI-R-181) plus the display-row layout
/// and scroll bookkeeping wrapping requires.
#[derive(Builder, Debug, Clone, Getters, CopyGetters, Setters)]
#[getset(set = "pub")]
pub struct MarkdownInputFieldState {
    /// The composed vim-modal editor state (UI-R-181): buffer, modes, motions, operators,
    /// registers, single-level undo, disabled flag. Built with `vim(true)`; its `language`
    /// stays `None` (markdown gets no auto-indent and no format-on-blur).
    #[getset(get = "pub")]
    #[builder(
        default = "CodeInputFieldStateBuilder::default().vim(true).build().expect(\"defaults\")"
    )]
    inner: CodeInputFieldState,
    /// Display rows per source line from the last render; empty before the first render.
    #[getset(skip)]
    #[builder(setter(skip), default)]
    rows_per_line: Vec<usize>,
    /// Visible height in display rows from the last render.
    #[getset(skip)]
    #[builder(setter(skip), default)]
    visible_rows: usize,
    /// Scroll position, in display rows (UI-R-137).
    #[getset(get_copy = "pub")]
    #[builder(default = "0")]
    row_scroll: usize,
    /// Accumulated count prefix (UI-R-134, UI-R-139).
    #[getset(skip)]
    #[builder(setter(skip), default)]
    pending_count: Option<usize>,
    /// `g` seen, awaiting `j`/`k` (UI-R-135).
    #[getset(skip)]
    #[builder(setter(skip), default)]
    pending_g: bool,
    /// Display row within the active source line, set by `gj`/`gk` stepping and reset by
    /// any motion that changes the active line (UI-E-088: no rendered-to-source column
    /// mapping is kept, so this tracks viewport position only, never the cursor column).
    #[getset(skip)]
    #[builder(setter(skip), default)]
    line_row: usize,
    /// The count carried from a count-prefixed `yy`'s first `y`, awaiting the second `y`
    /// (UI-R-139: `2yy` yanks two lines, not `2y`).
    #[getset(skip)]
    #[builder(setter(skip), default)]
    pending_yank_count: Option<usize>,
    /// The count carried from a count-prefixed `dd`'s first `d`, awaiting the second `d`
    /// (UI-R-134: `2dd` deletes two lines, not `2d`).
    #[getset(skip)]
    #[builder(setter(skip), default)]
    pending_delete_count: Option<usize>,
}

impl MarkdownInputFieldState {
    pub fn content(&self) -> String {
        self.inner.content()
    }

    pub fn set_content(&mut self, s: &str) {
        self.inner.set_content(s);
        self.inner.set_active_line(0);
        self.inner.set_cursor_col(0);
        self.rows_per_line.clear();
        self.visible_rows = 0;
        self.row_scroll = 0;
        self.line_row = 0;
    }

    /// Whether the field is read-only (the composed editor's `disabled` flag, UI-R-181).
    pub fn read_only(&self) -> bool {
        self.inner.disabled()
    }

    /// Enabling read-only forces the inner editor back to `Normal` (UI-R-155): the wrapper's
    /// read-only filter only ever inspects the current key, not the mode, so an editor left
    /// in `Insert`/`Visual` would keep inserting/selecting right past it.
    pub fn set_read_only(&mut self, read_only: bool) {
        self.inner.set_disabled(read_only);
        if read_only {
            if self.inner.vim_mode() != VimMode::Normal {
                self.inner.handle_events(KeyModifiers::NONE, KeyCode::Esc);
            }
            self.pending_count = None;
            self.pending_g = false;
        }
    }

    pub fn vim_mode(&self) -> VimMode {
        self.inner.vim_mode()
    }

    pub fn mode_label(&self) -> Option<&'static str> {
        self.inner.mode_label()
    }

    pub fn active_line(&self) -> usize {
        self.inner.active_line()
    }

    pub fn cursor_col(&self) -> usize {
        self.inner.cursor_col()
    }

    pub fn lines(&self) -> &Vec<String> {
        self.inner.lines()
    }

    pub fn selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        self.inner.selection_range()
    }

    /// Hands the widget's last-rendered layout to the state so navigation can address
    /// display rows (UI-R-135, UI-R-136, UI-R-137).
    pub(crate) fn sync_layout(&mut self, rows_per_line: Vec<usize>, visible_rows: usize) {
        self.rows_per_line = rows_per_line;
        self.visible_rows = visible_rows;
        let max_line = self.rows_per_line.len().saturating_sub(1);
        if self.inner.active_line() <= max_line {
            let rows_here = self
                .rows_per_line
                .get(self.inner.active_line())
                .copied()
                .unwrap_or(1);
            self.line_row = self.line_row.min(rows_here.saturating_sub(1));
        }
        self.clamp_scroll();
    }

    /// Sets the cursor's display row within the active source line, computed by the widget
    /// from the wrapped styled-source layout (UI-R-137: a viewport following the cursor as
    /// it types past a wrap needs this, since `gj`/`gk` are the only other writer of
    /// `line_row` and neither fires while typing).
    pub(crate) fn sync_cursor_row(&mut self, row: usize) {
        self.line_row = row;
    }

    /// Sum of `rows_per_line[..active_line]` plus the cursor's row inside its own line, `0`
    /// when the layout cache is empty.
    pub(crate) fn cursor_display_row(&self) -> usize {
        if self.rows_per_line.is_empty() {
            return 0;
        }
        let active = self.inner.active_line().min(self.rows_per_line.len() - 1);
        let base: usize = self.rows_per_line[..active].iter().sum();
        base + self.line_row
    }

    fn total_display_rows(&self) -> usize {
        self.rows_per_line.iter().sum()
    }

    fn clamp_scroll(&mut self) {
        if self.visible_rows == 0 {
            return;
        }
        let row = self.cursor_display_row();
        if row < self.row_scroll {
            self.row_scroll = row;
        } else if row >= self.row_scroll + self.visible_rows {
            self.row_scroll = row + 1 - self.visible_rows;
        }
    }

    /// Moves the cursor `delta` display rows (positive down, negative up), clamped to the
    /// first/last display row (UI-R-136, UI-E-076); no-op when the layout cache is empty.
    fn step_display_row(&mut self, delta: isize) {
        let total = self.total_display_rows();
        if total == 0 {
            return;
        }
        let current = self.cursor_display_row() as isize;
        let target = (current + delta).clamp(0, total as isize - 1) as usize;
        let mut acc = 0usize;
        for (i, &n) in self.rows_per_line.iter().enumerate() {
            if target < acc + n {
                self.inner.set_active_line(i);
                self.line_row = target - acc;
                break;
            }
            acc += n;
        }
        self.clamp_scroll();
    }

    fn filter_read_only(&self, modifiers: KeyModifiers, code: KeyCode) -> Option<EventResult> {
        if !matches!(modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) {
            return None;
        }
        if let KeyCode::Char('h' | 'l' | '0' | '$' | 'w' | 'b' | 'e') = code {
            return Some(EventResult::Consumed);
        }
        if let KeyCode::Char(
            'x' | 'p' | 'P' | 'u' | 'd' | 'i' | 'a' | 'I' | 'A' | 'o' | 'O' | 'v' | 'V',
        ) = code
        {
            return Some(EventResult::Unhandled(modifiers, code));
        }
        if matches!(code, KeyCode::Enter | KeyCode::Backspace | KeyCode::Delete) {
            return Some(EventResult::Unhandled(modifiers, code));
        }
        None
    }
}

impl SetFocus for MarkdownInputFieldState {
    fn set_focused(&mut self, focus: bool) {
        self.inner.set_focused(focus);
    }
}

impl IsFocus for MarkdownInputFieldState {
    fn is_focused(&self) -> bool {
        self.inner.is_focused()
    }
}

impl HandleEvents for MarkdownInputFieldState {
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        // UI-R-181: Insert mode is the composed editor's, unmodified — every key (digits,
        // `g`, `j`/`k`, `d`, `y`, `G`, Ctrl+D/Ctrl+U, everything) goes straight to it before
        // any of this wrapper's Normal-mode count/prefix/read-only logic runs, so nothing
        // the wrapper intercepts elsewhere can be swallowed here instead of typed. No
        // pending chord survives Insert mode, whichever key is entering or leaving it.
        if self.inner.vim_mode() == VimMode::Insert {
            self.pending_count = None;
            self.pending_g = false;
            self.pending_yank_count = None;
            self.pending_delete_count = None;
            let result = self.inner.handle_events(modifiers, code);
            self.line_row = 0;
            self.clamp_scroll();
            return result;
        }

        if modifiers == KeyModifiers::NONE {
            if let KeyCode::Char(c @ '1'..='9') = code {
                let digit = c as usize - '0' as usize;
                self.pending_count = Some(self.pending_count.unwrap_or(0) * 10 + digit);
                self.pending_g = false;
                return EventResult::Consumed;
            }
            if code == KeyCode::Char('0')
                && let Some(n) = self.pending_count
            {
                self.pending_count = Some(n * 10);
                self.pending_g = false;
                return EventResult::Consumed;
            }
        }

        if self.inner.disabled()
            && let Some(result) = self.filter_read_only(modifiers, code)
        {
            self.pending_count = None;
            return result;
        }

        if code != KeyCode::Char('y') {
            self.pending_yank_count = None;
        }
        if code != KeyCode::Char('d') {
            self.pending_delete_count = None;
        }

        if self.pending_g {
            self.pending_g = false;
            if modifiers == KeyModifiers::NONE {
                match code {
                    KeyCode::Char('j') | KeyCode::Char('k') => {
                        self.pending_count = None;
                        if self.rows_per_line.is_empty() {
                            return self.inner.handle_events(modifiers, code);
                        }
                        let delta = if code == KeyCode::Char('j') { 1 } else { -1 };
                        self.step_display_row(delta);
                        return EventResult::Consumed;
                    }
                    KeyCode::Char('g') => {
                        self.pending_count = None;
                        self.inner
                            .handle_events(KeyModifiers::NONE, KeyCode::Char('g'));
                        let result = self
                            .inner
                            .handle_events(KeyModifiers::NONE, KeyCode::Char('g'));
                        self.line_row = 0;
                        self.clamp_scroll();
                        return result;
                    }
                    _ => {}
                }
            }
        }

        if modifiers == KeyModifiers::NONE && code == KeyCode::Char('g') {
            self.pending_g = true;
            return EventResult::Consumed;
        }

        if modifiers == KeyModifiers::CONTROL && matches!(code, KeyCode::Char('d' | 'u')) {
            self.pending_count = None;
            if self.rows_per_line.is_empty() || self.visible_rows == 0 {
                return EventResult::Consumed;
            }
            let half = (self.visible_rows / 2).max(1) as isize;
            let dir = if code == KeyCode::Char('d') { 1 } else { -1 };
            let total = self.total_display_rows();
            let max_scroll = total.saturating_sub(self.visible_rows) as isize;
            self.row_scroll = (self.row_scroll as isize + dir * half).clamp(0, max_scroll) as usize;
            self.step_display_row(dir * half);
            return EventResult::Consumed;
        }

        if modifiers == KeyModifiers::NONE && matches!(code, KeyCode::Char('j' | 'k')) {
            let count = self.pending_count.take().unwrap_or(1).max(1);
            let mut result = EventResult::Consumed;
            for _ in 0..count {
                result = self.inner.handle_events(modifiers, code);
            }
            self.line_row = 0;
            self.clamp_scroll();
            return result;
        }

        if modifiers == KeyModifiers::NONE && code == KeyCode::Char('y') {
            if let Some(count) = self.pending_yank_count.take() {
                let result = self.inner.handle_events(modifiers, code);
                self.inner.yank_lines(count.max(1));
                return result;
            }
            if let Some(count) = self.pending_count.take() {
                self.pending_yank_count = Some(count);
                return self.inner.handle_events(modifiers, code);
            }
        }

        if modifiers == KeyModifiers::NONE && code == KeyCode::Char('d') {
            if let Some(count) = self.pending_delete_count.take() {
                self.inner.cancel_pending_chord();
                self.inner.delete_lines(count.max(1));
                self.line_row = 0;
                self.clamp_scroll();
                return EventResult::Consumed;
            }
            if let Some(count) = self.pending_count.take() {
                self.pending_delete_count = Some(count);
                return self.inner.handle_events(modifiers, code);
            }
        }

        self.pending_count = None;
        let result = self.inner.handle_events(modifiers, code);
        self.line_row = 0;
        self.clamp_scroll();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::code_input_field::RegisterKind;

    fn key(s: &mut MarkdownInputFieldState, c: char) -> EventResult {
        s.handle_events(KeyModifiers::NONE, KeyCode::Char(c))
    }

    fn ctrl(s: &mut MarkdownInputFieldState, c: char) -> EventResult {
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char(c))
    }

    fn state_with(content: &str) -> MarkdownInputFieldState {
        let mut s = MarkdownInputFieldStateBuilder::default()
            .build()
            .expect("defaults");
        s.set_content(content);
        s
    }

    #[test]
    /// UI-R-181 — the markdown state composes the code editor's buffer, modes, motions,
    /// registers and single-level undo.
    fn ut_composes_code_editor_modes_motions_registers_and_undo() {
        let mut s = state_with("one\ntwo\nthree");
        key(&mut s, 'i');
        assert_eq!(s.vim_mode(), VimMode::Insert);
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('X'));
        s.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert_eq!(s.vim_mode(), VimMode::Normal);
        assert!(s.content().starts_with('X'));
        key(&mut s, 'u');
        assert_eq!(s.content(), "one\ntwo\nthree");
        key(&mut s, 'j');
        key(&mut s, 'y');
        key(&mut s, 'y');
        assert_eq!(s.inner().register(), Some(("two", RegisterKind::Linewise)));
    }

    #[test]
    /// UI-R-134 — `j`/`k` address source lines, ignoring display-row wrapping.
    fn ut_j_and_k_move_source_lines_ignoring_wrapping() {
        let mut s = state_with("one\ntwo\nthree");
        s.sync_layout(vec![3, 1, 1], 10);
        key(&mut s, 'j');
        assert_eq!(s.active_line(), 1);
        key(&mut s, 'k');
        assert_eq!(s.active_line(), 0);
    }

    #[test]
    /// UI-R-134 — a count prefix repeats `j`/`k` that many source lines.
    fn ut_count_prefix_repeats_j_and_k() {
        let mut s = state_with("a\nb\nc\nd\ne");
        key(&mut s, '3');
        key(&mut s, 'j');
        assert_eq!(s.active_line(), 3);
    }

    #[test]
    /// UI-R-135 — `gj`/`gk` move one display row, staying inside a wrapped source line.
    fn ut_gj_and_gk_move_one_display_row_inside_a_wrapped_line() {
        let mut s = state_with("wrapped\nsecond");
        s.sync_layout(vec![3, 1], 10);
        key(&mut s, 'g');
        key(&mut s, 'j');
        assert_eq!(
            s.active_line(),
            0,
            "gj should stay on the wrapped line's own rows first"
        );
        assert_eq!(s.cursor_display_row(), 1);
        key(&mut s, 'g');
        key(&mut s, 'j');
        assert_eq!(s.cursor_display_row(), 2);
        key(&mut s, 'g');
        key(&mut s, 'j');
        assert_eq!(
            s.active_line(),
            1,
            "gj crosses into the next source line's row"
        );
        key(&mut s, 'g');
        key(&mut s, 'k');
        assert_eq!(s.active_line(), 0);
        assert_eq!(s.cursor_display_row(), 2);
    }

    #[test]
    /// UI-R-136 — `Ctrl+D`/`Ctrl+U` move half a screen of display rows.
    fn ut_ctrl_d_and_ctrl_u_move_half_a_screen_of_display_rows() {
        let mut s = state_with("a\nb\nc\nd\ne\nf\ng\nh");
        s.sync_layout(vec![1, 1, 1, 1, 1, 1, 1, 1], 4);
        ctrl(&mut s, 'd');
        assert_eq!(s.cursor_display_row(), 2);
        ctrl(&mut s, 'd');
        assert_eq!(s.cursor_display_row(), 4);
        ctrl(&mut s, 'u');
        assert_eq!(s.cursor_display_row(), 2);
    }

    #[test]
    /// UI-R-136 — `Ctrl+D`/`Ctrl+U` also scroll the view by the same half-screen of display
    /// rows the cursor moves, not just follow the cursor into view.
    fn ut_ctrl_d_and_ctrl_u_scroll_the_view_by_half_a_screen() {
        let mut s = state_with("a\nb\nc\nd\ne\nf\ng\nh");
        s.sync_layout(vec![1, 1, 1, 1, 1, 1, 1, 1], 4);
        ctrl(&mut s, 'd');
        assert_eq!(s.row_scroll(), 2);
        assert_eq!(s.cursor_display_row(), 2);
        ctrl(&mut s, 'd');
        assert_eq!(s.row_scroll(), 4);
        assert_eq!(s.cursor_display_row(), 4);
        ctrl(&mut s, 'u');
        assert_eq!(s.row_scroll(), 2);
        assert_eq!(s.cursor_display_row(), 2);
    }

    #[test]
    /// UI-E-076 — `Ctrl+D`/`Ctrl+U` clamp at the first/last display row, no wrap-around.
    fn ut_ctrl_d_and_ctrl_u_clamp_at_the_first_and_last_display_row() {
        let mut s = state_with("a\nb\nc");
        s.sync_layout(vec![1, 1, 1], 4);
        ctrl(&mut s, 'u');
        assert_eq!(s.cursor_display_row(), 0);
        ctrl(&mut s, 'd');
        ctrl(&mut s, 'd');
        ctrl(&mut s, 'd');
        assert_eq!(s.cursor_display_row(), 2);
    }

    #[test]
    /// UI-R-137 — the viewport always keeps the cursor's display row visible.
    fn ut_viewport_always_keeps_the_cursor_display_row_visible() {
        let mut s = state_with("a\nb\nc\nd\ne\nf\ng\nh");
        s.sync_layout(vec![1, 1, 1, 1, 1, 1, 1, 1], 3);
        for _ in 0..6 {
            key(&mut s, 'j');
        }
        let row = s.cursor_display_row();
        assert!(row >= s.row_scroll() && row < s.row_scroll() + 3);
    }

    #[test]
    /// implementation detail — gj/gk fall back to j/k when the layout cache is empty.
    fn ut_gj_gk_fall_back_to_j_k_with_no_layout_cache() {
        let mut s = state_with("one\ntwo\nthree");
        key(&mut s, 'g');
        key(&mut s, 'j');
        assert_eq!(s.active_line(), 1);
    }

    #[test]
    /// UI-R-139 — read-only, `j`, `k`, counts and `yy` remain available.
    fn ut_read_only_keeps_j_k_counts_and_yy() {
        let mut s = state_with("one\ntwo\nthree");
        s.set_read_only(true);
        key(&mut s, 'j');
        assert_eq!(s.active_line(), 1);
        key(&mut s, 'k');
        assert_eq!(s.active_line(), 0);
        key(&mut s, 'y');
        key(&mut s, 'y');
        assert_eq!(s.inner().register(), Some(("one", RegisterKind::Linewise)));
    }

    #[test]
    /// UI-R-139 — read-only, a multi-digit count (`10j`) still repeats the motion by the
    /// full count: `0` accumulates into a pending count instead of being eaten as `$`'s
    /// horizontal-motion sibling.
    fn ut_read_only_multi_digit_count_repeats_the_motion() {
        let mut s = state_with("a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl");
        s.set_read_only(true);
        key(&mut s, '1');
        key(&mut s, '0');
        key(&mut s, 'j');
        assert_eq!(s.active_line(), 10);
    }

    #[test]
    /// UI-R-139 — read-only, `gg`/`G`/`Ctrl+D`/`Ctrl+U` remain available for line and
    /// display-row navigation, alongside `j`/`k`, counts and `yy`.
    fn ut_read_only_keeps_gg_g_ctrl_d_and_ctrl_u() {
        let mut s = state_with("a\nb\nc\nd\ne\nf\ng\nh");
        s.sync_layout(vec![1, 1, 1, 1, 1, 1, 1, 1], 4);
        s.set_read_only(true);
        key(&mut s, 'j');
        key(&mut s, 'G');
        assert_eq!(s.active_line(), 7);
        key(&mut s, 'g');
        key(&mut s, 'g');
        assert_eq!(s.active_line(), 0);
        ctrl(&mut s, 'd');
        assert_eq!(s.cursor_display_row(), 2);
        ctrl(&mut s, 'u');
        assert_eq!(s.cursor_display_row(), 0);
    }

    #[test]
    /// UI-R-139 — read-only, a count-prefixed `yy` yanks that many lines.
    fn ut_read_only_count_prefixed_yy_yanks_that_many_lines() {
        let mut s = state_with("one\ntwo\nthree");
        s.set_read_only(true);
        key(&mut s, '2');
        key(&mut s, 'y');
        key(&mut s, 'y');
        assert_eq!(
            s.inner().register(),
            Some(("one\ntwo", RegisterKind::Linewise))
        );
    }

    #[test]
    /// UI-R-139 — `2yy` requires both `y` presses; a lone count-prefixed `y` yanks nothing and
    /// does not desync inner's own pending chord.
    fn ut_count_prefixed_single_y_does_not_yank_or_desync_the_chord() {
        let mut s = state_with("one\ntwo\nthree");
        key(&mut s, '2');
        key(&mut s, 'y');
        assert_eq!(s.inner().register(), None, "a lone y must not yank yet");
        key(&mut s, 'y');
        assert_eq!(
            s.inner().register(),
            Some(("one\ntwo", RegisterKind::Linewise))
        );
    }

    #[test]
    /// UI-R-155 — read-only, every mutating and mode-entry key is reported unhandled.
    fn ut_read_only_reports_mutating_and_mode_entry_keys_unhandled() {
        let mut s = state_with("one\ntwo");
        s.set_read_only(true);
        for c in [
            'x', 'p', 'P', 'u', 'd', 'i', 'a', 'I', 'A', 'o', 'O', 'v', 'V',
        ] {
            let result = s.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
            assert!(
                matches!(result, EventResult::Unhandled(_, _)),
                "{c:?} (no modifier) should be unhandled"
            );
            let result = s.handle_events(KeyModifiers::SHIFT, KeyCode::Char(c));
            assert!(
                matches!(result, EventResult::Unhandled(_, _)),
                "{c:?} (SHIFT) should be unhandled"
            );
        }
        for code in [KeyCode::Enter, KeyCode::Backspace, KeyCode::Delete] {
            let result = s.handle_events(KeyModifiers::NONE, code);
            assert!(
                matches!(result, EventResult::Unhandled(_, _)),
                "{code:?} should be unhandled"
            );
        }
        assert_eq!(
            s.vim_mode(),
            VimMode::Normal,
            "the widget must never leave Normal mode"
        );
    }

    #[test]
    /// UI-E-088 — the cursor column on the revealed cursor line is a source column.
    fn ut_cursor_column_is_a_source_column_on_the_revealed_line() {
        let mut s = state_with("hello world");
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('l'));
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('l'));
        assert_eq!(s.cursor_col(), 2);
        assert_eq!(
            &s.lines()[s.active_line()][s.cursor_col()..s.cursor_col() + 1],
            "l"
        );
    }

    #[test]
    /// UI-E-089 — read-only, `h`/`l`/`0`/`$`/`w`/`b`/`e` are consumed and ignored.
    fn ut_read_only_horizontal_motions_are_consumed_without_moving() {
        let mut s = state_with("hello world");
        s.set_read_only(true);
        let before = (s.active_line(), s.cursor_col());
        for c in ['h', 'l', '0', '$', 'w', 'b', 'e'] {
            let result = key(&mut s, c);
            assert!(matches!(result, EventResult::Consumed));
            let result = s.handle_events(KeyModifiers::SHIFT, KeyCode::Char(c));
            assert!(
                matches!(result, EventResult::Consumed),
                "{c:?} (SHIFT) should be consumed too"
            );
        }
        assert_eq!((s.active_line(), s.cursor_col()), before);
    }

    #[test]
    /// UI-R-134 — a horizontal-motion no-op filtered in read-only mode still clears any
    /// pending count, so a stray digit before it does not leak into the next motion.
    fn ut_read_only_filtered_key_clears_the_pending_count() {
        let mut s = state_with("a\nb\nc\nd\ne");
        s.set_read_only(true);
        key(&mut s, '2');
        key(&mut s, 'x'); // filtered as unhandled, but must still drop the pending "2"
        key(&mut s, 'j');
        assert_eq!(
            s.active_line(),
            1,
            "the stray count must not carry over to j"
        );
    }

    #[test]
    /// UI-R-134 — a count prefix repeats `dd` that many source lines, mirroring `yy`.
    fn ut_count_prefixed_dd_deletes_that_many_lines() {
        let mut s = state_with("one\ntwo\nthree\nfour");
        key(&mut s, '2');
        key(&mut s, 'd');
        key(&mut s, 'd');
        assert_eq!(s.content(), "three\nfour");
        assert_eq!(
            s.inner().register(),
            Some(("one\ntwo", RegisterKind::Linewise))
        );
    }

    #[test]
    /// UI-R-135 — a digit typed while `g` is pending cancels the `g` chord instead of being
    /// folded into `gj`/`gk`'s count.
    fn ut_digit_after_g_cancels_the_pending_g_chord() {
        let mut s = state_with("wrapped\nsecond");
        s.sync_layout(vec![3, 1], 10);
        key(&mut s, 'g');
        key(&mut s, '3');
        key(&mut s, 'j');
        assert_eq!(
            s.active_line(),
            1,
            "3j after a canceled g should move 3 source lines, landing on the last one"
        );
    }

    #[test]
    /// UI-R-181 — Insert mode is the composed editor's, unmodified: `g` is not a
    /// pending-chord prefix there, so `g` then a letter inserts both characters.
    fn ut_g_is_typed_verbatim_in_insert_mode() {
        let mut s = state_with("");
        key(&mut s, 'i');
        key(&mut s, 'g');
        key(&mut s, 'x');
        s.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert_eq!(s.content(), "gx");
    }

    #[test]
    /// UI-R-181 — Insert mode forwards every key the wrapper intercepts elsewhere (a digit,
    /// `g`, `j`, `d`, `y`, `G`, Ctrl+D/Ctrl+U) straight to the composed editor unchanged:
    /// the printable ones type verbatim and the mode never leaves Insert.
    fn ut_insert_mode_forwards_every_wrapper_intercepted_key_verbatim() {
        let mut s = state_with("");
        key(&mut s, 'i');
        for c in ['1', 'g', 'j', 'd', 'y', 'G'] {
            key(&mut s, c);
        }
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('d'));
        s.handle_events(KeyModifiers::CONTROL, KeyCode::Char('u'));
        assert_eq!(s.vim_mode(), VimMode::Insert);
        assert_eq!(s.content(), "1gjdyG");
    }

    #[test]
    /// UI-R-155 — enabling read-only while Insert mode is active forces the editor back to
    /// `Normal` immediately, so the read-only filter is never bypassed by a mode it can't see.
    fn ut_enabling_read_only_forces_insert_mode_back_to_normal() {
        let mut s = state_with("one");
        key(&mut s, 'i');
        assert_eq!(s.vim_mode(), VimMode::Insert);
        s.set_read_only(true);
        assert_eq!(s.vim_mode(), VimMode::Normal);
        let result = key(&mut s, 'x');
        assert!(matches!(result, EventResult::Unhandled(_, _)));
        assert_eq!(s.content(), "one");
        assert_eq!(s.vim_mode(), VimMode::Normal);
    }

    #[test]
    /// UI-R-137 — the viewport follows the cursor's own wrapped row within the active line
    /// (set by the widget via `sync_cursor_row`), not just whole source lines, so typing
    /// past a wrap keeps the cursor visible.
    fn ut_sync_cursor_row_feeds_the_viewport_clamp() {
        let mut s = state_with("a");
        s.sync_layout(vec![5], 3);
        s.sync_cursor_row(4);
        s.sync_layout(vec![5], 3);
        assert_eq!(s.cursor_display_row(), 4);
        assert!(s.row_scroll() <= 4 && s.row_scroll() + 3 > 4);
    }

    #[test]
    /// UI-R-139 — read-only, `gg`/`G` remain available for jumping to the first/last line.
    fn ut_read_only_gg_and_shift_g_jump_to_first_and_last_line() {
        let mut s = state_with("one\ntwo\nthree");
        s.set_read_only(true);
        key(&mut s, 'j');
        s.handle_events(KeyModifiers::NONE, KeyCode::Char('G'));
        assert_eq!(s.active_line(), 2);
        key(&mut s, 'g');
        key(&mut s, 'g');
        assert_eq!(s.active_line(), 0);
    }
}
