mod cli;
mod config;
mod mem;
mod msg;
mod net;
mod register;
mod sync;
mod ui;
mod util;

use lua::module::{RegisterModule, ValueType};
use lua::{
    module::StaticsModule, module::TimeModule, Context as LuaContext,
    ContextBuilder as LuaContextBuilder,
};

use crate::cli::ArgParser;
use crate::cli::Commands;
use crate::config::Config;
use crate::config::FileType;
use crate::mem::memory::Memory;
use crate::msg::{Command, LogMsg, Status};
use crate::net::rtu::client::Client as RtuClient;
use crate::net::rtu::server::Server as RtuServer;
use crate::net::tcp::client::Client as TcpClient;
use crate::net::tcp::server::Server as TcpServer;
use crate::register::{Definition, Handler, Value};
use crate::sync::channel::DuplexChannelPair;
use crate::ui::App;
use crate::util::tokio::spawn_detach;
use crate::util::{async_cloned, convert::Converter, str, Expect};

use anyhow::anyhow;
use clap::Parser;
use itertools::Itertools;
use std::collections::HashMap;
use std::default::Default;
use std::fs::File;
use std::io::Write;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::channel;
use tokio_modbus::prelude::SlaveId;

fn run() -> Result<(), anyhow::Error> {
    // Parse all arguments
    let args = ArgParser::parse();

    // Read configuration
    let config = Arc::new(if let Some(ref path) = args.config {
        Config::read(path)?
    } else {
        Config::default()
    });

    let mut memory = Memory::<SlaveId>::default();
    for (name, def) in config.definitions.iter() {
        memory.add_ranges(def.get_slave_id().unwrap_or(0), &[def.get_range()]);
    }

    let status_channel_pair = DuplexChannelPair::new(10);
    let logger_channel_pair = DuplexChannelPair::new(10);
    let status = channel::<Status>(10);
    let logger = channel::<LogMsg>(10);
    let commands = channel::<Command>(10);

    // Initialize tokio runtime for modbus server
    let runtime = Runtime::new().panic(|e| format!("Failed to create runtime. [{}]", e));

    let lua_data = HashMap::new();
    lua_data.insert(str!("IsClient"), ValueType::Bool(args.client));

    let lua: LuaContext<String> = LuaContextBuilder::default()
        .with_stdlib()
        .with_module(TimeModule::default())
        .with_module(StaticsModule::from(lua_data))
        .with_module(RegisterModule::init(handle))
        .build()
        .expect("Lua Runtime startup failed");

    match args.command {
        Commands::Convert(ref format) => match args.config {
            Some(ref src) => {
                let (src_ty, dest_ty, dest) = match src {
                    _ if src.ends_with(".json") => {
                        (FileType::Json, FileType::Toml, src.to_owned() + ".toml")
                    }
                    _ if src.ends_with(".toml") => {
                        (FileType::Toml, FileType::Json, src.to_owned() + ".json")
                    }
                    _ => panic!("Invalid file types"),
                };
                Converter::convert(src, src_ty, &dest, dest_ty);
                return Ok(());
            }
            None => return Err(anyhow!("No configuration file specified")),
        },
        Commands::Tcp(ref tcp_config) => match args.client {
            true => {
                runtime.block_on(async move {
                    spawn_detach(async move {
                        let mut client = TcpClient::new(
                            &config, tcp_config, memory, status.0, commands.1, logger.0,
                        );
                        client
                            .run(
                                config.net.delay_after_connect_ms,
                                config.net.interval_ms,
                                config.net.timeout_ms,
                            )
                            .await
                    })
                    .await
                });
            }
            false => {
                //runtime.block_on(async_cloned!(memory; {
                //    spawn_detach(async move {
                //        let server = TcpServer::new(&config, memory, status_sender, log_sender);
                //        server.run().await
                //    })
                //    .await
                //}));
            }
        },
        Commands::Rtu(ref rtu_config) => match args.client {
            true => {
                runtime.block_on(async move {
                    spawn_detach(async move {
                        let mut client = RtuClient::new(rtu_config, config.net.clone(), operations);
                        client.attach_ui(ui);
                        client.run().await
                    })
                    .await
                });
            }
            false => {
                runtime.block_on(async_cloned!(memory; {
                    spawn_detach(async move {
                        let server = RtuServer::new(&config, memory, status_sender, log_sender);
                        server.run().await
                    })
                    .await
                }));
            }
        },
    };

    if args.client {
        match args.command {
            Commands::Tcp(config) => {
                runtime.block_on(async_cloned!(interval_ms, app_config, memory; {
                    spawn_detach(async move {
                        let mut client = TcpClient::new(app_config, config, memory, status_sender, cmd_receiver, log_sender);
                        client.run(delay_after_connect_ms, interval_ms, timeout_ms).await
                    })
                    .await
                }));
            }
            Commands::Rtu(config) => {
                runtime.block_on(async_cloned!(interval_ms, app_config, memory; {
                    spawn_detach(async move {
                        let mut client = RtuClient::new(app_config, config, memory, status_sender, cmd_receiver, log_sender);
                        client.run(delay_after_connect_ms, interval_ms, timeout_ms).await
                    })
                    .await
                }));
            }
            _ => {}
        }
    } else {
        match args.command {
            Commands::Tcp(config) => {
                runtime.block_on(async_cloned!(memory; {
                    spawn_detach(async move {
                        let server = TcpServer::new(config, memory, status_sender, log_sender);
                        server.run().await
                    })
                    .await
                }));
            }
            Commands::Rtu(config) => {
                runtime.block_on(async_cloned!(memory; {
                    spawn_detach(async move {
                        let server = RtuServer::new(config, memory, status_sender, log_sender);
                        server.run().await
                    })
                    .await
                }));
            }
            Commands::Convert(format) => {
                if let Some(ref path) = cfg_path {
                    let idx = path.chars().rev().find_position(|c| *c == '.');
                    let mut path = path.clone();
                    if let Some((idx, _)) = idx {
                        let _ = path.split_off(path.len() - idx - 1);
                    }
                    match format.file_type {
                        FileType::Toml => {
                            let content = toml::to_string::<AppConfig>(
                                &(*app_config
                                    .lock()
                                    .expect("Failed to serialize configuration")),
                            )
                            .expect("Failed to serialize configuration to toml");
                            let mut file = File::create(path.to_owned() + ".toml")
                                .expect("Failed to open output file");
                            write!(file, "{}", content).expect("Failed to write file");
                        }
                        FileType::Json => {
                            let content = serde_json::to_string_pretty::<AppConfig>(
                                &(*app_config
                                    .lock()
                                    .expect("Failed to serialize configuration")),
                            )
                            .expect("Failed to serialize configuration to json");
                            let mut file = File::create(path.to_owned() + ".json")
                                .expect("Failed to open output file");
                            write!(file, "{}", content).expect("Failed to write file");
                        }
                    }
                }
                return;
            }
        }
    };

    // Initialize register handler
    let register_handler = Handler::new(app_config.clone(), memory.clone());
    {
        for def in app_config.lock().unwrap().definitions.values() {
            if let Some(value) = def.get_default() {
                let s: String = match value {
                    Value::Str(v) => v.to_string(),
                    Value::Num(v) => format!("{}", v),
                    Value::Float(v) => format!("{}", v),
                };
                if let Ok(v) = def.get_type().encode(&s) {
                    if memory
                        .lock()
                        .unwrap()
                        .write(def.get_slave_id().unwrap_or(0), def.get_range(), &v)
                        .is_err()
                    {}
                }
            }
        }
    }

    // Run UI
    let app = App::new(register_handler, app_config);
    let cmd_sender = if args.client { Some(cmd_sender) } else { None };
    app.run(status_receiver, log_receiver, cmd_sender, lua_runtime)
        .panic(|e| format!("Run app failed [{}]", e));
    //runtime.block_on(async { crate::util::tokio::join_all().await });
}

fn main() {
    if let Err(e) = run() {
        panic!("Error: {}", e);
    }
}
