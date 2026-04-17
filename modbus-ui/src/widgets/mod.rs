mod input_field;
mod selection;
mod table;
mod text;

use crossterm::event::{KeyCode, KeyModifiers};
pub use input_field::*;
use ratatui::layout::{HorizontalAlignment, Margin};
use ratatui::widgets::{StatefulWidget, Widget as RenderWidget};
use ratatui::{buffer::Buffer, layout::Rect};
pub use selection::*;
pub use table::*;
pub use text::*;

use crate::traits::{IsFocus, Margins};
use crate::{
    EventResult,
    traits::{HandleEvents, SetFocus},
};
use std::fmt::Debug;

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

pub trait GetValue {
    type ValueType;

    fn get_value(&self) -> Self::ValueType;
}

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
