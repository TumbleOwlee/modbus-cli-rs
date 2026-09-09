//! Rename prompt of the script dialog (UI-R-055): a small popup, pre-filled with the selected
//! script's current name. `Enter` commits, `Esc` cancels; the host refuses an empty or duplicate
//! name and leaves the prompt open.

use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_ui::{
    Border, COLOR_SCHEME,
    state::{InputFieldState, InputFieldStateBuilder},
    style::{InputFieldStyleBuilder, TextStyle},
    traits::{HandleEvents, SetFocus},
    widgets::{InputField, InputFieldBuilder, Text, TextBuilder, Widget},
};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, HorizontalAlignment, Layout, Margin, Rect},
    widgets::{Block, Clear, StatefulWidget, Widget as UiWidget},
};

use crate::view::border_style;

/// Outcome of feeding a key into a [`RenamePrompt`].
#[derive(Debug, PartialEq, Eq)]
pub enum RenameEvent {
    /// Key eaten, the prompt stays open.
    Consumed,
    /// `Esc`: drop the prompt, leave the name unchanged.
    Cancel,
    /// `Enter`: the host should try to apply this (trimmed) name.
    Commit(String),
}

/// Outcome of [`route_rename`]: what the host dialog should do about an open rename prompt.
#[derive(Debug, PartialEq, Eq)]
pub enum RenameOutcome {
    /// No prompt was open; the caller should route the key itself.
    NotActive,
    /// The prompt captured the key (cancelling itself if applicable).
    Consumed,
    /// The user confirmed a name. The prompt is **not** cleared: the host clears it only if the
    /// rename was accepted, so a refused name leaves the prompt open (UI-R-055).
    Commit(String),
}

/// The rename popup: one pre-filled input field plus a keybind hint.
pub struct RenamePrompt {
    input: Widget<InputFieldState, InputField<String>>,
    keybinds: Widget<String, Text>,
}

impl RenamePrompt {
    /// Open the prompt on `current` — the field starts pre-filled, cursor at the end, so `Enter`
    /// alone is a no-op rename rather than an empty one.
    pub fn new(current: &str) -> Self {
        let mut input = Widget {
            state: InputFieldStateBuilder::default()
                .focused(true)
                .disabled(false)
                .placeholder(Some("Script name".to_string()))
                .build()
                .expect("all required builder fields are set"),
            widget: InputFieldBuilder::default()
                .border(Border::Full(Margin::new(1, 0)))
                .title(Some(("New name", HorizontalAlignment::Left).into()))
                .style(
                    InputFieldStyleBuilder::default()
                        .border(border_style())
                        .build()
                        .expect("all required builder fields are set"),
                )
                .margin(Margin {
                    vertical: 0,
                    horizontal: 0,
                })
                .build()
                .expect("all required builder fields are set"),
        };
        input.state.set_input(current.to_string());
        input.state.set_cursor(current.chars().count());
        input.state.set_focused(true);

        Self {
            input,
            keybinds: Widget {
                state: "<Enter>: rename | <Esc>: cancel".to_string(),
                widget: TextBuilder::default()
                    .margin(Margin {
                        vertical: 0,
                        horizontal: 1,
                    })
                    .horizontal_alignment(HorizontalAlignment::Center)
                    .style(TextStyle::default())
                    .build()
                    .expect("all required builder fields are set"),
            },
        }
    }

    /// Feed one key while the prompt is open.
    pub fn handle_key(&mut self, modifiers: KeyModifiers, code: KeyCode) -> RenameEvent {
        match (modifiers, code) {
            (KeyModifiers::NONE, KeyCode::Esc) => RenameEvent::Cancel,
            (KeyModifiers::NONE, KeyCode::Enter) => {
                RenameEvent::Commit(self.input.state.input().trim().to_string())
            }
            _ => {
                let _ = self.input.state.handle_events(modifiers, code);
                RenameEvent::Consumed
            }
        }
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let [_, hc, _] = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(46),
            Constraint::Min(1),
        ])
        .areas(area);
        // 2 border + 2 margin + 3 input + 1 keybinds = 8
        let [_, popup, _] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(8),
            Constraint::Min(1),
        ])
        .areas(hc);

        let block = Block::bordered()
            .style(
                ratatui::prelude::Style::default()
                    .fg(COLOR_SCHEME.hi)
                    .bg(COLOR_SCHEME.bg),
            )
            .title_alignment(HorizontalAlignment::Center)
            .title(" Rename Script ");
        let inner = block.inner(popup).inner(Margin::new(2, 1));
        UiWidget::render(&Clear, popup, buf);
        block.render(popup, buf);

        let [input_area, keybinds_area] =
            Layout::vertical([Constraint::Length(3), Constraint::Length(1)]).areas(inner);

        StatefulWidget::render(&self.input.widget, input_area, buf, &mut self.input.state);
        StatefulWidget::render(
            &self.keybinds.widget,
            keybinds_area,
            buf,
            &mut self.keybinds.state,
        );
    }
}

/// Feed one key through `rename`, if the rename prompt is open. Clears `*rename` on cancel; on
/// commit the prompt is left in place for the host to keep or clear depending on whether the new
/// name was accepted.
pub fn route_rename(
    rename: &mut Option<RenamePrompt>,
    modifiers: KeyModifiers,
    code: KeyCode,
) -> RenameOutcome {
    let Some(prompt) = rename.as_mut() else {
        return RenameOutcome::NotActive;
    };
    match prompt.handle_key(modifiers, code) {
        RenameEvent::Consumed => RenameOutcome::Consumed,
        RenameEvent::Cancel => {
            *rename = None;
            RenameOutcome::Consumed
        }
        RenameEvent::Commit(name) => RenameOutcome::Commit(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// UI-R-055 — the prompt opens pre-filled with the current name.
    fn ut_prompt_is_prefilled() {
        let prompt = RenamePrompt::new("boot");
        assert_eq!(prompt.input.state.input(), "boot");
    }

    #[test]
    /// UI-R-202, UI-R-203 — `Enter` commits the trimmed field content, `Esc` cancels.
    fn ut_enter_commits_trimmed_and_esc_cancels() {
        let mut prompt = RenamePrompt::new("boot");
        assert_eq!(
            prompt.handle_key(KeyModifiers::NONE, KeyCode::Enter),
            RenameEvent::Commit("boot".to_string())
        );
        assert_eq!(
            prompt.handle_key(KeyModifiers::NONE, KeyCode::Esc),
            RenameEvent::Cancel
        );
    }

    #[test]
    /// Typing edits the rename prompt's field.
    fn ut_typing_edits_the_field() {
        let mut prompt = RenamePrompt::new("a");
        assert_eq!(
            prompt.handle_key(KeyModifiers::NONE, KeyCode::Char('b')),
            RenameEvent::Consumed
        );
        assert_eq!(
            prompt.handle_key(KeyModifiers::NONE, KeyCode::Enter),
            RenameEvent::Commit("ab".to_string())
        );
    }

    #[test]
    /// UI-R-202, UI-R-203 — `Esc` clears the prompt; a commit leaves it in place for the host to judge.
    fn ut_route_cancel_clears_commit_keeps() {
        let mut rename = Some(RenamePrompt::new("a"));
        assert_eq!(
            route_rename(&mut rename, KeyModifiers::NONE, KeyCode::Enter),
            RenameOutcome::Commit("a".to_string())
        );
        assert!(
            rename.is_some(),
            "a commit must not clear the prompt itself"
        );

        assert_eq!(
            route_rename(&mut rename, KeyModifiers::NONE, KeyCode::Esc),
            RenameOutcome::Consumed
        );
        assert!(rename.is_none());
    }

    #[test]
    /// Routing reports NotActive when no rename prompt is open.
    fn ut_route_not_active_when_none() {
        let mut rename: Option<RenamePrompt> = None;
        assert_eq!(
            route_rename(&mut rename, KeyModifiers::NONE, KeyCode::Enter),
            RenameOutcome::NotActive
        );
    }

    #[test]
    fn ut_render_does_not_panic() {
        let mut prompt = RenamePrompt::new("boot");
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        prompt.render(area, &mut buf);
    }
}
