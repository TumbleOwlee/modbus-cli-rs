use derive_builder::Builder;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{Frame, layout::Constraint};
use std::{io::Stdout, time::Duration};
use ui::{
    AlternateScreen, EventResult,
    state::{TableState, TableStateBuilder},
    traits::HandleEvents,
    widgets::{Table, TableBuilder},
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

// Simple app consisting of single input field
#[derive(Builder, Debug)]
struct App {
    pub state: TableState<Item, 2>,
}

// Render simple input field
fn ui(f: &mut Frame, app: &mut App) {
    let table: Table<Item, 2> = TableBuilder::default()
        .header(["Name".to_string(), "Value".to_string()])
        .build()
        .unwrap();
    //f.render_stateful_widget(table, f.area(), &mut app.state);
}

fn main() {
    let mut screen: AlternateScreen<Stdout> =
        AlternateScreen::new().expect("Failed to create alternate screen.");

    // Create app state
    let mut app = AppBuilder::default()
        .state(
            TableStateBuilder::default()
                .values(vec![
                    Item::new("Value 0".to_string(), 0),
                    Item::new("Value 1".to_string(), 1),
                    Item::new("Value 2".to_string(), 2),
                    Item::new("Value 3".to_string(), 3),
                    Item::new("Value 4".to_string(), 4),
                    Item::new("Value 5".to_string(), 5),
                    Item::new("Value 6".to_string(), 6),
                ])
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();

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
                        app.state.set_focused(!app.state.focused());
                    } else if app.state.focused() {
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
    let value = app.state.values().get(app.state.selection()).unwrap();
    println!(
        "Selection: {{ name: {}, value: {} }}",
        value.name, value.value
    );
}
