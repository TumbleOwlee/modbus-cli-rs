// Example binary: unwrap keeps the demo focused on the widget being shown.
#![allow(clippy::unwrap_used)]

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ferrowl_ui::{
    AlternateScreen,
    state::{CommandLineOutcome, CommandLineState, CommandLineStateBuilder},
    widgets::{CommandLine, CommandLineBuilder},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    widgets::{Paragraph, Widget},
};
use std::{io::Stdout, time::Duration};

struct App {
    command: CommandLine,
    state: CommandLineState,
}

impl Default for App {
    fn default() -> Self {
        let mut state = CommandLineStateBuilder::default().build().unwrap();
        state.set_hint("press : to open the command line, q to quit".to_string());
        let command = CommandLineBuilder::default()
            .help(vec![
                (":q".to_string(), "quit".to_string()),
                (":e".to_string(), "show an error".to_string()),
            ])
            .build()
            .unwrap();
        Self { command, state }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let [filler, command_row] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
    Paragraph::new("command-line demo").render(filler, f.buffer_mut());
    f.render_stateful_widget(&app.command, command_row, &mut app.state);
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
            if !app.state.is_open() {
                match (key.modifiers, key.code) {
                    (KeyModifiers::NONE, KeyCode::Char('q')) => break,
                    (KeyModifiers::NONE, KeyCode::Char(':')) => app.state.open(),
                    _ => {}
                }
                continue;
            }

            match app.state.handle_key(key.modifiers, key.code) {
                Some(CommandLineOutcome::Submit(text)) => {
                    if text == "e" {
                        app.state.set_error(Some("unknown command".to_string()));
                    } else {
                        app.state.set_notice(Some(format!("submitted: {text}")));
                    }
                }
                Some(CommandLineOutcome::Cancel | CommandLineOutcome::Consumed) | None => {}
            }
        }
    }
}
