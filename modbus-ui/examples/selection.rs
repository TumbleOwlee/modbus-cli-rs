use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use derive_builder::Builder;
use modbus_ui::{
    AlternateScreen, EventResult,
    state::{SelectionState, SelectionStateBuilder},
    style::SelectionStyle,
    traits::{HandleEvents, ToLabel},
    types::Border,
    widgets::SelectionBuilder,
};
use ratatui::{
    Frame, layout::Constraint, layout::Layout, layout::Margin, style::palette::tailwind,
};
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

impl ToLabel for Item {
    fn to_label(&self) -> String {
        self.name.clone()
    }
}

// Simple app consisting of single input field
#[derive(Builder, Debug)]
struct App {
    pub state: SelectionState<Item>,
}

// Render simple input field
fn ui(f: &mut Frame, app: &mut App) {
    let layout = Layout::vertical([Constraint::Length(5)]);
    let rects = f.area().layout_vec(&layout);
    let layout = Layout::horizontal([Constraint::Length(30)]);
    let rects = rects[0].layout_vec(&layout);
    let selection = SelectionBuilder::default()
        .title(Some("Mode".into()))
        .border(Border::Full(Margin::new(1, 1)))
        .margin(Margin {
            vertical: 0,
            horizontal: 1,
        })
        .style(SelectionStyle {
            focused: ratatui::prelude::Style::default()
                .bg(tailwind::INDIGO.c400)
                .fg(tailwind::BLACK),
            border: ratatui::prelude::Style::default().fg(tailwind::INDIGO.c400),
            ..SelectionStyle::default()
        })
        .build()
        .unwrap();

    f.render_stateful_widget(selection, rects[0], &mut app.state);
}

fn main() {
    // Create app state
    let mut app = AppBuilder::default()
        .state(
            SelectionStateBuilder::default()
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

    let mut screen: AlternateScreen<Stdout> =
        AlternateScreen::new().expect("Failed to create alternate screen.");

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
