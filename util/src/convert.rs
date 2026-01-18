use anyhow::anyhow;
use clap::ValueEnum;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::File,
    io::{BufReader, Write},
};

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum FileType {
    Toml,
    Json,
}
pub struct Converter {}

impl Converter {
    pub fn convert<'a, T: Serialize + DeserializeOwned>(
        src: &str,
        src_type: FileType,
        dest: &str,
        dest_type: FileType,
    ) -> Result<(), anyhow::Error> {
        let data: T = match src_type {
            FileType::Toml => {
                let content = std::fs::read_to_string(src)?;
                toml::from_str::<T>(&content)
                    .map_err(|e| anyhow!("Failed to parse TOML ({})", e))?
            }
            FileType::Json => {
                let file = File::open(src)?;
                let reader = BufReader::new(file);
                serde_json::from_reader(reader)
                    .map_err(|e| anyhow!("Failed to parse JSON ({})", e))?
            }
        };

        match dest_type {
            FileType::Toml => {
                let content = toml::to_string::<T>(&data)?;
                let mut file = File::create(dest)?;
                write!(file, "{}", content).map_err(|e| anyhow!("Failed to parse JSON ({})", e))
            }
            FileType::Json => {
                let content = serde_json::to_string_pretty::<T>(&data)?;
                let mut file = File::create(dest)?;
                write!(file, "{}", content).map_err(|e| anyhow!("Failed to parse JSON ({})", e))
            }
        }
    }
}
