use crate::instance::Instance;
use crate::instance::config::ClientConfig;
use crate::instance::config::ServerConfig;

use log::Log;
use memory::Memory;
use memory::Range;
use memory::Type;
use std::marker::PhantomData;
use std::sync::Arc;
use tokio::sync::RwLock;

type Key = u8;

const MAX_LINE_LENGTH: usize = 256;
const LOG_SIZE: usize = 80;

pub struct Definition {
    pub address: usize,
    pub length: usize,
}

pub struct Config {
    pub client: bool,
    pub config: net::Config,
    pub definitions: Vec<Definition>,
}

pub struct Module {
    instance: Instance<Key>,
    log: Arc<RwLock<Log<MAX_LINE_LENGTH, LOG_SIZE>>>,
}

impl Module {
    pub fn new(key: net::Key<Key>, config: Config) -> Self {
        let mut memory = Memory::default();

        for def in &config.definitions {
            let range = Range::new(def.address, def.length);
            memory.add_ranges(
                key.clone(),
                &memory::Kind::Combined(Type::Register),
                &[range],
            );
        }

        let memory = Arc::new(RwLock::new(memory));

        let operations = config
            .definitions
            .iter()
            .map(|d| {
                let range = Range::new(d.address, d.length);
                net::Operation {
                    slave_id: key.slave_id,
                    fn_code: net::FunctionCode::ReadInputRegisters,
                    range,
                }
            })
            .collect();
        let operations = Arc::new(RwLock::new(operations));

        let log = Arc::new(RwLock::new(Log::<MAX_LINE_LENGTH, LOG_SIZE>::init()));

        let instance = match config.config {
            net::Config::Tcp(cfg) => {
                let cfg = Arc::new(RwLock::new(cfg));

                if config.client {
                    let cfg = ClientConfig {
                        id: Key::default(),
                        config: cfg.clone(),
                        operations,
                        memory: memory.clone(),
                    };
                    Instance::<Key>::with_tcp_client(cfg)
                } else {
                    let cfg = ServerConfig {
                        id: Key::default(),
                        config: cfg.clone(),
                        memory: memory.clone(),
                    };
                    Instance::<Key>::with_tcp_server(cfg)
                }
            }
            net::Config::Rtu(cfg) => {
                let cfg = Arc::new(RwLock::new(cfg));

                if config.client {
                    let cfg = ClientConfig {
                        id: Key::default(),
                        config: cfg.clone(),
                        operations,
                        memory: memory.clone(),
                    };
                    Instance::<Key>::with_rtu_client(cfg)
                } else {
                    let cfg = ServerConfig {
                        id: Key::default(),
                        config: cfg.clone(),
                        memory: memory.clone(),
                    };
                    Instance::<Key>::with_rtu_server(cfg)
                }
            }
        };

        Self { instance, log }
    }

    pub async fn start(&mut self) -> Result<(), crate::instance::error::Error> {
        let log = self.log.clone();
        self.instance
            .start(
                async move |s| {
                    log.write().await.write(&s);
                },
                async |s| println!("STATUS: {}", s),
            )
            .await
    }

    pub async fn stop(&mut self) -> Result<(), crate::instance::error::Error> {
        self.instance.stop().await
    }

    pub async fn print_log(&mut self) {
        let mut log = self.log.write().await;
        while let Some(msg) = log.take() {
            println!("{}", msg);
        }
    }
}
