//! Key routing for the non-dialog panes: command line, table/log navigation, tab switching.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_ui::EventResult;
use ferrowl_ui::traits::{HandleEvents, SetFocus};

use crate::app::{DIGIT_CHORD_TIMEOUT, KeyMode};

use super::{App, DrawSurface, Focus};

/// Result of feeding one digit into the `Ctrl+t` tab-jump chord.
#[derive(Debug, PartialEq, Eq)]
enum DigitOutcome {
    /// First digit could still be the start of a valid two-digit index: hold it and wait.
    Wait(usize),
    /// Jump straight to this tab index.
    Jump(usize),
    /// No tab can match; drop the chord.
    Ignore,
}

/// Decide what a newly typed digit `d` means for the tab-jump chord, given any `pending` first
/// digit and the number of tabs `ntabs`. Pure so the chord logic is testable without an `App`.
fn digit_outcome(pending: Option<usize>, d: usize, ntabs: usize) -> DigitOutcome {
    match pending {
        None => {
            // A leading 0 never needs a second digit: every "0x" index is reachable as "x".
            if d != 0 && d * 10 < ntabs {
                DigitOutcome::Wait(d)
            } else if d < ntabs {
                DigitOutcome::Jump(d)
            } else {
                DigitOutcome::Ignore
            }
        }
        Some(first) => {
            let idx = first * 10 + d;
            DigitOutcome::Jump(if idx < ntabs { idx } else { first })
        }
    }
}

impl<S: DrawSurface> App<S> {
    pub(super) async fn handle_command_key(
        &mut self,
        modifiers: KeyModifiers,
        code: KeyCode,
    ) -> bool {
        match code {
            KeyCode::Esc => self.exit_command(),
            KeyCode::Enter => {
                let cmd = self.command.state.input().trim().to_string();
                self.exit_command();
                return self.run_command(&cmd).await;
            }
            _ => {
                let _ = self.command.state.handle_events(modifiers, code);
            }
        }
        false
    }

    /// Returns `true` when the app must quit.
    pub(super) fn handle_nav_key(&mut self, modifiers: KeyModifiers, code: KeyCode) -> bool {
        // The help dialog is modal: it eats every key so nothing leaks to the view beneath it.
        if self.help_open {
            match code {
                KeyCode::Esc | KeyCode::Char('q' | '?') => {
                    self.help_open = false;
                    self.help_scroll = 0;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    self.help_scroll = self.help_scroll.saturating_add(1)
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.help_scroll = self.help_scroll.saturating_sub(1)
                }
                KeyCode::Char('g') => self.help_scroll = 0,
                // Clamped to the last line at render time, which knows the viewport height.
                KeyCode::Char('G') => self.help_scroll = u16::MAX,
                _ => {}
            }
            return false;
        }
        match (&self.keymode, modifiers, code) {
            // Window switch
            (None, KeyModifiers::CONTROL, KeyCode::Char('w')) => {
                self.keymode = Some(KeyMode::CtrlWin)
            }
            (Some(KeyMode::CtrlWin), _, KeyCode::Char('j' | 'k') | KeyCode::Down | KeyCode::Up) => {
                self.keymode = None;
                self.toggle_pane();
            }
            // Tab switch
            (None, KeyModifiers::CONTROL, KeyCode::Char('t')) => {
                self.keymode = Some(KeyMode::CtrlTab)
            }
            (Some(KeyMode::CtrlTab), _, KeyCode::Char('l')) => {
                self.keymode = None;
                self.next_tab();
            }
            (Some(KeyMode::CtrlTab), _, KeyCode::Char('h')) => {
                self.keymode = None;
                self.prev_tab();
            }
            // Ctrl+t then a digit: jump straight there, or wait for a second digit if one could
            // still form a valid two-digit index.
            (Some(KeyMode::CtrlTab), _, KeyCode::Char(c)) if c.is_ascii_digit() => {
                let d = c
                    .to_digit(10)
                    .expect("c is an ASCII digit, matched by the guard above")
                    as usize;
                match digit_outcome(None, d, self.tabs.len()) {
                    DigitOutcome::Wait(first) => {
                        self.keymode = Some(KeyMode::TabDigit {
                            first,
                            deadline: Instant::now() + DIGIT_CHORD_TIMEOUT,
                        });
                    }
                    DigitOutcome::Jump(idx) => {
                        self.keymode = None;
                        self.switch_tab(idx);
                    }
                    DigitOutcome::Ignore => self.keymode = None,
                }
            }
            // Second digit of the chord, within the timeout window.
            (Some(KeyMode::TabDigit { first, .. }), _, KeyCode::Char(c)) if c.is_ascii_digit() => {
                let first = *first;
                let d = c
                    .to_digit(10)
                    .expect("c is an ASCII digit, matched by the guard above")
                    as usize;
                self.keymode = None;
                if let DigitOutcome::Jump(idx) = digit_outcome(Some(first), d, self.tabs.len()) {
                    self.switch_tab(idx);
                }
            }
            // Command
            (None, _, KeyCode::Char(':'))
                if !self
                    .tabs
                    .get_mut(self.active)
                    .is_some_and(|t| t.view.is_overlay_active()) =>
            {
                self.enter_command()
            }
            // Keybind help, guarded like `:` so `?` still types into module edit fields.
            (None, _, KeyCode::Char('?'))
                if !self
                    .tabs
                    .get_mut(self.active)
                    .is_some_and(|t| t.view.is_overlay_active()) =>
            {
                self.help_open = true
            }
            (_, _, _) => {
                // A pending single-digit jump that never got a second digit still commits before
                // the key is forwarded to the tab.
                if let Some(KeyMode::TabDigit { first, .. }) = self.keymode {
                    self.switch_tab(first);
                }
                self.keymode = None;
                self.forward_nav(modifiers, code);
            }
        }
        false
    }

    /// Forward a key to the active tab, which dispatches to whichever of its panes (content view or
    /// log) currently holds focus. Returns `true` if consumed.
    fn forward_nav(&mut self, modifiers: KeyModifiers, code: KeyCode) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return false;
        };
        match self.focus {
            Focus::Content => matches!(tab.handle_events(modifiers, code), EventResult::Consumed),
            Focus::Command | Focus::Dialog => false,
        }
    }

    fn enter_command(&mut self) {
        self.set_content_focus(false);
        self.focus = Focus::Command;
        self.command.state.set_input(String::new());
        self.command.state.set_cursor(0);
        self.command.state.set_focused(true);
    }

    fn exit_command(&mut self) {
        self.command.state.set_focused(false);
        self.command.state.set_input(String::new());
        self.command.state.set_cursor(0);
        self.focus = Focus::Content;
        self.set_content_focus(true);
    }

    /// `Ctrl+w` j/k: toggle focus between the active tab's content view and its log pane. Only
    /// reachable while the content surface is focused (the modal layers route keys elsewhere).
    fn toggle_pane(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.focus_next();
        }
    }

    pub(super) fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.set_content_focus(false);
            self.active = (self.active + 1) % self.tabs.len();
            self.set_content_focus(self.focus == Focus::Content);
        }
    }

    pub(super) fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.set_content_focus(false);
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
            self.set_content_focus(self.focus == Focus::Content);
        }
    }

    /// Jump straight to tab `idx`. Out-of-range indices are a silent no-op.
    pub(super) fn switch_tab(&mut self, idx: usize) {
        if idx < self.tabs.len() && idx != self.active {
            self.set_content_focus(false);
            self.active = idx;
            self.set_content_focus(self.focus == Focus::Content);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// UI-R-011 — a single digit jumps immediately when no two-digit index could start with it.
    fn single_digit_jumps_immediately_when_at_most_ten_tabs() {
        assert_eq!(digit_outcome(None, 3, 5), DigitOutcome::Jump(3));
    }

    #[test]
    /// UI-R-074 — a first digit waits for a second when a valid two-digit index could start with it.
    fn first_digit_waits_when_it_could_start_a_two_digit_index() {
        assert_eq!(digit_outcome(None, 1, 25), DigitOutcome::Wait(1));
    }

    #[test]
    /// UI-R-074 — a valid two-digit combination jumps to that index.
    fn valid_two_digit_combo_jumps() {
        assert_eq!(digit_outcome(Some(1), 2, 25), DigitOutcome::Jump(12));
    }

    #[test]
    /// UI-R-075 — an out-of-range two-digit combination falls back to the first-digit jump.
    fn invalid_combo_falls_back_to_first_digit() {
        assert_eq!(digit_outcome(Some(1), 9, 15), DigitOutcome::Jump(1));
    }

    #[test]
    /// UI-R-011 — an out-of-range single digit is ignored.
    fn out_of_range_single_digit_is_ignored() {
        assert_eq!(digit_outcome(None, 7, 5), DigitOutcome::Ignore);
    }

    #[test]
    /// UI-R-011 — a leading zero jumps immediately regardless of tab count.
    fn leading_zero_jumps_immediately_even_with_many_tabs() {
        assert_eq!(digit_outcome(None, 0, 25), DigitOutcome::Jump(0));
    }

    use crate::app::testkit::{MockView, build_app};

    /// Feed `Ctrl+<lead>` then `key` through the nav handler as a two-key chord.
    fn chord(app: &mut App<crate::app::testkit::MockScreen>, lead: char, key: KeyCode) {
        app.handle_nav_key(KeyModifiers::CONTROL, KeyCode::Char(lead));
        app.handle_nav_key(KeyModifiers::empty(), key);
    }

    fn app_with(names: &[&str]) -> App<crate::app::testkit::MockScreen> {
        build_app(names.iter().map(|n| MockView::pair(n).0.boxed()).collect())
    }

    #[test]
    /// UI-R-006, UI-R-072 — the `:` command line removes focus from the content pane while open and restores
    /// it on close, and every transition routes through the single focus choke point.
    fn ut_command_line_removes_and_restores_content_focus() {
        let mut app = app_with(&["a"]);
        assert!(app.tabs[0].view.is_focused(), "content starts focused");
        assert!(
            !app.tabs[0].is_log_focused(),
            "focus is content, not log — never both"
        );

        app.enter_command();
        assert_eq!(app.focus, Focus::Command);
        assert!(
            !app.tabs[0].view.is_focused(),
            "content unfocused while command open"
        );

        app.exit_command();
        assert_eq!(app.focus, Focus::Content);
        assert!(
            app.tabs[0].view.is_focused(),
            "content focus restored on close"
        );
    }

    #[test]
    /// UI-R-005, UI-R-071 — an open modal layer consumes the keys its lower layers would otherwise receive:
    /// while the keybind-help dialog is open, a `:` that would open the command line at the content
    /// layer is swallowed, leaving help open and the command line unopened.
    fn ut_open_help_layer_consumes_lower_layer_keys() {
        let mut app = app_with(&["a"]);
        assert!(!app.help_open);

        app.handle_nav_key(KeyModifiers::empty(), KeyCode::Char('?'));
        assert!(app.help_open, "`?` opens the keybind-help layer");

        // `:` would open the command line at the content layer; the open help layer eats it.
        app.handle_nav_key(KeyModifiers::empty(), KeyCode::Char(':'));
        assert!(app.help_open, "help stays open");
        assert_eq!(
            app.focus,
            Focus::Content,
            "the command line must not have opened beneath the modal help layer"
        );

        // Esc dismisses help, restoring the lower layers.
        app.handle_nav_key(KeyModifiers::empty(), KeyCode::Esc);
        assert!(!app.help_open, "Esc closes the help layer");
    }

    #[test]
    /// UI-R-009 — the `Ctrl+w` chord toggles focus between the active tab's content view and its
    /// log pane.
    fn ut_ctrl_w_chord_toggles_content_and_log_focus() {
        let mut app = app_with(&["a"]);
        assert!(!app.tabs[0].is_log_focused());

        chord(&mut app, 'w', KeyCode::Char('j'));
        assert!(
            app.tabs[0].is_log_focused(),
            "Ctrl+w j moves focus to the log pane"
        );

        chord(&mut app, 'w', KeyCode::Char('k'));
        assert!(
            !app.tabs[0].is_log_focused(),
            "Ctrl+w k moves focus back to content"
        );
    }

    #[test]
    /// UI-R-010 — the `Ctrl+t` chord: `l`/`h` step to the next/previous tab wrapping at the ends,
    /// and a digit begins an index jump.
    fn ut_ctrl_t_chord_switches_tabs_wrapping_and_by_digit() {
        let mut app = app_with(&["a", "b", "c"]);
        assert_eq!(app.active, 0);

        chord(&mut app, 't', KeyCode::Char('l'));
        assert_eq!(app.active, 1, "l advances");
        chord(&mut app, 't', KeyCode::Char('l'));
        chord(&mut app, 't', KeyCode::Char('l'));
        assert_eq!(app.active, 0, "l wraps past the last tab");

        chord(&mut app, 't', KeyCode::Char('h'));
        assert_eq!(app.active, 2, "h wraps past the first tab");

        chord(&mut app, 't', KeyCode::Char('1'));
        assert_eq!(app.active, 1, "a digit jumps straight to that index");
    }

    #[test]
    /// UI-R-012 — a jump to an out-of-range or already-active index is a silent no-op, and tab
    /// switching is safe with zero or one tabs.
    fn ut_tab_jump_out_of_range_or_active_is_a_noop_and_safe_at_edges() {
        let mut app = app_with(&["a", "b"]);
        app.switch_tab(9);
        assert_eq!(app.active, 0, "out-of-range index ignored");
        app.switch_tab(0);
        assert_eq!(app.active, 0, "already-active index ignored");
        app.switch_tab(1);
        assert_eq!(app.active, 1, "valid index switches");

        // One tab: every switch is a no-op, no panic.
        let mut one = app_with(&["only"]);
        one.switch_tab(3);
        one.next_tab();
        one.prev_tab();
        assert_eq!(one.active, 0);

        // Zero tabs (startup selector open): switching must not panic.
        let mut none = build_app(vec![]);
        none.switch_tab(0);
        none.next_tab();
        none.prev_tab();
        assert_eq!(none.active, 0);
    }

    #[test]
    /// UI-R-077 — a non-digit key while a single-digit chord is pending commits the pending jump
    /// first, then is delivered normally to the tab the jump landed on.
    fn ut_non_digit_commits_the_pending_tab_jump_and_is_then_delivered() {
        let (t0, h0) = MockView::pair("t0");
        let (t1, h1) = MockView::pair("t1");
        let mut views = vec![t0.boxed(), t1.boxed()];
        for n in 2..25 {
            views.push(MockView::pair(&format!("t{n}")).0.boxed());
        }
        let mut app = build_app(views);

        app.handle_nav_key(KeyModifiers::CONTROL, KeyCode::Char('t'));
        app.handle_nav_key(KeyModifiers::empty(), KeyCode::Char('1'));
        assert!(matches!(
            app.keymode,
            Some(KeyMode::TabDigit { first: 1, .. })
        ));
        assert_eq!(app.active, 0, "the jump has not landed yet");

        app.handle_nav_key(KeyModifiers::empty(), KeyCode::Char('j'));

        assert_eq!(app.active, 1, "the pending jump committed");
        assert!(app.keymode.is_none());
        assert_eq!(
            h1.keys(),
            vec![(KeyModifiers::empty(), KeyCode::Char('j'))],
            "the key is delivered to the tab the jump landed on"
        );
        assert!(
            h0.keys().is_empty(),
            "the key is not delivered to the tab the jump left"
        );
    }
}
