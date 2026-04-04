mod input_field;
mod selection;
mod table;

use crossterm::event::{KeyCode, KeyModifiers};
pub use input_field::*;
use ratatui::widgets::{StatefulWidget, Widget as RenderWidget};
use ratatui::{buffer::Buffer, layout::Rect};
pub use selection::*;
pub use table::*;

use crate::traits::AsConstraint;
use crate::{
    EventResult,
    traits::{HandleEvents, SetFocus},
};
use std::fmt::Debug;

#[derive(Debug, Clone)]
pub struct Widget<S, W> {
    pub state: S,
    pub widget: W,
}

impl<S, W> Widget<S, W>
where
    W: AsConstraint<State = S>,
{
    pub fn horizontal(&self, height: Option<u16>) -> ratatui::layout::Constraint {
        self.widget.horizontal(&self.state, height)
    }

    pub fn vertical(&self, width: Option<u16>) -> ratatui::layout::Constraint {
        self.widget.vertical(&self.state, width)
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
