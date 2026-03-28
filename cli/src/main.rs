#![feature(async_fn_traits)]

mod instance;
mod module;

use clap::Parser;
use tokio::runtime::Runtime;
use util::Expect;

use crate::module::Definition;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct CliArgs {}

async fn run() {
    let config = net::tcp::Config {
        ip: "127.0.0.1".to_string(),
        port: 8080,
        timeout_ms: 3000,
        delay_ms: 1000,
        interval_ms: 1000,
    };

    let mut module1 = module::Module::new(
        net::Key::create(1),
        module::Config {
            client: false,
            config: net::Config::Tcp(config.clone()),
            definitions: vec![Definition {
                address: 0,
                length: 10,
            }],
        },
    );
    module1.start().await.panic(|e| format!("{}", e));

    let mut module2 = module::Module::new(
        net::Key::create(1),
        module::Config {
            client: true,
            config: net::Config::Tcp(config.clone()),
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
    let _ = CliArgs::parse();

    // Initialize tokio runtime for modbus server
    let runtime = Runtime::new().panic(|e| format!("Failed to create runtime. [{}]", e));
    runtime.block_on(async {
        run().await;
    });
}
