//! Top-level application state and the async event/redraw loop.
//!
//! Submodules split the `App` impl by concern: key routing ([`keys`]), overlay/dialog
//! lifecycle ([`overlay`]), `:` command execution ([`commands`]) and frame rendering
//! ([`mod@render`]).

mod commands;
mod help;
mod keys;
mod overlay;
mod render;
#[cfg(test)]
mod testkit;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ferrowl_lua::module::ModuleDirectory;
use ferrowl_ring::Ring;
use ferrowl_ui::traits::SetFocus;
use ferrowl_ui::{AlternateScreen, DrawSurface};
use ferrowl_ui_derive::{Focus, focusable};
use ratatui::{buffer::Buffer, layout::Rect};
use std::io::Stdout;
use std::time::{Duration, Instant};

use crate::config::script::ScriptDef;
use crate::dialog::scripts::ScriptDialog;
use crate::module::type_descriptor::SetupView;
use crate::module::type_select::TypeSelectDialog;
use crate::module::view::{ModuleView, SharedLog};
use crate::registry::{ModuleRegistry, dedupe_names};
use crate::session::SessionSim;
use crate::view::command::{CommandLine, new_command_line};
use crate::view::log::{LogEntry, LogView, format_timestamp, new_log_view};

use render::render;

/// How often the UI redraws when no input arrives (drives live value updates).
const REDRAW_INTERVAL: Duration = Duration::from_millis(100);

/// How long to wait for a second digit after `Ctrl+t` + first digit before jumping to the tab
/// indexed by the first digit alone.
const DIGIT_CHORD_TIMEOUT: Duration = Duration::from_millis(800);

/// Ring-log dimensions for the on-screen log pane.
pub const LOG_MAX_LINE: usize = 256;
pub const LOG_SIZE: usize = 80;

/// Severity of a log line, shown as a colorized column on every log pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warning,
    Error,
}

impl Level {
    /// Label used both in the on-screen `Level` column (padded to width by the table widget)
    /// and the file-sink prefix.
    pub fn label(&self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warning => "WARNING",
            Level::Error => "ERROR",
        }
    }
}

impl From<ferrowl_lua::module::LogLevel> for Level {
    fn from(level: ferrowl_lua::module::LogLevel) -> Self {
        match level {
            ferrowl_lua::module::LogLevel::Info => Level::Info,
            ferrowl_lua::module::LogLevel::Warning => Level::Warning,
            ferrowl_lua::module::LogLevel::Error => Level::Error,
        }
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// On-screen log: a fixed-capacity ring of timestamped lines, optionally mirrored to a file so the
/// full history survives the ring's eviction (the `:log <file>` feature).
pub struct LogRing {
    ring: Ring<(u64, Level, String), LOG_SIZE>,
    /// Append-mode file sink set by `:log <file>`; when present, every line is also persisted.
    sink: Option<std::io::BufWriter<std::fs::File>>,
    /// Total number of lines ever pushed (never reset, never wraps in practice). Lets a consumer
    /// that only holds `peek_n` snapshots (e.g. the headless runner) tell exactly how many lines
    /// are new since its last look, even across ring eviction.
    written: u64,
}

impl LogRing {
    pub fn init() -> Self {
        Self {
            ring: Ring::new(),
            sink: None,
            written: 0,
        }
    }

    pub fn write(&mut self, level: Level, msg: &str) {
        let ts = ferrowl_util::time::now_unix_ms();
        let line: String = msg.chars().take(LOG_MAX_LINE).collect();
        // Persist to the file sink first (unbounded history), then push into the bounded ring.
        // Lines are buffered; `flush` runs once per UI tick (and on sink teardown via drop).
        if let Some(writer) = self.sink.as_mut() {
            use std::io::Write;
            let _ = writeln!(
                writer,
                "[{}] [{}] {line}",
                format_timestamp(ts),
                level.label()
            );
        }
        self.ring.push((ts, level, line));
        self.written += 1;
    }

    /// Total number of lines ever pushed. Monotonic; use the delta between two reads to count
    /// new lines since the last one, independent of ring eviction.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Flush buffered file-sink lines to disk. Called once per UI tick so a burst of log
    /// lines costs one syscall instead of one per line.
    pub fn flush(&mut self) {
        if let Some(writer) = self.sink.as_mut() {
            use std::io::Write;
            let _ = writer.flush();
        }
    }

    /// Point the log at a file (append): `base` resolves to `<stem>.<name>.<ext>` next to it via
    /// [`module_log_path`](crate::view::log::module_log_path). `None`, or a file that can't be
    /// opened, disables file logging.
    pub fn set_log_file(&mut self, base: Option<&str>, name: &str) {
        self.sink = base.and_then(|base| {
            let path = crate::view::log::module_log_path(base, name);
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
                .map(std::io::BufWriter::new)
        });
    }

    pub fn peek_n(&self, n: usize) -> Vec<(u64, Level, String)> {
        self.ring.peek_n(n).into_iter().cloned().collect()
    }

    pub fn clear(&mut self) {
        self.ring.clear();
    }
}

/// Which top-level surface receives input. The content↔log split lives inside the active [`Tab`]
/// (its `#[derive(Focus)]` enum), so `App` only tracks the modal layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Focus {
    /// The active tab's content/log panes (the tab decides which of the two).
    Content,
    Command,
    Dialog,
}

/// The active modal creation dialog (`:new`/`:load`), if any.
///
/// `:new` opens [`Overlay::TypeSelect`] first; confirming it swaps in
/// [`Overlay::Creation`] for the chosen module type's setup dialog.
enum Overlay {
    TypeSelect(Box<TypeSelectDialog>),
    Creation(Box<dyn SetupView>),
}

impl Overlay {
    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        match self {
            Overlay::TypeSelect(d) => d.render(area, buf),
            Overlay::Creation(sv) => sv.render(area, buf),
        }
    }

    fn focus_next(&mut self) {
        match self {
            Overlay::TypeSelect(_) => {}
            Overlay::Creation(sv) => sv.focus_next(),
        }
    }

    fn focus_previous(&mut self) {
        match self {
            Overlay::TypeSelect(_) => {}
            Overlay::Creation(sv) => sv.focus_previous(),
        }
    }

    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> ferrowl_ui::EventResult {
        match self {
            Overlay::TypeSelect(d) => d.handle_events(modifiers, code),
            Overlay::Creation(sv) => sv.handle_events(modifiers, code),
        }
    }
}

/// Per-module UI state shown under one tab.
/// A module tab: the content view plus its log pane. Itself a focusable node — `#[derive(Focus)]`
/// makes "switch content↔log" a `focus_next()` and lets `App` toggle the whole tab's focus
/// (recursing into whichever pane is active).
#[focusable]
#[derive(Focus)]
pub struct Tab {
    pub name: String,
    pub log: SharedLog,
    #[focus]
    pub view: Box<dyn ModuleView>,
    #[focus]
    pub log_view: LogView,
}

impl Tab {
    pub fn new_from_view(name: String, view: Box<dyn ModuleView>) -> Self {
        let log = view.log();
        Self {
            name,
            log,
            view,
            log_view: new_log_view(),
            focus: TabFocus::View,
            view_focused: false,
        }
    }

    /// True when the tab is focused and its log pane (not the content view) has focus.
    pub fn is_log_focused(&self) -> bool {
        self.view_focused && matches!(self.focus, TabFocus::LogView)
    }

    /// Swap in a replacement content view (e.g. after an OCPP role switch), carrying over the log
    /// channel and re-applying focus so the fresh view isn't left unfocused mid-session.
    pub fn replace_view(&mut self, new_view: Box<dyn ModuleView>) {
        self.view = new_view;
        self.log = self.view.log();
        if self.view_focused && matches!(self.focus, TabFocus::View) {
            self.view.set_focused(true);
        }
    }
}

#[derive(Clone, Copy)]
pub enum KeyMode {
    CtrlWin,
    CtrlTab,
    /// `Ctrl+t` followed by one digit, waiting to see if a second digit forms a two-digit tab
    /// index before `deadline` elapses.
    TabDigit {
        first: usize,
        deadline: Instant,
    },
}

/// Top-level application: owns the terminal and all module tabs, and runs the async
/// event/redraw loop inside the tokio runtime.
pub struct App<S: DrawSurface = AlternateScreen<Stdout>> {
    screen: S,
    tabs: Vec<Tab>,
    active: usize,
    focus: Focus,
    command: CommandLine,
    overlay: Option<Overlay>,
    keymode: Option<KeyMode>,
    /// The `?` keybind help dialog: whether it is open and its scroll offset.
    help_open: bool,
    help_scroll: u16,
    /// The tab bar's scroll offset, owned by the widget (UI-R-119).
    tab_scroll: usize,
    /// Live `C_Module` session registry, rebuilt from `tabs` whenever the tab set or a view
    /// changes (see [`Self::rebuild_registry`]).
    registry: ModuleRegistry,
    /// MB-R-150 — the single session-wide Rtu/Ascii serial-path registry, attached to every
    /// tab's view (independent of `module_host()` participation) whenever `rebuild_registry`
    /// runs.
    serial_paths: crate::module::modbus::SerialPathRegistry,
    /// The `:session` dialog, if open.
    session_dialog: Option<Box<ScriptDialog>>,
    /// Current session-level Lua scripts and sim-cycle interval, applied to `session_sim` and
    /// written by `:session` (edited) and `save_session` (persisted). Loaded at startup from the
    /// session file(s) passed on the command line, if any.
    session_scripts: Vec<ScriptDef>,
    session_interval: Duration,
    /// Log the session sim writes to, shown at the bottom of the `:session` dialog.
    session_log: SharedLog,
    /// Runs `session_scripts` against `registry` on `session_interval`, restarting whenever
    /// either changes; stopped while no script is enabled.
    session_sim: SessionSim,
}

/// UI-R-007: only key **press** events drive command/navigation; release and repeat kinds are
/// ignored. Factored out of the run loop so the filter is testable without a real event source.
fn should_act_on(kind: KeyEventKind) -> bool {
    kind == KeyEventKind::Press
}

/// UI-R-057: no tab and no open modal layer ⇒ nothing to interact with, so exit.
fn is_empty_shell(tab_count: usize, modal_open: bool) -> bool {
    tab_count == 0 && !modal_open
}

impl App<AlternateScreen<Stdout>> {
    /// Builds the app on the real terminal (raw mode, alternate screen).
    pub fn new(
        tabs: Vec<Tab>,
        session_scripts: Vec<ScriptDef>,
        session_interval: Duration,
    ) -> std::io::Result<Self> {
        Self::with_screen(
            AlternateScreen::new()?,
            tabs,
            session_scripts,
            session_interval,
        )
    }
}

impl<S: DrawSurface> App<S> {
    /// Builds the app on an injected screen. `new` wraps this with the real
    /// terminal; tests pass a `DrawSurface` mock.
    pub(crate) fn with_screen(
        screen: S,
        tabs: Vec<Tab>,
        session_scripts: Vec<ScriptDef>,
        session_interval: Duration,
    ) -> std::io::Result<Self> {
        let (overlay, focus) = if tabs.is_empty() {
            (
                Some(Overlay::TypeSelect(Box::new(TypeSelectDialog::new()))),
                Focus::Dialog,
            )
        } else {
            (None, Focus::Content)
        };
        let registry = ModuleRegistry::new();
        let serial_paths = crate::module::modbus::SerialPathRegistry::new();
        let session_log: SharedLog = std::sync::Arc::new(tokio::sync::RwLock::new(LogRing::init()));
        let mut session_sim = SessionSim::new(
            std::sync::Arc::new(registry.clone()) as std::sync::Arc<dyn ModuleDirectory>,
            session_log.clone(),
        );
        session_sim.set_interval(session_interval);
        session_sim.set_scripts(session_scripts.clone());
        let mut app = Self {
            screen,
            tabs,
            active: 0,
            focus,
            command: new_command_line(),
            overlay,
            keymode: None,
            help_open: false,
            help_scroll: 0,
            tab_scroll: 0,
            registry,
            serial_paths,
            session_dialog: None,
            session_scripts,
            session_interval,
            session_log,
            session_sim,
        };
        // Give the starting tab keyboard focus (unless a creation dialog is up).
        app.set_content_focus(app.focus == Focus::Content);
        app.rebuild_registry();
        Ok(app)
    }

    /// Snapshot the session-level `C_Module` registry from the current tab set: `Tab::name` ->
    /// `ModuleView::module_host`, skipping tabs whose view doesn't participate. Also attaches the
    /// single session-wide MB-R-150 serial-path registry to every tab's view (`set_serial_paths`,
    /// independent of `module_host()` participation). Call whenever the tab set or a view's
    /// identity changes (create/close/rename, session load, a `take_replacement` swap) so
    /// `C_Module` scripts see the current modules and every Rtu/Ascii view shares the same
    /// conflict registry.
    pub(crate) fn rebuild_registry(&mut self) {
        for tab in &mut self.tabs {
            tab.view.set_serial_paths(self.serial_paths.clone());
        }
        let modules = self
            .tabs
            .iter()
            .filter_map(|tab| Some((tab.name.clone(), tab.view.module_host()?)))
            .collect();
        self.registry.replace_all(modules);
    }

    /// Focus (or unfocus) the active tab's content/log panes. The single choke point for the
    /// event-driven focus model: every transition that changes `self.focus` routes through here so
    /// the tab's stored widget focus never goes stale.
    fn set_content_focus(&mut self, on: bool) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.set_focused(on);
        }
    }

    /// Run the async UI loop until the user quits.
    pub async fn run(&mut self) -> std::io::Result<()> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Event>(64);
        std::thread::spawn(move || {
            while let Ok(ev) = event::read() {
                if tx.blocking_send(ev).is_err() {
                    break;
                }
            }
        });

        loop {
            // UI-R-057: no tab and no modal layer left is not a resting state — the only way to
            // reach it is cancelling the startup selector (UI-R-008), so exit rather than sit on an
            // empty shell. Checked at loop top so the frame after the cancel breaks before drawing.
            if self.should_exit_empty() {
                break;
            }

            self.expire_pending_tab_digit();

            self.refresh_snapshot().await;
            self.draw()?;

            match tokio::time::timeout(REDRAW_INTERVAL, rx.recv()).await {
                Ok(Some(Event::Key(key))) if should_act_on(key.kind) => {
                    if self.handle_key(key.modifiers, key.code).await {
                        break;
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => {}
            }
        }
        Ok(())
    }

    /// UI-R-076 — a `Ctrl+t` first digit whose second-digit window elapsed commits its jump
    /// instead of being dropped.
    fn expire_pending_tab_digit(&mut self) {
        if let Some(KeyMode::TabDigit { first, deadline }) = self.keymode
            && Instant::now() >= deadline
        {
            self.keymode = None;
            self.switch_tab(first);
        }
    }

    async fn refresh_snapshot(&mut self) {
        // Refresh *every* tab's module, not just the active one: background modules must keep
        // sending/receiving while another tab is shown (e.g. an OCPP CSMS keeps driving its Lua
        // sim so a CS tab sees inbound traffic live, instead of only when the tab is switched).
        // Poll all refreshes concurrently so tick latency is bounded by the slowest tab, not the
        // sum of all tabs.
        let refreshes = self.tabs.iter_mut().map(|tab| tab.view.refresh());
        futures_util::future::join_all(refreshes).await;
        // One flush per tick amortizes the file sink instead of flushing per log line.
        for tab in self.tabs.iter() {
            tab.log.write().await.flush();
        }
        self.session_log.write().await.flush();
        let mut registry_stale = false;
        for tab in self.tabs.iter_mut() {
            // A view may request to be replaced (e.g. OCPP role switched in the edit dialog).
            if let Some(new_view) = tab.view.take_replacement() {
                tab.replace_view(new_view);
                registry_stale = true;
            }
            let name = tab.view.name();
            if name != tab.name {
                tab.name = name;
                registry_stale = true;
            }
        }
        if registry_stale {
            self.resolve_duplicate_tab_names().await;
            self.rebuild_registry();
        }

        if let Some(dialog) = self.session_dialog.as_mut() {
            let entries = crate::dialog::scripts::snapshot_log(&self.session_log, LOG_SIZE).await;
            dialog.set_log_entries(entries);
        }

        if self.active >= self.tabs.len() {
            return;
        }
        let active = self.active;
        // Tail the log ring unless the user is reading the log pane of the active tab.
        let follow = !self.tabs[active].is_log_focused();

        let log = self.tabs[active].log.clone();
        let lines = {
            let guard = log.read().await;
            guard.peek_n(LOG_SIZE)
        };

        let entries: Vec<LogEntry> = lines
            .into_iter()
            .map(|(ts, level, msg)| LogEntry {
                timestamp: format_timestamp(ts),
                level,
                message: msg.trim_end_matches('\u{0}').to_string(),
            })
            .collect();

        let tab = &mut self.tabs[active];
        tab.log_view.state.set_values(entries);
        if follow {
            tab.log_view.state.move_to_bottom();
        }
    }

    /// Resolve any tab name now shared with another tab — e.g. after a `:edit` rename — by
    /// auto-suffixing the later duplicate(s) the same way session load does (`dedupe_names`), then
    /// warns into the renamed tab's own log. Keeps `Tab::name` unique at all times so `C_Module`
    /// lookups by name are never ambiguous.
    async fn resolve_duplicate_tab_names(&mut self) {
        let names: Vec<String> = self.tabs.iter().map(|t| t.name.clone()).collect();
        let resolved = dedupe_names(&names);
        for (tab, (original, resolved)) in
            self.tabs.iter_mut().zip(names.iter().zip(resolved.iter()))
        {
            if resolved != original {
                tab.name = resolved.clone();
                tab.log.write().await.write(
                    Level::Warning,
                    &format!(
                        "Warning: tab name '{original}' collided with another tab — renamed to '{resolved}'"
                    ),
                );
            }
        }
    }

    fn draw(&mut self) -> std::io::Result<()> {
        let screen = &mut self.screen;
        let tabs = &mut self.tabs;
        let command = &mut self.command;
        let overlay = self.overlay.as_mut();
        let session_dialog = self.session_dialog.as_deref_mut();
        let active = self.active;
        let focus = self.focus;
        let help_open = self.help_open;
        let help_scroll = &mut self.help_scroll;
        let tab_scroll = &mut self.tab_scroll;
        screen.draw(|f| {
            render(
                f,
                tabs,
                active,
                focus,
                command,
                overlay,
                session_dialog,
                help_open,
                help_scroll,
                tab_scroll,
            )
        })?;
        Ok(())
    }

    /// Returns `true` when the application should quit.
    async fn handle_key(&mut self, modifiers: KeyModifiers, code: KeyCode) -> bool {
        match self.focus {
            Focus::Command => self.handle_command_key(modifiers, code).await,
            Focus::Dialog if self.session_dialog.is_some() => {
                self.handle_session_dialog_key(modifiers, code)
            }
            Focus::Dialog => self.handle_dialog_key(modifiers, code).await,
            Focus::Content => self.handle_nav_key(modifiers, code),
        }
    }

    /// Route a key to the open `:session` dialog. Returns `true` when it signals close (its
    /// close-confirm popup was confirmed), which applies the working copy to `session_sim`.
    fn handle_session_dialog_key(&mut self, modifiers: KeyModifiers, code: KeyCode) -> bool {
        let Some(dialog) = self.session_dialog.as_mut() else {
            return false;
        };
        let done = dialog.handle_events(modifiers, code);
        if let Some(script) = dialog.take_run_request() {
            self.session_sim.run_once(script.name, script.code);
        }
        if done {
            let dialog = self.session_dialog.take().expect("checked above");
            let (scripts, interval) = dialog.resolve();
            self.session_scripts.clone_from(&scripts);
            self.session_interval = interval;
            // Interval first, then scripts — same order as construction, so the restart
            // triggered by `set_scripts` already runs on the new interval.
            self.session_sim.set_interval(interval);
            self.session_sim.set_scripts(scripts);
            self.focus = Focus::Content;
            self.set_content_focus(true);
        }
        false
    }

    /// UI-R-057: true once the app holds no tab and no modal layer (creation/type-select overlay,
    /// session dialog, or keybind-help), i.e. there is nothing left to interact with.
    fn should_exit_empty(&self) -> bool {
        let modal_open = self.overlay.is_some() || self.session_dialog.is_some() || self.help_open;
        is_empty_shell(self.tabs.len(), modal_open)
    }

    fn close_overlay(&mut self) {
        self.overlay = None;
        self.focus = Focus::Content;
        self.set_content_focus(true);
    }

    async fn log_active(&self, level: Level, message: String) {
        if let Some(tab) = self.tabs.get(self.active) {
            tab.log.write().await.write(level, &message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrowl_test_support::reserve_temp_dir;
    use std::io::Read;

    #[test]
    /// UI-R-057 — exit only when zero tabs and no modal layer remain; any tab, or any open
    /// layer with zero tabs (e.g. the startup selector before cancel), keeps the app running.
    fn ut_is_empty_shell() {
        assert!(is_empty_shell(0, false));
        assert!(!is_empty_shell(0, true)); // startup selector still open
        assert!(!is_empty_shell(1, false)); // a tab exists
        assert!(!is_empty_shell(3, false));
        assert!(!is_empty_shell(2, true));
    }

    #[test]
    /// UI-R-043 — a tab owns a bounded ring log of timestamped, severity-tagged lines
    /// (Info/Warning/Error), retaining only its most recent capacity.
    fn ut_log_ring_is_bounded_and_severity_tagged() {
        let mut ring = LogRing::init();
        ring.write(Level::Info, "info line");
        ring.write(Level::Warning, "warn line");
        ring.write(Level::Error, "error line");

        let recent = ring.peek_n(3);
        // Each entry keeps its severity and a timestamp alongside the message.
        assert_eq!(
            recent.iter().map(|(_, lvl, _)| *lvl).collect::<Vec<_>>(),
            vec![Level::Info, Level::Warning, Level::Error]
        );
        assert!(
            recent.iter().all(|(ts, _, _)| *ts > 0),
            "lines are timestamped"
        );

        // The ring is bounded: writing past capacity retains only the most recent LOG_SIZE lines.
        for i in 0..(LOG_SIZE + 20) {
            ring.write(Level::Info, &format!("l{i}"));
        }
        assert_eq!(ring.peek_n(LOG_SIZE + 100).len(), LOG_SIZE);
    }

    #[test]
    /// UI-R-044 — a long line is truncated to the per-line cap, and the total-written counter is
    /// monotonic across ring eviction.
    fn ut_log_truncates_long_lines_and_counts_monotonically() {
        let mut ring = LogRing::init();
        // A line over the cap is stored truncated to exactly LOG_MAX_LINE characters.
        ring.write(Level::Info, &"x".repeat(LOG_MAX_LINE + 50));
        assert_eq!(ring.peek_n(1)[0].2.chars().count(), LOG_MAX_LINE);

        // Writing past the ring capacity evicts oldest entries, but `written()` keeps counting.
        for i in 0..(LOG_SIZE as u64 + 10) {
            ring.write(Level::Info, &format!("l{i}"));
        }
        assert_eq!(ring.written(), 1 + LOG_SIZE as u64 + 10);
        // The ring itself retains only its bounded capacity.
        assert_eq!(ring.peek_n(LOG_SIZE + 100).len(), LOG_SIZE);
    }

    #[test]
    /// UI-R-050 — the color scheme is a single compile-time constant selected by build feature,
    /// never switched at runtime.
    fn ut_color_scheme_is_compile_time_constant() {
        // Usable in a `const` context ⇒ resolved at compile time; a runtime-switchable value
        // could not appear here. Exactly one such constant exists, chosen by build feature.
        const SCHEME: ferrowl_ui::ColorScheme = ferrowl_ui::COLOR_SCHEME;
        let _ = SCHEME.bg;
    }

    #[test]
    /// UI-R-085 — a configured file sink buffers lines and flushes them to disk on flush/teardown, timestamped.
    fn log_ring_persists_lines_to_file_sink() {
        let dir = reserve_temp_dir("ferrowl_logring");
        let base = dir.join("test.log");
        let base = base.to_str().unwrap();
        let name = "csms";
        let path = crate::view::log::module_log_path(base, name);

        let mut ring = LogRing::init();
        ring.set_log_file(Some(base), name);
        ring.write(Level::Info, "first line");
        ring.write(Level::Info, "second line");
        // Writes are buffered until the per-tick flush.
        ring.flush();
        let mut buffered = String::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut buffered)
            .unwrap();
        assert!(buffered.contains("first line"));
        // Disabling the sink flushes/drops the writer.
        ring.set_log_file(None, name);

        let mut contents = String::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert!(contents.contains("first line"));
        assert!(contents.contains("second line"));
        // Lines are timestamped.
        assert!(contents.trim_start().starts_with('['));
    }

    #[test]
    /// NF-R-054 — `LogRing::set_log_file` resolves a `~`-prefixed base against the real home
    /// directory (via `view::log::module_log_path`, itself tilde-aware).
    fn ut_log_ring_set_log_file_expands_tilde() {
        let base = "~/ferrowl_nfr042_logring_test.log";
        let name = "csms";
        let path = crate::view::log::module_log_path(base, name);
        let _ = std::fs::remove_file(&path);

        let mut ring = LogRing::init();
        ring.set_log_file(Some(base), name);
        ring.write(Level::Info, "tilde line");
        ring.flush();

        let mut buffered = String::new();
        std::fs::File::open(&path)
            .unwrap()
            .read_to_string(&mut buffered)
            .unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(buffered.contains("tilde line"));
    }

    #[test]
    /// UI-R-003 — the app owns an ordered tab list with one active index; switching the active tab
    /// changes only the index, leaving the order intact.
    fn ut_ordered_tab_list_with_single_active_index() {
        use super::testkit::{MockView, build_app};
        let mut app = build_app(
            ["a", "b", "c"]
                .iter()
                .map(|n| MockView::pair(n).0.boxed())
                .collect(),
        );
        let names: Vec<String> = app.tabs.iter().map(|t| t.name.clone()).collect();
        assert_eq!(names, ["a", "b", "c"]);
        assert_eq!(
            app.active, 0,
            "exactly one active tab, defaulting to the first"
        );

        app.switch_tab(2);
        assert_eq!(app.active, 2);
        let names_after: Vec<String> = app.tabs.iter().map(|t| t.name.clone()).collect();
        assert_eq!(
            names_after,
            ["a", "b", "c"],
            "switching does not reorder tabs"
        );
    }

    #[test]
    /// UI-R-020 — while the command line is focused, the help popup lists both the app-level
    /// commands and the active view's advertised module commands.
    fn ut_command_help_lists_app_and_active_view_commands() {
        use super::testkit::{MOCK_COMMAND, MockView, build_app};
        let (v, _h) = MockView::pair("a");
        let mut app = build_app(vec![v.boxed()]);
        // The popup only renders while the command line holds focus.
        app.focus = Focus::Command;
        app.draw().unwrap();
        let text = app.screen.text();
        assert!(text.contains("quit"), "app-level command listed");
        assert!(text.contains("save"), "app-level :write/:save listed");
        assert!(
            text.contains(MOCK_COMMAND),
            "active view's module command merged into the popup"
        );
    }

    #[test]
    /// UI-R-007 — only key press events are acted upon; release and repeat kinds are ignored for
    /// command/navigation purposes.
    fn ut_only_key_press_events_are_acted_on() {
        assert!(should_act_on(KeyEventKind::Press));
        assert!(!should_act_on(KeyEventKind::Release));
        assert!(!should_act_on(KeyEventKind::Repeat));
    }

    #[test]
    /// The generic screen seam lets an `App` be built on a mock surface and drawn without a real
    /// terminal; `draw` succeeds and the mock records the frame.
    fn ut_app_draws_onto_mock_screen() {
        use super::testkit::MockScreen;
        let mut app =
            App::with_screen(MockScreen::new(), vec![], vec![], Duration::from_secs(1)).unwrap();
        app.draw().unwrap();
        assert_eq!(app.screen.draws, 1);
    }

    #[test]
    /// UI-R-002 — the screen is laid out top-to-bottom: a one-row tab bar on top, then the content
    /// area, then the log pane, with the one-row command line on the bottom.
    fn ut_layout_is_tab_bar_top_command_line_bottom() {
        use super::testkit::{MockView, build_app};
        let mut app = build_app(vec![MockView::pair("alpha").0.boxed()]);
        app.draw().unwrap();
        let buf = app.screen.buffer();
        let (w, h) = (buf.area.width, buf.area.height);
        let row = |y: u16| -> String { (0..w).map(|x| buf[(x, y)].symbol()).collect() };

        assert!(row(0).contains("[0] alpha"), "tab bar is the top row");
        assert!(
            !row(1).contains("[0] alpha"),
            "the tab bar is a single row, not repeated below"
        );
        assert!(
            row(h - 1).contains(":  command"),
            "the command line is the bottom row"
        );
    }

    #[test]
    /// MB-R-150 — `rebuild_registry` attaches the app's single session-wide serial-path registry
    /// to every tab's view (`set_serial_paths`), independent of `module_host()`'s participation.
    /// The same registry instance goes to every tab: a claim made through one tab's copy is
    /// visible through another's.
    fn ut_rebuild_registry_attaches_shared_serial_paths_registry_to_every_tab() {
        use super::testkit::{MockView, build_app};
        let (a, ha) = MockView::pair("a");
        let (b, hb) = MockView::pair("b");
        let app = build_app(vec![a.boxed(), b.boxed()]);
        // `with_screen` already calls `rebuild_registry()` once at construction.
        let _ = &app;

        let reg_a = ha.serial_paths().expect("tab a must receive a registry");
        let reg_b = hb.serial_paths().expect("tab b must receive a registry");
        reg_a.claim("A", "/dev/ttyUSB0");
        assert_eq!(
            reg_b.conflict("B", "/dev/ttyUSB0"),
            Some("A".to_string()),
            "every tab must share the same session-wide registry instance"
        );
    }

    /// A Modbus Rtu server `ModuleSpec` on the shared MB-R-150 test path, named `name`.
    fn mb_r_150_rtu_server_spec(name: &str) -> crate::config::ModuleSpec {
        use crate::config::{Endpoint, ModuleSpec, Role};
        ModuleSpec {
            name: name.to_string(),
            device: String::new(),
            role: Role::Server,
            endpoint: Endpoint::Rtu {
                path: "/nonexistent/mb-r-150-app-e2e".into(),
                baud_rate: 9600,
                parity: None,
                data_bits: None,
                stop_bits: None,
            },
        }
    }

    /// A real (non-mock) `ModbusModuleView` boxed for a tab, built from `mb_r_150_rtu_server_spec`.
    fn mb_r_150_rtu_server_view(name: &str) -> Box<dyn ModuleView> {
        use crate::config::DeviceConfig;
        use crate::module::modbus::ModbusModule;
        use crate::module::modbus::view::ModbusModuleView;
        let spec = mb_r_150_rtu_server_spec(name);
        let device = DeviceConfig::default();
        let module = ModbusModule::new(&spec, &device);
        Box::new(ModbusModuleView::new(module, spec, device))
    }

    #[tokio::test]
    /// MB-R-150 — end to end through the real `App` wiring (no mocks): two Rtu server tabs
    /// configured on the same nonexistent path, started after `rebuild_registry()` (which
    /// attaches the shared session registry to both), both report a symmetric path conflict —
    /// proving MB-R-150 is not "first one wins" — and neither log shows an OS-level open-failure
    /// string, proving the OS-level open was actually skipped rather than merely coincidentally
    /// failing the same way.
    async fn ut_two_rtu_server_instances_sharing_a_path_both_report_conflict() {
        let mut app = super::testkit::build_app(vec![
            mb_r_150_rtu_server_view("a"),
            mb_r_150_rtu_server_view("b"),
        ]);
        // `with_screen` already called `rebuild_registry()` once, attaching the shared registry
        // to both tabs before either starts.
        for tab in &mut app.tabs {
            let _ = tab.view.handle_command("start").await;
        }
        // Let the background reconnect loop run its first attempt and log it.
        tokio::time::sleep(Duration::from_millis(200)).await;

        for tab in &app.tabs {
            let log = tab.view.log();
            let lines: Vec<String> = log
                .read()
                .await
                .peek_n(20)
                .into_iter()
                .map(|(_, _, l)| l)
                .collect();
            assert!(
                lines.iter().any(|l| l.contains("already in use by module")),
                "tab '{}' must report the path conflict, got: {lines:?}",
                tab.name
            );
            assert!(
                !lines
                    .iter()
                    .any(|l| l.to_lowercase().contains("device busy")
                        || l.to_lowercase().contains("os error")),
                "tab '{}' must never reach the OS-level open, got: {lines:?}",
                tab.name
            );
        }

        for tab in &mut app.tabs {
            let _ = tab.view.handle_command("stop").await;
        }
    }

    #[tokio::test]
    /// MB-R-200 — "recovers automatically once the conflicting instance stops": once one of two
    /// Rtu server instances sharing a path is stopped, the surviving instance's own next attempt
    /// does not report a conflict.
    async fn ut_stopping_one_instance_lets_the_other_recover() {
        let mut app = super::testkit::build_app(vec![
            mb_r_150_rtu_server_view("a"),
            mb_r_150_rtu_server_view("b"),
        ]);
        for tab in &mut app.tabs {
            let _ = tab.view.handle_command("start").await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            app.tabs[0]
                .view
                .log()
                .read()
                .await
                .peek_n(20)
                .iter()
                .any(|(_, _, l)| l.contains("already in use by module")),
            "both instances must be in conflict before the recovery step"
        );

        let log_a = app.tabs[0].view.log();
        let written_before_stop = log_a.read().await.written();

        // Stop "b"; "a" is left alone on the path.
        let _ = app.tabs[1].view.handle_command("stop").await;

        // MB-R-150's own registry (App::serial_paths) drives this, not each tab's log content —
        // the shared registry is the single source of truth for "who's still claiming the path".
        assert_eq!(
            app.serial_paths
                .conflict("a", "/nonexistent/mb-r-150-app-e2e"),
            None,
            "stopping 'b' must release its claim, clearing the conflict 'a' would see on its \
             next attempt"
        );

        // Observe "a"'s own subsequent behavior, not just the registry precondition: wait past
        // one full backoff interval (`BackoffPolicy::default().initial` = 1s, see
        // `ferrowl-util/src/backoff.rs`) so "a" actually re-runs its attempt, then check its log
        // for a *fresh* conflict line. There is no positive "recovered"/"reconnected" message to
        // poll for instead — by design (edge-cases.md: "an external holder still surfaces as an
        // ordinary OS-level open failure/retry, same as before"), an *ordinary* open failure
        // against this nonexistent path logs nothing at all, both before and after this feature;
        // only the conflict branch itself ever logs (`rtu::server::run`'s `log.invoke` added for
        // MB-R-150). So the strongest available end-to-end signal that "a" stopped treating "b"
        // as a conflict is exactly this: zero *new* conflict lines across an attempt that did
        // fire — if the stale claim were still consulted, "a" would keep re-logging the same
        // conflict every attempt, same as it did in the first 200ms above.
        tokio::time::sleep(Duration::from_millis(1300)).await;
        let after_stop = log_a.read().await;
        let new_count = after_stop.written().saturating_sub(written_before_stop) as usize;
        let new_lines: Vec<String> = after_stop
            .peek_n(new_count)
            .into_iter()
            .map(|(_, _, l)| l)
            .collect();
        assert!(
            !new_lines
                .iter()
                .any(|l| l.contains("already in use by module")),
            "'a' must not log a fresh conflict once 'b' has released its claim, got: {new_lines:?}"
        );
        drop(after_stop);

        let _ = app.tabs[0].view.handle_command("stop").await;
    }

    #[tokio::test]
    /// UI-R-040 — every tick polls all tabs' views to refresh, not just the active one, so
    /// background modules stay current.
    async fn ut_tick_refreshes_all_tabs_including_background() {
        use super::testkit::{MockView, build_app};
        let (a, ha) = MockView::pair("a");
        let (b, hb) = MockView::pair("b");
        let (c, hc) = MockView::pair("c");
        let mut app = build_app(vec![a.boxed(), b.boxed(), c.boxed()]);

        app.refresh_snapshot().await;
        assert_eq!(ha.refreshes(), 1, "active tab refreshed");
        assert_eq!(hb.refreshes(), 1, "background tab refreshed");
        assert_eq!(hc.refreshes(), 1, "background tab refreshed");

        app.refresh_snapshot().await;
        assert_eq!(hb.refreshes(), 2, "each tick refreshes again");
    }

    #[test]
    /// UI-R-041 — the UI redraws on every tick, and the idle redraw timeout is ≈100 ms.
    fn ut_redraw_each_tick_and_idle_interval_is_100ms() {
        use super::testkit::{MockView, build_app};
        assert_eq!(REDRAW_INTERVAL, Duration::from_millis(100));
        let mut app = build_app(vec![MockView::pair("a").0.boxed()]);
        let before = app.screen.draws;
        app.draw().unwrap();
        app.draw().unwrap();
        assert_eq!(app.screen.draws, before + 2, "each tick draws a frame");
    }

    #[tokio::test]
    /// UI-R-042 — a pending view replacement is applied on the next tick, carrying over the tab's
    /// log channel and focus and rebuilding the session-module registry.
    async fn ut_pending_view_replacement_applied_on_next_tick() {
        use super::testkit::{MockView, build_app};
        use std::sync::Arc;

        let replacement = {
            let (r, _hr) = MockView::pair("replacement");
            r.with_host("mock").boxed()
        };
        let (orig, _ho) = MockView::pair("original");
        let orig = orig.with_replacement(replacement).with_host("mock").boxed();
        let mut app = build_app(vec![orig]);

        assert!(app.tabs[0].view.is_focused(), "active tab starts focused");
        assert_eq!(app.registry.list(), vec!["original".to_string()]);

        app.refresh_snapshot().await;

        assert_eq!(app.tabs[0].view.name(), "replacement", "view swapped");
        assert!(
            app.tabs[0].view.is_focused(),
            "focus carried onto the replacement view"
        );
        assert!(
            Arc::ptr_eq(&app.tabs[0].log, &app.tabs[0].view.log()),
            "tab's log channel tracks the replacement view"
        );
        assert_eq!(
            app.registry.list(),
            vec!["replacement".to_string()],
            "session-module registry rebuilt from the new tab set"
        );
    }

    #[tokio::test]
    /// UI-R-069 — a tab whose name collides with an earlier tab's is auto-suffixed `" (n)"`.
    /// UI-R-070 — the same rename writes a warning line into the renamed tab's own log, not
    /// into the survivor's.
    async fn ut_rename_collision_suffixes_the_later_tab_and_warns_into_its_log() {
        use super::testkit::{MockView, build_app};

        let (a, _ha) = MockView::pair("A");
        let (replacement, _hr) = MockView::pair("A");
        let (b, _hb) = MockView::pair("B");
        let mut app = build_app(vec![
            a.boxed(),
            b.with_replacement(replacement.boxed()).boxed(),
        ]);

        app.refresh_snapshot().await;

        assert_eq!(app.tabs[0].name, "A", "the first occurrence keeps its name");
        assert_eq!(app.tabs[1].name, "A (2)");

        let tab1_log = app.tabs[1].log.write().await.peek_n(LOG_SIZE);
        assert!(
            tab1_log
                .iter()
                .any(|(_, level, msg)| *level == Level::Warning
                    && msg.contains("collided with another tab")
                    && msg.contains("renamed to 'A (2)'")),
            "renamed tab's log carries the collision warning: {tab1_log:?}"
        );

        let tab0_log = app.tabs[0].log.write().await.peek_n(LOG_SIZE);
        assert!(
            !tab0_log
                .iter()
                .any(|(_, _, msg)| msg.contains("collided with another tab")),
            "the survivor's log carries no collision warning: {tab0_log:?}"
        );
    }

    #[test]
    /// UI-R-076 — a `Ctrl+t` first digit whose second-digit window elapsed commits its jump
    /// instead of being dropped; one still comfortably inside its window is left pending.
    fn ut_expired_tab_digit_chord_commits_the_pending_jump() {
        use super::testkit::{MockView, build_app};

        let mut app = build_app(
            (0..25)
                .map(|n| MockView::pair(&format!("t{n}")).0.boxed())
                .collect(),
        );
        app.keymode = Some(KeyMode::TabDigit {
            first: 1,
            deadline: Instant::now() - Duration::from_millis(1),
        });
        app.expire_pending_tab_digit();
        assert_eq!(app.active, 1);
        assert!(app.keymode.is_none());

        app.keymode = Some(KeyMode::TabDigit {
            first: 2,
            deadline: Instant::now() + Duration::from_secs(60),
        });
        app.expire_pending_tab_digit();
        assert_eq!(app.active, 1, "not yet expired: no jump happened");
        assert!(
            matches!(app.keymode, Some(KeyMode::TabDigit { first: 2, .. })),
            "not yet expired: pending chord left intact"
        );
    }
}
