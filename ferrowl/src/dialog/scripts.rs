//! Lua script manager dialog, shared by the session (`:session`) and the OCPP/Modbus module views.
//! Layout: an interval field on top, the script table (an On/Off status over a "New Script" name
//! input) on the left, the code editor for the selected script on the right, and a read-only tail
//! of the owner's script log at the bottom.
//!
//! One flat Tab order: Interval → script table → name input → Templates button → code editor → log
//! → Interval (`Shift+Tab` reversed; the code editor is skipped while no script is selected). `t`
//! toggles a script, `d` deletes (with confirmation), `c` toggles compact rows, `e` runs the
//! selected script once (see [`ScriptDialog::take_run_request`]), `Enter` on the table renames the
//! selected script (UI-R-055), Enter in the name input creates a new (enabled) script, and the
//! Templates button opens the bundled-template browser (UI-R-052). Edits are live on a working
//! copy, applied when the dialog closes via [`ScriptDialog::resolve`]. `Esc` opens a close
//! confirmation popup (Enter/Space confirms, Esc dismisses it).
//!
//! `?` opens a help overlay, which page depending on focus: from the script table, the table's own
//! keybinds (UI-R-056); from the code editor's Normal mode, the Lua bindings for `context`.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_ui::{
    Border, COLOR_SCHEME,
    state::{ButtonState, CodeInputFieldState, InputFieldState, InputFieldStateBuilder, VimMode},
    style::InputFieldStyleBuilder,
    traits::{HandleEvents, SetFocus},
    widgets::{
        Button, CodeInputField, InputField, InputFieldBuilder, Validate, ValidateResult, Widget,
    },
};
use ferrowl_ui_derive::{Focus, focusable};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, HorizontalAlignment, Layout, Margin, Rect},
    widgets::{Block, Clear, StatefulWidget, Widget as UiWidget},
};

use crate::config::script::ScriptDef;
use crate::dialog::close_confirm::{CloseConfirmDialog, CloseConfirmOutcome, route_close_confirm};
use crate::dialog::help::{HelpOutcome, HelpOverlay, route_help};
use crate::dialog::lua_help::{ScriptContext, lua_help_overlay};
use crate::dialog::rename::{RenameOutcome, RenamePrompt, route_rename};
use crate::dialog::script_keys::script_keys_overlay;
use crate::dialog::script_manager::{self, ScriptManagerRef, ScriptTable};
use crate::dialog::template_browser::{
    TemplateBrowser, TemplateBrowserOutcome, route_template_browser,
};
use crate::module::modbus::dialog::{
    ConfirmDeleteDialog, DeleteConfirmOutcome, route_delete_confirm,
};
use crate::module::view::SharedLog;
use crate::view::border_style;
use crate::view::log::{LogEntry, LogView, format_timestamp, new_log_view};

/// Sim-cycle interval input validator: must parse as a finite, positive number of seconds
/// (mirrors `Session::interval_duration`'s sanitization, but rejected at the field instead of
/// silently falling back).
#[derive(Clone, Debug)]
pub struct Interval();

impl Validate for Interval {
    fn validate(input: &str) -> ValidateResult {
        match input.trim().parse::<f64>() {
            Ok(v) if v.is_finite() && v > 0.0 => ValidateResult::None,
            Ok(_) => ValidateResult::Error("Interval must be a positive number".to_string()),
            Err(e) => ValidateResult::Error(e.to_string()),
        }
    }

    fn allowed_char(c: char) -> bool {
        c.is_ascii_digit() || c == '.'
    }
}

/// The script manager dialog. Works on a private copy of the scripts/interval; the caller applies
/// the result via [`ScriptDialog::resolve`] on close.
///
/// Tab order (declaration order below): interval → script table → name input → Templates button →
/// code editor (skipped while no script is selected) → log.
#[focusable]
#[derive(Focus)]
pub struct ScriptDialog {
    #[focus]
    interval: Widget<InputFieldState, InputField<Interval>>,
    /// Interval the dialog was opened with — the fallback if the field is left invalid at close,
    /// so a fat-fingered edit reverts to what was already running instead of a hardcoded default.
    initial_interval: Duration,
    scripts: Vec<ScriptDef>,
    #[focus]
    table: ScriptTable,
    #[focus]
    name_input: Widget<InputFieldState, InputField<String>>,
    #[focus]
    templates_button: Widget<ButtonState, Button>,
    #[focus(when = self.selected().is_some())]
    code: Widget<CodeInputFieldState, CodeInputField>,
    #[focus]
    log: LogView,
    confirm: Option<ConfirmDeleteDialog>,
    /// The template browser opened by the Templates button (UI-R-052).
    template_browser: Option<TemplateBrowser>,
    /// The rename prompt opened by `Enter` on the script table (UI-R-055).
    rename: Option<RenamePrompt>,
    /// Set by `e` on the script table: the selected script, to be executed once by the owner
    /// (which holds the Lua modules the dialog knows nothing about). Pulled out with
    /// [`ScriptDialog::take_run_request`] after each key.
    pending_run: Option<ScriptDef>,
    /// Compact (no vertical row margin) script table; toggled with `c`. Default off (margin 1).
    compact: bool,
    close_confirm: Option<CloseConfirmDialog>,
    context: ScriptContext,
    /// The open `?` help overlay: the Lua bindings (from the code editor) or the script table's
    /// keybinds (from the table). Which page it shows is decided when it is opened (UI-R-056).
    help: Option<HelpOverlay>,
}

impl ScriptDialog {
    pub fn new(scripts: &[ScriptDef], interval: Duration, context: ScriptContext) -> Self {
        let scripts = scripts.to_vec();
        let mut interval_field = interval_input();
        set_input(&mut interval_field, &format_interval(interval));
        let mut dialog = Self {
            interval: interval_field,
            initial_interval: interval,
            table: script_manager::script_table(script_manager::rows(&scripts)),
            name_input: script_manager::name_input(border_style()),
            templates_button: script_manager::templates_button(),
            code: script_manager::code_editor(border_style()),
            log: new_log_view(),
            focus: ScriptDialogFocus::Interval,
            view_focused: true,
            confirm: None,
            template_browser: None,
            rename: None,
            pending_run: None,
            compact: false,
            close_confirm: None,
            context,
            help: None,
            scripts,
        };
        dialog.sync_code_from_selection();
        dialog
    }

    /// Apply the working copy back to the caller: the validated interval (falling back to the
    /// interval the dialog was opened with if the field is currently invalid — an invalid field
    /// must never propagate a bogus duration, and must not silently discard whatever was already
    /// running either) and the scripts list, with the open editor flushed into the selected script
    /// first so unsaved keystrokes aren't lost.
    pub fn resolve(mut self) -> (Vec<ScriptDef>, Duration) {
        self.flush_code_to_selection();
        let interval = parse_interval(self.interval.state.input()).unwrap_or(self.initial_interval);
        (self.scripts, interval)
    }

    /// Refresh the read-only log pane from a snapshot of the owner's script log ring. Called by
    /// the owner once per tick while the dialog is open. Follows the tail only while the pane is
    /// unfocused, so a user scrolling through the log isn't yanked back down every tick.
    pub fn set_log_entries(&mut self, entries: Vec<LogEntry>) {
        self.log.state.set_values(entries);
        if self.focus != ScriptDialogFocus::Log {
            self.log.state.move_to_bottom();
        }
    }

    fn manager(&mut self) -> ScriptManagerRef<'_> {
        ScriptManagerRef {
            scripts: &mut self.scripts,
            table: &mut self.table,
            name_input: &mut self.name_input,
            code: &mut self.code,
            compact: &mut self.compact,
        }
    }

    fn selected(&self) -> Option<usize> {
        script_manager::selected(&self.scripts, &self.table)
    }

    /// Load the selected script's code into the editor. Without a selection the editor is
    /// disabled: it shows only its placeholder and there is nothing edits could apply to.
    fn sync_code_from_selection(&mut self) {
        self.manager().sync_code_from_selection();
    }

    /// Write the editor's content back into the selected script.
    fn flush_code_to_selection(&mut self) {
        self.manager().flush_code_to_selection();
    }

    /// Create a new enabled script from the name input (rejecting empty / duplicate names).
    fn create_script(&mut self) {
        self.manager().create_script();
    }

    fn toggle_compact(&mut self) {
        self.manager().toggle_compact();
    }

    fn toggle_selected(&mut self) {
        self.manager().toggle_selected();
    }

    fn delete_selected(&mut self) {
        self.manager().delete_selected();
    }

    /// Queue the selected script for a one-shot run (UI-R-051). The editor is flushed first so the
    /// run uses what is on screen, not the last-synced copy; the script's enabled flag is ignored
    /// — running a disabled script on demand is the point. No selection: nothing to run.
    fn request_run(&mut self) {
        self.flush_code_to_selection();
        self.pending_run = self.selected().map(|i| self.scripts[i].clone());
    }

    /// Open the rename prompt on the selected script (UI-R-055). No selection: nothing to rename.
    fn request_rename(&mut self) {
        if let Some(i) = self.selected() {
            self.rename = Some(RenamePrompt::new(&self.scripts[i].name));
        }
    }

    /// Take the script queued by `e`, if any. The owner calls this after every key and executes
    /// the script once against its own Lua modules (SC-R-035).
    pub fn take_run_request(&mut self) -> Option<ScriptDef> {
        self.pending_run.take()
    }

    /// Handle a key. Returns `true` when the dialog should close (confirmed via the close-confirm
    /// popup).
    pub fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> bool {
        // An open help overlay (Lua bindings or script-table keys) takes precedence over everything
        // else (UI-R-056).
        match route_help(&mut self.help, modifiers, code) {
            HelpOutcome::NotActive => {}
            HelpOutcome::Consumed => return false,
        }

        // The template browser takes precedence over the dialog's own keys while open (UI-R-053).
        match route_template_browser(&mut self.template_browser, modifiers, code) {
            TemplateBrowserOutcome::NotActive => {}
            TemplateBrowserOutcome::Consumed => return false,
            TemplateBrowserOutcome::Insert(template) => {
                self.manager().insert_template(template);
                return false;
            }
        }

        // The rename prompt takes precedence too — notably over `Esc`, which cancels the prompt
        // rather than opening the dialog's close-confirm (UI-R-055).
        match route_rename(&mut self.rename, modifiers, code) {
            RenameOutcome::NotActive => {}
            RenameOutcome::Consumed => return false,
            RenameOutcome::Commit(name) => {
                // A refused (empty/duplicate) name leaves the prompt open to be corrected.
                if self.manager().rename_selected(&name) {
                    self.rename = None;
                }
                return false;
            }
        }

        // The close-confirm popup takes precedence once open.
        match route_close_confirm(&mut self.close_confirm, modifiers, code) {
            CloseConfirmOutcome::NotActive => {}
            CloseConfirmOutcome::Close => return true,
            CloseConfirmOutcome::Consumed => return false,
        }

        // Delete-confirmation sub-dialog takes precedence.
        match route_delete_confirm(&mut self.confirm, modifiers, code) {
            DeleteConfirmOutcome::NotActive => {}
            DeleteConfirmOutcome::Confirmed => {
                self.delete_selected();
                return false;
            }
            DeleteConfirmOutcome::Consumed => return false,
        }

        // Intercept `?` before offering the key to the editor, but only in the code editor's
        // Normal mode: in Insert/Visual mode it is valid Lua text and must fall through
        // unchanged.
        if self.focus == ScriptDialogFocus::Code
            && self.code.state.vim_mode() == VimMode::Normal
            && let (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('?')) =
                (modifiers, code)
        {
            self.help = Some(lua_help_overlay(self.context));
            return false;
        }

        // The vim-modal code editor must see keys before the dialog: in Insert mode it
        // consumes Esc (back to Normal) and Tab/BackTab (indent/dedent); only keys it
        // leaves unhandled (e.g. Esc/Tab in Normal mode) fall through to dialog handling.
        if self.focus == ScriptDialogFocus::Code
            && let ferrowl_ui::EventResult::Consumed =
                self.code.state.handle_events(modifiers, code)
        {
            self.flush_code_to_selection();
            return false;
        }

        match (modifiers, code) {
            (KeyModifiers::NONE, KeyCode::Tab) => self.focus_next(),
            (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::BackTab) => self.focus_previous(),
            (KeyModifiers::NONE, KeyCode::Esc) => {
                self.close_confirm = Some(CloseConfirmDialog::new());
            }
            _ => match self.focus {
                ScriptDialogFocus::Interval => {
                    let _ = self.interval.state.handle_events(modifiers, code);
                }
                ScriptDialogFocus::Table => match (modifiers, code) {
                    // UI-R-056 — the table's bindings are documented in an overlay, not in a title
                    // that no longer has room for them.
                    (KeyModifiers::NONE | KeyModifiers::SHIFT, KeyCode::Char('?')) => {
                        self.help = Some(script_keys_overlay());
                    }
                    (KeyModifiers::NONE, KeyCode::Enter) => self.request_rename(),
                    (KeyModifiers::NONE, KeyCode::Char('e')) => self.request_run(),
                    (KeyModifiers::NONE, KeyCode::Char('t')) => self.toggle_selected(),
                    (KeyModifiers::NONE, KeyCode::Char('c')) => self.toggle_compact(),
                    (KeyModifiers::NONE, KeyCode::Char('d')) => {
                        if let Some(i) = self.selected() {
                            self.confirm = Some(ConfirmDeleteDialog::new(&self.scripts[i].name));
                        }
                    }
                    _ => {
                        let _ = self.table.state.handle_events(modifiers, code);
                        self.sync_code_from_selection();
                    }
                },
                ScriptDialogFocus::NameInput => match (modifiers, code) {
                    (KeyModifiers::NONE, KeyCode::Enter) => self.create_script(),
                    _ => {
                        let _ = self.name_input.state.handle_events(modifiers, code);
                    }
                },
                ScriptDialogFocus::TemplatesButton => {
                    if let (KeyModifiers::NONE, KeyCode::Enter | KeyCode::Char(' ')) =
                        (modifiers, code)
                    {
                        self.template_browser = Some(TemplateBrowser::new(self.context));
                    }
                }
                // Code-focus keys were already offered to the editor above; anything
                // reaching this arm was left unhandled by it.
                ScriptDialogFocus::Code => {}
                ScriptDialogFocus::Log => {
                    let _ = self.log.state.handle_events(modifiers, code);
                }
            },
        }
        false
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        // Centered box covering most of the screen.
        let [_, hc, _] = Layout::horizontal([
            Constraint::Percentage(10),
            Constraint::Percentage(80),
            Constraint::Percentage(10),
        ])
        .areas(area);
        let [_, vc, _] = Layout::vertical([
            Constraint::Percentage(5),
            Constraint::Percentage(90),
            Constraint::Percentage(5),
        ])
        .areas(hc);

        UiWidget::render(&Clear, vc, buf);
        let block = Block::bordered()
            .style(
                ratatui::prelude::Style::default()
                    .fg(COLOR_SCHEME.hi)
                    .bg(COLOR_SCHEME.bg),
            )
            .title_alignment(HorizontalAlignment::Center)
            .title(match self.context {
                ScriptContext::Session => "Session",
                ScriptContext::Modbus => "Modbus Scripts",
                ScriptContext::OcppClient => "OCPP Client Scripts",
                ScriptContext::OcppServer => "OCPP Server Scripts",
            });
        let block_inner = block.inner(vc);
        block.render(vc, buf);
        let inner = block_inner.inner(Margin {
            vertical: 1,
            horizontal: 2,
        });

        let [scripts_area, log_area] =
            Layout::vertical([Constraint::Min(10), Constraint::Length(10)]).areas(inner);

        // Script manager pane: interval over the table over the name input on the left, code
        // editor right.
        let [left, right] =
            Layout::horizontal([Constraint::Max(50), Constraint::Min(1)]).areas(scripts_area);
        let [interval_area, list_area, input_area, button_area] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .areas(left);

        self.interval
            .state
            .set_focused(self.focus == ScriptDialogFocus::Interval);
        self.table
            .state
            .set_focused(self.focus == ScriptDialogFocus::Table);
        self.name_input
            .state
            .set_focused(self.focus == ScriptDialogFocus::NameInput);
        self.templates_button
            .state
            .set_focused(self.focus == ScriptDialogFocus::TemplatesButton);
        self.code
            .state
            .set_focused(self.focus == ScriptDialogFocus::Code);
        self.log
            .state
            .set_focused(self.focus == ScriptDialogFocus::Log);

        StatefulWidget::render(
            &self.interval.widget,
            interval_area,
            buf,
            &mut self.interval.state,
        );
        StatefulWidget::render(&self.table.widget, list_area, buf, &mut self.table.state);
        StatefulWidget::render(
            &self.name_input.widget,
            input_area,
            buf,
            &mut self.name_input.state,
        );
        StatefulWidget::render(
            &self.templates_button.widget,
            button_area,
            buf,
            &mut self.templates_button.state,
        );
        StatefulWidget::render(&self.code.widget, right, buf, &mut self.code.state);
        StatefulWidget::render(&self.log.widget, log_area, buf, &mut self.log.state);

        if let Some(confirm) = self.confirm.as_mut() {
            confirm.render(vc, buf);
        }

        if let Some(browser) = self.template_browser.as_mut() {
            browser.render(area, buf);
        }

        if let Some(rename) = self.rename.as_mut() {
            rename.render(area, buf);
        }

        if let Some(help) = self.help.as_mut() {
            help.render(area, buf);
        }

        if let Some(confirm) = self.close_confirm.as_mut() {
            confirm.render(area, buf);
        }
    }
}

fn parse_interval(input: &str) -> Option<Duration> {
    let v = input.trim().parse::<f64>().ok()?;
    (v.is_finite() && v > 0.0).then(|| Duration::from_secs_f64(v))
}

fn format_interval(interval: Duration) -> String {
    let secs = interval.as_secs_f64();
    // Trim a trailing ".0" so a whole-second interval reads as "1", not "1.0000..".
    let text = format!("{secs:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn set_input(widget: &mut Widget<InputFieldState, InputField<Interval>>, value: &str) {
    widget.state.set_input(value.to_string());
    widget.state.set_cursor(value.chars().count());
}

fn interval_input() -> Widget<InputFieldState, InputField<Interval>> {
    Widget {
        state: InputFieldStateBuilder::default()
            .focused(false)
            .disabled(false)
            .placeholder(Some("1.0".to_string()))
            .allowed_for::<Interval>()
            .build()
            .expect("all required builder fields are set"),
        widget: InputFieldBuilder::default()
            .border(Border::Full(Margin::new(1, 0)))
            .title(Some(
                ("Interval (seconds)", HorizontalAlignment::Left).into(),
            ))
            .style(
                InputFieldStyleBuilder::default()
                    .build()
                    .expect("all required builder fields are set"),
            )
            .margin(Margin {
                vertical: 0,
                horizontal: 0,
            })
            .build()
            .expect("all required builder fields are set"),
    }
}

/// Build `LogEntry` rows from a raw `(timestamp_ms, level, message)` ring snapshot, matching the
/// formatting `App::refresh_snapshot` applies to tab logs.
pub fn entries_from_ring(lines: Vec<(u64, crate::app::Level, String)>) -> Vec<LogEntry> {
    lines
        .into_iter()
        .map(|(ts, level, msg)| LogEntry {
            timestamp: format_timestamp(ts),
            level,
            message: msg.trim_end_matches('\u{0}').to_string(),
        })
        .collect()
}

/// Snapshot `log`'s current lines as render-ready entries. Async because the log is behind a
/// `tokio::sync::RwLock`.
pub async fn snapshot_log(log: &SharedLog, max: usize) -> Vec<LogEntry> {
    let lines = log.read().await.peek_n(max);
    entries_from_ring(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> ScriptDialog {
        ScriptDialog::new(
            &[ScriptDef {
                name: "boot".into(),
                code: String::new(),
                enabled: true,
            }],
            Duration::from_secs_f64(2.5),
            ScriptContext::Modbus,
        )
    }

    #[test]
    fn ut_new_prefills_interval_and_scripts() {
        let d = dialog();
        assert_eq!(d.interval.state.input(), "2.5");
        let (scripts, _) = d.resolve();
        assert_eq!(scripts.len(), 1);
    }

    #[test]
    /// UI-R-048 — the interval field validator accepts a positive finite value.
    fn ut_interval_validate_accepts_positive_finite() {
        assert!(matches!(Interval::validate("2.5"), ValidateResult::None));
        assert!(matches!(Interval::validate("1"), ValidateResult::None));
    }

    #[test]
    /// UI-R-048 — the interval field validator rejects non-finite/non-positive values.
    fn ut_interval_validate_rejects_bad_values() {
        assert!(matches!(Interval::validate("0"), ValidateResult::Error(_)));
        assert!(matches!(Interval::validate("-1"), ValidateResult::Error(_)));
        assert!(matches!(
            Interval::validate("nan"),
            ValidateResult::Error(_)
        ));
        assert!(matches!(
            Interval::validate("inf"),
            ValidateResult::Error(_)
        ));
        assert!(matches!(
            Interval::validate("abc"),
            ValidateResult::Error(_)
        ));
    }

    #[test]
    /// UI-R-048 — the interval field's per-character filter admits only valid characters.
    fn ut_interval_allowed_char() {
        assert!(Interval::allowed_char('5'));
        assert!(Interval::allowed_char('.'));
        assert!(!Interval::allowed_char('-'));
        assert!(!Interval::allowed_char('e'));
        assert!(!Interval::allowed_char(' '));
        assert!(!Interval::allowed_char('a'));
    }

    #[test]
    fn ut_resolve_round_trips_scripts_and_interval() {
        let d = dialog();
        let (scripts, interval) = d.resolve();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].name, "boot");
        assert_eq!(interval, Duration::from_secs_f64(2.5));
    }

    #[test]
    fn ut_resolve_falls_back_on_invalid_interval() {
        // Falls back to the interval the dialog was opened with (2.5s, per `dialog()`), not a
        // hardcoded default — an invalid edit must not silently reset an already-running sim.
        let mut d = dialog();
        set_input(&mut d.interval, "not-a-number");
        let (_, interval) = d.resolve();
        assert_eq!(interval, Duration::from_secs_f64(2.5));
    }

    // The dialog-wide Tab rotation: Interval → table → name input → code editor → log →
    // Interval. The fixture has one script, so the code editor is reachable.
    #[test]
    /// UI-R-022 — Tab rotates focus through every enabled dialog field.
    fn ut_tab_rotates_through_all_fields() {
        let mut d = dialog();
        assert_eq!(d.focus, ScriptDialogFocus::Interval);
        let expected = [
            ScriptDialogFocus::Table,
            ScriptDialogFocus::NameInput,
            ScriptDialogFocus::TemplatesButton,
            ScriptDialogFocus::Code,
            ScriptDialogFocus::Log,
            ScriptDialogFocus::Interval,
        ];
        for focus in expected {
            assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Tab));
            assert_eq!(d.focus, focus);
        }
    }

    #[test]
    /// UI-R-022 — Shift+Tab rotates dialog focus in reverse.
    fn ut_backtab_rotates_in_reverse() {
        let mut d = dialog();
        let expected = [
            ScriptDialogFocus::Log,
            ScriptDialogFocus::Code,
            ScriptDialogFocus::TemplatesButton,
            ScriptDialogFocus::NameInput,
            ScriptDialogFocus::Table,
            ScriptDialogFocus::Interval,
        ];
        for focus in expected {
            assert!(!d.handle_events(KeyModifiers::SHIFT, KeyCode::BackTab));
            assert_eq!(d.focus, focus);
        }
    }

    // Without a selected script the code editor is disabled and both rotations skip it.
    #[test]
    /// UI-R-078 — the focus cycle skips a disabled field (the code editor).
    fn ut_rotation_skips_disabled_code_editor() {
        let mut d = ScriptDialog::new(&[], Duration::from_secs(1), ScriptContext::Modbus);
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> name input
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> templates button
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // code skipped -> log
        assert_eq!(d.focus, ScriptDialogFocus::Log);
        d.handle_events(KeyModifiers::SHIFT, KeyCode::BackTab); // code skipped -> templates button
        assert_eq!(d.focus, ScriptDialogFocus::TemplatesButton);
    }

    #[test]
    /// UI-R-023 — Esc on the dialog opens the close-confirm rather than closing immediately.
    fn ut_esc_does_not_close() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Esc));
        assert_eq!(d.focus, ScriptDialogFocus::Table);
        assert!(d.close_confirm.is_some());
    }

    #[test]
    /// UI-R-048 — typing into the interval field updates its input.
    fn ut_typing_in_interval_updates_input() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('9'));
        assert!(d.interval.state.input().contains('9'));
    }

    #[test]
    /// UI-R-058 — the script table adds, toggles-enabled, and deletes scripts in its working list.
    fn ut_create_toggle_delete_script() {
        let mut d = ScriptDialog::new(&[], Duration::from_secs(1), ScriptContext::Modbus);
        // Tab to the name input, type a name, Enter creates an enabled script.
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> name input
        for c in "sim".chars() {
            d.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
        }
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        let (scripts, _) = d.resolve();
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].name, "sim");
        assert!(scripts[0].enabled);
    }

    #[test]
    /// UI-R-051 — `e` on the script table requests a one-shot run of the selected script.
    fn ut_e_requests_run_of_selected_script() {
        let mut d = ScriptDialog::new(
            &[ScriptDef {
                name: "boot".into(),
                code: "print(1)".into(),
                enabled: false,
            }],
            Duration::from_secs(1),
            ScriptContext::Modbus,
        );
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        // Edit in the code editor, then come back to the table without leaving the dialog: the
        // run must see the edited buffer, not the code the script was created with.
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> name input
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> templates button
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> code editor
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('i')); // vim insert
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('x'));
        d.handle_events(KeyModifiers::NONE, KeyCode::Esc); // back to Normal
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> log
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> interval
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table

        assert!(d.take_run_request().is_none(), "no run requested yet");
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('e'));

        let run = d.take_run_request().expect("e queues a run");
        assert_eq!(run.name, "boot");
        assert!(
            run.code.contains('x'),
            "unsaved edit must run: {}",
            run.code
        );
        assert!(!run.enabled, "a disabled script still runs on demand");
        assert!(d.take_run_request().is_none(), "request is taken once");
    }

    #[test]
    /// UI-R-199 — `e` with no script selected is a no-op.
    fn ut_e_without_selection_is_noop() {
        let mut d = ScriptDialog::new(&[], Duration::from_secs(1), ScriptContext::Modbus);
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table (empty)
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('e'));
        assert!(d.take_run_request().is_none());
    }

    #[test]
    /// UI-R-051 — `e` is a table binding: in the name input it is literal text, not a run.
    fn ut_e_in_name_input_types_char() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> name input
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('e'));
        assert!(d.take_run_request().is_none());
        assert_eq!(d.name_input.state.input(), "e");
    }

    #[test]
    fn ut_entries_from_ring_trims_nul_padding() {
        let entries = entries_from_ring(vec![(
            0,
            crate::app::Level::Info,
            "hello\u{0}\u{0}".to_string(),
        )]);
        assert_eq!(entries[0].message, "hello");
    }

    #[test]
    fn ut_format_interval_trims_trailing_zeros() {
        assert_eq!(format_interval(Duration::from_secs_f64(1.0)), "1");
        assert_eq!(format_interval(Duration::from_secs_f64(0.25)), "0.25");
    }

    #[test]
    fn ut_layout_scripts_table_wide_screen() {
        let wide_area = Rect {
            x: 0,
            y: 0,
            width: 220,
            height: 30,
        };
        let [left, right] =
            Layout::horizontal([Constraint::Max(50), Constraint::Min(1)]).areas(wide_area);
        assert_eq!(left.width, 50);
        assert!(right.width >= 1);
    }

    #[test]
    fn ut_layout_scripts_table_narrow_screen() {
        let narrow_area = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 30,
        };
        let [left, right] =
            Layout::horizontal([Constraint::Max(50), Constraint::Min(1)]).areas(narrow_area);
        assert!(left.width < 50);
        assert!(right.width >= 1);
    }

    // --- close-confirm / lua-help integration -------------------------------

    #[test]
    /// UI-R-023 — Esc then Enter confirms the close and dismisses the dialog.
    fn ut_esc_then_enter_closes() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Esc));
        assert!(d.close_confirm.is_some());
        assert!(d.handle_events(KeyModifiers::NONE, KeyCode::Enter));
    }

    #[test]
    /// UI-R-023 — Esc in the close-confirm returns to the dialog.
    fn ut_esc_in_confirm_keeps_dialog() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Esc));
        assert!(d.close_confirm.is_some());
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Esc));
        assert!(d.close_confirm.is_none());
    }

    #[test]
    /// UI-R-023 — Space in the close-confirm confirms the close.
    fn ut_space_in_confirm_closes() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert!(d.close_confirm.is_some());
        assert!(d.handle_events(KeyModifiers::NONE, KeyCode::Char(' ')));
    }

    #[test]
    /// UI-R-023 — Esc from the code editor's Normal mode opens the close-confirm.
    fn ut_esc_from_code_normal_opens_confirm() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> name input
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> templates button
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> code
        assert_eq!(d.focus, ScriptDialogFocus::Code);
        assert_eq!(d.code.state.vim_mode(), VimMode::Normal);
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Esc));
        assert!(d.close_confirm.is_some());
    }

    #[test]
    /// UI-R-023 — Esc from Insert mode returns to Normal without opening the close-confirm.
    fn ut_insert_esc_goes_normal_no_confirm() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> name input
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> templates button
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> code
        assert_eq!(d.focus, ScriptDialogFocus::Code);
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('i'));
        assert_eq!(d.code.state.vim_mode(), VimMode::Insert);
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Esc));
        assert_eq!(d.code.state.vim_mode(), VimMode::Normal);
        assert!(d.close_confirm.is_none());
    }

    #[test]
    /// UI-R-014 — `:` types into the name input rather than entering command mode.
    fn ut_colon_in_name_input_types() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> name input
        assert_eq!(d.focus, ScriptDialogFocus::NameInput);
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Char(':')));
        assert_eq!(d.name_input.state.input(), ":");
    }

    #[test]
    /// UI-R-014 — `:` inserts into the code editor in Insert mode.
    fn ut_colon_in_code_insert_inserts() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> name input
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> templates button
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> code
        assert_eq!(d.focus, ScriptDialogFocus::Code);
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('i'));
        assert_eq!(d.code.state.vim_mode(), VimMode::Insert);
        d.handle_events(KeyModifiers::NONE, KeyCode::Char(':'));
        assert!(d.code.state.content().contains(':'));
    }

    #[test]
    /// UI-R-014 — `:` in the code editor's Normal mode opens no command line.
    fn ut_colon_in_code_normal_no_overlay() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> name input
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> templates button
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> code
        assert_eq!(d.focus, ScriptDialogFocus::Code);
        assert_eq!(d.code.state.vim_mode(), VimMode::Normal);
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Char(':')));
        assert!(d.close_confirm.is_none());
        assert!(d.help.is_none());
    }

    #[test]
    /// UI-R-023, UI-R-092 — `d` on the selected script opens the delete confirmation; Esc in it cancels the delete.
    fn ut_confirm_esc_still_cancels() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('d'));
        assert!(d.confirm.is_some());
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Esc));
        assert!(d.confirm.is_none());
    }

    #[test]
    /// UI-R-056 — `?` opens the keybind help only from the code editor's Normal mode (guarded).
    fn ut_question_opens_bindings_only_code_normal() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table

        // From Code Insert mode: `?` is text.
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> name input
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> templates button
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> code
        assert_eq!(d.focus, ScriptDialogFocus::Code);
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('i'));
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('?'));
        assert!(d.code.state.content().contains('?'));
        assert!(d.help.is_none());

        // From Code Normal mode: `?` opens the overlay.
        d.handle_events(KeyModifiers::NONE, KeyCode::Esc);
        assert_eq!(d.code.state.vim_mode(), VimMode::Normal);
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('?'));
        assert!(d.help.is_some());
    }

    #[test]
    /// UI-R-186 — Esc/q/? close the bindings help overlay.
    fn ut_bindings_close_keys() {
        for close_key in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('?')] {
            let mut d = dialog();
            d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
            d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> name input
            d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> templates button
            d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> code
            d.handle_events(KeyModifiers::NONE, KeyCode::Char('?'));
            assert!(d.help.is_some());
            assert!(!d.handle_events(KeyModifiers::NONE, close_key));
            assert!(d.help.is_none());
        }
    }

    // --- templates ------------------------------------------------------

    /// Tab to the Templates button (interval → table → name input → button).
    fn focus_templates_button(d: &mut ScriptDialog) {
        while d.focus != ScriptDialogFocus::TemplatesButton {
            d.handle_events(KeyModifiers::NONE, KeyCode::Tab);
        }
    }

    #[test]
    /// UI-R-052 — the Templates button sits in the focus cycle and opens the template browser.
    fn ut_templates_button_in_focus_cycle_and_opens_browser() {
        for open_key in [KeyCode::Enter, KeyCode::Char(' ')] {
            let mut d = dialog();
            d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
            d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> name input
            d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> templates button
            assert_eq!(d.focus, ScriptDialogFocus::TemplatesButton);

            assert!(d.template_browser.is_none());
            assert!(!d.handle_events(KeyModifiers::NONE, open_key));
            assert!(d.template_browser.is_some(), "{open_key:?} must open it");
        }
    }

    #[test]
    /// UI-R-201 — the open template browser takes precedence over all other dialog keys.
    fn ut_open_browser_takes_precedence_over_dialog_keys() {
        let mut d = dialog();
        focus_templates_button(&mut d);
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);

        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Esc));
        assert!(d.template_browser.is_none(), "Esc closes the browser");
        assert!(d.close_confirm.is_none(), "and not the dialog");
    }

    #[test]
    /// UI-R-054 — confirming a template appends it as a new enabled script.
    fn ut_insert_template_appends_enabled_script() {
        let mut d = dialog();
        focus_templates_button(&mut d);
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter); // open browser
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Enter)); // insert first template
        assert!(d.template_browser.is_none());

        let template = crate::script_template::templates(ScriptContext::Modbus)[0];
        assert_eq!(d.selected(), Some(1), "the new script is selected");
        let (scripts, _) = d.resolve();
        assert_eq!(scripts.len(), 2);
        assert_eq!(scripts[1].name, template.name);
        assert_eq!(scripts[1].code, template.code);
        assert!(scripts[1].enabled);
    }

    #[test]
    /// UI-R-054 — inserting the same template twice suffixes the second rather than refusing it.
    fn ut_insert_duplicate_template_auto_suffixes() {
        let mut d = dialog();
        for _ in 0..2 {
            focus_templates_button(&mut d);
            d.handle_events(KeyModifiers::NONE, KeyCode::Enter); // open browser
            d.handle_events(KeyModifiers::NONE, KeyCode::Enter); // insert
        }
        let template = crate::script_template::templates(ScriptContext::Modbus)[0];
        let (scripts, _) = d.resolve();
        assert_eq!(scripts.len(), 3);
        assert_eq!(scripts[1].name, template.name);
        assert_eq!(scripts[2].name, format!("{}-2", template.name));
    }

    // --- rename ---------------------------------------------------------

    #[test]
    /// UI-R-055, UI-R-202, UI-R-204 — Enter on a selected script opens the rename prompt and renames it.
    fn ut_enter_on_table_renames_selected_script() {
        let mut d = ScriptDialog::new(
            &[ScriptDef {
                name: "boot".into(),
                code: "print('hi')".into(),
                enabled: false,
            }],
            Duration::from_secs(1),
            ScriptContext::Modbus,
        );
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Enter));
        assert!(d.rename.is_some());

        for c in ['-', '2'] {
            d.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
        }
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Enter));
        assert!(d.rename.is_none(), "an accepted name closes the prompt");

        let (scripts, _) = d.resolve();
        assert_eq!(scripts[0].name, "boot-2");
        assert_eq!(scripts[0].code, "print('hi')");
        assert!(!scripts[0].enabled);
    }

    #[test]
    /// UI-R-089 — an empty or duplicate name is refused and the prompt stays open.
    fn ut_rename_refuses_empty_and_duplicate() {
        let mut d = ScriptDialog::new(
            &[
                ScriptDef {
                    name: "boot".into(),
                    code: String::new(),
                    enabled: true,
                },
                ScriptDef {
                    name: "other".into(),
                    code: String::new(),
                    enabled: true,
                },
            ],
            Duration::from_secs(1),
            ScriptContext::Modbus,
        );
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table (row 0: boot)
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter); // open the prompt

        // Clear the field, then commit an empty name.
        for _ in 0..4 {
            d.handle_events(KeyModifiers::NONE, KeyCode::Backspace);
        }
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(d.rename.is_some(), "empty name must be refused");

        // Type the other script's name: also refused.
        for c in "other".chars() {
            d.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
        }
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        assert!(d.rename.is_some(), "duplicate name must be refused");

        assert_eq!(d.scripts[0].name, "boot", "the name is unchanged");
    }

    #[test]
    /// UI-R-203, UI-R-080, UI-R-090 — Esc cancels the rename (the prompt consumes it, so no close-confirm opens); Enter on an empty table is a no-op.
    fn ut_rename_esc_cancels_and_empty_table_is_noop() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        d.handle_events(KeyModifiers::NONE, KeyCode::Enter);
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('x'));
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Esc));
        assert!(d.rename.is_none());
        assert!(d.close_confirm.is_none(), "Esc belonged to the prompt");
        assert_eq!(d.scripts[0].name, "boot");

        let mut empty = ScriptDialog::new(&[], Duration::from_secs(1), ScriptContext::Modbus);
        empty.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table (empty)
        assert!(!empty.handle_events(KeyModifiers::NONE, KeyCode::Enter));
        assert!(empty.rename.is_none());
    }

    // --- script-table keybind help --------------------------------------

    #[test]
    /// UI-R-056 — `?` on the script table opens the keybind help, with or without a selection.
    fn ut_question_mark_on_table_opens_script_keys_help() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        assert!(!d.handle_events(KeyModifiers::NONE, KeyCode::Char('?')));
        assert!(d.help.is_some());

        let mut empty = ScriptDialog::new(&[], Duration::from_secs(1), ScriptContext::Modbus);
        empty.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table (empty)
        empty.handle_events(KeyModifiers::NONE, KeyCode::Char('?'));
        assert!(empty.help.is_some());
    }

    #[test]
    /// UI-R-186 — `Esc`, `q` and `?` each close the overlay.
    fn ut_script_keys_help_closes_on_esc_q_question() {
        for close_key in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('?')] {
            let mut d = dialog();
            d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
            d.handle_events(KeyModifiers::NONE, KeyCode::Char('?'));
            assert!(d.help.is_some());
            assert!(!d.handle_events(KeyModifiers::NONE, close_key));
            assert!(d.help.is_none());
        }
    }

    #[test]
    /// UI-R-187 — the script-keys help overlay takes precedence over other dialog keys.
    fn ut_script_keys_help_takes_precedence() {
        let mut d = dialog();
        d.handle_events(KeyModifiers::NONE, KeyCode::Tab); // -> table
        d.handle_events(KeyModifiers::NONE, KeyCode::Char('?'));

        // `d` would open the delete-confirm, `e` would queue a run, `Enter` would rename — while
        // the overlay is open, none of them reach the table.
        for key in [KeyCode::Char('d'), KeyCode::Char('e'), KeyCode::Enter] {
            assert!(!d.handle_events(KeyModifiers::NONE, key));
        }
        assert!(d.confirm.is_none());
        assert!(d.rename.is_none());
        assert!(d.take_run_request().is_none());
        assert!(d.help.is_some(), "only Esc/q/? close it");
    }
}
