//! Shared script-table logic for [`ScriptDialog`](crate::dialog::scripts::ScriptDialog), the one
//! dialog type used both for the session (`:session`) and the OCPP/Modbus module views: it embeds
//! a script-table + name-input + code-editor surface (table of scripts with an On/Off status over
//! a "New Script" name input on the left, a code editor for the selected script on the right).
//!
//! The widget *fields* stay on each dialog struct (so `#[derive(Focus)]`'s `#[focus]` tags keep
//! working — a shared owning struct can't be tagged piecemeal from the outer dialog); this module
//! only extracts the duplicated *logic* that operates on them, via [`ScriptManagerRef`], a bundle
//! of `&mut` borrows into the caller's own fields.

use ferrowl_syntax::Language;
use ferrowl_ui::{
    Border,
    state::{
        ButtonState, ButtonStateBuilder, CodeInputFieldState, CodeInputFieldStateBuilder,
        InputFieldState, InputFieldStateBuilder, TableState, TableStateBuilder,
    },
    style::{ButtonStyle, InputFieldStyleBuilder, TableStyleBuilder},
    widgets::{
        Button, ButtonBuilder, CodeInputField, CodeInputFieldBuilder, InputField,
        InputFieldBuilder, Table, TableBuilder, Widget,
    },
};
use ferrowl_ui_derive::TableEntry;
use ratatui::{
    layout::{HorizontalAlignment, Margin},
    style::Style,
};

use crate::config::script::ScriptDef;
use crate::script_template::ScriptTemplate;

#[derive(Clone, Debug, Default, TableEntry)]
#[table_entry(header = ScriptHeader)]
pub(crate) struct ScriptRow {
    #[column(name = "Name", min = 10, max = 40)]
    name: String,
    #[column(name = "Status", min = 6, max = 6)]
    status: String,
}

pub(crate) type ScriptTable = Widget<TableState<ScriptRow, 2>, Table<ScriptRow, ScriptHeader, 2>>;

/// Selected row index, if any, bounds-checked against `scripts` (the table's own selection can
/// briefly point past the end right after a delete, before the row refresh lands).
pub(crate) fn selected(scripts: &[ScriptDef], table: &ScriptTable) -> Option<usize> {
    let sel = table.state.table_state().selected()?;
    (sel < scripts.len()).then_some(sel)
}

/// Bundle of `&mut` borrows into a dialog's own script-manager fields, so the shared logic below
/// can operate on them without the dialog owning a nested `ScriptManager` struct (which would break
/// `#[derive(Focus)]`'s per-field `#[focus]` tags).
pub(crate) struct ScriptManagerRef<'a> {
    pub scripts: &'a mut Vec<ScriptDef>,
    pub table: &'a mut ScriptTable,
    pub name_input: &'a mut Widget<InputFieldState, InputField<String>>,
    pub code: &'a mut Widget<CodeInputFieldState, CodeInputField>,
    pub compact: &'a mut bool,
}

impl ScriptManagerRef<'_> {
    pub fn selected(&self) -> Option<usize> {
        selected(self.scripts, self.table)
    }

    /// Load the selected script's code into the editor. Without a selection the editor is
    /// disabled: it shows only its placeholder and there is nothing edits could apply to.
    pub fn sync_code_from_selection(&mut self) {
        let content = self
            .selected()
            .map(|i| self.scripts[i].code.clone())
            .unwrap_or_default();
        self.code.state.set_content(&content);
        self.code.state.set_disabled(self.selected().is_none());
    }

    /// Write the editor's content back into the selected script.
    pub fn flush_code_to_selection(&mut self) {
        if let Some(i) = self.selected() {
            self.scripts[i].code = self.code.state.content();
        }
    }

    pub fn refresh_rows(&mut self) {
        self.table.state.set_values(rows(self.scripts));
    }

    /// Create a new enabled script from the name input (rejecting empty / duplicate names).
    pub fn create_script(&mut self) {
        let name = self.name_input.state.input().trim().to_string();
        if name.is_empty() || self.scripts.iter().any(|s| s.name == name) {
            return;
        }
        self.scripts.push(ScriptDef {
            name,
            code: String::new(),
            enabled: true,
        });
        self.refresh_rows();
        self.table.state.move_to_bottom();
        self.name_input.state.set_input(String::new());
        self.name_input.state.set_cursor(0);
        self.sync_code_from_selection();
    }

    /// Append a bundled template as a new enabled script and select it (UI-R-054). The code is
    /// **copied** — the inserted script has no link back to the template. A name already taken is
    /// suffixed rather than refused, so picking the same template twice always produces a script.
    pub fn insert_template(&mut self, template: &ScriptTemplate) {
        self.scripts.push(ScriptDef {
            name: unique_name(self.scripts, template.name),
            code: template.code.to_string(),
            enabled: true,
        });
        self.refresh_rows();
        self.table.state.move_to_bottom();
        self.sync_code_from_selection();
    }

    /// Rename the selected script (UI-R-055). Returns `false` — refusing — for an empty name or one
    /// another script already holds; renaming a script to its own current name is accepted as a
    /// no-op. Code and enabled flag are untouched either way.
    pub fn rename_selected(&mut self, new_name: &str) -> bool {
        let Some(i) = self.selected() else {
            return false;
        };
        let name = new_name.trim();
        if name.is_empty() {
            return false;
        }
        if self
            .scripts
            .iter()
            .enumerate()
            .any(|(j, s)| j != i && s.name == name)
        {
            return false;
        }
        self.scripts[i].name = name.to_string();
        self.refresh_rows();
        true
    }

    pub fn toggle_compact(&mut self) {
        *self.compact = !*self.compact;
        self.table.widget.set_row_margin(Margin {
            vertical: if *self.compact { 0 } else { 1 },
            horizontal: 0,
        });
    }

    pub fn toggle_selected(&mut self) {
        if let Some(i) = self.selected() {
            self.scripts[i].enabled = !self.scripts[i].enabled;
            self.refresh_rows();
        }
    }

    pub fn delete_selected(&mut self) {
        if let Some(i) = self.selected() {
            self.scripts.remove(i);
            self.refresh_rows();
            self.table.state.move_up();
            self.sync_code_from_selection();
        }
    }
}

/// `base` if free, else the first free `base-2`, `base-3`, … (UI-R-054).
fn unique_name(scripts: &[ScriptDef], base: &str) -> String {
    let taken = |name: &str| scripts.iter().any(|s| s.name == name);
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|name| !taken(name))
        .expect("an unbounded suffix search always terminates")
}

pub(crate) fn rows(scripts: &[ScriptDef]) -> Vec<ScriptRow> {
    scripts
        .iter()
        .map(|s| ScriptRow {
            name: s.name.clone(),
            status: if s.enabled { "On" } else { "Off" }.to_string(),
        })
        .collect()
}

pub(crate) fn script_table(rows: Vec<ScriptRow>) -> ScriptTable {
    Widget {
        state: TableStateBuilder::default()
            .values(rows)
            .build()
            .expect("all required builder fields are set"),
        widget: TableBuilder::default()
            .border(Border::Full(Margin::new(1, 0)))
            // UI-R-056 — the full binding list lived here and overflowed the panel; it now lives in
            // the `?` overlay, and the title only points at it.
            .title(Some("Scripts (?: help)".into()))
            .style(
                TableStyleBuilder::default()
                    .build()
                    .expect("all required builder fields are set"),
            )
            .row_margin(Margin {
                vertical: 1,
                horizontal: 0,
            })
            .build()
            .expect("all required builder fields are set"),
    }
}

pub(crate) fn name_input(border: Style) -> Widget<InputFieldState, InputField<String>> {
    Widget {
        state: InputFieldStateBuilder::default()
            .focused(false)
            .disabled(false)
            .placeholder(Some("New Script".to_string()))
            .build()
            .expect("all required builder fields are set"),
        widget: InputFieldBuilder::default()
            .border(Border::Full(Margin::new(1, 0)))
            .title(Some(("New Script", HorizontalAlignment::Left).into()))
            .style(
                InputFieldStyleBuilder::default()
                    .border(border)
                    .build()
                    .expect("all required builder fields are set"),
            )
            .margin(Margin {
                vertical: 0,
                horizontal: 0,
            })
            .build()
            .expect("all required builder fields are set"),
    }
}

/// The *Templates* button (UI-R-052): opens the template browser from the script dialog.
pub(crate) fn templates_button() -> Widget<ButtonState, Button> {
    Widget {
        state: ButtonStateBuilder::default()
            .focused(false)
            .label("TEMPLATES".to_string())
            .disabled(false)
            .build()
            .expect("all required builder fields are set"),
        widget: ButtonBuilder::default()
            .border_margin(Margin::new(1, 0))
            .margin(Margin {
                vertical: 0,
                horizontal: 0,
            })
            .style(ButtonStyle::default())
            .horizontal_alignment(HorizontalAlignment::Center)
            .build()
            .expect("all required builder fields are set"),
    }
}

pub(crate) fn code_editor(border: Style) -> Widget<CodeInputFieldState, CodeInputField> {
    Widget {
        state: CodeInputFieldStateBuilder::default()
            .focused(false)
            .disabled(false)
            .placeholder(Some("-- select or create a script".to_string()))
            .language(Some(Language::Lua))
            .build()
            .expect("all required builder fields are set"),
        widget: CodeInputFieldBuilder::default()
            .border(Border::Full(Margin::new(1, 0)))
            .title(Some("Code".into()))
            .style(
                InputFieldStyleBuilder::default()
                    .border(border)
                    .build()
                    .expect("all required builder fields are set"),
            )
            .margin(Margin {
                vertical: 0,
                horizontal: 0,
            })
            .build()
            .expect("all required builder fields are set"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_fixture<'a>(
        scripts: &'a mut Vec<ScriptDef>,
        table: &'a mut ScriptTable,
        name_input: &'a mut Widget<InputFieldState, InputField<String>>,
        code: &'a mut Widget<CodeInputFieldState, CodeInputField>,
        compact: &'a mut bool,
    ) -> ScriptManagerRef<'a> {
        ScriptManagerRef {
            scripts,
            table,
            name_input,
            code,
            compact,
        }
    }

    #[test]
    /// UI-R-089 — a new script name that is empty or duplicates another is refused.
    fn ut_create_script_rejects_empty_and_duplicate_names() {
        let mut scripts = vec![ScriptDef {
            name: "boot".into(),
            code: String::new(),
            enabled: true,
        }];
        let mut table = script_table(rows(&scripts));
        let mut name_input = name_input(Style::default());
        let mut code = code_editor(Style::default());
        let mut compact = false;
        let mut mgr = manager_fixture(
            &mut scripts,
            &mut table,
            &mut name_input,
            &mut code,
            &mut compact,
        );

        mgr.name_input.state.set_input("boot".to_string());
        mgr.create_script();
        assert_eq!(mgr.scripts.len(), 1, "duplicate name must be rejected");

        mgr.name_input.state.set_input(String::new());
        mgr.create_script();
        assert_eq!(mgr.scripts.len(), 1, "empty name must be rejected");

        mgr.name_input.state.set_input("second".to_string());
        mgr.create_script();
        assert_eq!(mgr.scripts.len(), 2);
        assert_eq!(mgr.scripts[1].name, "second");
    }

    #[test]
    /// UI-R-058, UI-R-091, UI-R-092 — `t` toggles and `d` deletes the selected script in the working list.
    fn ut_toggle_and_delete_selected() {
        let mut scripts = vec![ScriptDef {
            name: "a".into(),
            code: String::new(),
            enabled: true,
        }];
        let mut table = script_table(rows(&scripts));
        let mut name_input = name_input(Style::default());
        let mut code = code_editor(Style::default());
        let mut compact = false;
        let mut mgr = manager_fixture(
            &mut scripts,
            &mut table,
            &mut name_input,
            &mut code,
            &mut compact,
        );

        mgr.toggle_selected();
        assert!(!mgr.scripts[0].enabled);
        mgr.toggle_selected();
        assert!(mgr.scripts[0].enabled);

        mgr.delete_selected();
        assert!(mgr.scripts.is_empty());
    }

    #[test]
    /// UI-R-188 — the script table's title advertises only the help overlay, not individual bindings.
    fn ut_script_table_title_advertises_help_only() {
        let table = script_table(rows(&[]));
        let title = format!("{:?}", table.widget);
        assert!(title.contains("Scripts (?: help)"), "{title}");
        for binding in ["e: run", "t: toggle", "d: delete", "c: compact"] {
            assert!(!title.contains(binding), "title still lists '{binding}'");
        }
    }

    #[test]
    /// UI-R-054 — the suffix search skips every taken name, not just the base.
    fn ut_unique_name_suffixes_past_taken_names() {
        let scripts = vec![
            ScriptDef {
                name: "ramp".into(),
                code: String::new(),
                enabled: true,
            },
            ScriptDef {
                name: "ramp-2".into(),
                code: String::new(),
                enabled: true,
            },
        ];
        assert_eq!(unique_name(&scripts, "ramp"), "ramp-3");
        assert_eq!(unique_name(&scripts, "sine"), "sine");
    }

    #[test]
    /// UI-R-089 — renaming a script to its own current name is accepted.
    fn ut_rename_to_own_name_is_accepted() {
        let mut scripts = vec![ScriptDef {
            name: "boot".into(),
            code: "x".into(),
            enabled: true,
        }];
        let mut table = script_table(rows(&scripts));
        let mut name_input = name_input(Style::default());
        let mut code = code_editor(Style::default());
        let mut compact = false;
        let mut mgr = manager_fixture(
            &mut scripts,
            &mut table,
            &mut name_input,
            &mut code,
            &mut compact,
        );

        assert!(mgr.rename_selected("boot"));
        assert!(!mgr.rename_selected("   "), "blank name is refused");
        assert_eq!(mgr.scripts[0].name, "boot");
        assert_eq!(mgr.scripts[0].code, "x");
    }

    #[test]
    /// UI-R-093 — `c` toggles the script table between compact and normal rows.
    fn ut_toggle_compact_flips_row_margin() {
        let mut scripts: Vec<ScriptDef> = Vec::new();
        let mut table = script_table(rows(&scripts));
        let mut name_input = name_input(Style::default());
        let mut code = code_editor(Style::default());
        let mut compact = false;
        let mut mgr = manager_fixture(
            &mut scripts,
            &mut table,
            &mut name_input,
            &mut code,
            &mut compact,
        );

        assert!(!*mgr.compact);
        mgr.toggle_compact();
        assert!(*mgr.compact);
        mgr.toggle_compact();
        assert!(!*mgr.compact);
    }
}
