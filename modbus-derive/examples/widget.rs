extern crate proc_macro;
use modbus_derive::{Focus, focusable};

#[derive(Default, Clone, Debug)]
struct State;

impl modbus_ui::traits::SetFocus for State {
    fn set_focused(&mut self, _focus: bool) {}
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
#[derive(Default, Clone, Debug, Focus)]
struct App {
    #[focus]
    pub name: State,
    #[focus]
    pub lastname: State,
}

fn main() {
    let mut app = App::default();
    app.focus_previous();
    println!("{:?}", app);
}
