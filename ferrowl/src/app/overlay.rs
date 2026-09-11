//! Overlay (modal dialog) lifecycle: creation dialog (`:new`/`:load`), tab creation.

use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_ui::EventResult;

use crate::dialog::lua_help::ScriptContext;
use crate::module::MODULE_TYPES;
use crate::module::modbus::setup::ModbusSetupView;
use crate::module::type_select::TypeSelectDialog;
use crate::module::view::ModuleView;

use super::{App, DrawSurface, Focus, Level, Overlay, Tab};

impl<S: DrawSurface> App<S> {
    pub(super) async fn handle_dialog_key(
        &mut self,
        modifiers: KeyModifiers,
        code: KeyCode,
    ) -> bool {
        // Offer the key to the active dialog first; only fall back to the default Esc/Enter/
        // Tab/BackTab handling when the dialog leaves it unhandled. This is behavior-preserving
        // today (no widget state consumes those keys) and lets a future popup widget intercept
        // them while open.
        let result = match self.overlay.as_mut() {
            Some(o) => o.handle_events(modifiers, code),
            None => EventResult::Unhandled(modifiers, code),
        };
        if let EventResult::Unhandled(modifiers, code) = result {
            match (modifiers, code) {
                (KeyModifiers::NONE, KeyCode::Enter) => self.confirm_overlay().await,
                (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::BackTab) => {
                    if let Some(o) = self.overlay.as_mut() {
                        o.focus_previous();
                    }
                }
                (KeyModifiers::NONE, KeyCode::Tab) => {
                    if let Some(o) = self.overlay.as_mut() {
                        o.focus_next();
                    }
                }
                _ => {}
            }
        }
        // A confirmed close popup in the type-selector or the active creation dialog requests a close.
        let close_requested = match self.overlay.as_mut() {
            Some(Overlay::TypeSelect(d)) => d.take_close_request(),
            Some(Overlay::Creation(sv)) => sv.close_requested(),
            None => false,
        };
        if close_requested {
            self.close_overlay();
        }
        false
    }

    /// Confirm the active creation overlay.
    ///
    /// From the type selector this swaps in the chosen module type's setup dialog; from a
    /// setup dialog it creates a new tab when the dialog validates.
    async fn confirm_overlay(&mut self) {
        if let Some(Overlay::TypeSelect(d)) = &self.overlay {
            let setup = (MODULE_TYPES[d.selected_index()].new_setup_view)();
            self.overlay = Some(Overlay::Creation(setup));
            return;
        }

        let action = match &self.overlay {
            Some(Overlay::Creation(sv)) => sv.confirm().map(|(name, factory)| (name, factory())),
            _ => None,
        };
        if let Some((name, view)) = action {
            if self.tabs.iter().any(|t| t.name == name) {
                // No error slot on `SetupView` to surface this in-dialog (the field-level
                // red-border validation is purely static, see `dialog::NonEmpty`); leave the
                // dialog open and nudge via the active tab's log instead.
                self.log_active(
                    Level::Warning,
                    format!("Name '{name}' already in use by another tab"),
                )
                .await;
                return;
            }
            self.create_tab(name, view).await;
        }
    }

    /// Open the module-type selector for a new module tab (`:n`/`:new`).
    pub(super) fn enter_new(&mut self) {
        self.set_content_focus(false);
        self.overlay = Some(Overlay::TypeSelect(Box::new(TypeSelectDialog::new())));
        self.focus = Focus::Dialog;
    }

    /// Open the session-level scripts/interval dialog (`:session`).
    pub(super) fn enter_session(&mut self) {
        self.set_content_focus(false);
        self.session_dialog = Some(Box::new(crate::dialog::scripts::ScriptDialog::new(
            &self.session_scripts,
            self.session_interval,
            ScriptContext::Session,
        )));
        self.focus = Focus::Dialog;
    }

    /// Open the creation dialog pre-filled with an optional device-config path (`:l`).
    pub(super) fn enter_load(&mut self, path: Option<&str>) {
        let mut sv = ModbusSetupView::new_create();
        if let Some(path) = path {
            sv.dialog_mut()
                .config_path
                .state
                .set_input(path.to_string());
            sv.dialog_mut()
                .config_path
                .state
                .set_cursor(path.chars().count());
        }
        self.set_content_focus(false);
        self.overlay = Some(Overlay::Creation(Box::new(sv)));
        self.focus = Focus::Dialog;
    }

    /// Create and append a new tab from a `Box<dyn ModuleView>`, start its module, then
    /// close the overlay.
    async fn create_tab(&mut self, name: String, view: Box<dyn ModuleView>) {
        self.tabs.push(Tab::new_from_view(name, view));
        self.active = self.tabs.len() - 1;
        self.rebuild_registry();
        let result = self.tabs[self.active].view.handle_command("start").await;
        if let crate::module::view::CommandResult::Handled(Some((level, msg))) = result {
            self.log_active(level, msg).await;
        }
        self.close_overlay();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testkit::{MockSetup, MockView, build_app};

    fn app_with(names: &[&str]) -> App<crate::app::testkit::MockScreen> {
        build_app(names.iter().map(|n| MockView::pair(n).0.boxed()).collect())
    }

    async fn active_log_lines(app: &App<crate::app::testkit::MockScreen>) -> Vec<String> {
        app.tabs[app.active]
            .log
            .read()
            .await
            .peek_n(crate::app::LOG_SIZE)
            .into_iter()
            .map(|(_, _, line)| line)
            .collect()
    }

    #[tokio::test]
    /// UI-R-196 — confirming the new-module type selector swaps in the chosen type's setup dialog.
    async fn ut_confirm_type_selector_swaps_in_setup_dialog() {
        let mut app = app_with(&[]);
        app.enter_new();
        assert!(matches!(app.overlay, Some(Overlay::TypeSelect(_))));

        app.confirm_overlay().await;
        assert!(
            matches!(app.overlay, Some(Overlay::Creation(_))),
            "confirming the type selector must swap in the setup dialog"
        );
    }

    #[tokio::test]
    /// UI-R-198 — a new-module dialog failing validation stays open: confirming it creates no tab.
    async fn ut_confirm_invalid_setup_dialog_stays_open() {
        let mut app = app_with(&[]);
        app.overlay = Some(Overlay::Creation(Box::new(MockSetup::invalid("bad"))));
        app.confirm_overlay().await;

        assert_eq!(app.tabs.len(), 0, "an invalid dialog must not create a tab");
        assert!(
            app.overlay.is_some(),
            "an invalid dialog stays open on confirm"
        );
    }

    #[tokio::test]
    /// UI-R-025, UI-R-197 — confirming a creation dialog whose name collides with an existing tab is refused
    /// with a warning in the active tab's log and leaves the dialog open, never overwriting or
    /// duplicating the name; a non-colliding name creates and starts the tab and closes the dialog.
    async fn ut_creating_a_colliding_tab_name_is_refused_with_the_dialog_left_open() {
        let mut app = app_with(&["a"]);

        // Confirm a creation dialog that resolves to the already-used name "a".
        app.overlay = Some(Overlay::Creation(Box::new(MockSetup::new("a"))));
        app.focus = Focus::Dialog;
        app.confirm_overlay().await;

        assert_eq!(app.tabs.len(), 1, "a colliding name must not add a tab");
        assert!(
            app.overlay.is_some(),
            "the setup dialog stays open on refusal"
        );
        assert!(
            active_log_lines(&app)
                .await
                .iter()
                .any(|l| l.contains("already in use")),
            "a warning must be logged into the active tab"
        );

        // A distinct name is accepted: the tab is created, started, and the dialog closes.
        let (setup, handle_slot) = MockSetup::new_with_handle("b");
        app.overlay = Some(Overlay::Creation(Box::new(setup)));
        app.confirm_overlay().await;
        assert_eq!(app.tabs.len(), 2, "a unique name creates the tab");
        assert!(app.overlay.is_none(), "the dialog closes after creation");
        assert_eq!(
            handle_slot.lock().unwrap().as_ref().unwrap().commands(),
            vec!["start".to_string()],
            "a valid confirm must start the newly created tab's view"
        );
    }
}
