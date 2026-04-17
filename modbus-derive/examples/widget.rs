extern crate proc_macro;
use derive_builder::Builder;
use modbus_derive::{Focus, focusable};

#[derive(Default, Clone, Debug)]
struct State;

impl modbus_ui::traits::SetFocus for State {
    fn set_focused(&mut self, _focus: bool) {}
}

impl modbus_ui::traits::IsFocus for State {
    fn is_focused(&self) -> bool {
        true
    }
}

impl modbus_ui::traits::HandleEvents for State {
    fn handle_events(
        &mut self,
        _modifiers: crossterm::event::KeyModifiers,
        _code: crossterm::event::KeyCode,
    ) -> modbus_ui::EventResult {
        modbus_ui::EventResult::Consumed
    }
}

#[focusable]
#[derive(Builder, Debug, Focus)]
struct App {
    #[focus]
    pub name: State,
    #[focus]
    pub lastname: State,
}

fn main() {
    let mut app = AppBuilder::default()
        .name(State::default())
        .lastname(State::default())
        .build()
        .expect("App builder failed.");
    app.focus_previous();
    println!("{:?}", app);
}
