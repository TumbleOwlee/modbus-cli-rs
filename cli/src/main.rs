mod instance;

use instance::{Instance, ServerConfig};
use memory::Memory;
use std::sync::{Arc, RwLock};
use tokio::runtime::Runtime;
use util::Expect;

use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct CliArgs {}

fn main() {
    let args = CliArgs::parse();

    let config = net::tcp::Config {
        ip: "127.0.0.1".to_string(),
        port: 8080,
        timeout_ms: 3000,
        delay_ms: 1000,
        interval_ms: 500,
    };
    let config = Arc::new(RwLock::new(config));

    let memory = Memory::default();
    let memory = Arc::new(RwLock::new(memory));

    let srv_cfg = ServerConfig {
        id: 10,
        config,
        memory,
    };

    let mut instance = Instance::with_tcp_server(srv_cfg);

    // Initialize tokio runtime for modbus server
    let runtime = Runtime::new().panic(|e| format!("Failed to create runtime. [{}]", e));

    runtime
        .block_on(async { instance.start().await })
        .panic(|e| format!("Failed to start instance. [{}]", e));
    runtime
        .block_on(async move { instance.stop().await })
        .panic(|e| format!("Failed to stop instance. [{}]", e));
}
