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
        .title(Some("Name".to_string()))
        .border(true)
        .multiline(true)
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
        })
        .build()
        .unwrap();

    let horizontal_layout: [Rect; 3] =
        Layout::horizontal([Constraint::Min(1), Constraint::Max(70), Constraint::Min(1)])
            .areas(f.area());
    let vertical_layout: [Rect; 5] = Layout::vertical([
        Constraint::Min(1),
        input.vertical(),
        Constraint::Length(5),
        input.vertical(),
        Constraint::Min(1),
    ])
    .areas(horizontal_layout[1]);

    f.render_stateful_widget(input.clone(), vertical_layout[1], &mut app.states[0]);

    let layout = Layout::horizontal([input.horizontal(), input.horizontal(), input.horizontal()]);
    let rects2: [Rect; 3] = vertical_layout[2].layout(&layout);
    f.render_stateful_widget(input.clone(), rects2[0], &mut app.states[1]);
    f.render_stateful_widget(input.clone(), rects2[1], &mut app.states[2]);
    f.render_stateful_widget(input.clone(), rects2[2], &mut app.states[3]);

    let layout = Layout::horizontal([input.horizontal(), input.horizontal()]);
    let rects3: [Rect; 2] = vertical_layout[3].layout(&layout);
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
