//! Consumer round-trip tests for `#[derive(TableEntry)]`.

use ferrowl_ui::widgets::{Header, TableEntry};
use ferrowl_ui_derive::TableEntry;
use ratatui::style::{Color, Style};

#[derive(TableEntry)]
struct ScriptRow {
    #[column(name = "Name", min = 10, max = 40)]
    name: String,
    #[column(name = "Status", min = 6, max = 6)]
    status: String,
}

#[test]
fn it_values_are_column_fields_in_order() {
    let row = ScriptRow {
        name: "boot".into(),
        status: "running".into(),
    };
    assert_eq!(row.values(), ["boot".to_string(), "running".to_string()]);
}

#[test]
fn it_default_height_is_one() {
    let row = ScriptRow {
        name: "x".into(),
        status: "y".into(),
    };
    assert_eq!(row.height(), 1);
}

#[test]
fn it_default_cell_styles_are_none() {
    let row = ScriptRow {
        name: "x".into(),
        status: "y".into(),
    };
    assert_eq!(row.cell_styles(), [None, None]);
}

#[test]
fn it_header_names_and_widths_from_attrs() {
    // Default companion type is `<StructName>Header`.
    assert_eq!(
        ScriptRowHeader::header(),
        ["Name".to_string(), "Status".to_string()]
    );
    let w = ScriptRowHeader::widths();
    assert_eq!((w[0].min, w[0].max), (10, 40));
    assert_eq!((w[1].min, w[1].max), (6, 6));
}

#[derive(TableEntry)]
#[row(height = 3)]
#[table_entry(header = KvHeader)]
struct KvRow {
    #[column(name = "Key", min = 16, max = 30)]
    key: String,
    #[column(name = "Value", min = 10, max = 40)]
    value: String,
    // Not a column: must be ignored by the derive.
    #[allow(dead_code)]
    hidden: u32,
}

#[test]
fn it_custom_height_and_header_name() {
    let row = KvRow {
        key: "k".into(),
        value: "v".into(),
        hidden: 7,
    };
    assert_eq!(row.height(), 3);
    assert_eq!(row.values(), ["k".to_string(), "v".to_string()]);
    assert_eq!(KvHeader::header(), ["Key".to_string(), "Value".to_string()]);
}

#[derive(TableEntry)]
#[table_entry(styles = cs_styles)]
struct CsRow {
    #[column(name = "Station", min = 18, max = 40)]
    name: String,
    #[column(name = "State", min = 12, max = 12)]
    state: String,
}

fn cs_styles(row: &CsRow) -> [Option<Style>; 2] {
    let s = match row.state.as_str() {
        "Connected" => Some(Style::default().fg(Color::Green)),
        _ => None,
    };
    [None, s]
}

#[test]
fn it_styles_path_drives_cell_styles() {
    let connected = CsRow {
        name: "cs1".into(),
        state: "Connected".into(),
    };
    assert_eq!(
        connected.cell_styles(),
        [None, Some(Style::default().fg(Color::Green))]
    );

    let down = CsRow {
        name: "cs2".into(),
        state: "Disconnected".into(),
    };
    assert_eq!(down.cell_styles(), [None, None]);
}
