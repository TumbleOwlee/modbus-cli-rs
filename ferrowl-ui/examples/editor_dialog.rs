// Example binary: unwrap keeps the demo focused on the widget being shown.
#![allow(clippy::unwrap_used)]

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ferrowl_ui::{
    AlternateScreen,
    state::{EditorDialogOutcome, EditorDialogState},
    widgets::{EditorDialog, EditorDialogBuilder},
};
use ratatui::{Frame, widgets::Paragraph};
use std::{io::Stdout, time::Duration};

struct App {
    dialog: EditorDialog,
    state: EditorDialogState,
    last_confirmed: String,
}

impl Default for App {
    fn default() -> Self {
        let dialog = EditorDialogBuilder::default()
            .title("Notes".to_string())
            .build()
            .unwrap();
        Self {
            dialog,
            state: EditorDialogState::default(),
            last_confirmed: String::new(),
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let background = format!(
        "editor-dialog demo\ne to open, q to quit\nlast confirmed: {}",
        app.last_confirmed
    );
    f.render_widget(Paragraph::new(background), f.area());
    if app.state.is_open() {
        f.render_stateful_widget(&app.dialog, f.area(), &mut app.state);
    }
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
                    (KeyModifiers::NONE, KeyCode::Char('e')) => app.state.open(),
                    _ => {}
                }
                continue;
            }

            if let Some(EditorDialogOutcome::Confirmed(text)) =
                app.state.handle_key(key.modifiers, key.code)
            {
                app.last_confirmed = text;
            }
        }
    }
}
