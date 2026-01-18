use crate::mem::Config as MemoryConfig;
use crate::net::Config as NetConfig;
use crate::register::Definition;
use crate::util::str;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::default::Default;
use std::fs::File;
use std::io::BufReader;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum FileType {
    Toml,
    Json,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct UiConfig {
    pub history_length: usize,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { history_length: 50 }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Config {
    pub ui: UiConfig,
    pub net: NetConfig,
    pub memory: MemoryConfig<u8>,
    pub definitions: HashMap<String, Definition>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ui: UiConfig::default(),
            net: NetConfig::default(),
            memory: MemoryConfig::default(),
            definitions: HashMap::new(),
        }
    }
}

impl Config {
    /// Read register configuration from file
    pub fn read(path: &str) -> anyhow::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        if let Ok(c) = serde_json::from_reader(reader) {
            Ok(c)
        } else {
            let content = std::fs::read_to_string(path)?;
            toml::from_str(&content).map_err(|e| e.into())
        }
    }
}
