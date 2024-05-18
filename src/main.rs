mod memory;
mod modbus;
mod register;
mod test;
mod tokio;
mod util;

use crate::memory::{Memory, Range};
use crate::modbus::Server;
use crate::register::{Definition, RegisterHandler};
use crate::util::{str, Expect};

use clap::Parser;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::runtime::Runtime;
use tokio::{join_all, spawn_detach};
use tokio_modbus::server::tcp::{accept_tcp_connection, Server as TcpServer};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Configuration to load
    config: String,

    /// Verbose
    #[arg(short, long, default_value_t = false)]
    verbose: bool,

    /// Either interface to use for listener or target address
    #[arg(short, long, default_value_t = str!("127.0.0.1"))]
    ip: String,

    // Either local port for listening or target port
    #[arg(short, long, default_value_t = 502)]
    port: u16,
}

fn main() {
    let args = Args::parse();

    // Read register definitions
    let definitions =
        read_config(&args.config).panic(|e| format!("Failed to read configuration file. [{}]", e));

    // Initialize memory storage for all registers
    let memory = Arc::new(Mutex::new(Memory::<1024, u16>::new(Range::new(
        definitions.iter().fold(0xFFFFu16, |min, (_, def)| {
            std::cmp::min(min, def.get_address())
        }),
        definitions.iter().fold(0x0000u16, |max, (_, def)| {
            std::cmp::min(max, def.get_address())
        }) + 1,
    ))));

    // Initialize register handler
    let mut register_handler = RegisterHandler::new(&definitions, memory.clone());

    // Initialize tokio runtime for modbus server
    let runtime = Runtime::new().expect("Failed to create runtime.");
    runtime.block_on(async move {
        spawn_detach(async move { run_server(args.ip, args.port, memory).await }).await
    });

    // Update register values from memory
    let _ = register_handler.update();

    // Block until all jobs are done
    runtime.block_on(async { join_all().await });
}

/// Read register configuration from file
fn read_config(path: &str) -> anyhow::Result<HashMap<String, Definition>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader).map_err(|e| e.into())
}

/// Run modbus server to provide read and write operations
async fn run_server(
    ip: String,
    port: u16,
    memory: Arc<Mutex<Memory<1024, u16>>>,
) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", ip, port).parse()?;
    let listener = TcpListener::bind(addr)
        .await
        .panic(|e| format!("Failed to bind to address {}:{} [{}]", ip, port, e));
    let server = TcpServer::new(listener);
    let new_service = |_socket_addr| Ok(Some(Server::new(memory.clone())));
    let on_connected = |stream, socket_addr| async move {
        accept_tcp_connection(stream, socket_addr, new_service)
    };
    let on_process_error = |err| {
        eprintln!("{err}");
    };
    server
        .serve(&on_connected, on_process_error)
        .await
        .panic(|e| format!("Serve server failed [{}]", e));
    Ok(())
}
