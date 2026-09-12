//! Execution of `:` commands against the active tab: tab lifecycle and session persistence.
//! Module-specific commands are forwarded to the active view as raw strings.

use ferrowl_util::convert::{Converter, FileType};

use crate::config::Session;
use crate::module::view::CommandResult;

use super::{App, DrawSurface, Level};

/// Pure validation: usable index or error text. Unit-testable without an App.
fn validate_copy_index(
    idx: Option<usize>,
    tab_count: usize,
    active: usize,
) -> Result<usize, String> {
    let idx = idx.ok_or_else(|| "usage: :script copy <tab-index>".to_string())?;
    if idx >= tab_count {
        // `tab_count == 0` can't happen in practice (App always has a tab), but guard against
        // the `tab_count - 1` underflow anyway.
        return Err(match tab_count.checked_sub(1) {
            Some(max) => format!("no tab [{idx}] (0..={max})"),
            None => format!("no tab [{idx}] (no tabs open)"),
        });
    }
    if idx == active {
        return Err("cannot copy from the active tab".to_string());
    }
    Ok(idx)
}

impl<S: DrawSurface> App<S> {
    /// Execute a parsed `:` command. Returns `true` when the app should quit.
    pub(super) async fn run_command(&mut self, input: &str) -> bool {
        use crate::command::Cmd;
        match crate::command::parse(input) {
            Cmd::Empty => {}
            Cmd::Quit => {
                if self.tabs.len() <= 1 {
                    return true;
                }
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.view.handle_command("stop").await;
                    // UI-R-316 — a deferred stop only signals the task; join it (bounded) before
                    // the tab is dropped, so the tab close never abandons the task detached.
                    let started = std::time::Instant::now();
                    while tab.view.lifecycle_pending()
                        && started.elapsed() < crate::module::view::SETTLE_BOUND
                    {
                        tab.view.refresh().await;
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                }
                self.tabs.remove(self.active);
                self.active = self.active.min(self.tabs.len() - 1);
                self.rebuild_registry();
            }
            Cmd::QuitAll => return true,
            Cmd::New => self.enter_new(),
            Cmd::Load(path) => self.enter_load(path.as_deref()),
            Cmd::Session => self.enter_session(),
            Cmd::Write(path) => {
                let path = path.unwrap_or_else(|| "session.toml".to_string());
                match self.save_session(&path) {
                    Ok(()) => {
                        self.log_active(Level::Info, format!("Saved session to {path}"))
                            .await
                    }
                    Err(e) => {
                        self.log_active(Level::Error, format!("Save failed: {e}"))
                            .await
                    }
                }
            }
            Cmd::Log(file) => match file.as_deref() {
                Some("clear") => {
                    if let Some(tab) = self.tabs.get(self.active) {
                        tab.log.write().await.clear();
                    }
                }
                // Any other `:log ...` arg (e.g. a file path) is module-specific.
                _ => self.forward_to_view(input).await,
            },
            Cmd::ScriptCopy(idx) => {
                let (level, msg) = self.copy_scripts(idx);
                self.log_active(level, msg).await;
            }
            Cmd::Swap(from, to) => {
                let len = self.tabs.len();
                if from != to && from < len && to < len {
                    self.tabs.swap(from, to);
                }
            }
            // Everything not recognised at the app level is forwarded to the active view.
            Cmd::Unknown(_) => {
                let result = if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.view.handle_command(input).await
                } else {
                    CommandResult::Unhandled
                };
                match result {
                    CommandResult::Handled(msg) => {
                        if let Some((level, m)) = msg {
                            self.log_active(level, m).await;
                        }
                        if let Some(tab) = self.tabs.get_mut(self.active) {
                            tab.log = tab.view.log();
                        }
                    }
                    CommandResult::Unhandled => {
                        self.log_active(Level::Warning, format!("Unknown command ':{input}'"))
                            .await;
                    }
                }
            }
        }
        false
    }

    /// Forward a raw command string to the active view and log any returned message.
    async fn forward_to_view(&mut self, cmd: &str) {
        let result = if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.view.handle_command(cmd).await
        } else {
            CommandResult::Unhandled
        };
        if let CommandResult::Handled(Some((level, msg))) = result {
            self.log_active(level, msg).await;
        }
    }

    /// Save the current module instances as a session file.
    fn save_session(&self, path: &str) -> Result<(), String> {
        let ty = FileType::from_path(path)
            .ok_or_else(|| format!("unknown format for '{path}' (use .toml or .json)"))?;
        let modules: Vec<serde_json::Value> = self
            .tabs
            .iter()
            .filter_map(|t| t.view.session_spec())
            .collect();
        let session = Session {
            version: Some(crate::config::VERSION.to_string()),
            modules,
            scripts: self.session_scripts.clone(),
            interval: self.session_interval.as_secs_f64(),
        };
        Converter::save(&session, path, ty).map_err(|e| format!("{e:?}"))
    }

    /// `:script copy <idx>` — replace the active tab's script list with tab `<idx>`'s.
    fn copy_scripts(&mut self, idx: Option<usize>) -> (Level, String) {
        let src = match validate_copy_index(idx, self.tabs.len(), self.active) {
            Ok(i) => i,
            Err(e) => return (Level::Warning, e),
        };
        // Clone source list first; avoids a split borrow across tabs.
        let Some(scripts) = self.tabs[src].view.scripts().map(<[_]>::to_vec) else {
            return (Level::Warning, format!("tab [{src}] has no script support"));
        };
        let n = scripts.len();
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return (
                Level::Warning,
                "active module has no script support".to_string(),
            );
        };
        if tab.view.set_scripts(scripts) {
            (
                Level::Info,
                format!("Replaced scripts with {n} script(s) from tab [{src}]"),
            )
        } else {
            (
                Level::Warning,
                "active module has no script support".to_string(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn active_log_lines(app: &App<crate::app::testkit::MockScreen>) -> Vec<(Level, String)> {
        app.tabs[app.active]
            .log
            .read()
            .await
            .peek_n(crate::app::LOG_SIZE)
            .into_iter()
            .map(|(_, level, line)| (level, line))
            .collect()
    }

    #[tokio::test]
    /// UI-R-191 — a command the active tab's view handles has any `(level, message)` it returns
    /// appended to that tab's log.
    async fn ut_handled_command_message_appended_to_tab_log() {
        use crate::app::testkit::{MockView, build_app};
        let (v, _h) = MockView::pair("m");
        let v = v.with_command_message(Level::Error, "boom");
        let mut app = build_app(vec![v.boxed()]);
        app.run_command("frobnicate").await;
        let lines = active_log_lines(&app).await;
        assert!(
            lines
                .iter()
                .any(|(level, msg)| *level == Level::Error && msg == "boom"),
            "the view's returned (level, message) must land in the tab log: {lines:?}"
        );
    }

    #[tokio::test]
    /// UI-R-192 — a command the active tab's view leaves unhandled makes the application log
    /// `Unknown command ':<input>'` at Warning.
    async fn ut_unhandled_command_logs_unknown_command_warning() {
        use crate::app::testkit::{MockView, build_app};
        let (v, _h) = MockView::pair("m");
        let v = v.with_command_unhandled();
        let mut app = build_app(vec![v.boxed()]);
        app.run_command("bogus").await;
        let lines = active_log_lines(&app).await;
        assert!(
            lines
                .iter()
                .any(|(level, msg)| *level == Level::Warning && msg == "Unknown command ':bogus'"),
            "an unhandled command must log Unknown command at Warning: {lines:?}"
        );
    }

    #[tokio::test]
    /// UI-R-193 — the level of a command result message is chosen by the producer, never
    /// re-derived from message text: a message that reads like an error, tagged Info by its
    /// producer, is logged at Info.
    async fn ut_command_message_level_is_the_producers_not_derived_from_text() {
        use crate::app::testkit::{MockView, build_app};
        let (v, _h) = MockView::pair("m");
        let v = v.with_command_message(Level::Info, "fatal error: boom");
        let mut app = build_app(vec![v.boxed()]);
        app.run_command("frobnicate").await;
        let lines = active_log_lines(&app).await;
        assert!(
            lines
                .iter()
                .any(|(level, msg)| *level == Level::Info && msg == "fatal error: boom"),
            "the producer's chosen level must be used verbatim, not re-derived from wording: {lines:?}"
        );
    }

    #[test]
    /// UI-R-017 — `:script copy <tab-index>` validates its index (usage error, out-of-range, self-copy).
    fn ut_validate_copy_index() {
        assert_eq!(
            validate_copy_index(None, 3, 0),
            Err("usage: :script copy <tab-index>".to_string())
        );
        assert_eq!(
            validate_copy_index(Some(5), 3, 0),
            Err("no tab [5] (0..=2)".to_string())
        );
        assert_eq!(
            validate_copy_index(Some(1), 3, 1),
            Err("cannot copy from the active tab".to_string())
        );
        assert_eq!(validate_copy_index(Some(2), 3, 0), Ok(2));
    }

    use crate::app::Focus;
    use crate::app::testkit::{MockView, build_app};
    use crossterm::event::{KeyCode, KeyModifiers};
    use ferrowl_test_support::reserve_temp_dir;
    use serde_json::json;

    #[tokio::test]
    /// UI-R-015 — in command mode `Esc` cancels (discards the buffer, restores content focus),
    /// `Enter` submits the trimmed buffer, and an empty submission is a no-op.
    async fn ut_command_mode_esc_cancels_enter_submits_trimmed_empty_noop() {
        // Enter submits the buffer trimmed: the active view sees the command without surrounding
        // whitespace.
        let (v, handle) = MockView::pair("a");
        let mut app = build_app(vec![v.boxed()]);
        app.focus = Focus::Command;
        app.command.state.set_input("  frobnicate  ".to_string());
        let quit = app
            .handle_command_key(KeyModifiers::empty(), KeyCode::Enter)
            .await;
        assert!(!quit);
        assert_eq!(handle.commands(), vec!["frobnicate".to_string()]);
        assert_eq!(
            app.focus,
            Focus::Content,
            "content focus restored after submit"
        );

        // Esc discards the buffer without submitting and restores content focus.
        let (v, handle) = MockView::pair("a");
        let mut app = build_app(vec![v.boxed()]);
        app.focus = Focus::Command;
        app.command.state.set_input("frobnicate".to_string());
        app.handle_command_key(KeyModifiers::empty(), KeyCode::Esc)
            .await;
        assert_eq!(app.focus, Focus::Content);
        assert_eq!(app.command.state.input(), "", "buffer discarded on cancel");
        assert!(handle.commands().is_empty(), "Esc does not submit");

        // An empty (whitespace-only) submission does nothing.
        let (v, handle) = MockView::pair("a");
        let mut app = build_app(vec![v.boxed()]);
        app.focus = Focus::Command;
        app.command.state.set_input("   ".to_string());
        let quit = app
            .handle_command_key(KeyModifiers::empty(), KeyCode::Enter)
            .await;
        assert!(!quit);
        assert!(handle.commands().is_empty(), "empty submit is a no-op");
        assert_eq!(app.focus, Focus::Content);
    }

    #[tokio::test]
    /// UI-R-019 — `:quit` closes the active tab (stopping its module first) and quits only when it
    /// is the last tab; `:qall` quits immediately regardless of tab count.
    async fn ut_quit_closes_active_tab_qall_quits_immediately() {
        let (a, ha) = MockView::pair("a");
        let (b, _hb) = MockView::pair("b");
        let mut app = build_app(vec![a.boxed(), b.boxed()]);

        // With two tabs, :quit closes the active one (stopping it) but does not quit the app.
        assert!(!app.run_command("quit").await);
        assert_eq!(app.tabs.len(), 1);
        assert!(
            ha.commands().contains(&"stop".to_string()),
            "the closed tab's module was stopped before removal"
        );
        assert_eq!(app.tabs[0].name, "b", "the surviving tab becomes active");

        // On the last remaining tab, :quit quits the app.
        assert!(app.run_command("quit").await);

        // :qall quits immediately without closing tabs one by one.
        let (x, _hx) = MockView::pair("x");
        let (y, _hy) = MockView::pair("y");
        let mut app = build_app(vec![x.boxed(), y.boxed()]);
        assert!(app.run_command("qall").await);
        assert_eq!(
            app.tabs.len(),
            2,
            ":qall signals quit without removing tabs"
        );
    }

    #[tokio::test]
    /// UI-R-316 — closing a tab (not the last one) whose stop settles promptly waits for it, then
    /// removes the tab with the stop already joined.
    async fn ut_tab_close_settles_pending_stop_within_the_bound() {
        let (a, ha) = MockView::pair("a");
        let a = a.with_pending_stop_settling_after(2);
        let (b, _hb) = MockView::pair("b");
        let mut app = build_app(vec![a.boxed(), b.boxed()]);

        assert!(!app.run_command("quit").await);
        assert_eq!(
            app.tabs.len(),
            1,
            "the tab is removed once the stop settles"
        );
        assert!(
            ha.refreshes() >= 2,
            "the settle loop must have driven refresh() until lifecycle_pending() cleared"
        );
    }

    #[tokio::test]
    /// UI-R-316 — closing a tab whose stop never settles still proceeds once the settle bound
    /// expires, instead of hanging the command loop.
    async fn ut_tab_close_proceeds_when_the_settle_bound_expires() {
        let (a, _ha) = MockView::pair("a");
        let a = a.with_pending_stop_never_settling();
        let (b, _hb) = MockView::pair("b");
        let mut app = build_app(vec![a.boxed(), b.boxed()]);

        let before = std::time::Instant::now();
        assert!(!app.run_command("quit").await);
        let elapsed = before.elapsed();
        assert_eq!(
            app.tabs.len(),
            1,
            "the tab must still be removed once the settle bound expires"
        );
        assert!(
            elapsed >= crate::module::view::SETTLE_BOUND,
            "quit took {elapsed:?}, expected the settle loop to actually wait out the full bound \
             rather than returning immediately"
        );
        assert!(
            elapsed < crate::module::view::SETTLE_BOUND + std::time::Duration::from_millis(300),
            "quit took {elapsed:?}, expected to give up around the settle bound"
        );
    }

    #[tokio::test]
    /// CS-R-030, CS-R-069, CS-R-070 — `:write` saves the current instances as a session file,
    /// defaulting the target to `session.toml` and choosing the encoding from the path extension.
    async fn ut_write_defaults_to_session_toml_and_encodes_by_extension() {
        let dir = reserve_temp_dir("ferrowl_cs030");

        // Default target is session.toml, resolved relative to the working directory.
        let toml_path = dir.join("session.toml");
        let (v, _h) = MockView::pair("m");
        let mut app = build_app(vec![v.with_session_spec(json!({"type": "mock"})).boxed()]);
        app.run_command(&format!("write {}", toml_path.to_str().unwrap()))
            .await;
        assert!(toml_path.exists(), "explicit .toml target written");

        // The default name is exactly "session.toml".
        assert_eq!(
            crate::command::parse("write"),
            crate::command::Cmd::Write(None),
            ":write with no argument carries no path, so the default applies",
        );

        // A .json extension selects JSON encoding.
        let json_path = dir.join("out.json");
        let (v, _h) = MockView::pair("m");
        let mut app = build_app(vec![v.with_session_spec(json!({"type": "mock"})).boxed()]);
        app.run_command(&format!("write {}", json_path.to_str().unwrap()))
            .await;
        let text = std::fs::read_to_string(&json_path).unwrap();
        assert!(
            text.trim_start().starts_with('{'),
            "JSON encoding from .json"
        );
    }

    #[tokio::test]
    /// CS-R-031, CS-R-061 — a save persists configuration only, never live runtime state: the written
    /// modules are exactly each view's config spec.
    async fn ut_write_persists_config_not_runtime_state() {
        let dir = reserve_temp_dir("ferrowl_cs031");
        let spec = json!({"type": "mock", "addr": "127.0.0.1:5020"});
        let (v, _h) = MockView::pair("m");
        let mut app = build_app(vec![v.with_session_spec(spec.clone()).boxed()]);
        let path = dir.join("s.toml");
        let ps = path.to_str().unwrap();
        app.run_command(&format!("write {ps}")).await;

        let loaded = crate::config::load_session(ps).unwrap();
        assert_eq!(
            loaded.modules,
            vec![spec],
            "saved modules are the view's config spec with no runtime fields added"
        );
    }

    #[tokio::test]
    /// CS-R-032 — a `:write` writes the session file and nothing else: no device-config file is
    /// emitted alongside it.
    async fn ut_write_emits_only_the_session_file() {
        let dir = reserve_temp_dir("ferrowl_cs032");
        let (v, _h) = MockView::pair("m");
        let mut app = build_app(vec![v.with_session_spec(json!({"type": "mock"})).boxed()]);
        let path = dir.join("only.toml");
        app.run_command(&format!("write {}", path.to_str().unwrap()))
            .await;

        let files: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            files,
            vec!["only.toml".to_string()],
            "only the session file"
        );
    }
}
