//! Integration coverage for `ferrowl-util`'s public surface: the [`Converter`] file
//! (de)serialization helpers and [`FileType`] inference, the `str!` macro, the [`Expect`]
//! trait, and the tracked-task `tokio::spawn_detach`/`join_all` pair.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ferrowl_test_support::reserve_temp_dir;
use ferrowl_util::convert::{Converter, Error, FileType};
use ferrowl_util::{Expect, str};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Config {
    name: String,
    count: u32,
    flags: Vec<bool>,
}

fn sample() -> Config {
    Config {
        name: "ferrowl".into(),
        count: 3,
        flags: vec![true, false, true],
    }
}

#[test]
fn it_save_then_load_roundtrips_toml() {
    let dir = reserve_temp_dir("ferrowl_util_api");
    let path = dir.join("sample.toml");
    let path = path.to_str().unwrap();
    Converter::save(&sample(), path, FileType::Toml).expect("saves TOML");
    let back: Config = Converter::load(path, FileType::Toml).expect("loads TOML");
    assert_eq!(back, sample());
}

#[test]
fn it_save_then_load_roundtrips_json() {
    let dir = reserve_temp_dir("ferrowl_util_api");
    let path = dir.join("sample.json");
    let path = path.to_str().unwrap();
    Converter::save(&sample(), path, FileType::Json).expect("saves JSON");
    let back: Config = Converter::load(path, FileType::Json).expect("loads JSON");
    assert_eq!(back, sample());
}

#[test]
fn it_convert_toml_to_json_preserves_data() {
    let dir = reserve_temp_dir("ferrowl_util_api");
    let toml = dir.join("sample.toml");
    let toml = toml.to_str().unwrap();
    let json = dir.join("sample.json");
    let json = json.to_str().unwrap();
    Converter::save(&sample(), toml, FileType::Toml).expect("saves source TOML");
    Converter::convert::<Config>(toml, FileType::Toml, json, FileType::Json)
        .expect("converts TOML to JSON");
    let back: Config = Converter::load(json, FileType::Json).expect("loads converted JSON");
    assert_eq!(back, sample());
}

#[test]
/// CS-R-002 — encoding is selected solely from the path extension, case-insensitive; an
/// unrecognized extension or none at all is `None`, never sniffed from content.
fn it_filetype_infers_from_extension() {
    assert_eq!(FileType::from_path("a/b.toml"), Some(FileType::Toml));
    assert_eq!(FileType::from_path("a/b.JSON"), Some(FileType::Json));
    assert_eq!(FileType::from_path("a/b.yaml"), None);
    assert_eq!(FileType::from_path("noext"), None);
}

#[test]
fn it_load_missing_file_is_deserialize_error() {
    let err = Converter::load::<Config>("/no/such/ferrowl/file.toml", FileType::Toml)
        .expect_err("a missing file cannot be loaded");
    assert!(matches!(err, Error::Deserialize(_)));
}

#[test]
fn it_str_macro_returns_owned_string() {
    let owned: String = str!("literal");
    assert_eq!(owned, "literal".to_string());
}

#[test]
fn it_expect_trait_returns_value_on_ok() {
    let r: Result<u8, &str> = Ok(7);
    assert_eq!(r.panic(|e| format!("unexpected: {e}")), 7);
}

#[test]
#[should_panic(expected = "boom happened")]
fn it_expect_trait_panics_with_formatted_message_on_err() {
    let r: Result<u8, &str> = Err("boom");
    let _ = r.panic(|e| format!("{e} happened"));
}

#[tokio::test]
async fn it_spawn_detach_runs_task_and_join_all_awaits_it() {
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();
    ferrowl_util::tokio::spawn_detach(async move {
        flag.store(true, Ordering::SeqCst);
    })
    .await;
    ferrowl_util::tokio::join_all().await;
    assert!(
        done.load(Ordering::SeqCst),
        "join_all must await the detached task to completion"
    );
}
