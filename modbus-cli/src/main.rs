#![feature(async_fn_traits)]

mod dialog;
mod instance;
mod module;

use std::{io::Stdout, time::Duration};

use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use modbus_ui::{AlternateScreen, EventResult, traits::HandleEvents};
use modbus_util::{Expect, tokio::spawn_detach};
use ratatui::layout::{Constraint, Layout, Rect};
use tokio::runtime::Runtime;

use crate::{dialog::EditInputDialog, dialog::EditSelectionDialog, module::Definition};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct CliArgs {}

async fn run() {
    let config = modbus_net::tcp::Config {
        ip: "127.0.0.1".to_string(),
        port: 8080,
        timeout_ms: 3000,
        delay_ms: 1000,
        interval_ms: 1000,
    };

    let mut module1 = module::Module::new(
        modbus_net::Key::create(1),
        module::Config {
            client: false,
            config: modbus_net::Config::Tcp(config.clone()),
            definitions: vec![Definition {
                address: 0,
                length: 10,
            }],
        },
    );

    let mut module2 = module::Module::new(
        modbus_net::Key::create(1),
        module::Config {
            client: true,
            config: modbus_net::Config::Tcp(config.clone()),
            definitions: vec![Definition {
                address: 0,
                length: 10,
            }],
        },
    );

    module1.start().await.panic(|e| format!("{}", e));
    module2.start().await.panic(|e| format!("{}", e));

    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    module1.stop().await.panic(|e| format!("{}", e));
    module2.stop().await.panic(|e| format!("{}", e));
}

fn main() {
    // Initialize tokio runtime for modbus server
    let runtime = Runtime::new().panic(|e| format!("Failed to create runtime. [{}]", e));

    // Spawn modbus modules in the background
    runtime.block_on(async move {
        spawn_detach(async move {
            run().await;
        })
        .await
    });

    let mut screen: AlternateScreen<Stdout> =
        AlternateScreen::new().expect("Failed to create alternate screen.");

    // Release terminal on panic to show error messages
    let handler = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        AlternateScreen::<Stdout>::release();
        handler(panic);
    }));

    let mut edit_input_dialog = EditInputDialog::new();
    let mut edit_selection_dialog = EditSelectionDialog::new(vec!["Dog", "Cat", "Horse"]);

    loop {
        // Draw app
        if let Err(e) = screen.draw(|f| {
            let layout: [Rect; 2] =
                Layout::horizontal([Constraint::Min(1), Constraint::Min(1)]).areas(f.area());
            edit_selection_dialog.render(layout[0], f.buffer_mut());
            edit_input_dialog.render(layout[1], f.buffer_mut());
        }) {
            drop(screen);
            eprintln!("Failed to draw screen. [{}]", e);
            return;
        }

        // Check for events
        if event::poll(Duration::from_millis(50)).unwrap() {
            if let Event::Key(key) = event::read().unwrap() {
                if key.kind == KeyEventKind::Press {
                    if let KeyCode::Esc = key.code {
                        break;
                    } else {
                        let event_result: EventResult =
                            edit_input_dialog.handle_events(key.modifiers, key.code);
                        match event_result {
                            EventResult::Unhandled(_, KeyCode::Enter) => {
                                break;
                            }
                            EventResult::Unhandled(KeyModifiers::SHIFT, KeyCode::BackTab)
                            | EventResult::Unhandled(KeyModifiers::SHIFT, KeyCode::Tab) => {
                                edit_input_dialog.focus_previous();
                            }
                            EventResult::Unhandled(_, KeyCode::Tab) => {
                                edit_input_dialog.focus_next();
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}
