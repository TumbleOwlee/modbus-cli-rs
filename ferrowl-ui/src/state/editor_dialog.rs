use crossterm::event::{KeyCode, KeyModifiers};
use derive_builder::Builder;

use crate::state::{MarkdownInputFieldState, MarkdownInputFieldStateBuilder, VimMode};
use crate::traits::{HandleEvents, SetFocus};

/// Outcome of a key offered to an open [`EditorDialogState`] via
/// [`handle_key`](EditorDialogState::handle_key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorDialogOutcome {
    /// `Enter` in `Normal` mode with non-blank text confirmed and closed the dialog
    /// (UI-R-202); carries the field's text exactly as typed, untrimmed.
    Confirmed(String),
    /// `Esc` in `Normal` mode cancelled and closed the dialog (UI-R-204).
    Cancelled,
    /// The key stayed inside the dialog: blank-text `Enter` in `Normal` (UI-R-203), or any
    /// key offered to the field (UI-R-205, UI-E-095).
    Consumed,
}

/// State of an [`EditorDialog`](crate::widgets::EditorDialog): a
/// [`MarkdownInputFieldState`] plus the open flag. Both fields mutate only through
/// [`open`](Self::open) and [`handle_key`](Self::handle_key).
#[derive(Builder, Debug, Clone)]
pub struct EditorDialogState {
    #[builder(
        default = "MarkdownInputFieldStateBuilder::default().build().expect(\"MarkdownInputFieldStateBuilder fields all default\")"
    )]
    field: MarkdownInputFieldState,
    #[builder(default = "false")]
    open: bool,
}

impl Default for EditorDialogState {
    fn default() -> Self {
        EditorDialogStateBuilder::default()
            .build()
            .expect("EditorDialogStateBuilder fields all default")
    }
}

impl EditorDialogState {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn field(&self) -> &MarkdownInputFieldState {
        &self.field
    }

    pub(crate) fn field_mut(&mut self) -> &mut MarkdownInputFieldState {
        &mut self.field
    }

    /// UI-R-201 — opens with empty text in `Normal` mode, whatever mode the field was left
    /// in: the blur/focus cycle resets `CodeInputFieldState`'s mode to `Normal`.
    pub fn open(&mut self) {
        self.open = true;
        SetFocus::set_focused(&mut self.field, false);
        SetFocus::set_focused(&mut self.field, true);
        self.field.set_content("");
    }

    fn close(&mut self) {
        self.open = false;
    }

    pub fn text(&self) -> String {
        self.field.content()
    }

    /// UI-R-202, UI-R-203, UI-R-204, UI-R-205 — `None` while the dialog is closed.
    pub fn handle_key(
        &mut self,
        modifiers: KeyModifiers,
        code: KeyCode,
    ) -> Option<EditorDialogOutcome> {
        if !self.open {
            return None;
        }
        if self.field.vim_mode() == VimMode::Normal {
            match code {
                KeyCode::Enter => {
                    if self.field.content().trim().is_empty() {
                        return Some(EditorDialogOutcome::Consumed);
                    }
                    let text = self.field.content();
                    self.close();
                    return Some(EditorDialogOutcome::Confirmed(text));
                }
                KeyCode::Esc => {
                    self.close();
                    return Some(EditorDialogOutcome::Cancelled);
                }
                _ => {}
            }
        }
        self.field.handle_events(modifiers, code);
        Some(EditorDialogOutcome::Consumed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_str(s: &mut EditorDialogState, text: &str) {
        for c in text.chars() {
            s.handle_key(KeyModifiers::NONE, KeyCode::Char(c));
        }
    }

    #[test]
    /// UI-R-201 — a freshly opened dialog is empty and in Normal mode.
    fn ut_dialog_opens_with_empty_text_in_normal_mode() {
        let mut s = EditorDialogState::default();
        s.open();
        assert_eq!(s.text(), "");
        assert_eq!(s.field().vim_mode(), VimMode::Normal);

        // Leave the field in Insert, then reopen: still Normal, still empty.
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('i'));
        type_str(&mut s, "stale text");
        assert_eq!(s.field().vim_mode(), VimMode::Insert);
        s.open();
        assert_eq!(s.text(), "");
        assert_eq!(s.field().vim_mode(), VimMode::Normal);
    }

    #[test]
    /// UI-R-202 — Enter in Normal with non-blank text confirms with the exact text and
    /// closes.
    fn ut_enter_in_normal_with_text_confirms_and_closes() {
        let mut s = EditorDialogState::default();
        s.open();
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('i'));
        type_str(&mut s, "hello");
        s.handle_key(KeyModifiers::NONE, KeyCode::Esc);
        assert_eq!(s.field().vim_mode(), VimMode::Normal);
        let outcome = s.handle_key(KeyModifiers::NONE, KeyCode::Enter);
        assert_eq!(
            outcome,
            Some(EditorDialogOutcome::Confirmed("hello".to_string()))
        );
        assert!(!s.is_open());
    }

    #[test]
    /// UI-R-203 — Enter in Normal with blank (empty or whitespace-only) text keeps the
    /// dialog open and reports consumed.
    fn ut_enter_in_normal_with_blank_text_keeps_the_dialog_open() {
        let mut s = EditorDialogState::default();
        s.open();
        let outcome = s.handle_key(KeyModifiers::NONE, KeyCode::Enter);
        assert_eq!(outcome, Some(EditorDialogOutcome::Consumed));
        assert!(s.is_open());

        s.handle_key(KeyModifiers::NONE, KeyCode::Char('i'));
        type_str(&mut s, "   ");
        s.handle_key(KeyModifiers::NONE, KeyCode::Esc);
        let outcome = s.handle_key(KeyModifiers::NONE, KeyCode::Enter);
        assert_eq!(outcome, Some(EditorDialogOutcome::Consumed));
        assert!(s.is_open());
    }

    #[test]
    /// UI-R-204 — Esc in Normal cancels and closes.
    fn ut_esc_in_normal_cancels_and_closes() {
        let mut s = EditorDialogState::default();
        s.open();
        let outcome = s.handle_key(KeyModifiers::NONE, KeyCode::Esc);
        assert_eq!(outcome, Some(EditorDialogOutcome::Cancelled));
        assert!(!s.is_open());
    }

    #[test]
    /// UI-R-205 — Esc in Insert only returns the field to Normal and leaves the dialog
    /// open.
    fn ut_esc_in_insert_returns_the_field_to_normal_and_leaves_the_dialog_open() {
        let mut s = EditorDialogState::default();
        s.open();
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('i'));
        assert_eq!(s.field().vim_mode(), VimMode::Insert);
        let outcome = s.handle_key(KeyModifiers::NONE, KeyCode::Esc);
        assert_eq!(outcome, Some(EditorDialogOutcome::Consumed));
        assert_eq!(s.field().vim_mode(), VimMode::Normal);
        assert!(s.is_open());
    }

    #[test]
    /// UI-E-095 — Enter in Insert splits the line and does not confirm.
    fn ut_enter_in_insert_splits_the_line_and_does_not_confirm() {
        let mut s = EditorDialogState::default();
        s.open();
        s.handle_key(KeyModifiers::NONE, KeyCode::Char('i'));
        type_str(&mut s, "ab");
        let outcome = s.handle_key(KeyModifiers::NONE, KeyCode::Enter);
        assert_eq!(outcome, Some(EditorDialogOutcome::Consumed));
        assert!(s.is_open());
        assert_eq!(s.text(), "ab\n");
    }

    #[test]
    /// While closed, no key is handled (UI-R-202..205 precondition "while open").
    fn ut_closed_dialog_returns_none() {
        let mut s = EditorDialogState::default();
        assert_eq!(s.handle_key(KeyModifiers::NONE, KeyCode::Enter), None);
    }
}
