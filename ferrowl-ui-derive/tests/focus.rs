use derive_builder::Builder;
use ferrowl_ui::traits::{IsFocus, SetFocus};
use ferrowl_ui_derive::{Focus, focusable};

#[derive(Default, Clone, Debug)]
struct Widget {
    focused: bool,
    /// Number of events routed to this widget, used to assert event dispatch targets.
    events: u32,
}

impl ferrowl_ui::traits::SetFocus for Widget {
    fn set_focused(&mut self, focus: bool) {
        self.focused = focus;
    }
}

impl ferrowl_ui::traits::IsFocus for Widget {
    fn is_focused(&self) -> bool {
        self.focused
    }
}

impl ferrowl_ui::traits::HandleEvents for Widget {
    fn handle_events(
        &mut self,
        modifiers: crossterm::event::KeyModifiers,
        code: crossterm::event::KeyCode,
    ) -> ferrowl_ui::EventResult {
        use crossterm::event::KeyCode;
        // Mirrors real leaf widgets (Button/InputField/Table/Selection): Tab/BackTab are never
        // consumed, falling through to `Unhandled` so an outer container's focus cycling (or,
        // for a `#[focus(nested)]` field, `NestedFocus` stepping) can act on them.
        if matches!(code, KeyCode::Tab | KeyCode::BackTab) {
            return ferrowl_ui::EventResult::Unhandled(modifiers, code);
        }
        self.events += 1;
        ferrowl_ui::EventResult::Consumed
    }
}

#[focusable]
#[derive(Builder, Debug, Focus)]
struct TestApp {
    #[focus]
    pub first: Widget,
    #[focus]
    pub second: Widget,
    #[focus]
    pub third: Widget,
}

fn make_app() -> TestApp {
    TestAppBuilder::default()
        .first(Widget::default())
        .second(Widget::default())
        .third(Widget::default())
        .focus(TestAppFocus::First)
        .view_focused(false)
        .build()
        .expect("TestApp builder failed")
}

#[test]
/// UI-R-049 — focus_next advances focus to the next focusable field.
fn it_focus_next_advances() {
    let mut app = make_app();
    // starts at First, moves to Second
    app.focus_next();
    assert!(app.second.is_focused());
}

#[test]
/// UI-R-049 — focus_next wraps from the last field back to the first.
fn it_focus_next_wraps_around() {
    let mut app = make_app();
    app.focus_next(); // → Second
    app.focus_next(); // → Third
    app.focus_next(); // → wraps back to First
    assert!(app.first.is_focused());
}

#[test]
/// UI-R-049 — focus_previous wraps from the first field to the last.
fn it_focus_previous_wraps_backward() {
    let mut app = make_app();
    // at First, previous wraps to Third
    app.focus_previous();
    assert!(app.third.is_focused());
}

#[test]
/// UI-R-049 — focus_previous reverses a focus_next step.
fn it_focus_previous_reverses_next() {
    let mut app = make_app();
    app.focus_next(); // → Second
    app.focus_previous(); // → First
    assert!(app.first.is_focused());
}

#[test]
/// UI-R-049 — exactly one field is focused after a focus step; the prior one is cleared.
fn it_exactly_one_widget_focused_after_switch() {
    let mut app = make_app();
    app.focus_next(); // → Second
    let focused = [&app.first, &app.second, &app.third]
        .iter()
        .filter(|w| w.is_focused())
        .count();
    assert_eq!(focused, 1);
    assert!(app.second.is_focused());

    app.focus_next(); // → Third; previous (Second) must be cleared
    assert!(!app.second.is_focused());
    assert!(app.third.is_focused());
}

#[test]
/// UI-R-049 — a focusable container routes key events to its currently-focused field.
fn it_handle_events_routes_to_focused_widget() {
    use crossterm::event::{KeyCode, KeyModifiers};
    use ferrowl_ui::traits::HandleEvents;

    let mut app = make_app(); // focus = First
    app.handle_events(KeyModifiers::NONE, KeyCode::Char('a'));
    assert_eq!(app.first.events, 1);
    assert_eq!(app.second.events, 0);
    assert_eq!(app.third.events, 0);

    app.focus_next(); // → Second
    app.handle_events(KeyModifiers::NONE, KeyCode::Char('b'));
    assert_eq!(app.first.events, 1);
    assert_eq!(app.second.events, 1);
    assert_eq!(app.third.events, 0);
}

// A view whose middle widget is focusable only when `second_enabled` is set, exercising the
// `#[focus(when = ...)]` gating path of the derive macro.
#[focusable]
#[derive(Builder, Debug, Focus)]
struct GatedApp {
    #[focus]
    pub first: Widget,
    #[focus(when = self.second_enabled)]
    pub second: Widget,
    #[focus]
    pub third: Widget,
    pub second_enabled: bool,
}

fn make_gated(second_enabled: bool, start: GatedAppFocus) -> GatedApp {
    GatedAppBuilder::default()
        .first(Widget::default())
        .second(Widget::default())
        .third(Widget::default())
        .second_enabled(second_enabled)
        .focus(start)
        .view_focused(false)
        .build()
        .expect("GatedApp builder failed")
}

#[test]
/// UI-R-049 — the focus cycle skips a field whose enabling condition is false.
fn it_focus_next_skips_disabled_gated_widget() {
    let mut app = make_gated(false, GatedAppFocus::First);
    app.focus_next(); // First → (Second disabled, skipped) → Third
    assert!(app.third.is_focused());
    assert!(!app.second.is_focused());
}

#[test]
/// UI-R-049 — the focus cycle lands on a gated field when its condition is true.
fn it_focus_next_lands_on_enabled_gated_widget() {
    let mut app = make_gated(true, GatedAppFocus::First);
    app.focus_next(); // First → Second (enabled)
    assert!(app.second.is_focused());
    assert!(!app.third.is_focused());
}

#[test]
/// UI-R-049 — reverse focus cycle also skips a disabled gated field.
fn it_focus_previous_skips_disabled_gated_widget() {
    let mut app = make_gated(false, GatedAppFocus::Third);
    app.focus_previous(); // Third → (Second disabled, skipped) → First
    assert!(app.first.is_focused());
    assert!(!app.second.is_focused());
}

#[test]
/// UI-R-049 — focusing the container focuses its first eligible field and reports focused.
fn it_set_focused_true_focuses_first_and_reports_focused() {
    let mut app = make_app(); // view unfocused, nothing focused
    assert!(!app.is_focused());
    app.set_focused(true);
    assert!(app.is_focused());
    assert!(app.first.is_focused());
    assert!(!app.second.is_focused());
    assert!(!app.third.is_focused());
}

#[test]
/// UI-R-049 — unfocusing the container clears every field's focus.
fn it_set_focused_false_clears_all_widgets() {
    let mut app = make_app();
    app.set_focused(true);
    app.focus_next(); // Second focused
    app.set_focused(false);
    assert!(!app.is_focused());
    let focused = [&app.first, &app.second, &app.third]
        .iter()
        .filter(|w| w.is_focused())
        .count();
    assert_eq!(focused, 0);
}

#[test]
/// UI-R-049 — re-focusing the container restores the previously-focused field.
fn it_set_focused_restores_prior_pane() {
    let mut app = make_app();
    app.set_focused(true);
    app.focus_next(); // remember Second
    app.set_focused(false);
    app.set_focused(true); // restore Second, not First
    assert!(app.second.is_focused());
    assert!(!app.first.is_focused());
}

#[test]
/// UI-R-049 — if the remembered field is now ineligible, focus falls back to the first eligible field.
fn it_set_focused_falls_back_to_first_eligible_when_remembered_ineligible() {
    // Remembered pane is the gated Second, but it is disabled → enable lands on the first
    // eligible pane in declaration order (First).
    let mut app = make_gated(false, GatedAppFocus::Second);
    app.set_focused(true);
    assert!(app.first.is_focused());
    assert!(!app.second.is_focused());
}

#[test]
/// UI-R-049 — a remembered gated field that is still eligible is kept on re-focus.
fn it_set_focused_keeps_remembered_eligible_gated_pane() {
    // Remembered Second is eligible (enabled) → kept on enable.
    let mut app = make_gated(true, GatedAppFocus::Second);
    app.set_focused(true);
    assert!(app.second.is_focused());
    assert!(!app.first.is_focused());
}

#[focusable(nestable)]
#[derive(Builder, Clone, Debug, Focus)]
struct Section {
    #[focus]
    pub a: Widget,
    #[focus]
    pub b: Widget,
}

fn make_section(start: SectionFocus) -> Section {
    SectionBuilder::default()
        .a(Widget::default())
        .b(Widget::default())
        .focus(start)
        .view_focused(false)
        .build()
        .expect("Section builder failed")
}

#[focusable(nestable)]
#[derive(Builder, Debug, Focus)]
struct SingleSection {
    #[focus]
    pub a: Widget,
}

fn make_single_section() -> SingleSection {
    SingleSectionBuilder::default()
        .a(Widget::default())
        .focus(SingleSectionFocus::A)
        .view_focused(false)
        .build()
        .expect("SingleSection builder failed")
}

#[test]
/// UI-R-049 — `try_focus_next` on a `#[focusable(nestable)]` struct steps to the next pane.
fn it_try_focus_next_steps_within_section() {
    let mut section = make_section(SectionFocus::A);
    assert!(section.try_focus_next());
    assert!(section.b.is_focused());
    assert!(!section.a.is_focused());
}

#[test]
/// UI-R-049 — `try_focus_next` at the last pane reports `false` and leaves position unchanged.
fn it_try_focus_next_false_at_last_pane() {
    let mut section = make_section(SectionFocus::B);
    section.b.set_focused(true);
    assert!(!section.try_focus_next());
    assert!(
        section.b.is_focused(),
        "must not disable the current pane on a failed scan"
    );
}

#[test]
/// UI-R-049 — `try_focus_previous` at the first pane reports `false` and leaves position unchanged.
fn it_try_focus_previous_false_at_first_pane() {
    let mut section = make_section(SectionFocus::A);
    section.a.set_focused(true);
    assert!(!section.try_focus_previous());
    assert!(section.a.is_focused());
}

#[test]
/// UI-R-049 — a single-field `#[focusable(nestable)]` struct reports `false` immediately from
/// `try_focus_next`/`try_focus_previous`, without panicking or looping.
fn it_single_field_section_try_focus_next_false_immediately() {
    let mut section = make_single_section();
    assert!(!section.try_focus_next());
    assert!(!section.try_focus_previous());
}

#[focusable]
#[derive(Builder, Debug, Focus)]
struct NestingApp {
    #[focus]
    pub before: Widget,
    #[focus(nested)]
    pub section: Section,
    #[focus]
    pub after: Widget,
}

fn make_nesting_app(start: NestingAppFocus, section_start: SectionFocus) -> NestingApp {
    NestingAppBuilder::default()
        .before(Widget::default())
        .section(make_section(section_start))
        .after(Widget::default())
        .focus(start)
        .view_focused(false)
        .build()
        .expect("NestingApp builder failed")
}

/// Simulates the verified production call-site pattern (e.g.
/// `ferrowl/src/module/modbus/view/mod.rs:407-437`): try the view's own `handle_events` first;
/// only an `Unhandled` Tab/BackTab falls back to the outer `focus_next()`/`focus_previous()`.
fn send_key_with_fallback(
    app: &mut NestingApp,
    modifiers: crossterm::event::KeyModifiers,
    code: crossterm::event::KeyCode,
) {
    use crossterm::event::KeyCode;
    use ferrowl_ui::traits::HandleEvents;
    if let ferrowl_ui::EventResult::Unhandled(_, code) = app.handle_events(modifiers, code) {
        match code {
            KeyCode::Tab => app.focus_next(),
            KeyCode::BackTab => app.focus_previous(),
            _ => {}
        }
    }
}

#[test]
/// UI-R-049 — forward Tab into a `#[focus(nested)]` field enters at its first eligible pane,
/// regardless of which pane it last remembered.
fn it_nested_forward_tab_enters_section_at_first_pane() {
    use crossterm::event::{KeyCode, KeyModifiers};

    // Remembers B, so a pre-fix (remembered-or-first) entry would land on B, not A.
    let mut app = make_nesting_app(NestingAppFocus::Before, SectionFocus::B);
    app.before.set_focused(true);
    send_key_with_fallback(&mut app, KeyModifiers::NONE, KeyCode::Tab);
    assert_eq!(app.focus, NestingAppFocus::Section);
    assert!(app.section.a.is_focused());
    assert!(!app.section.b.is_focused());
}

#[test]
/// UI-R-049 — Tab at the nested field's last pane advances the outer cycle, not back to its own
/// first pane.
fn it_nested_forward_tab_at_last_pane_advances_to_after() {
    use crossterm::event::{KeyCode, KeyModifiers};

    let mut app = make_nesting_app(NestingAppFocus::Section, SectionFocus::B);
    app.section.b.set_focused(true);
    send_key_with_fallback(&mut app, KeyModifiers::NONE, KeyCode::Tab);
    assert_eq!(app.focus, NestingAppFocus::After);
    assert!(app.after.is_focused());
    assert!(!app.section.b.is_focused());
}

#[test]
/// UI-R-049 — BackTab from the field after a `#[focus(nested)]` field lands on its *last* pane,
/// not its first: the case a direction-blind, remembered-or-first entry rule gets wrong.
fn it_nested_backtab_from_after_lands_on_last_pane() {
    use crossterm::event::{KeyCode, KeyModifiers};

    // section remembers A (its default from make_section), proving entry ignores the remembered
    // pane entirely on a backward step.
    let mut app = make_nesting_app(NestingAppFocus::After, SectionFocus::A);
    app.after.set_focused(true);
    send_key_with_fallback(&mut app, KeyModifiers::SHIFT, KeyCode::BackTab);
    assert_eq!(app.focus, NestingAppFocus::Section);
    assert!(
        app.section.b.is_focused(),
        "must land on the last pane, not the remembered/first one"
    );
    assert!(!app.section.a.is_focused());
}

#[test]
/// UI-R-049 — repeated forward Tab visits every pane inside a `#[focus(nested)]` field, purely
/// via `handle_events`, before leaving it (no outer `focus_next()` fallback needed for the
/// interior step).
fn it_nested_repeated_tab_steps_within_section_before_leaving() {
    use crossterm::event::{KeyCode, KeyModifiers};

    let mut app = make_nesting_app(NestingAppFocus::Before, SectionFocus::A);
    app.before.set_focused(true);

    send_key_with_fallback(&mut app, KeyModifiers::NONE, KeyCode::Tab); // Before -> Section (a)
    assert_eq!(app.focus, NestingAppFocus::Section);
    assert!(app.section.a.is_focused());

    send_key_with_fallback(&mut app, KeyModifiers::NONE, KeyCode::Tab); // a -> b, still inside Section
    assert_eq!(app.focus, NestingAppFocus::Section);
    assert!(app.section.b.is_focused());
    assert!(!app.section.a.is_focused());

    send_key_with_fallback(&mut app, KeyModifiers::NONE, KeyCode::Tab); // b -> After, leaving Section
    assert_eq!(app.focus, NestingAppFocus::After);
    assert!(app.after.is_focused());
}

#[test]
/// UI-R-049 — the `HandleEvents` arm for a `#[focus(nested)]` field converts a successful
/// `NestedFocus` step to `Consumed`, isolated from the higher-level "did focus end up in the
/// right place" assertions above.
fn it_nested_handle_events_arm_converts_unhandled_tab_to_consumed() {
    use crossterm::event::{KeyCode, KeyModifiers};
    use ferrowl_ui::traits::HandleEvents;

    let mut app = make_nesting_app(NestingAppFocus::Section, SectionFocus::A);
    app.section.a.set_focused(true);
    let result = app.handle_events(KeyModifiers::NONE, KeyCode::Tab);
    assert!(matches!(result, ferrowl_ui::EventResult::Consumed));
}

#[focusable]
#[derive(Builder, Debug, Focus)]
struct GatedNestingApp {
    #[focus]
    pub before: Widget,
    #[focus(nested, when = self.section_enabled)]
    pub section: Section,
    #[focus]
    pub after: Widget,
    pub section_enabled: bool,
}

#[test]
/// UI-R-049 — a `when`-gated `#[focus(nested)]` field that's currently ineligible is skipped by
/// the outer walk exactly as any other gated field is today.
fn it_nested_when_gated_section_skipped_when_ineligible() {
    use crossterm::event::{KeyCode, KeyModifiers};
    use ferrowl_ui::traits::HandleEvents;

    let mut app = GatedNestingAppBuilder::default()
        .before(Widget::default())
        .section(make_section(SectionFocus::A))
        .after(Widget::default())
        .section_enabled(false)
        .focus(GatedNestingAppFocus::Before)
        .view_focused(false)
        .build()
        .expect("GatedNestingApp builder failed");
    app.before.set_focused(true);

    if let ferrowl_ui::EventResult::Unhandled(_, KeyCode::Tab) =
        app.handle_events(KeyModifiers::NONE, KeyCode::Tab)
    {
        app.focus_next();
    }
    assert_eq!(app.focus, GatedNestingAppFocus::After);
    assert!(app.after.is_focused());
}

#[focusable(nestable)]
#[derive(Builder, Clone, Debug, Focus)]
struct GatedSingleSection {
    #[focus(when = self.a_enabled)]
    pub a: Widget,
    pub a_enabled: bool,
}

fn make_gated_single_section(a_enabled: bool) -> GatedSingleSection {
    GatedSingleSectionBuilder::default()
        .a(Widget::default())
        .a_enabled(a_enabled)
        .focus(GatedSingleSectionFocus::A)
        .view_focused(false)
        .build()
        .expect("GatedSingleSection builder failed")
}

#[focusable]
#[derive(Builder, Debug, Focus)]
struct NestingAppGatedInner {
    #[focus]
    pub before: Widget,
    #[focus(nested)]
    pub section: GatedSingleSection,
    #[focus]
    pub after: Widget,
}

#[test]
/// UI-R-049 — a `#[focusable(nestable)]` struct whose only field is currently `when`-ineligible
/// makes entry into it a no-op the parent's own walk skips past (the private
/// `__focus_enter_first_eligible` helper's failure path, observable one level up).
fn it_nested_entry_into_ineligible_single_pane_section_is_noop() {
    use crossterm::event::{KeyCode, KeyModifiers};
    use ferrowl_ui::traits::HandleEvents;

    let mut app = NestingAppGatedInnerBuilder::default()
        .before(Widget::default())
        .section(make_gated_single_section(false))
        .after(Widget::default())
        .focus(NestingAppGatedInnerFocus::Before)
        .view_focused(false)
        .build()
        .expect("NestingAppGatedInner builder failed");
    app.before.set_focused(true);

    if let ferrowl_ui::EventResult::Unhandled(_, KeyCode::Tab) =
        app.handle_events(KeyModifiers::NONE, KeyCode::Tab)
    {
        app.focus_next();
    }
    assert_eq!(app.focus, NestingAppGatedInnerFocus::After);
    assert!(app.after.is_focused());
    assert!(!app.section.a.is_focused());
}
