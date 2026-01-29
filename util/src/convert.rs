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

pub enum Error {
    Serialize(String),
    Deserialize(String),
}

pub struct Converter {}

impl Converter {
    pub fn convert<'a, T: Serialize + DeserializeOwned>(
        src: &str,
        src_type: FileType,
        dest: &str,
        dest_type: FileType,
    ) -> Result<(), Error> {
        let data: T = match src_type {
            FileType::Toml => {
                let content = std::fs::read_to_string(src)
                    .map_err(|e| Error::Serialize(format!("Failed to read TOML file [{}].", e)))?;
                toml::from_str::<T>(&content)
                    .map_err(|e| Error::Serialize(format!("Failed to deserialize TOML [{}].", e)))?
            }
            FileType::Json => {
                let file = File::open(src)
                    .map_err(|e| Error::Serialize(format!("Failed to open JSON file [{}].", e)))?;
                let reader = BufReader::new(file);
                serde_json::from_reader(reader)
                    .map_err(|e| Error::Serialize(format!("Failed to deserialize JSON [{}].", e)))?
            }
        };

        match dest_type {
            FileType::Toml => {
                let content = toml::to_string::<T>(&data)
                    .map_err(|e| Error::Serialize(format!("Failed to serialize TOML [{}].", e)))?;
                let mut file = File::create(dest).map_err(|e| {
                    Error::Serialize(format!("Failed to create TOML file [{}].", e))
                })?;
                write!(file, "{}", content)
                    .map_err(|e| Error::Serialize(format!("Failed to serialize TOML [{}].", e)))
            }
            FileType::Json => {
                let content = serde_json::to_string_pretty::<T>(&data)
                    .map_err(|e| Error::Serialize(format!("Failed to serialize JSON [{}].", e)))?;
                let mut file = File::create(dest).map_err(|e| {
                    Error::Serialize(format!("Failed to create JSON file [{}].", e))
                })?;
                write!(file, "{}", content)
                    .map_err(|e| Error::Serialize(format!("Failed to serialize JSON [{}].", e)))
            }
        }
    }
}
