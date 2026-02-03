use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{Frame, layout::Margin, style::palette::tailwind};
use std::{io::Stdout, time::Duration};
use ui::{
    AlternateScreen, Style, state::InputFieldState, traits::HandleEvents, widget::InputField,
};

// Simple app consisting of single input field
struct App {
    pub state: InputFieldState,
}

impl Default for App {
    fn default() -> Self {
        Self {
            state: InputFieldState::default().focus(),
        }
    }
}

// Render simple input field
fn ui(f: &mut Frame, app: &mut App) {
    let input = InputField::new()
        .title("Name".to_string())
        .bordered(true)
        .margins(Margin {
            vertical: 0,
            horizontal: 1,
        })
        .style(Style {
            focused: ratatui::prelude::Style::default(),
            cursor: ratatui::prelude::Style::default()
                .bg(tailwind::BLUE.c400)
                .fg(tailwind::WHITE),
            ..Style::default()
        });

    f.render_stateful_widget(input, f.area(), &mut app.state);
}

fn main() {
    let mut screen: AlternateScreen<Stdout> =
        AlternateScreen::new().expect("Failed to create alternate screen.");

    // Create app state
    let mut app = App::default();

    loop {
        // Draw app
        screen.draw(|f| ui(f, &mut app)).unwrap();

        // Check for events
        if event::poll(Duration::from_millis(50)).unwrap() {
            if let Event::Key(key) = event::read().unwrap() {
                if key.kind == KeyEventKind::Press {
                    if let KeyCode::Esc = key.code {
                        break;
                    } else {
                        app.state.handle_events(key.modifiers, key.code);
                    }
                }
            }
        }
    }
}
