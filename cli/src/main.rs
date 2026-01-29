#![feature(async_fn_traits)]

mod instance;

use instance::{ClientConfig, Instance, ServerConfig};
use log::Log;
use memory::{Memory, Range, Type};
use net::{Command, Operation};
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::RwLock;
use util::Expect;

use clap::Parser;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct CliArgs {}

const MAX_LINE_LENGTH: usize = 256;
const LOG_SIZE: usize = 80;

fn main() {
    let args = CliArgs::parse();

    let range = Range::new(0, 10);

    let key = net::Key::create(1);

    let mut memory = Memory::default();
    memory.add_ranges(key, &memory::Kind::Combined(Type::Register), &[range]);

    let memory = Arc::new(RwLock::new(memory));

    let config = net::tcp::Config {
        ip: "127.0.0.1".to_string(),
        port: 8080,
        timeout_ms: 3000,
        delay_ms: 1000,
        interval_ms: 2000,
    };
    let config = Arc::new(RwLock::new(config));

    let srv_cfg = ServerConfig {
        id: 0,
        config: config.clone(),
        memory: memory.clone(),
    };

    let mut instance1 = Instance::with_tcp_server(srv_cfg);

    let operations = Arc::new(RwLock::new(vec![Operation {
        slave_id: net::SlaveId::from(1),
        fn_code: net::FunctionCode::ReadHoldingRegisters,
        range: Range::new(0, 1),
    }]));

    let clt_cfg = ClientConfig {
        id: 0,
        config,
        memory: memory.clone(),
        operations,
    };
    let mut instance2 = Instance::with_tcp_client(clt_cfg);

    // Initialize tokio runtime for modbus server
    let runtime = Runtime::new().panic(|e| format!("Failed to create runtime. [{}]", e));

    let rw_log = Arc::new(RwLock::new(Log::<MAX_LINE_LENGTH, LOG_SIZE>::init()));

    runtime
        .block_on(async {
            let rw_log = rw_log.clone();
            let log = async move |s: String| {
                rw_log.write().await.write(&s);
            };
            let status = async |s| println!("STATUS: {}", s);
            instance1.start(log, status).await
        })
        .panic(|e| format!("Failed to start instance. [{}]", e));

    runtime
        .block_on(async {
            let rw_log = rw_log.clone();
            let log = async move |s: String| {
                rw_log.write().await.write(&s);
            };
            let status = async |s| println!("STATUS: {}", s);
            instance2.start(log, status).await
        })
        .panic(|e| format!("Failed to start instance. [{}]", e));

    runtime.block_on(async {
        instance2
            .send_command(Command::WriteSingleRegister(net::SlaveId::from(1), 0, 1234))
            .await;
        instance2
            .send_command(Command::WriteSingleRegister(net::SlaveId::from(2), 0, 1234))
            .await;
    });

    runtime.block_on(async {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        while let Some(msg) = rw_log.write().await.take() {
            println!("LOG: {}", msg);
        }
        let mem = memory.read().await;
        let values = mem.read(
            net::Key::create(1),
            &memory::Type::Register,
            &Range::new(0, 1),
        );
        println!("Read values: {:?}", values);
    });

    runtime
        .block_on(async move { instance1.stop().await })
        .panic(|e| format!("Failed to stop instance. [{}]", e));
    println!("Instance 1 stopped successfully.");

    runtime
        .block_on(async move { instance2.stop().await })
        .panic(|e| format!("Failed to stop instance. [{}]", e));
    println!("Instance 2 stopped successfully.");
}
