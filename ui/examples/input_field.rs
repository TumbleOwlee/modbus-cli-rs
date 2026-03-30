use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{Frame, layout::Layout, layout::Margin, layout::Rect, style::palette::tailwind};
use std::{io::Stdout, time::Duration};
use ui::{
    AlternateScreen, EventResult, Transition, state::InputFieldState, style::InputFieldStyle,
    traits::HandleEvents, widgets::InputField,
};

use ui::traits::AsConstraint;

// Simple app consisting of single input field
struct App {
    index: usize,
    states: Vec<InputFieldState>,
}

impl Default for App {
    fn default() -> Self {
        let mut states = vec![InputFieldState::default(); 4];
        Self { index: 0, states }
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
        .style(InputFieldStyle {
            focused: ratatui::prelude::Style::default().fg(tailwind::INDIGO.c400),
            cursor: ratatui::prelude::Style::default()
                .bg(tailwind::INDIGO.c400)
                .fg(tailwind::WHITE),
            ..InputFieldStyle::default()
        });

    let layout = Layout::vertical([input.vertical(), input.vertical(), input.vertical()]);
    let rects: [Rect; 3] = f.area().layout(&layout);
    f.render_stateful_widget(input.clone(), rects[0], &mut app.states[0]);

    let layout = Layout::horizontal([input.horizontal(), input.horizontal(), input.horizontal()]);
    let rects2: [Rect; 3] = rects[1].layout(&layout);
    f.render_stateful_widget(input.clone(), rects2[0], &mut app.states[1]);
    f.render_stateful_widget(input.clone(), rects2[1], &mut app.states[2]);
    f.render_stateful_widget(input.clone(), rects2[2], &mut app.states[3]);

    let layout = Layout::horizontal([input.horizontal(), input.horizontal()]);
    let rects3: [Rect; 2] = rects[2].layout(&layout);
    f.render_stateful_widget(input.clone(), rects3[0], &mut app.states[1]);
    f.render_stateful_widget(input.clone(), rects3[1], &mut app.states[2]);
}

fn main() {
    let mut screen: AlternateScreen<Stdout> =
        AlternateScreen::new().expect("Failed to create alternate screen.");

    // Create app state
    let mut app = App::default();

    loop {
        // Draw app
        screen.draw(|f| ui(f, &mut app)).unwrap();

        app.states[app.index].set_focus();

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
                            EventResult::Transition(Transition::FocusNext) => {
                                app.index = (app.index + 1) % 4;
                            }
                            EventResult::Transition(Transition::FocusPrevious) => {
                                app.index = (app.index + 3) % 4;
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
        println!("Input: {:?}", state.get_input());
    }
}
