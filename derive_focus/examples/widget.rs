extern crate proc_macro;
use derive_focus::{Focus, focusable};

#[derive(Default, Clone, Debug)]
struct State;

impl ui::traits::SetFocus for State {
    fn set_focused(&mut self, _focus: bool) {}
}

impl ui::traits::HandleEvents for State {
    fn handle_events(
        &mut self,
        _modifiers: crossterm::event::KeyModifiers,
        _code: crossterm::event::KeyCode,
    ) -> ui::EventResult {
        ui::EventResult::Consumed
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
