use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use derive_builder::Builder;
use modbus_ui::{
    AlternateScreen, EventResult,
    state::{TableState, TableStateBuilder},
    traits::HandleEvents,
    types::Border,
    widgets::{Header, Table, TableBuilder, TableEntry},
};
use ratatui::{Frame, layout::Margin};
use std::{io::Stdout, time::Duration};

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

#[derive(Clone)]
struct ItemHeader {}

impl Header<2> for ItemHeader {
    fn header() -> [String; 2] {
        ["Name".to_string(), "Value".to_string()]
    }

    fn widths() -> [u16; 2] {
        [50, 50]
    }
}

impl TableEntry<2> for Item {
    fn values(&self) -> [String; 2] {
        [self.name.clone(), format!("{}", self.value)]
    }

    fn height(&self) -> u16 {
        self.value as u16
    }
}

// Simple app consisting of single input field
#[derive(Builder, Debug)]
struct App {
    pub state: TableState<Item, 2>,
}

impl App {
    fn render(&mut self, f: &mut Frame) {
        let table: Table<Item, ItemHeader, 2> = TableBuilder::default()
            .margin(Margin::new(0, 0))
            .row_margin(Margin::new(0, 1))
            .border(Border::Full(Margin::new(1, 1)))
            .title(Some("Some Table".to_string()))
            .build()
            .unwrap();
        f.render_stateful_widget(table, f.area(), &mut self.state);
    }
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
                    Item::new("Value 1 Value 2 Value 3 Value 4 Value 5 Value 6 Value 7 Value 8 Value 6 Value 6 Value 6 Value 6 Value 6 Value 6 Value 6 Value 6".to_string(), 6),
                ])
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();

    loop {
        // Draw app
        screen.draw(|f| app.render(f)).unwrap();

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
    let value = app
        .state
        .values()
        .get(app.state.table_state().selected().unwrap_or(0))
        .unwrap();
    println!(
        "Selection: {{ name: {}, value: {} }}",
        value.name, value.value
    );
}
