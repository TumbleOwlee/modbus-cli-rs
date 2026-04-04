use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::palette::tailwind,
};
use std::{io::Stdout, time::Duration};
use ui::{
    AlternateScreen, EventResult,
    state::{InputFieldState, InputFieldStateBuilder},
    style::InputFieldStyle,
    traits::HandleEvents,
    widgets::{InputField, InputFieldBuilder},
};

use ui::traits::AsConstraint;

// Simple app consisting of single input field
struct App {
    index: usize,
    states: Vec<InputFieldState>,
}

impl Default for App {
    fn default() -> Self {
        let mut states = vec![
            InputFieldStateBuilder::default()
                .focused(false)
                .build()
                .unwrap();
            4
        ];
        states[0].set_focused(true);
        Self { index: 0, states }
    }
}

// Render simple input field
fn ui(f: &mut Frame, app: &mut App) {
    let input: InputField<String> = InputFieldBuilder::default()
        .title(Some("Input".to_string()))
        .border(true)
        .multiline(false)
        .margin(Margin {
            vertical: 0,
            horizontal: 1,
        })
        .style(InputFieldStyle {
            focused: ratatui::prelude::Style::default().fg(tailwind::INDIGO.c400),
            cursor: ratatui::prelude::Style::default()
                .bg(tailwind::INDIGO.c400)
                .fg(tailwind::WHITE),
            ..InputFieldStyle::default()
        })
        .overflow(false)
        .build()
        .unwrap();
    let input_overflow: InputField<String> = InputFieldBuilder::default()
        .title(Some("Input (O)".to_string()))
        .border(true)
        .multiline(false)
        .margin(Margin {
            vertical: 0,
            horizontal: 1,
        })
        .style(InputFieldStyle {
            focused: ratatui::prelude::Style::default().fg(tailwind::INDIGO.c400),
            cursor: ratatui::prelude::Style::default()
                .bg(tailwind::INDIGO.c400)
                .fg(tailwind::WHITE),
            ..InputFieldStyle::default()
        })
        .overflow(true)
        .build()
        .unwrap();

    let input_multiline: InputField<String> = InputFieldBuilder::default()
        .title(Some("Input (M)".to_string()))
        .border(true)
        .multiline(true)
        .margin(Margin {
            vertical: 0,
            horizontal: 1,
        })
        .style(InputFieldStyle {
            focused: ratatui::prelude::Style::default().fg(tailwind::INDIGO.c400),
            cursor: ratatui::prelude::Style::default()
                .bg(tailwind::INDIGO.c400)
                .fg(tailwind::WHITE),
            ..InputFieldStyle::default()
        })
        .overflow(false)
        .build()
        .unwrap();

    let input_multiline_overflow: InputField<String> = InputFieldBuilder::default()
        .title(Some("Input (M+O)".to_string()))
        .border(true)
        .multiline(true)
        .margin(Margin {
            vertical: 0,
            horizontal: 1,
        })
        .style(InputFieldStyle {
            focused: ratatui::prelude::Style::default().fg(tailwind::INDIGO.c400),
            cursor: ratatui::prelude::Style::default()
                .bg(tailwind::INDIGO.c400)
                .fg(tailwind::WHITE),
            ..InputFieldStyle::default()
        })
        .overflow(true)
        .build()
        .unwrap();

    let horizontal_layout: [Rect; 4] = Layout::horizontal([
        input.horizontal(&app.states[0], None),
        input_overflow.horizontal(&app.states[1], None),
        input_multiline.horizontal(&app.states[2], None),
        input_multiline_overflow.horizontal(&app.states[3], None),
    ])
    .areas(f.area());

    let inputs = [
        input,
        input_overflow,
        input_multiline,
        input_multiline_overflow,
    ];
    for i in 0..4 {
        let vertical_layout: [Rect; 2] = Layout::vertical([
            inputs[i].vertical(&app.states[i], Some(horizontal_layout[i].width)),
            Constraint::Min(1),
        ])
        .areas(horizontal_layout[i]);
        f.render_stateful_widget(&inputs[i], vertical_layout[0], &mut app.states[i]);
    }
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
                        let event_result =
                            app.states[app.index].handle_events(key.modifiers, key.code);
                        match event_result {
                            EventResult::Unhandled(_, KeyCode::Enter) => {
                                break;
                            }
                            EventResult::Unhandled(KeyModifiers::SHIFT, KeyCode::BackTab)
                            | EventResult::Unhandled(KeyModifiers::SHIFT, KeyCode::Tab) => {
                                app.states[app.index].set_focused(false);
                                app.index = (app.index + 3) % 4;
                                app.states[app.index].set_focused(true);
                            }
                            EventResult::Unhandled(_, KeyCode::Tab) => {
                                app.states[app.index].set_focused(false);
                                app.index = (app.index + 1) % 4;
                                app.states[app.index].set_focused(true);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    drop(screen);

    for state in app.states {
        println!("Input: {:?}", state.input());
    }
}
