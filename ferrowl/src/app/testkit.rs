//! Test doubles for driving `App` headlessly.
//!
//! Two seams the shell talks to are faked here so the nav/command/quit/tick behavior on `App`
//! can be exercised without a terminal or a real module:
//! - [`MockScreen`] — a [`DrawSurface`] backed by ratatui's `TestBackend`, so `App::draw` renders
//!   into an inspectable cell buffer instead of a tty.
//! - [`MockView`] — a [`ModuleView`] that records the calls `App` makes (`refresh`,
//!   `handle_command`) and lets a test pre-load a replacement, a session spec, and a module host.
//!
//! Shared under `#[cfg(test)]` because the nav (`keys`), command (`commands`) and tick (`mod`)
//! tests all need the same doubles.
//!
//! `dead_code` is allowed module-wide: this is a fixture toolbox whose builders are each consumed
//! by only a subset of the tests, so an unused method here means "no test needs that knob yet",
//! not a real dead branch.
#![allow(dead_code)]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_lua::module::ModuleHost;
use ferrowl_ui::traits::{IsFocus, SetFocus};
use ferrowl_ui::{DrawSurface, EventResult};
use mlua::{AnyUserData, Lua, Result as LuaResult};
use ratatui::Frame;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::{App, Level, LogRing};
use crate::config::script::ScriptDef;
use crate::module::modbus::SerialPathRegistry;
use crate::module::type_descriptor::{ModuleViewFactory, SetupView};
use crate::module::view::{
    CommandDescriptor, CommandFuture, CommandResult, ModuleView, RefreshFuture, SharedLog,
};

/// A `DrawSurface` test double backed by ratatui's `TestBackend`, letting an `App` be built and
/// drawn headlessly (no real terminal). Records how many frames it rendered and exposes the cell
/// buffer so layout/help tests can assert on what was painted.
pub(super) struct MockScreen {
    term: ratatui::Terminal<TestBackend>,
    pub(super) draws: usize,
}

impl MockScreen {
    pub(super) fn new() -> Self {
        Self::with_size(120, 40)
    }

    pub(super) fn with_size(w: u16, h: u16) -> Self {
        let term = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
        Self { term, draws: 0 }
    }

    /// The last rendered frame's cell buffer.
    pub(super) fn buffer(&self) -> &Buffer {
        self.term.backend().buffer()
    }

    /// Concatenate the buffer's cells into one string (row-major), for substring assertions.
    pub(super) fn text(&self) -> String {
        let buf = self.buffer();
        buf.content().iter().map(|c| c.symbol()).collect()
    }
}

impl DrawSurface for MockScreen {
    fn draw<F: FnOnce(&mut Frame)>(&mut self, render: F) -> std::io::Result<()> {
        self.term
            .draw(render)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        self.draws += 1;
        Ok(())
    }
}

/// A `ModuleHost` double: the smallest thing `App::rebuild_registry` needs to place a module in
/// the session registry. Reports a configurable kind/role; grants no Lua accessors.
struct MockHost {
    kind: &'static str,
}

impl ModuleHost for MockHost {
    fn kind(&self) -> &'static str {
        self.kind
    }
    fn role(&self) -> &'static str {
        "mock"
    }
    fn register_accessor(&self, _lua: &Lua) -> LuaResult<Option<AnyUserData>> {
        Ok(None)
    }
    fn ocpp_accessor(&self, _lua: &Lua) -> LuaResult<Option<AnyUserData>> {
        Ok(None)
    }
}

/// The module commands a [`MockView`] advertises, so a help test can assert the active view's list
/// is merged in.
pub(super) const MOCK_COMMAND: &str = "mockcmd";
const MOCK_CMDS: &[CommandDescriptor] = &[CommandDescriptor {
    name: MOCK_COMMAND,
    description: "mock module command",
}];

/// Handles a test keeps after moving a [`MockView`] into a `Tab`, to observe the calls `App` made
/// to it. All fields are `Arc`-shared with the live view.
#[derive(Clone)]
pub(super) struct MockHandle {
    refreshes: Arc<AtomicUsize>,
    renders: Arc<AtomicUsize>,
    commands: Arc<Mutex<Vec<String>>>,
    /// MB-R-150 — the registry `App::rebuild_registry` most recently attached via
    /// `set_serial_paths`, if any.
    serial_paths: Arc<Mutex<Option<SerialPathRegistry>>>,
    keys: Arc<Mutex<Vec<(KeyModifiers, KeyCode)>>>,
}

impl MockHandle {
    /// How many times `App` called `refresh` on this view.
    pub(super) fn refreshes(&self) -> usize {
        self.refreshes.load(Ordering::Relaxed)
    }

    /// How many times `App` called `render` on this view.
    pub(super) fn renders(&self) -> usize {
        self.renders.load(Ordering::Relaxed)
    }

    /// Every command string `App` forwarded to this view, in order.
    pub(super) fn commands(&self) -> Vec<String> {
        self.commands.lock().unwrap().clone()
    }

    /// Every `(modifiers, code)` pair `App` forwarded to this view's `handle_events`, in order.
    pub(super) fn keys(&self) -> Vec<(KeyModifiers, KeyCode)> {
        self.keys.lock().unwrap().clone()
    }

    /// MB-R-150 — the registry most recently passed to this view's `set_serial_paths`, if any.
    pub(super) fn serial_paths(&self) -> Option<SerialPathRegistry> {
        self.serial_paths.lock().unwrap().clone()
    }
}

/// A `ModuleView` test double: renders nothing, records `refresh`/`handle_command`, and can be
/// pre-loaded with a session spec, a one-shot replacement, and a module host.
pub(super) struct MockView {
    name: String,
    log: SharedLog,
    focused: bool,
    overlay_active: bool,
    session_spec: Option<serde_json::Value>,
    replacement: Option<Box<dyn ModuleView>>,
    host_kind: Option<&'static str>,
    refreshes: Arc<AtomicUsize>,
    renders: Arc<AtomicUsize>,
    commands: Arc<Mutex<Vec<String>>>,
    serial_paths: Arc<Mutex<Option<SerialPathRegistry>>>,
    keys: Arc<Mutex<Vec<(KeyModifiers, KeyCode)>>>,
    command_result: Option<CommandResult>,
}

impl MockView {
    /// A view plus the handle to observe it. `name` is the module/tab identity. Not `new` because
    /// it returns the observation handle alongside the view, not `Self`; chain builder methods on
    /// the returned view and `.boxed()` it for [`build_app`].
    pub(super) fn pair(name: &str) -> (MockView, MockHandle) {
        let refreshes = Arc::new(AtomicUsize::new(0));
        let renders = Arc::new(AtomicUsize::new(0));
        let commands = Arc::new(Mutex::new(Vec::new()));
        let serial_paths = Arc::new(Mutex::new(None));
        let keys = Arc::new(Mutex::new(Vec::new()));
        let handle = MockHandle {
            refreshes: refreshes.clone(),
            renders: renders.clone(),
            commands: commands.clone(),
            serial_paths: serial_paths.clone(),
            keys: keys.clone(),
        };
        let view = MockView {
            name: name.to_string(),
            log: Arc::new(tokio::sync::RwLock::new(LogRing::init())),
            focused: false,
            overlay_active: false,
            session_spec: None,
            replacement: None,
            host_kind: None,
            refreshes,
            renders,
            commands,
            serial_paths,
            keys,
            command_result: None,
        };
        (view, handle)
    }

    /// Make the next `handle_command` call return `Handled(Some((level, message)))` instead of
    /// the default `Handled(None)`.
    pub(super) fn with_command_message(mut self, level: Level, message: &str) -> Self {
        self.command_result = Some(CommandResult::Handled(Some((level, message.to_string()))));
        self
    }

    /// Make the next `handle_command` call return `Unhandled`.
    pub(super) fn with_command_unhandled(mut self) -> Self {
        self.command_result = Some(CommandResult::Unhandled);
        self
    }

    /// Give this view a session spec so `:write` serializes something for its tab.
    pub(super) fn with_session_spec(mut self, spec: serde_json::Value) -> Self {
        self.session_spec = Some(spec);
        self
    }

    /// Pre-load the view returned by the next `take_replacement` poll (one-shot).
    pub(super) fn with_replacement(mut self, replacement: Box<dyn ModuleView>) -> Self {
        self.replacement = Some(replacement);
        self
    }

    /// Make this view participate in the session-module registry under the given kind.
    pub(super) fn with_host(mut self, kind: &'static str) -> Self {
        self.host_kind = Some(kind);
        self
    }

    /// Re-box after a builder chain that started from an already-boxed `new`.
    pub(super) fn boxed(self) -> Box<dyn ModuleView> {
        Box::new(self)
    }
}

impl SetFocus for MockView {
    fn set_focused(&mut self, focus: bool) {
        self.focused = focus;
    }
}

impl IsFocus for MockView {
    fn is_focused(&self) -> bool {
        self.focused
    }
}

impl ModuleView for MockView {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn render(&mut self, _frame: &mut Frame, _area: Rect) {
        self.renders.fetch_add(1, Ordering::Relaxed);
    }

    fn render_overlay(&mut self, _frame: &mut Frame, _area: Rect) {}

    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        self.keys.lock().unwrap().push((modifiers, code));
        EventResult::Unhandled(modifiers, code)
    }

    fn refresh<'a>(&'a mut self) -> RefreshFuture<'a> {
        let refreshes = self.refreshes.clone();
        Box::pin(async move {
            refreshes.fetch_add(1, Ordering::Relaxed);
        })
    }

    fn is_overlay_active(&self) -> bool {
        self.overlay_active
    }

    fn handle_command<'a>(&'a mut self, cmd: &'a str) -> CommandFuture<'a> {
        let commands = self.commands.clone();
        let cmd = cmd.to_string();
        let result = self.command_result.take();
        Box::pin(async move {
            commands.lock().unwrap().push(cmd);
            result.unwrap_or(CommandResult::Handled(None))
        })
    }

    fn commands(&self) -> &[CommandDescriptor] {
        MOCK_CMDS
    }

    fn log(&self) -> SharedLog {
        self.log.clone()
    }

    fn session_spec(&self) -> Option<serde_json::Value> {
        self.session_spec.clone()
    }

    fn take_replacement(&mut self) -> Option<Box<dyn ModuleView>> {
        self.replacement.take()
    }

    fn scripts(&self) -> Option<&[ScriptDef]> {
        None
    }

    fn module_host(&self) -> Option<Arc<dyn ModuleHost>> {
        self.host_kind
            .map(|kind| Arc::new(MockHost { kind }) as Arc<dyn ModuleHost>)
    }

    fn set_serial_paths(&mut self, registry: SerialPathRegistry) {
        *self.serial_paths.lock().unwrap() = Some(registry);
    }
}

/// A [`SetupView`] test double whose `confirm` always validates to a fixed tab name backed by a
/// fresh [`MockView`]. Lets a creation-flow test drive `confirm_overlay` to either the create or
/// the name-collision branch without a real module setup dialog.
pub(super) struct MockSetup {
    name: String,
    valid: bool,
    handle_slot: Option<Arc<Mutex<Option<MockHandle>>>>,
}

impl MockSetup {
    pub(super) fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            valid: true,
            handle_slot: None,
        }
    }

    /// A dialog that never validates: `confirm()` always returns `None`.
    pub(super) fn invalid(name: &str) -> Self {
        Self {
            name: name.to_string(),
            valid: false,
            handle_slot: None,
        }
    }

    /// Like [`MockSetup::new`], but also hands back the [`MockHandle`] of the view its
    /// `confirm()` will build, populated once `App` actually invokes the factory — so a test can
    /// inspect the commands `App` sent the freshly created tab's view (e.g. that `start` ran).
    pub(super) fn new_with_handle(name: &str) -> (Self, Arc<Mutex<Option<MockHandle>>>) {
        let slot: Arc<Mutex<Option<MockHandle>>> = Arc::new(Mutex::new(None));
        (
            Self {
                name: name.to_string(),
                valid: true,
                handle_slot: Some(slot.clone()),
            },
            slot,
        )
    }
}

impl SetupView for MockSetup {
    fn render(&mut self, _area: Rect, _buf: &mut Buffer) {}
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        EventResult::Unhandled(modifiers, code)
    }
    fn focus_next(&mut self) {}
    fn focus_previous(&mut self) {}
    fn confirm(&self) -> Option<(String, ModuleViewFactory)> {
        if !self.valid {
            return None;
        }
        let name = self.name.clone();
        let handle_slot = self.handle_slot.clone();
        Some((
            self.name.clone(),
            Box::new(move || {
                let (view, handle) = MockView::pair(&name);
                if let Some(slot) = &handle_slot {
                    *slot.lock().unwrap() = Some(handle);
                }
                view.boxed()
            }),
        ))
    }
}

/// Build an `App` on a [`MockScreen`] from a list of pre-boxed views, one tab each. The tab names
/// come from each view's `name()`. A one-second session interval keeps the sim quiet.
pub(super) fn build_app(views: Vec<Box<dyn ModuleView>>) -> App<MockScreen> {
    let tabs = views
        .into_iter()
        .map(|view| super::Tab::new_from_view(view.name(), view))
        .collect();
    App::with_screen(
        MockScreen::new(),
        tabs,
        vec![],
        std::time::Duration::from_secs(1),
    )
    .unwrap()
}
