//! Parser for the vim-style `:` command line. Pure (no app state) so it is unit-testable;
//! execution lives in `App::run_command`.

/// A parsed command-line command.
#[derive(Debug, PartialEq, Eq)]
pub enum Cmd {
    Empty,
    Quit,
    QuitAll,
    New,
    Load(Option<String>),
    Write(Option<String>),
    Log(Option<String>),
    Swap(usize, usize),
    /// Open the session-level scripts/interval dialog.
    Session,
    /// `script copy <index>`; `None` = missing/unparseable index.
    ScriptCopy(Option<usize>),
    Unknown(String),
}

/// Parse a command line (without the leading `:`).
pub fn parse(input: &str) -> Cmd {
    let mut parts = input.split_whitespace();
    let Some(name) = parts.next() else {
        return Cmd::Empty;
    };
    let mut first = || parts.next().map(std::string::ToString::to_string);

    match name {
        "q" | "q!" | "quit" => Cmd::Quit,
        "qa" | "qa!" | "qall" => Cmd::QuitAll,
        "n" | "new" => Cmd::New,
        "l" | "load" => Cmd::Load(first()),
        "s" | "save" | "w" | "write" => Cmd::Write(first()),
        "log" => Cmd::Log(first()),
        "swap" => {
            let from = match parts.next().map(str::parse) {
                Some(Ok(v)) => v,
                _ => return Cmd::Unknown("swap".to_string()),
            };
            let to = match parts.next().map(str::parse) {
                Some(Ok(v)) => v,
                _ => return Cmd::Unknown("swap".to_string()),
            };
            Cmd::Swap(from, to)
        }
        "session" => Cmd::Session,
        "script" => match parts.next() {
            Some("copy") => Cmd::ScriptCopy(parts.next().and_then(|s| s.parse().ok())),
            _ => Cmd::Unknown("script".to_string()), // bare `:script` is view-specific (dialog)
        },
        other => Cmd::Unknown(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_empty() {
        assert_eq!(parse(""), Cmd::Empty);
        assert_eq!(parse("   "), Cmd::Empty);
    }

    #[test]
    /// UI-R-016 — tab lifecycle commands (`:quit`, `:qall`, `:new`) parse to their app-level variants.
    fn ut_app_commands() {
        assert_eq!(parse("q"), Cmd::Quit);
        assert_eq!(parse("quit"), Cmd::Quit);
        assert_eq!(parse("q!"), Cmd::Quit);
        assert_eq!(parse("qa"), Cmd::QuitAll);
        assert_eq!(parse("qall"), Cmd::QuitAll);
        assert_eq!(parse("new"), Cmd::New);
        assert_eq!(parse("n"), Cmd::New);
    }

    #[test]
    /// UI-R-016 — `:write`, `:load`, and `:log clear` parse with their optional path/argument.
    fn ut_optional_paths() {
        assert_eq!(parse("w"), Cmd::Write(None));
        assert_eq!(
            parse("write out.toml"),
            Cmd::Write(Some("out.toml".to_string()))
        );
        assert_eq!(
            parse("load dev.toml"),
            Cmd::Load(Some("dev.toml".to_string()))
        );
        assert_eq!(parse("log run.log"), Cmd::Log(Some("run.log".to_string())));
        assert_eq!(parse("log"), Cmd::Log(None));
        assert_eq!(parse("log clear"), Cmd::Log(Some("clear".to_string())));
    }

    #[test]
    fn ut_module_commands_are_unknown() {
        // Module-specific commands fall through to Unknown; the view handles them.
        assert_eq!(parse("start"), Cmd::Unknown("start".to_string()));
        assert_eq!(parse("stop"), Cmd::Unknown("stop".to_string()));
        assert_eq!(parse("restart"), Cmd::Unknown("restart".to_string()));
        assert_eq!(parse("e"), Cmd::Unknown("e".to_string()));
        assert_eq!(parse("edit"), Cmd::Unknown("edit".to_string()));
        assert_eq!(parse("a"), Cmd::Unknown("a".to_string()));
        assert_eq!(parse("add"), Cmd::Unknown("add".to_string()));
        assert_eq!(parse("reload"), Cmd::Unknown("reload".to_string()));
        assert_eq!(parse("compact"), Cmd::Unknown("compact".to_string()));
        assert_eq!(parse("set"), Cmd::Unknown("set".to_string()));
        assert_eq!(parse("wd"), Cmd::Unknown("wd".to_string()));
        assert_eq!(parse("lua"), Cmd::Unknown("lua".to_string()));
        assert_eq!(parse("order"), Cmd::Unknown("order".to_string()));
    }

    #[test]
    fn ut_unknown() {
        assert_eq!(parse("bogus"), Cmd::Unknown("bogus".to_string()));
    }

    #[test]
    /// UI-R-016 — `:session` parses to the session-script-management command.
    fn ut_session() {
        assert_eq!(parse("session"), Cmd::Session);
    }

    #[test]
    /// UI-R-016 — `:script copy` parses to the session-script-management command.
    fn ut_script_copy() {
        assert_eq!(parse("script copy 3"), Cmd::ScriptCopy(Some(3)));
        assert_eq!(parse("script copy"), Cmd::ScriptCopy(None));
        assert_eq!(parse("script copy x"), Cmd::ScriptCopy(None));
        // Bare `:script` stays Unknown so the view opens its script dialog.
        assert_eq!(parse("script"), Cmd::Unknown("script".to_string()));
    }
}
