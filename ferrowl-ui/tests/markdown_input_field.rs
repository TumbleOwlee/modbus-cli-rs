//! Buffer-render coverage for [`MarkdownInputField`], as `render.rs` does for the other
//! stateful widgets.

// Integration-test crate: an unwrap that fails is the test failing, same as an assertion.
#![allow(clippy::unwrap_used)]

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::StatefulWidget;

use ratatui::style::{Color, Style};

use ferrowl_syntax::SyntaxKind;
use ferrowl_ui::state::{MarkdownInputFieldState, MarkdownInputFieldStateBuilder};
use ferrowl_ui::style::MarkdownTheme;
use ferrowl_ui::traits::{HandleEvents, SetFocus};
use ferrowl_ui::widgets::MarkdownInputFieldBuilder;

fn buffer(w: u16, h: u16) -> Buffer {
    Buffer::empty(Rect::new(0, 0, w, h))
}

fn row_text(b: &Buffer, y: u16, w: u16) -> String {
    (0..w)
        .map(|x| b[(x, y)].symbol().chars().next().unwrap_or(' '))
        .collect()
}

fn state_with(content: &str) -> MarkdownInputFieldState {
    let mut s = MarkdownInputFieldStateBuilder::default()
        .build()
        .expect("defaults");
    s.set_content(content);
    SetFocus::set_focused(&mut s, true);
    s
}

#[test]
/// UI-R-182 — focused, editable, Normal mode: only the cursor's source line is drawn as
/// styled source (with markup visible); every other line is rendered (its own markup, a
/// bold marker, hidden — proving it is not drawn as source).
fn it_normal_mode_reveals_only_the_cursor_line_as_source() {
    let w = MarkdownInputFieldBuilder::default().build().unwrap();
    let mut s = state_with("# Heading\n**bold**");
    let mut b = buffer(30, 2);
    StatefulWidget::render(&w, Rect::new(0, 0, 30, 2), &mut b, &mut s);
    assert!(row_text(&b, 0, 30).starts_with("# Heading"));
    assert!(
        !row_text(&b, 1, 30).contains('*'),
        "the inactive line's bold markers must be hidden, proving it renders, not sources"
    );
    assert!(row_text(&b, 1, 30).starts_with("bold"));
}

#[test]
/// UI-R-183 — focused, editable, Insert or Visual mode: every line is drawn as styled
/// source, none rendered — the second line's bold markers stay visible in both modes.
fn it_insert_and_visual_draw_every_line_as_source() {
    let w = MarkdownInputFieldBuilder::default().build().unwrap();
    for enter in ['i', 'v'] {
        let mut s = state_with("# Heading\n**bold**");
        s.handle_events(KeyModifiers::NONE, KeyCode::Char(enter));
        let mut b = buffer(30, 2);
        StatefulWidget::render(&w, Rect::new(0, 0, 30, 2), &mut b, &mut s);
        assert!(row_text(&b, 0, 30).starts_with("# Heading"));
        assert!(
            row_text(&b, 1, 30).starts_with("**bold**"),
            "{enter:?} mode must show the second line's source markup, not its rendered form"
        );
    }
}

#[test]
/// UI-R-184 — unfocused, or read-only in any state, every line renders, cursor line
/// included: no line reveals its source.
fn it_unfocused_and_read_only_render_every_line_including_the_cursor_line() {
    let w = MarkdownInputFieldBuilder::default().build().unwrap();
    let mut s = state_with("# Heading\nplain text");
    SetFocus::set_focused(&mut s, false);
    let mut b = buffer(30, 2);
    StatefulWidget::render(&w, Rect::new(0, 0, 30, 2), &mut b, &mut s);
    assert!(
        !row_text(&b, 0, 30).starts_with('#'),
        "the heading marker must be hidden"
    );

    let mut s = state_with("# Heading\nplain text");
    s.set_read_only(true);
    let mut b = buffer(30, 2);
    StatefulWidget::render(&w, Rect::new(0, 0, 30, 2), &mut b, &mut s);
    assert!(
        !row_text(&b, 0, 30).starts_with('#'),
        "read-only must never reveal markup"
    );
}

#[test]
/// UI-R-129 — the revealed source line is styled by the markdown highlighter: a heading
/// marker (`#`) is styled distinctly from plain text.
fn it_revealed_source_line_is_styled_by_the_markdown_highlighter() {
    let w = MarkdownInputFieldBuilder::default().build().unwrap();
    let mut s = state_with("# Heading");
    let mut b = buffer(30, 1);
    StatefulWidget::render(&w, Rect::new(0, 0, 30, 1), &mut b, &mut s);
    let keyword = w.syntax_theme().style(SyntaxKind::Keyword);
    // Column 0 holds the cursor cell (UI-E-088), so assert on a later column of the
    // same whole-line Keyword span instead.
    assert_eq!(b[(5, 0)].fg, keyword.fg.unwrap());
}

#[test]
/// implementation detail (UI-R-130 and UI-R-142 are pinned by `markdown_render.rs`'s own
/// tests; this pins only the widget's own use of its wrap layout) — a line wider than the
/// widget wraps across several display rows and is never cut off, whether it is drawn as
/// styled source (Insert mode) or rendered (Normal mode, on an inactive line).
fn it_long_lines_wrap_across_display_rows_in_source_and_rendered_form() {
    let w = MarkdownInputFieldBuilder::default().build().unwrap();

    let mut s = state_with("abcdefghijklmnopqrstuvwxyz");
    s.handle_events(KeyModifiers::NONE, KeyCode::Char('i'));
    let mut b = buffer(10, 5);
    StatefulWidget::render(&w, Rect::new(0, 0, 10, 5), &mut b, &mut s);
    assert_eq!(row_text(&b, 0, 10), "abcdefghij");
    assert!(
        row_text(&b, 1, 10).starts_with("klmnopqrst"),
        "the styled-source path must wrap onto a following display row, not cut the line off"
    );
    assert!(row_text(&b, 2, 10).starts_with("uvwxyz"));

    let mut s = state_with("first\nthe quick brown fox jumps over the lazy dog");
    let mut b = buffer(10, 6);
    StatefulWidget::render(&w, Rect::new(0, 0, 10, 6), &mut b, &mut s);
    assert!(row_text(&b, 1, 10).trim_end().starts_with("the quick"));
    assert!(
        row_text(&b, 2, 10).contains("brown"),
        "the rendered path must wrap onto a following display row, not cut the line off"
    );
}

#[test]
/// implementation detail (UI-R-131 and UI-E-087 are pinned by `markdown_render.rs`'s own
/// tests; this pins only the widget's own use of its wrap layout) — a line of several short
/// words wider than the width breaks only at spaces, on both the styled-source path (Insert
/// mode) and the rendered path (Normal mode, inactive line): the character-boundary
/// fallback applies only when a single word is itself too wide, never here.
fn it_wraps_at_word_boundaries_not_mid_word_in_source_and_rendered_form() {
    let w = MarkdownInputFieldBuilder::default().build().unwrap();

    let mut s = state_with("abcd efgh ijkl");
    s.handle_events(KeyModifiers::NONE, KeyCode::Char('i'));
    let mut b = buffer(6, 3);
    StatefulWidget::render(&w, Rect::new(0, 0, 6, 3), &mut b, &mut s);
    assert_eq!(row_text(&b, 0, 6).trim_end(), "abcd");
    assert_eq!(row_text(&b, 1, 6).trim_end(), "efgh");
    assert_eq!(row_text(&b, 2, 6).trim_end(), "ijkl");

    let mut s = state_with("first\nabcd efgh ijkl");
    let mut b = buffer(6, 4);
    StatefulWidget::render(&w, Rect::new(0, 0, 6, 4), &mut b, &mut s);
    assert_eq!(row_text(&b, 1, 6).trim_end(), "abcd");
    assert_eq!(row_text(&b, 2, 6).trim_end(), "efgh");
    assert_eq!(row_text(&b, 3, 6).trim_end(), "ijkl");
}

#[test]
/// implementation detail (UI-R-137 is pinned by `MarkdownInputFieldState`'s own tests; this
/// pins only the widget's own feed into it) — the viewport follows the cursor onto a newly
/// wrapped row as typing wraps past the row currently visible, not just when a whole source
/// line changes.
fn it_scrolls_the_viewport_as_typing_wraps_past_the_visible_row() {
    let w = MarkdownInputFieldBuilder::default().build().unwrap();
    let mut s = state_with("");
    s.handle_events(KeyModifiers::NONE, KeyCode::Char('i'));
    for c in "abcdefg".chars() {
        s.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
    }
    let mut b = buffer(6, 1);
    StatefulWidget::render(&w, Rect::new(0, 0, 6, 1), &mut b, &mut s);
    assert_eq!(
        row_text(&b, 0, 6).trim_end(),
        "g",
        "the viewport must follow the cursor onto the wrapped row"
    );
}

#[test]
/// UI-E-146 — a line whose last display row exactly fills the available text width leaves
/// no free cell for the `Insert`-mode end-of-line cursor: nothing is painted, the row's
/// text still renders in full, and the render does not panic.
fn it_no_cursor_is_painted_when_the_last_display_row_exactly_fills_the_width() {
    let w = MarkdownInputFieldBuilder::default().build().unwrap();
    let mut s = state_with("");
    s.handle_events(KeyModifiers::NONE, KeyCode::Char('i'));
    for c in "hello worl".chars() {
        s.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
    }
    let mut b = buffer(10, 2);
    StatefulWidget::render(&w, Rect::new(0, 0, 10, 2), &mut b, &mut s);
    assert_eq!(row_text(&b, 0, 10), "hello worl");
    for x in 0..10 {
        assert_ne!(
            b[(x, 0)].bg,
            w.style().cursor.bg.unwrap(),
            "no cell of a row that exactly fills the width may carry the cursor style"
        );
    }
    assert_eq!(
        row_text(&b, 1, 10).trim_end(),
        "",
        "no extra display row is opened for the free cell that does not fit"
    );
}

#[test]
/// UI-R-313 — a cursor column one past the last character of its source line is painted on
/// the first free cell after the last character, not pulled back onto the last character.
fn it_insert_end_of_line_cursor_sits_on_the_free_cell_after_the_last_character() {
    let w = MarkdownInputFieldBuilder::default().build().unwrap();
    let mut s = state_with("");
    s.handle_events(KeyModifiers::NONE, KeyCode::Char('i'));
    for c in "hi".chars() {
        s.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
    }
    let mut b = buffer(10, 1);
    StatefulWidget::render(&w, Rect::new(0, 0, 10, 1), &mut b, &mut s);
    assert_eq!(
        b[(2, 0)].bg,
        w.style().cursor.bg.unwrap(),
        "the free cell after the last character must carry the cursor style"
    );
    assert_ne!(
        b[(1, 0)].bg,
        w.style().cursor.bg.unwrap(),
        "the last character's own cell must not carry the cursor style"
    );
}

#[test]
/// UI-R-312, UI-R-313 — on a wrapped line, the `Insert`-mode end-of-line cursor is painted
/// on the free cell after the last character of the line's last display row, not on the
/// first display row.
fn it_insert_end_of_line_cursor_on_a_wrapped_line_uses_the_last_display_row() {
    let w = MarkdownInputFieldBuilder::default().build().unwrap();
    let mut s = state_with("");
    s.handle_events(KeyModifiers::NONE, KeyCode::Char('i'));
    for c in "hello wo".chars() {
        s.handle_events(KeyModifiers::NONE, KeyCode::Char(c));
    }
    let mut b = buffer(6, 3);
    StatefulWidget::render(&w, Rect::new(0, 0, 6, 3), &mut b, &mut s);
    assert_eq!(row_text(&b, 0, 6).trim_end(), "hello");
    assert_eq!(row_text(&b, 1, 6).trim_end(), "wo");
    assert_eq!(
        b[(2, 1)].bg,
        w.style().cursor.bg.unwrap(),
        "the free cell after the last display row's text must carry the cursor style"
    );
    for x in 0..6 {
        assert_ne!(
            b[(x, 0)].bg,
            w.style().cursor.bg.unwrap(),
            "the first display row must not carry the cursor style"
        );
    }
}

#[test]
/// UI-R-312 — the cursor cell follows the character at the cursor's source column, offset
/// by the gutter when enabled.
fn it_cursor_cell_follows_the_source_column_and_the_gutter_offsets_it() {
    let w = MarkdownInputFieldBuilder::default()
        .line_numbers(true)
        .build()
        .unwrap();
    let mut s = state_with("hello");
    s.handle_events(KeyModifiers::NONE, KeyCode::Char('0'));
    s.handle_events(KeyModifiers::NONE, KeyCode::Char('l'));
    s.handle_events(KeyModifiers::NONE, KeyCode::Char('l'));
    let mut b = buffer(10, 1);
    StatefulWidget::render(&w, Rect::new(0, 0, 10, 1), &mut b, &mut s);
    assert_eq!(
        b[(4, 0)].bg,
        w.style().cursor.bg.unwrap(),
        "gutter width (2) plus source column (2) must locate the cursor cell"
    );
    assert_ne!(
        b[(2, 0)].bg,
        w.style().cursor.bg.unwrap(),
        "column 0 of the text must not carry the cursor style"
    );
    assert_ne!(b[(0, 0)].bg, w.style().cursor.bg.unwrap());
    assert_ne!(b[(1, 0)].bg, w.style().cursor.bg.unwrap());
}

#[test]
/// UI-R-138 — read-only, the cursor's source line highlights every one of its display
/// rows in the theme's highlighted-row style.
fn it_read_only_highlights_every_display_row_of_the_cursor_line() {
    let w = MarkdownInputFieldBuilder::default()
        .line_numbers(true)
        .build()
        .unwrap();
    let mut s = state_with("abc\none two three four five six seven eight");
    s.set_read_only(true);
    s.handle_events(KeyModifiers::NONE, KeyCode::Char('j'));
    let mut b = buffer(10, 5);
    StatefulWidget::render(&w, Rect::new(0, 0, 10, 5), &mut b, &mut s);
    let hi = w.markdown_theme().highlighted_row();
    assert_eq!(
        b[(0, 1)].bg,
        hi.bg.unwrap(),
        "the gutter cell of the cursor line's display row must be highlighted too"
    );
    assert_eq!(b[(2, 1)].bg, hi.bg.unwrap());
    assert_eq!(
        b[(2, 2)].bg,
        hi.bg.unwrap(),
        "every display row of the cursor line is highlighted"
    );
    assert_ne!(
        b[(2, 0)].bg,
        hi.bg.unwrap(),
        "the non-cursor line is not highlighted"
    );
}

#[test]
/// UI-R-140 — the gutter is off by default; enabled, it numbers only the first display
/// row of a wrapped source line and leaves continuation rows blank.
fn it_gutter_is_off_by_default_and_numbers_only_first_display_rows() {
    let w = MarkdownInputFieldBuilder::default().build().unwrap();
    let mut s = state_with("one two three four five six seven eight");
    SetFocus::set_focused(&mut s, false);
    let mut b = buffer(10, 4);
    StatefulWidget::render(&w, Rect::new(0, 0, 10, 4), &mut b, &mut s);
    assert_eq!(
        row_text(&b, 0, 10).chars().next(),
        Some('o'),
        "no gutter by default"
    );

    let w = MarkdownInputFieldBuilder::default()
        .line_numbers(true)
        .build()
        .unwrap();
    let mut b = buffer(10, 4);
    StatefulWidget::render(&w, Rect::new(0, 0, 10, 4), &mut b, &mut s);
    assert_eq!(row_text(&b, 0, 10).chars().next(), Some('1'));
    assert_eq!(
        row_text(&b, 1, 10).chars().next(),
        Some(' '),
        "continuation row blank"
    );
}

#[test]
/// UI-R-140, UI-R-148 — a horizontal rule row keeps its line number in the gutter and the
/// rule itself spans the full text width to the right of the gutter, not the gutter columns.
fn it_horizontal_rule_keeps_the_gutter_and_spans_the_text_width_after_it() {
    let w = MarkdownInputFieldBuilder::default()
        .line_numbers(true)
        .build()
        .unwrap();
    let mut s = state_with("one\n---\n");
    SetFocus::set_focused(&mut s, false);
    let mut b = buffer(10, 3);
    StatefulWidget::render(&w, Rect::new(0, 0, 10, 3), &mut b, &mut s);
    assert_eq!(
        row_text(&b, 1, 2).trim_end(),
        "2",
        "the rule row's gutter still shows its line number"
    );
    let rule_style = *w.markdown_theme().rule();
    for x in 2..10 {
        assert_eq!(
            b[(x, 1)].fg,
            rule_style.fg.unwrap(),
            "column {x} of the rule row's text width must be filled"
        );
    }
    assert_ne!(
        b[(0, 1)].fg,
        rule_style.fg.unwrap(),
        "the gutter column must not carry the rule style"
    );
}

#[test]
/// implementation detail — the markdown theme is injected on the builder (UI-R-141 is
/// pinned by `MarkdownTheme`'s own tests; this pins only the widget's own field).
fn it_markdown_theme_is_injected_on_the_builder() {
    let mut theme = MarkdownTheme::default();
    theme.set_rule(Style::default().fg(Color::Magenta));
    let w = MarkdownInputFieldBuilder::default()
        .markdown_theme(theme.clone())
        .build()
        .unwrap();
    assert_eq!(w.markdown_theme().rule(), theme.rule());
    assert_eq!(w.markdown_theme().rule().fg, Some(Color::Magenta));
}

#[test]
/// implementation detail (UI-E-077 states the absence of a consumer, which no test can
/// assert): the example's document, covering every construct, renders end to end without
/// panicking.
fn it_renders_a_document_covering_every_construct() {
    let w = MarkdownInputFieldBuilder::default().build().unwrap();
    let mut s = state_with(
        "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6\n\n\
         - a\n  - nested\n1. one\n- [ ] todo\n- [x] done\n\n\
         > quote\n>> nested quote\n\n---\n\n\
         ```lua\nlocal x = 1\n```\n\n\
         ```json\n{\"a\": 1}\n```\n\n\
         ```\nno info string\n```\n\n\
         **bold** *italic* `code` ~~strike~~ [link](url) ![image](url)\n\n\
         this line is deliberately much longer than the widget is wide so it must wrap\n\
         andthiswordisalsoextremelylongandcannotfitonanyrowwithoutbreaking\n",
    );
    SetFocus::set_focused(&mut s, false);
    let mut b = buffer(20, 30);
    StatefulWidget::render(&w, Rect::new(0, 0, 20, 30), &mut b, &mut s);
    assert!(
        !row_text(&b, 0, 20).contains('#'),
        "the heading marker must be hidden when rendered"
    );
    assert!(row_text(&b, 0, 20).contains("H1"));
}

#[test]
/// UI-R-188 — `measure` agrees with the display-row height a real render actually uses,
/// with and without the line-number gutter: the rendered height (last non-blank row's
/// index plus one, not a count of non-blank rows — that would drop the blank rows
/// separating blocks, which `measure` rightly counts) matches `measure`'s own count.
fn it_measure_agrees_with_the_rendered_buffer() {
    let content = "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6\n\n\
         - a\n  - nested\n1. one\n- [ ] todo\n- [x] done\n\n\
         > quote\n>> nested quote\n\n---\n\n\
         ```lua\nlocal x = 1\n```\n\n\
         ```json\n{\"a\": 1}\n```\n\n\
         ```\nno info string\n```\n\n\
         **bold** *italic* `code` ~~strike~~ [link](url) ![image](url)\n\n\
         this line is deliberately much longer than the widget is wide so it must wrap\n\
         andthiswordisalsoextremelylongandcannotfitonanyrowwithoutbreaking";
    for line_numbers in [false, true] {
        let w = MarkdownInputFieldBuilder::default()
            .line_numbers(line_numbers)
            .build()
            .unwrap();
        let mut s = state_with(content);
        SetFocus::set_focused(&mut s, false);
        let width = 20u16;
        let mut b = buffer(width, 60);
        StatefulWidget::render(&w, Rect::new(0, 0, width, 60), &mut b, &mut s);
        let rendered_height = (0..60u16)
            .rev()
            .find(|&y| row_text(&b, y, width).chars().any(|c| c != ' '))
            .map_or(0, |y| y as usize + 1);
        assert_eq!(
            w.measure(content, width),
            rendered_height,
            "line_numbers={line_numbers}"
        );
    }
}
