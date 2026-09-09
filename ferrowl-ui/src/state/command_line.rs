use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;
use getset::{Getters, Setters};

use crate::state::InputFieldState;
use crate::traits::{HandleEvents, SetFocus};

/// Outcome of a key offered to an open [`CommandLineState`] via
/// [`handle_key`](CommandLineState::handle_key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandLineOutcome {
    /// `Enter` closed the line; carries the trimmed input text (UI-R-191, UI-E-139).
    Submit(String),
    /// `Esc` closed the line (UI-R-192).
    Cancel,
    /// Any other key was offered to the inner input (UI-R-193).
    Consumed,
}

/// State of a [`CommandLine`](crate::widgets::CommandLine): an inner
/// [`InputFieldState`] plus the open flag, an error/notice message pair and a
/// hint string, all rendered in the precedence order of UI-R-194.
#[derive(Builder, Debug, Clone, Getters, Setters)]
#[getset(set = "pub")]
pub struct CommandLineState {
    #[getset(skip)]
    #[builder(default = "false")]
    open: bool,
    #[getset(skip)]
    #[builder(default = "InputFieldState::default()")]
    input: InputFieldState,
    #[getset(get = "pub")]
    #[builder(default = "None")]
    error: Option<String>,
    #[getset(get = "pub")]
    #[builder(default = "None")]
    notice: Option<String>,
    #[getset(get = "pub")]
    #[builder(default = "String::new()")]
    hint: String,
}

impl Default for CommandLineState {
    fn default() -> Self {
        CommandLineStateBuilder::default()
            .build()
            .expect("CommandLineStateBuilder fields all default")
    }
}

impl CommandLineState {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn input(&self) -> &InputFieldState {
        &self.input
    }

    pub(crate) fn input_mut(&mut self) -> &mut InputFieldState {
        &mut self.input
    }

    /// UI-R-189, UI-R-190 — opens the line, clears and focuses the input, and leaves
    /// `error`/`notice` untouched (UI-R-195, UI-E-140).
    pub fn open(&mut self) {
        self.open = true;
        self.input.set_input(String::new());
        self.input.set_cursor(0);
        SetFocus::set_focused(&mut self.input, true);
    }

    fn close(&mut self) {
        self.open = false;
        SetFocus::set_focused(&mut self.input, false);
    }

    /// `None` while closed (UI-R-191..193 apply only "while the command line is open").
    /// UI-R-191, UI-E-139, UI-R-192, UI-R-193, UI-R-195, UI-R-198.
    pub fn handle_key(
        &mut self,
        modifiers: KeyModifiers,
        code: KeyCode,
    ) -> Option<CommandLineOutcome> {
        if !self.open {
            return None;
        }
        match code {
            KeyCode::Enter => {
                let text = self.input.input().trim().to_string();
                self.close();
                Some(CommandLineOutcome::Submit(text))
            }
            KeyCode::Esc => {
                self.close();
                Some(CommandLineOutcome::Cancel)
            }
            _ => {
                self.input.handle_events(modifiers, code);
                Some(CommandLineOutcome::Consumed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_str(s: &mut CommandLineState, text: &str) {
        for c in text.chars() {
            s.handle_key(KeyModifiers::NONE, KeyCode::Char(c));
        }
    }

    #[test]
    /// UI-R-189, UI-R-190 — opening sets open, clears the text and focuses the input.
    fn ut_open_sets_open_clears_the_text_and_focuses_the_input() {
        let mut s = CommandLineState::default();
        s.open();
        type_str(&mut s, "stale");
        s.close();
        s.open();
        assert!(s.is_open());
        assert_eq!(s.input().input(), "");
        assert!(s.input().focused());
    }

    #[test]
    /// UI-R-191 — Enter reports a trimmed submit outcome and closes the line.
    fn ut_enter_submits_the_trimmed_text_and_closes() {
        let mut s = CommandLineState::default();
        s.open();
        type_str(&mut s, "  quit  ");
        let outcome = s.handle_key(KeyModifiers::NONE, KeyCode::Enter);
        assert_eq!(
            outcome,
            Some(CommandLineOutcome::Submit("quit".to_string()))
        );
        assert!(!s.is_open());
    }

    #[test]
    /// UI-E-139 — Enter on an empty input submits the empty string and closes.
    fn ut_enter_on_empty_input_submits_the_empty_string_and_closes() {
        let mut s = CommandLineState::default();
        s.open();
        let outcome = s.handle_key(KeyModifiers::NONE, KeyCode::Enter);
        assert_eq!(outcome, Some(CommandLineOutcome::Submit(String::new())));
        assert!(!s.is_open());
    }

    #[test]
    /// UI-R-192 — Esc reports a cancel outcome and closes the line.
    fn ut_esc_cancels_and_closes() {
        let mut s = CommandLineState::default();
        s.open();
        type_str(&mut s, "abc");
        let outcome = s.handle_key(KeyModifiers::NONE, KeyCode::Esc);
        assert_eq!(outcome, Some(CommandLineOutcome::Cancel));
        assert!(!s.is_open());
    }

    #[test]
    /// UI-R-193 — every other key reaches the inner input and reports consumed.
    fn ut_other_keys_reach_the_input_and_report_consumed() {
        let mut s = CommandLineState::default();
        s.open();
        let outcome = s.handle_key(KeyModifiers::NONE, KeyCode::Char('q'));
        assert_eq!(outcome, Some(CommandLineOutcome::Consumed));
        assert_eq!(s.input().input(), "q");
    }

    #[test]
    /// UI-R-195, UI-E-140 — error and notice persist until the consumer clears them; opening
    /// and closing the line does not touch either.
    fn ut_error_and_notice_persist_until_the_consumer_clears_them() {
        let mut s = CommandLineState::default();
        s.set_error(Some("bad command".to_string()));
        s.set_notice(Some("saved".to_string()));
        s.open();
        s.handle_key(KeyModifiers::NONE, KeyCode::Esc);
        assert_eq!(s.error(), &Some("bad command".to_string()));
        assert_eq!(s.notice(), &Some("saved".to_string()));
        s.set_error(None);
        assert_eq!(s.error(), &None);
        assert_eq!(s.notice(), &Some("saved".to_string()));
    }

    #[test]
    /// UI-R-198 — the submit outcome carries the raw trimmed string, including arguments;
    /// the state derives no command from it.
    fn ut_submit_carries_the_raw_trimmed_string_including_arguments() {
        let mut s = CommandLineState::default();
        s.open();
        type_str(&mut s, "  save path/to/file.toml  ");
        let outcome = s.handle_key(KeyModifiers::NONE, KeyCode::Enter);
        assert_eq!(
            outcome,
            Some(CommandLineOutcome::Submit(
                "save path/to/file.toml".to_string()
            ))
        );
    }

    #[test]
    /// UI-R-191, UI-R-192, UI-R-193 — each applies only "while the command line is open";
    /// a closed line reports nothing.
    fn ut_closed_line_returns_none() {
        let mut s = CommandLineState::default();
        assert_eq!(s.handle_key(KeyModifiers::NONE, KeyCode::Enter), None);
    }
}
