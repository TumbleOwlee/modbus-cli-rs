#![feature(async_fn_traits)]

mod dialog;
mod instance;
mod module;

use std::{io::Stdout, time::Duration};

use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use modbus_ui::{AlternateScreen, EventResult, traits::HandleEvents};
use modbus_util::Expect;
use tokio::runtime::Runtime;

use crate::{dialog::EditDialog, module::Definition};

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
    module1.start().await.panic(|e| format!("{}", e));

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
    module2.start().await.panic(|e| format!("{}", e));

    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    module1.stop().await.panic(|e| format!("{}", e));
    module2.stop().await.panic(|e| format!("{}", e));

    println!("Module 1 Log:");
    module1.print_log().await;
    println!("Module 2 Log:");
    module2.print_log().await;
}

fn main() {
    //    let _ = CliArgs::parse();
    //
    //    // Initialize tokio runtime for modbus server
    //    let runtime = Runtime::new().panic(|e| format!("Failed to create runtime. [{}]", e));
    //    runtime.block_on(async {
    //        run().await;
    //    });

    let mut edit_dialog = EditDialog::new();

    let mut screen: AlternateScreen<Stdout> =
        AlternateScreen::new().expect("Failed to create alternate screen.");

    let mut result = None;

    loop {
        // Draw app
        if let Err(e) = screen.draw(|f| edit_dialog.render(f.area(), f.buffer_mut())) {
            result = Some(e);
        }

        // Check for events
        if event::poll(Duration::from_millis(50)).unwrap() {
            if let Event::Key(key) = event::read().unwrap() {
                if key.kind == KeyEventKind::Press {
                    if let KeyCode::Esc = key.code {
                        break;
                    } else {
                        let event_result: EventResult =
                            edit_dialog.handle_events(key.modifiers, key.code);
                        match event_result {
                            EventResult::Unhandled(_, KeyCode::Enter) => {
                                break;
                            }
                            EventResult::Unhandled(KeyModifiers::SHIFT, KeyCode::BackTab)
                            | EventResult::Unhandled(KeyModifiers::SHIFT, KeyCode::Tab) => {
                                edit_dialog.focus_previous();
                            }
                            EventResult::Unhandled(_, KeyCode::Tab) => {
                                edit_dialog.focus_next();
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    drop(screen);

    if let Some(e) = result {
        println!("{}", e);
    }
}
