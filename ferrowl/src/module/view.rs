use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_ui::EventResult;
use ferrowl_ui::traits::{HandleEvents, IsFocus, SetFocus};
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::{Level, LogRing};
use crate::config::script::ScriptDef;

/// Generic log channel shared between a [`ModuleView`] and the owning [`Tab`].
pub type SharedLog = std::sync::Arc<tokio::sync::RwLock<LogRing>>;

/// Result returned by [`ModuleView::handle_command`].
pub enum CommandResult {
    /// Command was handled; optional `(level, message)` to append to the tab log. The producer
    /// picks the level explicitly — callers must never re-derive it by pattern-matching the
    /// message text.
    Handled(Option<(Level, String)>),
    /// Command is not known to this module.
    Unhandled,
}

/// One entry in a module's command help list.
#[derive(Clone, Copy)]
pub struct CommandDescriptor {
    pub name: &'static str,
    pub description: &'static str,
}

/// One command in a view's dispatch table: the accepted aliases, the help row advertising it,
/// and the constructor for the view's parsed-command value. Alias list, help text, and parse
/// target live in this one entry so the advertised list and the handled set cannot drift apart;
/// the exhaustive `match` on the parsed enum is what guarantees every entry has a handler.
pub struct CommandSpec<C> {
    /// First-token spellings that select this command (e.g. `&["wd", "write-device"]`).
    pub aliases: &'static [&'static str],
    /// The help row shown for this command.
    pub descriptor: CommandDescriptor,
    /// Build the parsed command from the argument remainder: `None` for a bare token,
    /// `Some(rest)` (trimmed, non-empty) when arguments followed it.
    pub build: fn(rest: Option<&str>) -> C,
}

/// Match `input` against `specs` by its exact first whitespace-delimited token (edge case
/// TUI 6.8: `setfoo` never matches `set`). The remainder after the token — trimmed, `None`
/// when empty — is passed to the matching entry's `build`. Returns `None` for an unknown
/// token; argument validation is the handler's job, applied only after the token matched.
pub fn parse_command<C>(specs: &[CommandSpec<C>], input: &str) -> Option<C> {
    let trimmed = input.trim();
    let (token, rest) = match trimmed.split_once(char::is_whitespace) {
        Some((token, rest)) => (token, rest.trim()),
        None => (trimmed, ""),
    };
    let spec = specs.iter().find(|s| s.aliases.contains(&token))?;
    Some((spec.build)(if rest.is_empty() {
        None
    } else {
        Some(rest)
    }))
}

/// Object-safe async return type for [`ModuleView::handle_command`].
pub type CommandFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = CommandResult> + 'a>>;

pub type RefreshFuture<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'a>>;

/// The trait every module content view must implement.
///
/// `Tab` and `App` interact with a module exclusively through this interface.
/// No module-type-specific types are visible outside the module's own directory.
///
/// A module view is a focusable node ([`SetFocus`] + [`IsFocus`]): the owning [`Tab`] toggles its
/// whole-view focus, and the view reads [`IsFocus::is_focused`] for focus-dependent rendering (e.g.
/// message-log autoscroll). Concrete views get these from `#[derive(Focus)]`.
pub trait ModuleView: SetFocus + IsFocus {
    fn name(&self) -> String;

    /// Render the module content area (everything except the log pane and tab bar).
    /// Focus-dependent rendering reads [`IsFocus::is_focused`] on `self`.
    fn render(&mut self, frame: &mut Frame, area: Rect);

    /// Render the module's open dialogs/overlays, if any. Called after content and the log pane are
    /// painted, so overlays may extend over the log area without being overwritten. No-ops when no
    /// overlay is open. `area` is the same content area passed to [`render`].
    fn render_overlay(&mut self, frame: &mut Frame, area: Rect);

    /// Handle a terminal key event. Returns `Consumed` or `Unhandled`.
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult;

    /// Pull a fresh snapshot from internal backends and update render state.
    /// Called once per UI tick before [`render`].
    fn refresh<'a>(&'a mut self) -> RefreshFuture<'a>;

    fn is_overlay_active(&self) -> bool;

    /// Execute a module command string asynchronously.
    ///
    /// Standard commands dispatched by App: `"start"`, `"stop"`, `"restart"`,
    /// `"reload"`, `"edit"`, `"add"`, `"compact"`, `"wd [path]"`, `"log <file>"`,
    /// `"set <reg> <val>"`.
    fn handle_command<'a>(&'a mut self, cmd: &'a str) -> CommandFuture<'a>;

    /// Module-specific commands shown in the help popup.
    fn commands(&self) -> &[CommandDescriptor];

    /// Module-specific keybinds shown in the `?` help dialog.
    fn keybinds(&self) -> &[CommandDescriptor] {
        &[]
    }

    /// The log channel written by this view's backend.
    fn log(&self) -> SharedLog;

    /// Serialize this module's config for session persistence, or `None` if unsupported.
    /// The returned value should include a `"type"` field so the loader can dispatch to
    /// the right deserializer (e.g. `"modbus"`, `"ocpp"`).
    fn session_spec(&self) -> Option<serde_json::Value> {
        None
    }

    /// Take a view that should replace this one in its tab, if the view requested one (e.g. the
    /// OCPP role was switched in the edit dialog, turning a client view into a server view).
    /// Polled by `App` once per tick after [`refresh`]. Default: never replaced.
    fn take_replacement(&mut self) -> Option<Box<dyn ModuleView>> {
        None
    }

    /// The module's Lua script list, or `None` when the module has no script support.
    fn scripts(&self) -> Option<&[ScriptDef]> {
        None
    }

    /// Replace the module's script list and apply it to any running sim the same way
    /// the script dialog does. Returns `false` when unsupported.
    fn set_scripts(&mut self, _scripts: Vec<ScriptDef>) -> bool {
        false
    }

    /// MB-R-150 — attach this session's live Rtu/Ascii path-conflict registry. Default: no-op —
    /// only the Modbus client/server/monitor views participate; every other module type (OCPP)
    /// ignores this.
    fn set_serial_paths(&mut self, _registry: crate::module::modbus::SerialPathRegistry) {}

    /// A snapshot of this module's session-level `C_Module` Lua surface, or `None` when the
    /// module type doesn't participate (none currently opt out, but the default keeps the trait
    /// extensible). Polled by `App::rebuild_registry` whenever the tab set changes.
    fn module_host(&self) -> Option<std::sync::Arc<dyn ferrowl_lua::module::ModuleHost>> {
        None
    }
}

// Forwarding impls so a boxed module view is itself a focusable, event-handling node — lets the
// owning `Tab` carry it as a `#[focus]` field under `#[derive(Focus)]`.
impl SetFocus for Box<dyn ModuleView> {
    fn set_focused(&mut self, focus: bool) {
        (**self).set_focused(focus);
    }
}

impl IsFocus for Box<dyn ModuleView> {
    fn is_focused(&self) -> bool {
        (**self).is_focused()
    }
}

impl HandleEvents for Box<dyn ModuleView> {
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        (**self).handle_events(modifiers, code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    enum Cmd {
        Start,
        Save(Option<String>),
    }

    fn specs() -> [CommandSpec<Cmd>; 2] {
        [
            CommandSpec {
                aliases: &["start"],
                descriptor: CommandDescriptor {
                    name: ":start",
                    description: "start",
                },
                build: |_| Cmd::Start,
            },
            CommandSpec {
                aliases: &["wd", "write-device"],
                descriptor: CommandDescriptor {
                    name: ":wd | :write-device [path]",
                    description: "save",
                },
                build: |rest| Cmd::Save(rest.map(str::to_string)),
            },
        ]
    }

    #[test]
    /// Every alias of a table entry selects the same command.
    fn ut_parse_command_matches_every_alias() {
        let specs = specs();
        assert_eq!(parse_command(&specs, "wd"), Some(Cmd::Save(None)));
        assert_eq!(parse_command(&specs, "write-device"), Some(Cmd::Save(None)));
        assert_eq!(parse_command(&specs, "start"), Some(Cmd::Start));
    }

    #[test]
    /// The remainder after the first token is trimmed and passed to `build`; empty becomes `None`.
    fn ut_parse_command_passes_trimmed_remainder() {
        let specs = specs();
        assert_eq!(
            parse_command(&specs, "  wd   a b.toml  "),
            Some(Cmd::Save(Some("a b.toml".into())))
        );
        assert_eq!(parse_command(&specs, " wd  "), Some(Cmd::Save(None)));
    }

    #[test]
    /// TUI edge case 6.8 — commands match on the exact first token, so a prefix typo is unknown.
    fn ut_parse_command_exact_first_token_only() {
        let specs = specs();
        assert_eq!(parse_command(&specs, "startx"), None);
        assert_eq!(parse_command(&specs, "wdx path"), None);
        assert_eq!(parse_command(&specs, ""), None);
    }
}
