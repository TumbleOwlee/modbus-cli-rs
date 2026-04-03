extern crate proc_macro;
use derive_focus::{Focus, focusable};

#[derive(Default, Clone, Debug)]
struct State;

impl State {
    fn set_focused(&mut self, _focus: bool) {}
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
