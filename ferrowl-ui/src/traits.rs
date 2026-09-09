//! Common traits implemented by widget states and views.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Margin;
use std::io::{Stderr, Stdout, stderr, stdout};

use crate::EventResult;

/// Receives a key event and reports whether it was consumed.
pub trait HandleEvents {
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult;
}

/// Constructs the output stream an
/// [`AlternateScreen`](crate::AlternateScreen) writes to (stdout or stderr).
pub trait Init {
    fn init() -> Self;
}

impl Init for Stdout {
    fn init() -> Self {
        stdout()
    }
}

impl Init for Stderr {
    fn init() -> Self {
        stderr()
    }
}

/// Converts a value into the label text a widget displays for it.
pub trait ToLabel {
    fn to_label(&self) -> String;
}

impl ToLabel for String {
    fn to_label(&self) -> String {
        self.clone()
    }
}

impl ToLabel for &str {
    fn to_label(&self) -> String {
        self.to_string()
    }
}

pub trait SetFocus {
    fn set_focused(&mut self, focus: bool);
}

pub trait IsFocus {
    fn is_focused(&self) -> bool;
}

/// Boundary-aware focus stepping for a `#[focusable(nestable)]` struct embedded as a
/// `#[focus(nested)]` field of another. Unlike `focus_next`/`focus_previous` (which always
/// wrap), these stop and report `false` — leaving position unchanged — once already at the
/// struct's own last/first eligible pane, so the embedding parent knows to advance its own
/// cycle instead of wrapping back into this one.
///
/// Known limitation: if the currently-focused inner pane's own `handle_events` consumes
/// Tab/BackTab itself (e.g. `CodeInputFieldState` in Insert mode) rather than returning
/// `Unhandled`, that keystroke never reaches this trait's methods and no stepping occurs —
/// same as how Tab is already swallowed at any level today when a widget wants it.
pub trait NestedFocus {
    fn try_focus_next(&mut self) -> bool;
    fn try_focus_previous(&mut self) -> bool;
}

/// The margin a widget reserves around its content (e.g. for borders).
pub trait Margins {
    fn margins(&self) -> Margin;
}

/// A single entry offered by a [`SuggestionProvider`] for a
/// [`SuggestInput`](crate::widgets::SuggestInput)'s popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// Full text the input is replaced with when this suggestion is accepted.
    pub value: String,
    /// Text shown for this entry in the popup list.
    pub label: String,
    /// If `true`, accepting this suggestion re-queries the provider with the
    /// new input and keeps the popup open (e.g. a directory to descend
    /// into); if `false`, accepting it closes the popup.
    pub partial: bool,
}

/// Supplies the candidate [`Suggestion`]s for a
/// [`SuggestInput`](crate::widgets::SuggestInput) given its current text.
pub trait SuggestionProvider {
    fn suggest(&self, input: &str) -> Vec<Suggestion>;
}

/// Common focus cycling for an overlay payload, routed from a
/// `#[derive(Overlay)]` enum's generated `Tab`/`BackTab` handling.
///
/// Implemented by each overlay variant that opts into `#[overlay(focus_cycle)]`,
/// adapting whatever the payload calls internally (`focus_next`/`focus_previous`,
/// `focus_step`, …) to a single `forward: bool` step.
pub trait OverlayKeys {
    fn focus_cycle(&mut self, forward: bool);
}

impl<T> OverlayKeys for Box<T>
where
    T: OverlayKeys,
{
    fn focus_cycle(&mut self, forward: bool) {
        self.as_mut().focus_cycle(forward)
    }
}

/// Implements [`OverlayKeys`] for one or more types by forwarding `forward` to the
/// `focus_next`/`focus_previous` inherent methods `#[derive(Focus)]` gives them — the common case
/// for an overlay payload that opts into `#[overlay(focus_cycle)]` with no bespoke stepping logic.
#[macro_export]
macro_rules! impl_overlay_keys {
    ($($t:ty),+ $(,)?) => {
        $(
            impl $crate::traits::OverlayKeys for $t {
                fn focus_cycle(&mut self, forward: bool) {
                    if forward {
                        self.focus_next();
                    } else {
                        self.focus_previous();
                    }
                }
            }
        )+
    };
}

/// Outcome of a `#[derive(Overlay)]` enum's common-key router (`route_keys`):
/// whether the key closed the overlay, cycled its focus, or was left for the
/// view's own `Enter`/custom handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayRoute {
    /// `Esc` closed an `#[overlay(esc_close)]` variant; overlay is now `None`.
    Closed,
    /// `Tab`/`BackTab` cycled focus on a `#[overlay(focus_cycle)]` variant.
    Cycled,
    /// Key not handled by common routing; the view should handle it.
    Unhandled,
}
