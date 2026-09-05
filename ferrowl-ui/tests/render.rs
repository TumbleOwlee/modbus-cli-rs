//! Render-path coverage for the `widgets`, `style`, and `traits` modules:
//! every widget is drawn into an off-screen [`Buffer`] in the configurations
//! that exercise its border, focus, wrapping, scrolling, and placeholder
//! branches. We assert that rendering does not panic and (where cheap) that
//! something was drawn; the goal is to drive the layout/scroll logic.

// Integration-test crate: an unwrap that fails is the test failing, same as an assertion.
#![allow(clippy::unwrap_used)]

use ratatui::buffer::Buffer;
use ratatui::layout::{HorizontalAlignment, Margin, Rect};
use ratatui::widgets::{StatefulWidget, Widget as RWidget};

use ferrowl_syntax::{Language, SyntaxKind};
use ferrowl_ui::Border;
use ferrowl_ui::state::{
    ButtonStateBuilder, CodeInputFieldStateBuilder, InputFieldStateBuilder, SelectionStateBuilder,
    SuggestInputStateBuilder, TabBarState, TableStateBuilder,
};
use ferrowl_ui::style::{
    ButtonStyle, InputFieldStyle, SelectionStyle, SuggestInputStyle, SyntaxTheme, TabBarStyle,
    TableStyle, TextStyle,
};
use ferrowl_ui::traits::{Init, Suggestion, SuggestionProvider, ToLabel};
use ferrowl_ui::widgets::{
    ButtonBuilder, CodeInputFieldBuilder, Header, InputFieldBuilder, SelectionBuilder,
    SuggestInputBuilder, TabBarBuilder, TableBuilder, TableEntry, TextBuilder, Title, Width,
};

fn buffer(w: u16, h: u16) -> Buffer {
    Buffer::empty(Rect::new(0, 0, w, h))
}

fn full_border() -> Border {
    Border::Full(Margin::new(0, 0))
}

// ---- traits ----

#[test]
fn init_and_to_label() {
    // `Init` for the standard streams.
    let _o = std::io::Stdout::init();
    let _e = std::io::Stderr::init();
    // `ToLabel` for owned and borrowed strings.
    assert_eq!(String::from("x").to_label(), "x");
    assert_eq!("y".to_label(), "y");
}

// ---- styles ----

#[test]
fn style_defaults_build() {
    let _ = ButtonStyle::default();
    let _ = InputFieldStyle::default();
    let _ = TabBarStyle::default();
    let _ = SelectionStyle::default();
    let _ = SuggestInputStyle::default();
    let _ = TableStyle::default();
    let _ = TextStyle::default();
}

// ---- Button ----

#[test]
fn button_render_variants() {
    let widget = ButtonBuilder::default().build().unwrap();

    // Focused (default), short label.
    let mut st = ButtonStateBuilder::default()
        .label("OK".to_string())
        .build()
        .unwrap();
    let mut b = buffer(20, 3);
    StatefulWidget::render(&widget, Rect::new(0, 0, 20, 3), &mut b, &mut st);

    // Disabled -> general style; owned-widget render path.
    let mut st = ButtonStateBuilder::default()
        .label("NO".to_string())
        .disabled(true)
        .build()
        .unwrap();
    let mut b = buffer(20, 3);
    StatefulWidget::render(widget.clone(), Rect::new(0, 0, 20, 3), &mut b, &mut st);

    // Long label in a narrow area -> wrapping fold; multiline + alignment.
    let wide = ButtonBuilder::default()
        .multiline(true)
        .horizontal_alignment(HorizontalAlignment::Center)
        .build()
        .unwrap();
    let mut st = ButtonStateBuilder::default()
        .label("a very long button label that wraps".to_string())
        .build()
        .unwrap();
    let mut b = buffer(8, 6);
    StatefulWidget::render(&wide, Rect::new(0, 0, 8, 6), &mut b, &mut st);
}

// ---- Text ----

#[test]
fn text_render_variants() {
    // Borderless, short.
    let t = TextBuilder::default().build().unwrap();
    let mut s = "hello".to_string();
    let mut b = buffer(20, 3);
    StatefulWidget::render(&t, Rect::new(0, 0, 20, 3), &mut b, &mut s);

    // Bordered + title + multiline + long text that wraps; owned render path.
    let t = TextBuilder::default()
        .border(full_border())
        .title(Some("title".into()))
        .multiline(true)
        .build()
        .unwrap();
    let mut s = "a fairly long string that should wrap across rows".to_string();
    let mut b = buffer(10, 8);
    StatefulWidget::render(t, Rect::new(0, 0, 10, 8), &mut b, &mut s);
}

// ---- InputField ----

#[test]
fn input_field_render_variants() {
    // Empty + focused -> placeholder fallback text and cursor drawn.
    let f = InputFieldBuilder::<String>::default().build().unwrap();
    let mut st = InputFieldStateBuilder::default().build().unwrap();
    let mut b = buffer(20, 1);
    StatefulWidget::render(&f, Rect::new(0, 0, 20, 1), &mut b, &mut st);

    // Autofill on an empty field.
    let mut st = InputFieldStateBuilder::default()
        .autofill(Some("auto".to_string()))
        .build()
        .unwrap();
    let mut b = buffer(20, 1);
    StatefulWidget::render(&f, Rect::new(0, 0, 20, 1), &mut b, &mut st);

    // Numeric field with invalid, overflowing input -> error style + scroll window.
    let f = InputFieldBuilder::<i32>::default()
        .border(full_border())
        .title(Some("n".into()))
        .build()
        .unwrap();
    let mut st = InputFieldStateBuilder::default()
        .input("not-a-number-and-very-long-indeed".to_string())
        .cursor(12)
        .build()
        .unwrap();
    let mut b = buffer(24, 3);
    StatefulWidget::render(&f, Rect::new(0, 0, 24, 3), &mut b, &mut st);

    // Unfocused + error input -> unfocused error-style border branch.
    let f = InputFieldBuilder::<i32>::default()
        .border(full_border())
        .title(Some("n".into()))
        .build()
        .unwrap();
    let mut st = InputFieldStateBuilder::default()
        .focused(false)
        .input("not-a-number".to_string())
        .build()
        .unwrap();
    let mut b = buffer(24, 3);
    StatefulWidget::render(&f, Rect::new(0, 0, 24, 3), &mut b, &mut st);

    // Disabled (no cursor) and the plain `Widget` impl (builds default state).
    let f = InputFieldBuilder::<String>::default().build().unwrap();
    let mut st = InputFieldStateBuilder::default()
        .disabled(true)
        .build()
        .unwrap();
    let mut b = buffer(20, 1);
    StatefulWidget::render(&f, Rect::new(0, 0, 20, 1), &mut b, &mut st);

    let mut b = buffer(20, 1);
    RWidget::render(
        InputFieldBuilder::<String>::default().build().unwrap(),
        Rect::new(0, 0, 20, 1),
        &mut b,
    );
}

// ---- CodeInputField ----

#[test]
fn code_input_field_render_variants() {
    // Empty + unfocused + placeholder -> placeholder branch (bordered, titled).
    let w = CodeInputFieldBuilder::default()
        .border(full_border())
        .title(Some("code".into()))
        .build()
        .unwrap();
    let mut st = CodeInputFieldStateBuilder::default()
        .focused(false)
        .placeholder(Some("type lua...".to_string()))
        .build()
        .unwrap();
    let mut b = buffer(20, 6);
    StatefulWidget::render(&w, Rect::new(0, 0, 20, 6), &mut b, &mut st);

    // Multi-line content, focused, narrow area -> gutter, scroll, cursor, h-scroll.
    let w = CodeInputFieldBuilder::default().build().unwrap();
    let mut st = CodeInputFieldStateBuilder::default().build().unwrap();
    st.set_content("local x = 1\nlocal y = 2\nprint(x + y)\nreturn x");
    st.set_active_line(3);
    st.set_cursor_col(50);
    let mut b = buffer(8, 2);
    StatefulWidget::render(w, Rect::new(0, 0, 8, 2), &mut b, &mut st);
}

#[test]
/// UI-R-039 — rendered Lua highlighting maps keyword/comment kinds to their theme colors.
fn code_input_field_lua_syntax_highlighting() {
    let theme = SyntaxTheme::default();
    let content = "local x = 1 -- hi";
    let w = CodeInputFieldBuilder::default().build().unwrap();
    let mut st = CodeInputFieldStateBuilder::default()
        .language(Some(Language::Lua))
        .build()
        .unwrap();
    st.set_content(content);
    let mut b = buffer(40, 1);
    StatefulWidget::render(&w, Rect::new(0, 0, 40, 1), &mut b, &mut st);

    // gutter_width = line_count.to_string().len() + 1 = "1".len() + 1 = 2.
    let content_x = 2u16;

    // "local" keyword starts at char 0.
    assert_eq!(b[(content_x, 0)].fg, theme.keyword().fg.unwrap());

    // "-- hi" comment: find its start via the syntax crate directly.
    let (spans, _) = ferrowl_syntax::highlight_line(
        Language::Lua,
        content,
        ferrowl_syntax::LineState::default(),
    );
    let comment_start = spans
        .iter()
        .find(|(_, _, kind)| *kind == SyntaxKind::Comment)
        .map(|(start, _, _)| *start)
        .expect("expected a comment span");
    assert_eq!(
        b[(content_x + comment_start as u16, 0)].fg,
        theme.comment().fg.unwrap()
    );
}

#[test]
/// UI-R-039 — rendered JSON highlighting maps key/string kinds to their theme colors.
fn code_input_field_json_key_and_string_styles() {
    let theme = SyntaxTheme::default();
    let content = r#"{"key": "value"}"#;
    let w = CodeInputFieldBuilder::default().build().unwrap();
    let mut st = CodeInputFieldStateBuilder::default()
        .language(Some(Language::Json))
        .build()
        .unwrap();
    st.set_content(content);
    let mut b = buffer(40, 1);
    StatefulWidget::render(&w, Rect::new(0, 0, 40, 1), &mut b, &mut st);

    let content_x = 2u16;

    let (spans, _) = ferrowl_syntax::highlight_line(
        Language::Json,
        content,
        ferrowl_syntax::LineState::default(),
    );
    let key_start = spans
        .iter()
        .find(|(_, _, kind)| *kind == SyntaxKind::Key)
        .map(|(start, _, _)| *start)
        .expect("expected a key span");
    let string_start = spans
        .iter()
        .find(|(_, _, kind)| *kind == SyntaxKind::String)
        .map(|(start, _, _)| *start)
        .expect("expected a string span");

    assert_eq!(
        b[(content_x + key_start as u16, 0)].fg,
        theme.key().fg.unwrap()
    );
    assert_eq!(
        b[(content_x + string_start as u16, 0)].fg,
        theme.string().fg.unwrap()
    );
}

#[test]
/// UI-R-046 — horizontal scroll clips a highlighted span to the visible content window.
fn code_input_field_h_scroll_clips_mid_span() {
    let theme = SyntaxTheme::default();
    let content = r#"local s = "abcdefghijklmnopqrstuvwxyz""#;
    let (spans, _) = ferrowl_syntax::highlight_line(
        Language::Lua,
        content,
        ferrowl_syntax::LineState::default(),
    );
    let (string_start, string_end, _) = spans
        .into_iter()
        .find(|(_, _, kind)| *kind == SyntaxKind::String)
        .expect("expected a string span");
    assert!(
        string_end - string_start >= 6,
        "string span too short for test"
    );
    let h_scroll = string_start + 2;

    let w = CodeInputFieldBuilder::default().build().unwrap();
    let mut st = CodeInputFieldStateBuilder::default()
        .language(Some(Language::Lua))
        .build()
        .unwrap();
    st.set_content(content);
    // Keep the cursor away from content_x (offset 0) so its overlay style doesn't
    // mask the syntax color we're asserting on there.
    st.set_cursor_col(h_scroll + 3);
    st.set_h_scroll(h_scroll);
    let mut b = buffer(12, 1);
    StatefulWidget::render(&w, Rect::new(0, 0, 12, 1), &mut b, &mut st);

    // gutter_width = 2 (single line); the clipped span should appear at content_x
    // even though its unclipped start (`string_start`) is earlier in the line.
    let content_x = 2u16;
    assert_eq!(
        b[(content_x, 0)].fg,
        theme.string().fg.unwrap(),
        "expected string style at the clipped screen offset"
    );
}

// ---- TabBar ----

#[test]
/// UI-R-114, UI-R-116, UI-R-121, UI-R-174 — padded title columns written
/// one character per column, active-tab style over every cell, under
/// `Horizontal` layout.
fn it_tab_bar_horizontal_render_variants() {
    let w = TabBarBuilder::<String>::default()
        .padding(Margin::new(1, 1))
        .build()
        .unwrap();
    let mut st = TabBarState {
        titles: vec!["Tab".to_string()],
        active: 0,
        offset: 0,
    };
    let mut b = buffer(5, 3);
    StatefulWidget::render(&w, Rect::new(0, 0, 5, 3), &mut b, &mut st);
    let row = |b: &Buffer, y: u16| {
        (0..5)
            .map(|x| b[(x, y)].symbol().to_string())
            .collect::<String>()
    };
    assert_eq!(row(&b, 0), "     ");
    assert_eq!(row(&b, 1), " Tab ");
    assert_eq!(row(&b, 2), "     ");
    let selected = TabBarStyle::default().selected;
    for y in 0..3 {
        for x in 0..5 {
            assert_eq!(b[(x, y)].fg, selected.fg.unwrap());
            assert_eq!(b[(x, y)].bg, selected.bg.unwrap());
        }
    }
}

#[test]
/// UI-R-115, UI-R-116, UI-R-120, UI-R-121 — padded title rows
/// written one character per row, and active-block style identical whether
/// the widget is rendered by reference or consumed by value.
fn it_tab_bar_render_variants() {
    let w = TabBarBuilder::<String>::default()
        .padding(Margin::new(1, 1))
        .direction(ratatui::layout::Direction::Vertical)
        .build()
        .unwrap();
    let titles = vec!["Tab".to_string()];
    let mut st = TabBarState {
        titles: titles.clone(),
        active: 0,
        offset: 0,
    };
    let mut b1 = buffer(3, 5);
    StatefulWidget::render(&w, Rect::new(0, 0, 3, 5), &mut b1, &mut st);
    let row = |b: &Buffer, y: u16| {
        format!(
            "{}{}{}",
            b[(0, y)].symbol(),
            b[(1, y)].symbol(),
            b[(2, y)].symbol()
        )
    };
    assert_eq!(row(&b1, 0), "   ");
    assert_eq!(row(&b1, 1), " T ");
    assert_eq!(row(&b1, 2), " a ");
    assert_eq!(row(&b1, 3), " b ");
    assert_eq!(row(&b1, 4), "   ");
    let selected = TabBarStyle::default().selected;
    for y in 0..5 {
        for x in 0..3 {
            assert_eq!(b1[(x, y)].fg, selected.fg.unwrap());
            assert_eq!(b1[(x, y)].bg, selected.bg.unwrap());
        }
    }

    let mut st2 = TabBarState {
        titles,
        active: 0,
        offset: 0,
    };
    let mut b2 = buffer(3, 5);
    StatefulWidget::render(w, Rect::new(0, 0, 3, 5), &mut b2, &mut st2);
    for y in 0..5 {
        for x in 0..3 {
            assert_eq!(b1[(x, y)].symbol(), b2[(x, y)].symbol());
            assert_eq!(b1[(x, y)].fg, b2[(x, y)].fg);
            assert_eq!(b1[(x, y)].bg, b2[(x, y)].bg);
        }
    }
}

// ---- Selection ----

#[test]
fn selection_render_variants() {
    // Borderless, focused, selected item wider than the area -> horizontal offset.
    let w = SelectionBuilder::<String>::default().build().unwrap();
    let values: Vec<String> = vec![
        "short".to_string(),
        "a selected entry that is wider than the area".to_string(),
        "tail".to_string(),
    ];
    let mut st = SelectionStateBuilder::default()
        .values(values.clone())
        .build()
        .unwrap();
    st.move_down(); // select the wide row
    st.move_right(); // give it a horizontal offset
    let mut b = buffer(10, 5);
    StatefulWidget::render(&w, Rect::new(0, 0, 10, 5), &mut b, &mut st);

    // Bordered + title, unfocused; plain Widget impl (default state).
    let w = SelectionBuilder::<String>::default()
        .border(full_border())
        .title(Some("pick".into()))
        .build()
        .unwrap();
    let mut st = SelectionStateBuilder::default()
        .values(values)
        .focused(false)
        .build()
        .unwrap();
    let mut b = buffer(20, 6);
    StatefulWidget::render(&w, Rect::new(0, 0, 20, 6), &mut b, &mut st);
}

// ---- SuggestInput ----

#[derive(Debug, Clone)]
struct FixedProvider(Vec<&'static str>);

impl SuggestionProvider for FixedProvider {
    fn suggest(&self, input: &str) -> Vec<Suggestion> {
        if input.is_empty() {
            return vec![];
        }
        self.0
            .iter()
            .filter(|w| w.starts_with(input))
            .map(|w| Suggestion {
                value: w.to_string(),
                label: w.to_string(),
                partial: false,
            })
            .collect()
    }
}

fn opened_suggest_state(input: &str) -> ferrowl_ui::state::SuggestInputState<FixedProvider> {
    use ferrowl_ui::traits::HandleEvents;
    let mut st = SuggestInputStateBuilder::default()
        .provider(FixedProvider(vec!["apple", "apricot", "avocado"]))
        .build()
        .unwrap();
    for c in input.chars() {
        st.handle_events(
            ratatui::crossterm::event::KeyModifiers::NONE,
            ratatui::crossterm::event::KeyCode::Char(c),
        );
    }
    st
}

#[test]
/// UI-R-026 — the completion popup renders below its anchor when there is room.
fn suggest_input_popup_renders_below_anchor() {
    let w = SuggestInputBuilder::<String, FixedProvider>::default()
        .build()
        .unwrap();
    let mut st = opened_suggest_state("a");
    assert!(st.suggestions_open());

    let mut b = buffer(20, 10);
    let area = Rect::new(2, 2, 10, 1);
    StatefulWidget::render(&w, area, &mut b, &mut st);
    w.render_overlay(Rect::new(0, 0, 20, 10), &mut b, &mut st);

    assert_eq!(st.anchor(), Some(area));
    // Plenty of room below -> the popup's top border lands right under the
    // anchor row (area.y + area.height == 3), not above it.
    assert_ne!(b[(area.x, area.y + area.height)].symbol(), " ");
    assert_eq!(b[(area.x, 0)].symbol(), " ");
}

#[test]
/// UI-R-026 — the completion popup flips above its anchor when there is no room below.
fn suggest_input_popup_flips_above_when_no_room_below() {
    let w = SuggestInputBuilder::<String, FixedProvider>::default()
        .build()
        .unwrap();
    let mut st = opened_suggest_state("a");

    // Anchor near the bottom of a small buffer leaves no room below.
    let mut b = buffer(20, 6);
    let area = Rect::new(2, 5, 10, 1);
    StatefulWidget::render(&w, area, &mut b, &mut st);
    w.render_overlay(Rect::new(0, 0, 20, 6), &mut b, &mut st);

    // 3 matches -> popup height 5 (3 rows + border); with no room below the
    // anchor (row 5 of a 6-row buffer) it must flip above, landing at row 0
    // where its top border is drawn.
    assert_ne!(b[(area.x, 0)].symbol(), " ");
}

#[test]
/// UI-R-026 — rendering the popup on a tiny buffer, or when closed, is a no-op not a panic.
fn suggest_input_popup_no_panic_on_tiny_buffer() {
    let w = SuggestInputBuilder::<String, FixedProvider>::default()
        .build()
        .unwrap();
    let mut st = opened_suggest_state("a");

    let mut b = buffer(1, 1);
    let area = Rect::new(0, 0, 1, 1);
    StatefulWidget::render(&w, area, &mut b, &mut st);
    w.render_overlay(Rect::new(0, 0, 1, 1), &mut b, &mut st);

    // Closed popup / empty suggestions must also be a no-op, not a panic.
    let mut closed = SuggestInputStateBuilder::default()
        .provider(FixedProvider(vec!["apple"]))
        .build()
        .unwrap();
    let mut b = buffer(10, 10);
    StatefulWidget::render(&w, Rect::new(0, 0, 10, 1), &mut b, &mut closed);
    w.render_overlay(Rect::new(0, 0, 10, 10), &mut b, &mut closed);
}

// ---- Title conversions + Widget<S, W> pair forwarding ----

#[test]
fn title_conversions_and_widget_pair() {
    use ferrowl_ui::traits::{HandleEvents, IsFocus, Margins, SetFocus};
    use ferrowl_ui::widgets::{GetValue, Widget as WidgetPair};
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    // All four `From` impls for `Title`.
    let _: Title = "t".into();
    let _: Title = String::from("t").into();
    let _: Title = ("t", HorizontalAlignment::Center).into();
    let _: Title = (String::from("t"), HorizontalAlignment::Right).into();

    // The pair forwards each trait to the appropriate half.
    let state = InputFieldStateBuilder::default()
        .input("hi".to_string())
        .build()
        .unwrap();
    let widget = InputFieldBuilder::<String>::default().build().unwrap();
    let mut pair = WidgetPair { state, widget };

    assert_eq!(pair.get_value(), "hi");
    let _ = pair.margins();
    pair.set_focused(false);
    assert!(!pair.is_focused());
    let _ = pair.handle_events(KeyModifiers::NONE, KeyCode::Char('x'));

    // Render through both the borrowed and owned `RenderWidget`/`StatefulWidget` impls.
    let mut b = buffer(20, 1);
    RWidget::render(&pair, Rect::new(0, 0, 20, 1), &mut b);
    let mut b = buffer(20, 1);
    RWidget::render(pair.clone(), Rect::new(0, 0, 20, 1), &mut b);

    let mut s = InputFieldStateBuilder::default().build().unwrap();
    let mut b = buffer(20, 1);
    StatefulWidget::render(&pair, Rect::new(0, 0, 20, 1), &mut b, &mut s);
    let mut b = buffer(20, 1);
    StatefulWidget::render(pair, Rect::new(0, 0, 20, 1), &mut b, &mut s);
}

// ---- Table ----

#[derive(Clone, Default)]
struct Row(String, String);

impl TableEntry<2> for Row {
    fn values(&self) -> [String; 2] {
        [self.0.clone(), self.1.clone()]
    }
    fn height(&self) -> u16 {
        1
    }
}

#[derive(Clone)]
struct Cols;

impl Header<2> for Cols {
    fn header() -> [String; 2] {
        ["Name".to_string(), "Value".to_string()]
    }
    fn widths() -> [Width; 2] {
        [Width { min: 4, max: 8 }, Width { min: 4, max: 20 }]
    }
}

#[test]
/// UI-R-046 — a table wider than its area renders with horizontal scroll.
fn table_render_variants() {
    let rows = vec![
        Row(
            "alpha".to_string(),
            "a value with several words to wrap".to_string(),
        ),
        Row("beta".to_string(), "short".to_string()),
        Row(
            "gamma".to_string(),
            "supercalifragilisticexpialidocious".to_string(),
        ),
    ];

    // Focused, area wide enough that total_width <= area.width (no h-scroll).
    let w = TableBuilder::<Row, Cols, 2>::default().build().unwrap();
    let mut st = TableStateBuilder::default()
        .values(rows.clone())
        .build()
        .unwrap();
    let mut b = buffer(40, 8);
    StatefulWidget::render(&w, Rect::new(0, 0, 40, 8), &mut b, &mut st);

    // Bordered + title, unfocused, narrow area -> total_width > area.width (h-scroll copy),
    // and a column rendered without whitespace splitting.
    let w = TableBuilder::<Row, Cols, 2>::default()
        .border(full_border())
        .title(Some("regs".into()))
        .split_by_whitespace([true, false])
        .build()
        .unwrap();
    let mut st = TableStateBuilder::default()
        .values(rows.clone())
        .focused(false)
        .build()
        .unwrap();
    let mut b = buffer(14, 8);
    StatefulWidget::render(&w, Rect::new(0, 0, 14, 8), &mut b, &mut st);
}

#[test]
/// UI-R-066 — a `Table` with fewer rows than its area's height fills the whole bordered area with the
/// table's own background — the row/header area below the last row must not be left at the
/// buffer's default (uncleared) style, matching every other bordered widget's own filled
/// background. Narrow enough that `total_width > area.width` (the h-scroll copy path, shared
/// with `table_render_variants`'s own narrow-area case) — that path renders into a fresh, blank
/// `Buffer` before copying the visible slice back, and must preserve the border's already-painted
/// background for any row/header area the table itself did not touch.
fn table_fills_unused_row_area_with_its_own_background_not_default() {
    let w = TableBuilder::<Row, Cols, 2>::default()
        .border(full_border())
        .build()
        .unwrap();
    let mut st = TableStateBuilder::default()
        .values(vec![Row("only".to_string(), "row".to_string())])
        .build()
        .unwrap();
    let mut b = buffer(10, 10);
    StatefulWidget::render(&w, Rect::new(0, 0, 10, 10), &mut b, &mut st);

    // Row 0 is the border, row 1 the header, row 2 the one data row; row 5 is well past the
    // single data row but still inside the bordered area — it must carry the table's own
    // (non-default) background, not `Color::Reset`.
    let below_last_row = &b[(5, 5)];
    assert_ne!(
        below_last_row.bg,
        ratatui::style::Color::Reset,
        "area below the last row must be filled with the table's own background"
    );
}

#[test]
/// UI-R-066 — `show_selection_marker(false)` (default `true`, every other table unaffected)
/// suppresses the selection highlight bar glyph (`█`) entirely, while the selected row's own
/// background still distinguishes it from unselected rows.
fn table_show_selection_marker_false_suppresses_bar_keeps_row_highlight() {
    let rows = vec![
        Row("alpha".to_string(), "one".to_string()),
        Row("beta".to_string(), "two".to_string()),
    ];

    let w = TableBuilder::<Row, Cols, 2>::default()
        .border(full_border())
        .show_selection_marker(false)
        .build()
        .unwrap();
    let mut st = TableStateBuilder::default().values(rows).build().unwrap();
    let mut b = buffer(30, 6);
    StatefulWidget::render(&w, Rect::new(0, 0, 30, 6), &mut b, &mut st);

    let has_bar = (0..30u16)
        .flat_map(|x| (0..6u16).map(move |y| (x, y)))
        .any(|(x, y)| b[(x, y)].symbol() == "█");
    assert!(!has_bar, "no selection bar glyph anywhere in the buffer");

    // Row 1 (border) is the header, row 2 the first (selected) data row, row 3 the second
    // (unselected) data row — their backgrounds must still differ, so selection stays visible.
    let selected_bg = b[(2, 2)].bg;
    let unselected_bg = b[(2, 3)].bg;
    assert_ne!(
        selected_bg, unselected_bg,
        "selected row's background still distinguishes it without the bar"
    );
}

#[test]
/// UI-R-066 — `show_selection_marker(false)`'s marker column must collapse entirely (no
/// leftover whitespace prefix) rather than stay reserved-but-blank. Comparing against the
/// marker-`true` render of the identical rows/area: every row's first non-border, non-header
/// content column starts one cell further left once the marker is gone.
fn table_show_selection_marker_false_collapses_marker_column() {
    let rows = || {
        vec![
            Row("alpha".to_string(), "one".to_string()),
            Row("beta".to_string(), "two".to_string()),
        ]
    };

    let with_marker = TableBuilder::<Row, Cols, 2>::default()
        .border(full_border())
        .build()
        .unwrap();
    let mut st_with = TableStateBuilder::default().values(rows()).build().unwrap();
    let mut b_with = buffer(30, 6);
    StatefulWidget::render(
        &with_marker,
        Rect::new(0, 0, 30, 6),
        &mut b_with,
        &mut st_with,
    );

    let without_marker = TableBuilder::<Row, Cols, 2>::default()
        .border(full_border())
        .show_selection_marker(false)
        .build()
        .unwrap();
    let mut st_without = TableStateBuilder::default().values(rows()).build().unwrap();
    let mut b_without = buffer(30, 6);
    StatefulWidget::render(
        &without_marker,
        Rect::new(0, 0, 30, 6),
        &mut b_without,
        &mut st_without,
    );

    // Row 2 is the first data row ("alpha"). With the marker column reserved, its 'a' starts
    // further right than without it (the marker column no longer eats horizontal space).
    let first_char_x = |b: &Buffer| {
        (0..30u16)
            .find(|&x| b[(x, 2)].symbol() == "a")
            .expect("row must render its 'alpha' text somewhere on row 2")
    };
    let x_with = first_char_x(&b_with);
    let x_without = first_char_x(&b_without);
    assert!(
        x_without < x_with,
        "collapsing the marker column must shift row content left \
         (with marker: x={x_with}, without marker: x={x_without})"
    );
}

#[test]
/// UI-R-109 — without the bar glyph, the selected row's own background is the only remaining
/// selection cue, so it must stay clearly visible even while the table itself isn't the
/// currently focused panel: `show_selection_marker(false)` always uses the strong `focused`
/// style, never the subtler alternating-row style a marker-on unfocused table falls back to
/// (which relies on the marker glyph itself as the selection cue instead).
fn table_show_selection_marker_false_keeps_selected_row_visibly_highlighted_when_unfocused() {
    let rows = vec![
        Row("alpha".to_string(), "one".to_string()),
        Row("beta".to_string(), "two".to_string()),
    ];

    let w = TableBuilder::<Row, Cols, 2>::default()
        .border(full_border())
        .show_selection_marker(false)
        .build()
        .unwrap();
    let mut st = TableStateBuilder::default()
        .values(rows)
        .focused(false)
        .build()
        .unwrap();
    let mut b = buffer(30, 6);
    StatefulWidget::render(&w, Rect::new(0, 0, 30, 6), &mut b, &mut st);

    // Row 2 (the selected first data row) must use the same strong `focused` background as a
    // focused selection would.
    let style = TableStyle::default();
    assert_eq!(
        b[(2, 2)].bg,
        style.focused().bg.expect("focused style always sets a bg"),
        "selected row must stay highlighted with the strong `focused` background even while \
         the table itself is unfocused, once the marker glyph is gone"
    );
}

#[test]
/// UI-R-066 — the marker gutter's reserved width tracks the table's real selection state: zero
/// with nothing selected (an empty table), the highlight glyph's own width once a row is
/// selected — not a flat constant that over- or under-reserves relative to what's actually
/// drawn.
fn table_marker_gutter_width_tracks_selection_state_empty_to_non_empty() {
    let w = TableBuilder::<Row, Cols, 2>::default().build().unwrap();
    let mut st = TableStateBuilder::default()
        .values(Vec::new())
        .build()
        .unwrap();
    st.set_values(Vec::new());
    // Area narrower than the table's own content width, so `total_width` reports the table's
    // real computed width rather than being clamped up to a wide area's width.
    let mut b = buffer(5, 8);
    StatefulWidget::render(&w, Rect::new(0, 0, 5, 8), &mut b, &mut st);
    let empty_width = st.total_width();

    // Content no wider than the header's own labels ("Name"/"Value"), so the only width change
    // between the empty and non-empty renders is the marker gutter, not the column widths too.
    st.set_values(vec![
        Row("abcd".to_string(), "one".to_string()),
        Row("wxyz".to_string(), "two".to_string()),
    ]);
    StatefulWidget::render(&w, Rect::new(0, 0, 5, 8), &mut b, &mut st);
    let selected_width = st.total_width();

    assert_eq!(
        selected_width - empty_width,
        3,
        "selecting a row must grow the reserved gutter by exactly the marker glyph's width (3), \
         not a mismatched fixed constant (empty: {empty_width}, selected: {selected_width})"
    );
}

#[derive(Clone, Default)]
struct SpanRow;

impl TableEntry<2> for SpanRow {
    fn values(&self) -> [String; 2] {
        ["ab".to_string(), "plain".to_string()]
    }
    fn height(&self) -> u16 {
        1
    }
    fn cell_spans(&self) -> [Option<Vec<(String, ratatui::style::Style)>>; 2] {
        [
            Some(vec![
                (
                    "a".to_string(),
                    ratatui::style::Style::default().fg(ratatui::style::Color::Red),
                ),
                (
                    "b".to_string(),
                    ratatui::style::Style::default().fg(ratatui::style::Color::Blue),
                ),
            ]),
            None,
        ]
    }
}

#[test]
/// UI-R-063 — a `cell_spans`-returning column renders each span with its own color, distinct
/// from its neighbor's, rather than one flat cell color (the mechanism UI-R-063's Memory-layout
/// per-byte/word coloring is built on).
fn table_cell_spans_render_distinct_per_span_colors() {
    let w = TableBuilder::<SpanRow, Cols, 2>::default().build().unwrap();
    // Two rows so row index 1 (rendered on buffer row 2) is unselected — the table's default
    // first-row selection patches `focused`'s own fg over cell content, which would otherwise
    // mask the spans' colors this test asserts on.
    let mut st = TableStateBuilder::default()
        .values(vec![SpanRow, SpanRow])
        .build()
        .unwrap();
    let mut b = buffer(40, 8);
    StatefulWidget::render(&w, Rect::new(0, 0, 40, 8), &mut b, &mut st);

    // Buffer row 2 = data row 1 (row 0 is the header, buffer row 1 = data row 0, selected); x=0
    // padding, x=1 padding, x=2 padding, then the first data column's spans: 'a' at x=3, 'b' at
    // x=4, rendered adjacently (same column layout as the selected row's own highlight-bar
    // column, which is blank — not `█` — on an unselected row).
    let a_cell = &b[(3, 2)];
    let b_cell = &b[(4, 2)];
    assert_eq!(a_cell.symbol(), "a");
    assert_eq!(b_cell.symbol(), "b");
    assert_eq!(a_cell.fg, ratatui::style::Color::Red);
    assert_eq!(b_cell.fg, ratatui::style::Color::Blue);
    assert_ne!(a_cell.fg, b_cell.fg);
}
