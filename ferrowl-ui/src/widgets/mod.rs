//! Stateful ratatui widgets: button, text, single-/multi-line input,
//! selection list, and table.

mod build;
mod button;
mod code_input_field;
mod input_field;
mod markdown_input_field;
mod markdown_render;
mod selection;
mod suggest_input;
mod tab_bar;
mod table;
mod text;

pub use build::*;
pub use button::*;
pub use code_input_field::*;
use crossterm::event::{KeyCode, KeyModifiers};
pub use input_field::*;
pub use markdown_input_field::*;
use ratatui::layout::{HorizontalAlignment, Margin};
use ratatui::style::Style;
use ratatui::widgets::{Block, StatefulWidget, Widget as RenderWidget};
use ratatui::{buffer::Buffer, layout::Rect};

use crate::Border;

/// If `border` is `Border::Full`, render a styled (optionally titled) bordered block into `area`
/// and return the inner content rect (after the border's inner margin). For `Border::None` the
/// `area` is returned unchanged. Shared by the input-field, selection and table widgets.
pub fn render_border(
    area: Rect,
    buf: &mut Buffer,
    border: &Border,
    title: Option<&Title>,
    style: Style,
) -> Rect {
    if let Border::Full(margin) = border {
        let mut block = Block::bordered().style(style);
        if let Some(title) = title {
            block = block
                .title(title.name.as_str())
                .title_alignment(title.alignment);
        }
        let inner = block.inner(area);
        block.render(area, buf);
        inner.inner(*margin)
    } else {
        area
    }
}
pub use selection::*;
pub use suggest_input::*;
pub use tab_bar::*;
pub use table::*;
pub use text::*;

use crate::traits::{IsFocus, Margins};
use crate::{
    EventResult,
    traits::{HandleEvents, SetFocus},
};
use std::fmt::Debug;

/// A widget title with horizontal alignment, shown in the border line.
/// Convertible from `&str`/`String` (left-aligned) or a
/// `(name, alignment)` tuple.
#[derive(Debug, Clone)]
pub struct Title {
    name: String,
    alignment: HorizontalAlignment,
}

impl From<&str> for Title {
    fn from(name: &str) -> Self {
        Self {
            name: name.to_string(),
            alignment: HorizontalAlignment::Left,
        }
    }
}

impl From<String> for Title {
    fn from(name: String) -> Self {
        Self {
            name,
            alignment: HorizontalAlignment::Left,
        }
    }
}

impl From<(&str, HorizontalAlignment)> for Title {
    fn from((name, alignment): (&str, HorizontalAlignment)) -> Self {
        Self {
            name: name.to_string(),
            alignment,
        }
    }
}

impl From<(String, HorizontalAlignment)> for Title {
    fn from((name, alignment): (String, HorizontalAlignment)) -> Self {
        Self { name, alignment }
    }
}

/// Extracts the current value a widget represents (input text, selected
/// item, selected row, …).
pub trait GetValue {
    type ValueType;

    fn get_value(&self) -> Self::ValueType;
}

/// Pairs a widget `W` with its state `S` so the pair can be stored, focused,
/// rendered, and fed events as one unit. All widget traits are forwarded to
/// the appropriate half (`state` for behavior, `widget` for rendering).
#[derive(Debug, Clone)]
pub struct Widget<S, W> {
    pub state: S,
    pub widget: W,
}

impl<S, W> GetValue for Widget<S, W>
where
    S: GetValue,
{
    type ValueType = S::ValueType;

    fn get_value(&self) -> S::ValueType {
        self.state.get_value()
    }
}

impl<S, W> Margins for Widget<S, W>
where
    W: Margins,
{
    fn margins(&self) -> Margin {
        self.widget.margins()
    }
}

impl<S, W> SetFocus for Widget<S, W>
where
    S: SetFocus,
{
    fn set_focused(&mut self, focus: bool) {
        self.state.set_focused(focus);
    }
}

impl<S, W> IsFocus for Widget<S, W>
where
    S: IsFocus,
{
    fn is_focused(&self) -> bool {
        self.state.is_focused()
    }
}

impl<S, W> HandleEvents for Widget<S, W>
where
    S: HandleEvents,
{
    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        self.state.handle_events(modifiers, code)
    }
}

impl<S, W> RenderWidget for Widget<S, W>
where
    W: RenderWidget,
{
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.widget.render(area, buf);
    }
}

impl<S, W> RenderWidget for &Widget<S, W>
where
    for<'a> &'a W: RenderWidget,
{
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.widget.render(area, buf)
    }
}

impl<S, W> StatefulWidget for Widget<S, W>
where
    W: StatefulWidget<State = S>,
{
    type State = S;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(self.widget, area, buf, state)
    }
}

impl<S, W> StatefulWidget for &Widget<S, W>
where
    for<'a> &'a W: StatefulWidget<State = S>,
{
    type State = S;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        StatefulWidget::render(&self.widget, area, buf, state)
    }
}

/// Render a single `$self.$field: Widget<S, W>` in `$area`. A generic function taking
/// `&mut Widget<S, W>` hits trait-solver overflow on the blanket `StatefulWidget for &Widget<S,
/// W>` impl above (ambiguous recursion through `W`), so this boilerplate is a macro instead —
/// each call site expands with `W` already concrete, sidestepping the overflow entirely.
#[macro_export]
macro_rules! render_field {
    ($self:ident, $field:ident, $area:expr, $buf:expr) => {
        ::ratatui::widgets::StatefulWidget::render(
            &$self.$field.widget,
            $area,
            $buf,
            &mut $self.$field.state,
        )
    };
}

/// Render any number of `$self.$field` widgets left-to-right across `$area`, either evenly
/// split (`field1, field2, ...`) or each sized by its own `Constraint`
/// (`field1 => Constraint::Length(12), field2 => Constraint::Min(1), ...`).
#[macro_export]
macro_rules! render_row {
    ($self:ident, $area:expr, $buf:expr; $($field:ident),+ $(,)?) => {{
        let __areas = ::ratatui::layout::Layout::horizontal(::std::vec![
            ::ratatui::layout::Constraint::Fill(1);
            [$(::std::stringify!($field)),+].len()
        ])
        .split($area);
        let mut __i = 0;
        $(
            $crate::render_field!($self, $field, __areas[__i], $buf);
            __i += 1;
        )+
    }};
    ($self:ident, $area:expr, $buf:expr; $($field:ident => $constraint:expr),+ $(,)?) => {{
        let __areas = ::ratatui::layout::Layout::horizontal([$($constraint),+]).split($area);
        let mut __i = 0;
        $(
            $crate::render_field!($self, $field, __areas[__i], $buf);
            __i += 1;
        )+
    }};
}

/// Render any number of `$self.$field` widgets top-to-bottom across `$area`, either evenly
/// split or each sized by its own `Constraint` — the vertical counterpart of `render_row!`.
#[macro_export]
macro_rules! render_col {
    ($self:ident, $area:expr, $buf:expr; $($field:ident),+ $(,)?) => {{
        let __areas = ::ratatui::layout::Layout::vertical(::std::vec![
            ::ratatui::layout::Constraint::Fill(1);
            [$(::std::stringify!($field)),+].len()
        ])
        .split($area);
        let mut __i = 0;
        $(
            $crate::render_field!($self, $field, __areas[__i], $buf);
            __i += 1;
        )+
    }};
    ($self:ident, $area:expr, $buf:expr; $($field:ident => $constraint:expr),+ $(,)?) => {{
        let __areas = ::ratatui::layout::Layout::vertical([$($constraint),+]).split($area);
        let mut __i = 0;
        $(
            $crate::render_field!($self, $field, __areas[__i], $buf);
            __i += 1;
        )+
    }};
}
