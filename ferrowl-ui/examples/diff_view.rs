// Example binary: unwrap keeps the demo focused on the widget being shown.
#![allow(clippy::unwrap_used)]

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ferrowl_ui::{
    AlternateScreen, Border,
    state::{DiffViewState, DiffViewStateBuilder, Side},
    traits::{HandleEvents, SetFocus},
    widgets::{DiffView, DiffViewBuilder},
};
use ratatui::{Frame, layout::Margin};
use std::{io::Stdout, time::Duration};

const DIFF: &str = "\
@@ -1,6 +1,7 @@
 local function greet(name)
-    print('hello, ' .. name)
+    print('hi, ' .. name)
+    print('nice to see you, ' .. name)
 end

 greet('world')
+greet('lua')
";

struct App {
    widget: DiffView,
    state: DiffViewState,
}

impl Default for App {
    fn default() -> Self {
        let mut state = DiffViewStateBuilder::default()
            .language(Some(ferrowl_syntax::Language::Lua))
            .build_with_diff(DIFF)
            .unwrap();
        state.set_focused(true);
        let widget = DiffViewBuilder::default()
            .border(Border::Full(Margin::new(1, 0)))
            .build()
            .unwrap();
        Self { widget, state }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    f.render_stateful_widget(&app.widget, f.area(), &mut app.state);
}

fn main() {
    let mut screen: AlternateScreen<Stdout> =
        AlternateScreen::new().expect("Failed to create alternate screen.");

    let mut app = App::default();

    loop {
        screen.draw(|f| ui(f, &mut app)).unwrap();

        if event::poll(Duration::from_millis(50)).unwrap()
            && let Event::Key(key) = event::read().unwrap()
            && key.kind == KeyEventKind::Press
        {
            // The diff widget has no Insert mode (UI-R-222), so `q` is always safe to
            // intercept before offering the key to the widget.
            match (key.modifiers, key.code) {
                (KeyModifiers::NONE, KeyCode::Char('q')) => break,
                (KeyModifiers::NONE, KeyCode::Tab) => {
                    let side = match app.state.focused_side() {
                        Side::Old => Side::New,
                        Side::New => Side::Old,
                    };
                    app.state.set_focused_side(side);
                }
                (modifiers, code) => {
                    app.state.handle_events(modifiers, code);
                }
            }
        }
    }
}
