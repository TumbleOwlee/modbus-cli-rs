mod instance;

use instance::{Instance, ServerConfig};
use log::Log;
use memory::Memory;
use std::sync::{Arc, RwLock};
use tokio::runtime::Runtime;
use util::Expect;

use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct CliArgs {}

const LEN: usize = 80;
const SIZE: usize = 256;

fn main() {
    let args = CliArgs::parse();

    let memory = Memory::default();
    let memory = Arc::new(RwLock::new(memory));

    let config = net::tcp::Config {
        ip: "127.0.0.1".to_string(),
        port: 8080,
        timeout_ms: 3000,
        delay_ms: 1000,
        interval_ms: 500,
    };
    let config = Arc::new(RwLock::new(config));

    let srv_cfg = ServerConfig {
        id: 10,
        config,
        memory: memory.clone(),
    };

    let mut instance1 = Instance::with_tcp_server(srv_cfg);

    let config = net::tcp::Config {
        ip: "127.0.0.1".to_string(),
        port: 8081,
        timeout_ms: 3000,
        delay_ms: 1000,
        interval_ms: 500,
    };
    let config = Arc::new(RwLock::new(config));

    let srv_cfg = ServerConfig {
        id: 10,
        config,
        memory,
    };
    let mut instance2 = Instance::with_tcp_server(srv_cfg);

    // Initialize tokio runtime for modbus server
    let runtime = Runtime::new().panic(|e| format!("Failed to create runtime. [{}]", e));

    let rw_log = Arc::new(RwLock::new(Log::<LEN, SIZE>::init()));

    runtime
        .block_on(async {
            let rw_log = rw_log.clone();
            let log = move |s: String| {
                let mut log = rw_log.write().unwrap();
                log.write(&s);
            };
            let status = |s| println!("STATUS: {}", s);
            instance1.start(log, status).await
        })
        .panic(|e| format!("Failed to start instance. [{}]", e));

    runtime
        .block_on(async {
            let log = move |s: String| {
                let mut log = rw_log.write().unwrap();
                log.write(&s);
            };
            let status = |s| println!("STATUS: {}", s);
            instance2.start(log, status).await
        })
        .panic(|e| format!("Failed to start instance. [{}]", e));

    runtime
        .block_on(async move { instance1.stop().await })
        .panic(|e| format!("Failed to stop instance. [{}]", e));

    runtime
        .block_on(async move { instance2.stop().await })
        .panic(|e| format!("Failed to stop instance. [{}]", e));
}
