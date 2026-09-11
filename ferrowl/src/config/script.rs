//! Shared Lua script definition, used by both the OCPP and Modbus device configs and
//! managed by the [`ScriptDialog`](crate::dialog::scripts::ScriptDialog).

use serde::{Deserialize, Serialize};

/// One named Lua simulation script attached to a device type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptDef {
    pub name: String,
    #[serde(default)]
    pub code: String,
    /// Whether the script runs in the simulation loop. Defaults to On (a freshly-created script
    /// and a flag-less file entry are both active).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// SC-R-062 — a script's persisted shape follows config-session's envelope: `code` and
    /// `enabled` are optional on read (default empty / on), and a full round-trip through both
    /// TOML and JSON files preserves the value.
    fn ut_script_def_persists_per_config_session_envelope() {
        use ferrowl_test_support::reserve_temp_dir;
        use ferrowl_util::convert::{Converter, FileType};

        let dir = reserve_temp_dir("ferrowl_script_def_envelope");
        let toml_path = dir.join("minimal.toml");
        std::fs::write(&toml_path, "name = \"s\"\n").unwrap();
        let parsed: ScriptDef =
            Converter::load(toml_path.to_str().unwrap(), FileType::Toml).unwrap();
        assert_eq!(
            parsed,
            ScriptDef {
                name: "s".into(),
                code: String::new(),
                enabled: true,
            }
        );

        let full = ScriptDef {
            name: "s".into(),
            code: "C_Log:Info(\"x\")".into(),
            enabled: false,
        };
        for (path, ty) in [
            (dir.join("full.toml"), FileType::Toml),
            (dir.join("full.json"), FileType::Json),
        ] {
            let path = path.to_str().unwrap();
            Converter::save(&full, path, ty).unwrap();
            let back: ScriptDef = Converter::load(path, ty).unwrap();
            assert_eq!(back, full);
        }
    }
}
