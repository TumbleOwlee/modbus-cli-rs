//! Tests for `#[derive(Overlay)]`: structural helpers + common-key routing.

use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_ui::traits::{OverlayKeys, OverlayRoute};
use ferrowl_ui_derive::Overlay;

#[derive(Debug, PartialEq)]
struct Editor {
    step: i32,
}
impl OverlayKeys for Editor {
    fn focus_cycle(&mut self, forward: bool) {
        self.step += if forward { 1 } else { -1 };
    }
}

#[derive(Debug, PartialEq)]
struct Setup {
    step: i32,
}
impl OverlayKeys for Setup {
    fn focus_cycle(&mut self, forward: bool) {
        self.step += if forward { 10 } else { -10 };
    }
}

#[derive(Debug, PartialEq)]
struct Confirm; // esc_close only, no focus cycling

#[derive(Debug, PartialEq)]
struct Picker; // untagged: never handled by common routing

#[derive(Debug, PartialEq, Overlay)]
enum Overlay {
    #[overlay(none)]
    None,
    #[overlay(esc_close, focus_cycle)]
    Edit(Editor),
    #[overlay(focus_cycle)]
    SetupV(Setup),
    #[overlay(esc_close)]
    Conf(Confirm),
    Plain(Picker),
}

const NONE: KeyModifiers = KeyModifiers::NONE;

fn route(o: &mut Overlay, code: KeyCode) -> OverlayRoute {
    o.route_keys(NONE, code)
}

#[test]
/// UI-R-021 — an overlay reports active for any variant other than None.
fn it_is_active() {
    assert!(!Overlay::None.is_active());
    assert!(Overlay::Edit(Editor { step: 0 }).is_active());
    assert!(Overlay::Plain(Picker).is_active());
}

#[test]
/// UI-R-021 — taking an overlay yields its payload and resets the slot to None.
fn it_take_leaves_none() {
    let mut o = Overlay::Edit(Editor { step: 3 });
    let taken = o.take();
    assert_eq!(taken, Overlay::Edit(Editor { step: 3 }));
    assert_eq!(o, Overlay::None);
}

#[test]
/// UI-R-021 — closing an overlay resets it to None.
fn it_close_resets_to_none() {
    let mut o = Overlay::SetupV(Setup { step: 1 });
    o.close();
    assert_eq!(o, Overlay::None);
}

#[test]
/// UI-R-079 — Esc requests close on an overlay variant that opted into esc_close.
fn it_esc_closes_esc_close_variant() {
    let mut o = Overlay::Edit(Editor { step: 0 });
    assert_eq!(route(&mut o, KeyCode::Esc), OverlayRoute::Closed);
    assert_eq!(o, Overlay::None);

    let mut o = Overlay::Conf(Confirm);
    assert_eq!(route(&mut o, KeyCode::Esc), OverlayRoute::Closed);
    assert_eq!(o, Overlay::None);
}

#[test]
/// UI-R-079 — Esc is unhandled (propagates) on a variant that did not opt into esc_close.
fn it_esc_unhandled_without_esc_close() {
    // SetupV is focus_cycle only; Plain is untagged. Neither closes on Esc.
    let mut o = Overlay::SetupV(Setup { step: 0 });
    assert_eq!(route(&mut o, KeyCode::Esc), OverlayRoute::Unhandled);
    assert!(o.is_active());

    let mut o = Overlay::Plain(Picker);
    assert_eq!(route(&mut o, KeyCode::Esc), OverlayRoute::Unhandled);
    assert!(o.is_active());
}

#[test]
/// UI-R-022 — Tab advances the overlay's field focus.
fn it_tab_cycles_focus_forward() {
    let mut o = Overlay::Edit(Editor { step: 0 });
    assert_eq!(route(&mut o, KeyCode::Tab), OverlayRoute::Cycled);
    assert_eq!(o, Overlay::Edit(Editor { step: 1 }));
}

#[test]
/// UI-R-022 — Shift+Tab/BackTab retreats the overlay's field focus.
fn it_backtab_cycles_focus_backward() {
    let mut o = Overlay::SetupV(Setup { step: 0 });
    assert_eq!(o.route_keys(NONE, KeyCode::BackTab), OverlayRoute::Cycled);
    assert_eq!(o, Overlay::SetupV(Setup { step: -10 }));
    // Shift+BackTab also cycles.
    assert_eq!(
        o.route_keys(KeyModifiers::SHIFT, KeyCode::BackTab),
        OverlayRoute::Cycled
    );
    assert_eq!(o, Overlay::SetupV(Setup { step: -20 }));
}

#[test]
/// UI-R-022 — Tab is unhandled on a variant without a focus cycle.
fn it_tab_unhandled_without_focus_cycle() {
    let mut o = Overlay::Conf(Confirm);
    assert_eq!(route(&mut o, KeyCode::Tab), OverlayRoute::Unhandled);
}

#[test]
/// UI-R-022 — keys other than the dialog defaults fall through as unhandled.
fn it_other_keys_unhandled() {
    let mut o = Overlay::Edit(Editor { step: 0 });
    assert_eq!(route(&mut o, KeyCode::Enter), OverlayRoute::Unhandled);
    assert_eq!(route(&mut o, KeyCode::Char('x')), OverlayRoute::Unhandled);
    // Untouched.
    assert_eq!(o, Overlay::Edit(Editor { step: 0 }));
}

#[test]
/// UI-R-021 — an inactive (None) overlay consumes no keys.
fn it_none_is_unhandled() {
    let mut o = Overlay::None;
    assert_eq!(route(&mut o, KeyCode::Esc), OverlayRoute::Unhandled);
    assert_eq!(route(&mut o, KeyCode::Tab), OverlayRoute::Unhandled);
}
