use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{Frame, layout::Margin, style::palette::tailwind};
use std::{io::Stdout, time::Duration};
use ui::{
    AlternateScreen, EventResult, Style, state::SelectionState, traits::HandleEvents,
    traits::ToLabel, widgets::Selection,
};

#[derive(Clone, Debug)]
struct Item {
    name: String,
    value: u32,
}

impl Item {
    fn new(name: String, value: u32) -> Self {
        Self { name, value }
    }
}

impl ToLabel for Item {
    fn to_label(&self) -> String {
        self.name.clone()
    }
}

// Simple app consisting of single input field
struct App {
    pub state: SelectionState<Item>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            state: SelectionState::default().focus(),
        }
    }
}

// Render simple input field
fn ui(f: &mut Frame, app: &mut App) {
    let selection = Selection::new()
        .title("Mode".to_string())
        .bordered(true)
        .margins(Margin {
            vertical: 0,
            horizontal: 1,
        })
        .max_lines(8)
        .style(Style {
            focused: ratatui::prelude::Style::default().fg(tailwind::INDIGO.c400),
            cursor: ratatui::prelude::Style::default()
                .bg(tailwind::INDIGO.c400)
                .fg(tailwind::WHITE),
            ..Style::default()
        });

    f.render_stateful_widget(selection, f.area(), &mut app.state);
}

fn main() {
    let mut screen: AlternateScreen<Stdout> =
        AlternateScreen::new().expect("Failed to create alternate screen.");

    // Create app state
    let mut app = App::default();
    app.state.set_values(vec![
        Item::new("Value 0".to_string(), 0),
        Item::new("Value 1".to_string(), 1),
        Item::new("Value 2".to_string(), 2),
        Item::new("Value 3".to_string(), 3),
        Item::new("Value 4".to_string(), 4),
        Item::new("Value 5".to_string(), 5),
        Item::new("Value 6".to_string(), 6),
    ]);

    loop {
        // Draw app
        screen.draw(|f| ui(f, &mut app)).unwrap();

        // Check for events
        if event::poll(Duration::from_millis(50)).unwrap() {
            if let Event::Key(key) = event::read().unwrap() {
                if key.kind == KeyEventKind::Press {
                    if let KeyCode::Esc = key.code {
                        break;
                    } else if let KeyCode::Char('d') = key.code {
                        app.state.set_focus(!app.state.in_focus());
                    } else if app.state.in_focus() {
                        let event_result: EventResult =
                            app.state.handle_events(key.modifiers, key.code);
                        if let EventResult::Unhandled(_, KeyCode::Enter) = event_result {
                            break;
                        }
                    }
                }
            }
        }
    }

    drop(screen);
    println!("Selection: {:?}", app.state.get_selection());
}
