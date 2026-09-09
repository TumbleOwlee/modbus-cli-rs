//! `ModbusMonitorModuleView` (UI-R-060/061): the monitor module's content view — a left panel
//! listing observed unit ids, and three right-hand sections scoped to the selected unit id: a
//! message table (MB-R-143), a memory layout grouped by table kind (MB-R-144), and a
//! resolved-registers table (MB-R-145), the last hidden entirely when no interpretation exists
//! for the selected unit id.

use crossterm::event::{KeyCode, KeyModifiers};
use ferrowl_codec::Kind;
use ferrowl_modbus::monitor::{MonitorRecord, RECORD_RING_CAPACITY, RecordStatus, SharedRecordLog};
use ferrowl_modbus::{Key, SlaveKey, UnitId};
use ferrowl_ui::EventResult;
use ferrowl_ui::traits::HandleEvents;
use ferrowl_ui::{
    Border,
    state::{TableState, TableStateBuilder},
    style::TableStyleBuilder,
    widgets::{Header, Table, TableBuilder, Widget},
};
use ferrowl_ui_derive::{Focus, Overlay, TableEntry, focusable};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::StatefulWidget;

use crate::app::Level;
use crate::config::device::{MonitorRegisterDef, Scalar};
use crate::config::{ModuleSpec, MonitorDeviceConfig};
use crate::module::modbus::dialog::parse_raw_value;
use crate::module::view::{
    CommandDescriptor, CommandFuture, CommandResult, CommandSpec, ModuleView, RefreshFuture,
    SharedLog, parse_command,
};

use super::ModbusMonitorModule;
use super::dialog::{
    EditInterpretationDialog, SubPopupOutcome, route_interpretation_body,
    route_interpretation_subpopups,
};
use super::setup_dialog::MonitorSetupDialog;

/// MB-R-148 — an open edit-interpretation overlay: the dialog plus the interpretation's original
/// name, needed for the edit-in-place lookup (`module.edit_interpretation`'s `old_name` — a
/// confirmed rename changes the map key, so the dialog's own current label input isn't enough
/// once the user edits it).
struct InterpretationEdit {
    dialog: EditInterpretationDialog,
    original_name: String,
}

/// The single modal overlay over the monitor view (mutually exclusive by construction). The
/// derive supplies `is_active`/`take`/`close` and common-key routing (`Tab`/`BackTab` cycle
/// focus on the tagged variants); each variant's `Esc`/`Enter`/inner dispatch stays in
/// `handle_events`.
#[derive(Overlay)]
enum MonitorOverlay {
    #[overlay(none)]
    None,
    /// `:add`/`:a` interpretation dialog (UI-R-061) — a fresh, non-`deletable`
    /// `EditInterpretationDialog`, scoped to the currently selected unit id, differing from
    /// `EditInterpretation` below in prefill, `deletable`, and where focus starts (mirrors the
    /// modbus module's
    /// own `EditInputDialog::new()`/`from_register` split). The struct's own
    /// `#[focus(when = ...)]` gates handle the alias-list-shown-vs-hidden presentation
    /// internally.
    #[overlay(focus_cycle)]
    Add(Box<EditInterpretationDialog>),
    /// `:edit`/`:e` re-setup dialog, prefilled from the current spec/device.
    /// `Esc` and `Tab` stay hand-routed below: the dialog consumes both itself, and `Esc` now
    /// reaches the dialog itself, whose own close-confirm popup (UI-R-112) decides.
    EditSetup(Box<MonitorSetupDialog>),
    /// MB-R-148 — `Enter` on a Resolved-registers row opens this, prefilled from the
    /// selected row.
    #[overlay(focus_cycle)]
    EditInterpretation(Box<InterpretationEdit>),
}

ferrowl_ui::impl_overlay_keys!(EditInterpretationDialog);

impl ferrowl_ui::traits::OverlayKeys for InterpretationEdit {
    fn focus_cycle(&mut self, forward: bool) {
        if forward {
            self.dialog.focus_next();
        } else {
            self.dialog.focus_previous();
        }
    }
}

/// Save `device` to `path`, mirroring `ModbusModuleView::save_device_to`'s pattern (stamps the
/// current `VERSION`, format from the path's extension).
fn save_device_to(device: &MonitorDeviceConfig, path: &str) -> CommandResult {
    use ferrowl_util::convert::{Converter, FileType};
    let Some(ty) = FileType::from_path(path) else {
        return CommandResult::Handled(Some((
            Level::Warning,
            format!("unknown format for '{path}' (use .toml or .json)"),
        )));
    };
    let mut device = device.clone();
    device.version = Some(crate::config::VERSION.to_string());
    match Converter::save(&device, path, ty) {
        Ok(()) => CommandResult::Handled(Some((
            Level::Info,
            format!("Saved device config to {path}"),
        ))),
        Err(e) => CommandResult::Handled(Some((Level::Error, format!("Save failed: {e:?}")))),
    }
}

/// The 4 register-table kinds a monitor's observed-value table groups its memory layout by
/// (MB-R-144), in display order.
const TABLE_KINDS: [Kind; 4] = [
    Kind::Coil,
    Kind::DiscreteInput,
    Kind::HoldingRegister,
    Kind::InputRegister,
];

/// One row of the Units panel: one observed unit id. A `Table`/`TableEntry` row like the other
/// panels, for rendering consistency across all four.
#[derive(Clone, Debug, Default, TableEntry)]
#[table_entry(header = UnitHeader)]
struct UnitRow {
    #[column(name = "Unit", min = 4, max = 4)]
    unit: String,
}

type UnitsTable = Widget<TableState<UnitRow, 1>, Table<UnitRow, UnitHeader, 1>>;

/// Build a fresh, empty `UnitsTable` widget/state pair, same builder shape as
/// `new_message_table`/`new_resolved_table`.
fn new_units_table() -> UnitsTable {
    Widget {
        state: TableStateBuilder::default()
            .focused(false)
            .values(Vec::new())
            .build()
            .expect("all required builder fields are set"),
        widget: TableBuilder::default()
            .border(Border::Full(ratatui::layout::Margin::new(1, 0)))
            .title(Some("Units".into()))
            .style(
                TableStyleBuilder::default()
                    .build()
                    .expect("all required builder fields are set"),
            )
            // The Units panel is a single narrow column (unit ids only) —
            // the selected row's own background/foreground change (still applied via
            // `row_highlight_style`) already conveys focus without the extra `█` bar,
            // which crowded the one data column.
            .show_selection_marker(false)
            .build()
            .expect("all required builder fields are set"),
    }
}

/// UI-R-062 — one row of the Messages table: a `MonitorRecord`, sourced from `module.records()`,
/// rendered most-recent-first in this fixed column order.
#[derive(Clone, Debug, Default, TableEntry)]
#[table_entry(header = MessageHeader, styles = message_row_styles)]
struct MessageRow {
    // Wide enough for `format_timestamp`'s full "YYYY-MM-DD HH:MM:SS.mmm" (23 chars)
    // to render on one line, not wrap/truncate.
    #[column(name = "Time", min = 23, max = 23)]
    time: String,
    #[column(name = "Status", min = 10, max = 25)]
    status: String,
    #[column(name = "Slave", min = 6, max = 6)]
    slave: String,
    #[column(name = "Operation", min = 14, max = 30)]
    operation: String,
    #[column(name = "Address", min = 8, max = 16)]
    address: String,
    #[column(name = "Quantity", min = 8, max = 16)]
    quantity: String,
    #[column(name = "Values/Payload", min = 10, max = 800)]
    payload: String,
    /// Not a `#[column]` — visual improvement: the Status column's own color (`success` for
    /// `RecordStatus::Ok`, `error` for any exception, `None`/default for `Unmatched`), consumed
    /// by `message_row_styles`.
    status_style: Option<ratatui::style::Style>,
}

type MessageTable = Widget<TableState<MessageRow, 7>, Table<MessageRow, MessageHeader, 7>>;

/// UI-R-062 — build a fresh, empty `MessageTable` widget/state pair, same builder shape
/// `ferrowl/src/module/modbus/table.rs`'s `TableView::new` already uses.
fn new_message_table() -> MessageTable {
    Widget {
        state: TableStateBuilder::default()
            .focused(false)
            .values(Vec::new())
            .build()
            .expect("all required builder fields are set"),
        widget: TableBuilder::default()
            .border(Border::Full(ratatui::layout::Margin::new(1, 0)))
            .title(Some("Messages".into()))
            .style(
                TableStyleBuilder::default()
                    .build()
                    .expect("all required builder fields are set"),
            )
            .build()
            .expect("all required builder fields are set"),
    }
}

/// UI-R-062's `Status` column text. The exception case renders the bare
/// `ExceptionCode` variant name (e.g. `IllegalDataAddress`), not the Debug-derived
/// `Exception(...)` wrapper around it.
fn format_record_status(status: &RecordStatus) -> String {
    match status {
        RecordStatus::Ok => "OK".to_string(),
        RecordStatus::Unmatched => "Unmatched".to_string(),
        RecordStatus::Exception(code) => format!("{code:?}"),
    }
}

/// Visual improvement — the Status column's own color: green for `Ok`, red for any exception,
/// `None` (default text color) for `Unmatched`.
fn record_status_style(status: &RecordStatus) -> Option<ratatui::style::Style> {
    use ferrowl_ui::COLOR_SCHEME;
    match status {
        RecordStatus::Ok => Some(ratatui::style::Style::default().fg(COLOR_SCHEME.success)),
        RecordStatus::Exception(_) => Some(ratatui::style::Style::default().fg(COLOR_SCHEME.error)),
        RecordStatus::Unmatched => None,
    }
}

/// Visual improvement — per-column styles for a `MessageRow`: only the Status column carries an
/// override (`row.status_style`), every other column keeps the table's default text color.
fn message_row_styles(row: &MessageRow) -> [Option<ratatui::style::Style>; 7] {
    [None, row.status_style, None, None, None, None, None]
}

/// Shared hex-words formatting (UI-R-062's Messages payload, UI-R-064's resolved-registers raw
/// value): 4-digit lowercase hex per word, bracket-wrapped, space-separated.
fn hex_words(words: &[u16]) -> String {
    format!(
        "[{}]",
        words
            .iter()
            .map(|v| format!("{v:04x}"))
            .collect::<Vec<_>>()
            .join(" ")
    )
}

/// UI-R-062's `Values/Payload` column text — empty for no shape or no values (MB-R-146's
/// record-status-to-value gating); one digit per bit for coil-family kinds, 4-digit
/// lowercase hex per word for register-family kinds.
fn format_record_payload(record: &MonitorRecord) -> String {
    let Some(shape) = &record.shape else {
        return String::new();
    };
    if shape.values.is_empty() {
        return String::new();
    }
    match shape.kind {
        Kind::Coil | Kind::DiscreteInput => format!(
            "[{}]",
            shape
                .values
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(" ")
        ),
        Kind::HoldingRegister | Kind::InputRegister => hex_words(&shape.values),
    }
}

/// UI-R-062's `Address`/`Quantity` columns — empty for no shape; `edge-cases.md`'s
/// `ReadWriteMultipleRegisters` row (both a read and a write address/quantity pair) renders
/// slash-separated, otherwise the single address/quantity.
fn format_record_address_quantity(record: &MonitorRecord) -> (String, String) {
    let Some(shape) = &record.shape else {
        return (String::new(), String::new());
    };
    match (shape.write_address, shape.write_quantity) {
        (Some(write_address), Some(write_quantity)) => (
            format!("{}/{}", shape.address, write_address),
            format!("{}/{}", shape.quantity, write_quantity),
        ),
        _ => (shape.address.to_string(), shape.quantity.to_string()),
    }
}

/// UI-R-062 — build one `MessageRow` for `unit`'s Messages table from a captured record.
/// `MonitorRecord.timestamp` is a monotonic `Instant` with no wall-clock reference of
/// its own, so `time` is derived by projecting the record's age (relative to `now`, an
/// `Instant` captured at the same moment as `wall_now`) back from `wall_now`, then formatted
/// with the same full-timestamp format the log pane uses
/// (`crate::view::log::format_timestamp`), not a relative "Xs ago" string.
fn message_row(
    unit: UnitId,
    record: &MonitorRecord,
    now: std::time::Instant,
    wall_now: std::time::SystemTime,
) -> MessageRow {
    let (address, quantity) = format_record_address_quantity(record);
    let elapsed = now.duration_since(record.timestamp);
    let wall = wall_now
        .checked_sub(elapsed)
        .unwrap_or(std::time::UNIX_EPOCH);
    let ms = wall
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    MessageRow {
        time: crate::view::log::format_timestamp(ms),
        status: format_record_status(&record.status),
        slave: unit.to_string(),
        operation: format!("{:?}", record.operation),
        address,
        quantity,
        payload: format_record_payload(record),
        status_style: record_status_style(&record.status),
    }
}

/// UI-R-064 — one row of the Resolved-registers table: the same column set as
/// `ferrowl::module::modbus::table::TableHeader` minus Slave ID/Access.
#[derive(Clone, Debug, Default, TableEntry)]
#[table_entry(header = ResolvedHeader)]
struct ResolvedRow {
    #[column(name = "Name", min = 6, max = 30)]
    name: String,
    #[column(name = "Description", min = 13, max = 40)]
    description: String,
    #[column(name = "Address", min = 10, max = 20)]
    address: String,
    #[column(name = "Kind", min = 6, max = 30)]
    kind: String,
    #[column(name = "Format", min = 10, max = 20)]
    format: String,
    #[column(name = "Length", min = 10, max = 20)]
    length: String,
    #[column(name = "Resolution", min = 12, max = 20)]
    resolution: String,
    #[column(name = "Value", min = 5, max = 40)]
    value: String,
    #[column(name = "Raw Value", min = 11, max = 800)]
    raw_value: String,
}

type ResolvedTable = Widget<TableState<ResolvedRow, 9>, Table<ResolvedRow, ResolvedHeader, 9>>;

/// UI-R-064 — build a fresh, empty `ResolvedTable` widget/state pair, same builder shape as
/// `new_message_table`.
fn new_resolved_table() -> ResolvedTable {
    Widget {
        state: TableStateBuilder::default()
            .focused(false)
            .values(Vec::new())
            .build()
            .expect("all required builder fields are set"),
        widget: TableBuilder::default()
            .border(Border::Full(ratatui::layout::Margin::new(1, 0)))
            .title(Some("Resolved registers".into()))
            .style(
                TableStyleBuilder::default()
                    .build()
                    .expect("all required builder fields are set"),
            )
            // Compact is the default here (`:compact` toggles it, tui/api-contract.md §2.1) — a
            // deliberate divergence from `TableView::new`'s own expanded-by-default: no vertical
            // padding.
            .row_margin(ratatui::layout::Margin {
                vertical: 0,
                horizontal: 0,
            })
            .build()
            .expect("all required builder fields are set"),
    }
}

/// UI-R-064 — build one `ResolvedRow` for `name`/`def`, reading `unit`'s currently observed
/// words from `table`. `value`/`raw_value` render `"(not yet observed)"`/empty when nothing has
/// been observed at `def`'s address yet.
fn resolved_row(
    name: &str,
    def: &MonitorRegisterDef,
    table: &ferrowl_modbus::monitor::ObservedTable,
    unit: UnitId,
) -> ResolvedRow {
    let key = Key::new(SlaveKey {
        slave_id: unit,
        kind: def.kind.clone(),
    });
    let address = match def.address() {
        ferrowl_codec::Address::Fixed(a) => a,
        ferrowl_codec::Address::Virtual => 0,
    };
    let width = def.format().width();
    let (value, raw_value) = match table.read_words(&key, address, width) {
        // Decode via the same path the modbus module's own `Definition::values`
        // (`ferrowl/src/module/modbus/table.rs`) uses for its Value column
        // (`self.register.decode(&raw)`, itself `ferrowl_codec::decode(&format, &raw)`), not a
        // raw `{words:?}` debug dump, and no `[...]` wrapping on Value (Raw Value keeps
        // its own bracketed hex format via `hex_words`).
        Some(words) => {
            let mut value = match ferrowl_codec::decode(&def.format(), &words) {
                Ok(v) => v.to_string(),
                Err(_) => "Error".to_string(),
            };
            let raw_value = hex_words(&words);
            // When the decoded value exactly matches one of the interpretation's
            // named values, show the label alone (not "label (value)", unlike the full modbus
            // module's own `Definition::values`), using the same
            // `Scalar::Int`-vs-raw-int-or-string matching logic (`table.rs`'s own `values()`).
            let raw_int = parse_raw_value(&raw_value);
            if let Some(named) = def.values.iter().find(|nv| match &nv.value {
                Scalar::Int(v) => raw_int == Some(*v) || value == v.to_string(),
                other => value == other.to_string(),
            }) {
                value = named.name.clone();
            }
            (value, raw_value)
        }
        None => ("(not yet observed)".to_string(), String::new()),
    };
    let resolution = match def.format().resolution() {
        Some(v) => format!("{v}"),
        None => "None".to_string(),
    };
    ResolvedRow {
        name: name.to_string(),
        description: def.description.clone(),
        address: address.to_string(),
        kind: format!("{:?}", def.kind),
        format: format!("{}", def.format()),
        length: format!("{width}"),
        resolution,
        value,
        raw_value,
    }
}

/// UI-R-063 — one hex-editor cell of the Memory-layout panel: an observed-or-not raw value at
/// one raw address (coil-family: one packed byte covering 8 bit addresses; register-family: one
/// word at one word address).
#[derive(Debug, Clone, Copy, PartialEq)]
struct MemoryCell {
    observed: bool,
    value: u16,
}

/// UI-R-063 — `(raw addresses per cell, cells per rendered line)` for `kind`: coil-family packs
/// 8 bits per byte-cell, 16 bytes (128 bit addresses) per line; register-family is 1 word per
/// cell, 8 words per line.
fn memory_cell_shape(kind: Kind) -> (u16, u16) {
    match kind {
        Kind::Coil | Kind::DiscreteInput => (8, 16),
        Kind::HoldingRegister | Kind::InputRegister => (1, 8),
    }
}

/// UI-R-063 — group `pairs` (from [`ModbusMonitorModuleView::memory_rows`], already
/// address-ordered `(address, value)` pairs) into fixed-width hex-editor lines per
/// `memory_cell_shape`, MSB-first bit-packed for coil-family kinds. Only lines containing at
/// least one observed cell are returned, sorted by starting address. A cell within an otherwise
/// -observed line that itself has no observed constituent bit/word renders unobserved
/// (`MemoryCell::observed == false`, `value == 0`) — for coil-family kinds this is a documented
/// sub-byte-granularity call: a byte with only some of its 8 bits observed still counts as an
/// observed cell, with its unobserved bits packed as `0`.
fn memory_lines(kind: Kind, pairs: &[(u16, u16)]) -> Vec<(u16, Vec<MemoryCell>)> {
    let (unit_per_cell, cells_per_line) = memory_cell_shape(kind);
    let observed: std::collections::HashMap<u16, u16> = pairs.iter().copied().collect();

    let mut line_starts: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();
    for &(address, _) in pairs {
        let cell_addr = address / unit_per_cell;
        let line_start_cell = (cell_addr / cells_per_line) * cells_per_line;
        line_starts.insert(line_start_cell * unit_per_cell);
    }

    line_starts
        .into_iter()
        .map(|line_start_address| {
            let line_start_cell = line_start_address / unit_per_cell;
            let cells = (0..cells_per_line)
                .map(|i| {
                    let cell_addr = line_start_cell + i;
                    if unit_per_cell == 8 {
                        let mut value: u16 = 0;
                        let mut cell_observed = false;
                        for bit in 0..8u16 {
                            let raw = cell_addr * 8 + bit;
                            if let Some(&bit_value) = observed.get(&raw) {
                                cell_observed = true;
                                if bit_value != 0 {
                                    value |= 1 << (7 - bit);
                                }
                            }
                        }
                        MemoryCell {
                            observed: cell_observed,
                            value,
                        }
                    } else {
                        match observed.get(&cell_addr) {
                            Some(&v) => MemoryCell {
                                observed: true,
                                value: v,
                            },
                            None => MemoryCell {
                                observed: false,
                                value: 0,
                            },
                        }
                    }
                })
                .collect();
            (line_start_address, cells)
        })
        .collect()
}

/// Whether `value`'s low byte is a printable ASCII character (space or graphic) — the same
/// printable-vs-`.` convention `Definition::values()` uses (`ferrowl/src/module/modbus/table.rs`).
fn is_printable_low_byte(value: u16) -> bool {
    let byte = (value & 0xFF) as u8;
    byte == b' ' || byte.is_ascii_graphic()
}

/// UI-R-063's value-class color for one cell: unobserved or an observed zero reads as neutral
/// (`placeholder`); an observed printable-ASCII low byte as normal text; any other observed
/// non-zero value flagged (`warning`).
fn memory_cell_value_style(cell: &MemoryCell) -> ratatui::style::Color {
    use ferrowl_ui::COLOR_SCHEME;
    if !cell.observed || cell.value == 0 {
        COLOR_SCHEME.placeholder
    } else if is_printable_low_byte(cell.value) {
        COLOR_SCHEME.text
    } else {
        COLOR_SCHEME.warning
    }
}

/// The character-representation column for one cell: `.` when unobserved or non-printable, else
/// the printable low-byte character (`Definition::values()`'s convention).
fn memory_cell_char(cell: &MemoryCell) -> char {
    if cell.observed && is_printable_low_byte(cell.value) {
        (cell.value & 0xFF) as u8 as char
    } else {
        '.'
    }
}

/// UI-R-063 — one row of the Memory-layout hex-editor render: a table-kind line, its starting
/// address, and its cells.
struct MemoryLine {
    kind: Kind,
    address: u16,
    cells: Vec<MemoryCell>,
}

/// UI-R-063 — flatten `memory_rows`'s per-kind pair-lists into one ordered sequence of
/// hex-editor lines.
fn memory_layout_lines(kind_rows: &[(Kind, Vec<(u16, u16)>)]) -> Vec<MemoryLine> {
    let mut out = Vec::new();
    for (kind, pairs) in kind_rows {
        for (address, cells) in memory_lines(kind.clone(), pairs) {
            out.push(MemoryLine {
                kind: kind.clone(),
                address,
                cells,
            });
        }
    }
    out
}

/// Whether any of `cell_address`'s `unit_per_cell` constituent raw addresses is MB-R-147
/// recency-active as of `now`.
fn memory_cell_recency_active(
    kind: Kind,
    cell_address: u16,
    unit_per_cell: u16,
    records: &[MonitorRecord],
    now: std::time::Instant,
) -> bool {
    (0..unit_per_cell).any(|i| {
        ferrowl_modbus::monitor::recency_active_at(records, kind.clone(), cell_address + i, now)
    })
}

/// One row of the Memory-layout table: a rendered line (starting address, its cells'
/// hex values space-separated, their character representation). Real `Table`/`TableEntry`
/// machinery, like the other panels.
///
/// UI-R-063 requires each individual byte/word to carry its own value-class/recency color; the
/// Hex/Ascii columns use `TableEntry::cell_spans` (`ferrowl-ui/src/widgets/table.rs`) to
/// carry one `(text, style)` span per cell, so every byte/word keeps its own color — not the
/// row's plain `cell_styles` (`Style`-per-whole-cell), which the Address column still uses since
/// it has no sub-cell structure to color independently.
#[derive(Clone, Debug, Default, TableEntry)]
#[table_entry(header = MemoryHeader, styles = memory_row_styles, spans = memory_row_spans)]
struct MemoryRow {
    // UI-R-063 — the line's table kind, same `Kind` `Display` naming the modbus module's own
    // register table uses.
    #[column(name = "Kind", min = 16, max = 16)]
    kind: String,
    #[column(name = "Address", min = 6, max = 10)]
    address: String,
    #[column(name = "Hex", min = 20, max = 80)]
    hex: String,
    #[column(name = "Ascii", min = 10, max = 20)]
    ascii: String,
    /// Not a `#[column]` — per-cell `(hex text, style)` spans for the Hex column, consumed by
    /// `memory_row_spans`.
    hex_spans: Vec<(String, ratatui::style::Style)>,
    /// Not a `#[column]` — per-cell `(ascii char, style)` spans for the Ascii column, consumed by
    /// `memory_row_spans`.
    ascii_spans: Vec<(String, ratatui::style::Style)>,
}

type MemoryTable = Widget<TableState<MemoryRow, 4>, Table<MemoryRow, MemoryHeader, 4>>;

/// Build a fresh, empty `MemoryTable` widget/state pair, same builder shape as
/// `new_message_table`/`new_resolved_table`.
fn new_memory_table() -> MemoryTable {
    Widget {
        state: TableStateBuilder::default()
            .focused(false)
            .values(Vec::new())
            .build()
            .expect("all required builder fields are set"),
        widget: TableBuilder::default()
            .border(Border::Full(ratatui::layout::Margin::new(1, 0)))
            .title(Some("Memory layout".into()))
            .style(
                TableStyleBuilder::default()
                    .build()
                    .expect("all required builder fields are set"),
            )
            .build()
            .expect("all required builder fields are set"),
    }
}

/// Colors Kind/Address by the same neutral border color the previous render's `{address:04x} `
/// prefix used — Hex/Ascii get their color from `memory_row_spans` instead (per-cell, not
/// per-row), so this returns `None` for those two columns (the render loop treats a `Some`
/// `cell_spans` entry as taking over that column's coloring entirely).
fn memory_row_styles(_row: &MemoryRow) -> [Option<ratatui::style::Style>; 4] {
    use ferrowl_ui::COLOR_SCHEME;
    let neutral = Some(ratatui::style::Style::default().fg(COLOR_SCHEME.border));
    [neutral, neutral, None, None]
}

/// UI-R-063 — per-cell spans for the Hex/Ascii columns: `row.hex_spans`/`row.ascii_spans`
/// already carry each cell's own `(text, style)` pair (computed in `memory_table_rows`); `None`
/// for the Kind/Address columns (no sub-cell structure to color independently).
fn memory_row_spans(row: &MemoryRow) -> [Option<Vec<(String, ratatui::style::Style)>>; 4] {
    [
        None,
        None,
        Some(row.hex_spans.clone()),
        Some(row.ascii_spans.clone()),
    ]
}

/// MB-R-147, UI-R-063 — one cell's own color: `hi` while an MB-R-147 recency marker is active for
/// any of its constituent raw addresses, else `memory_cell_value_style`'s value-class color.
fn memory_cell_style(
    kind: Kind,
    cell_address: u16,
    unit_per_cell: u16,
    cell: &MemoryCell,
    records: &[MonitorRecord],
    now: std::time::Instant,
) -> ratatui::style::Color {
    use ferrowl_ui::COLOR_SCHEME;
    if memory_cell_recency_active(kind, cell_address, unit_per_cell, records, now) {
        COLOR_SCHEME.hi
    } else {
        memory_cell_value_style(cell)
    }
}

/// Build the Memory-layout table's rows from `lines`: each line gets its hex/ascii
/// text plus one `(text, style)` span per cell (UI-R-063/MB-R-147, true per-byte/word
/// granularity via `TableEntry::cell_spans`).
fn memory_table_rows(
    lines: &[MemoryLine],
    records: &[MonitorRecord],
    now: std::time::Instant,
) -> Vec<MemoryRow> {
    lines
        .iter()
        .map(|line| {
            let MemoryLine {
                kind,
                address,
                cells,
            } = line;
            let unit_per_cell = memory_cell_shape(kind.clone()).0;
            let hex = cells
                .iter()
                .map(|cell| {
                    if unit_per_cell == 8 {
                        format!("{:02x}", cell.value)
                    } else {
                        format!("{:04x}", cell.value)
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            let ascii: String = cells.iter().map(memory_cell_char).collect();
            let last = cells.len().saturating_sub(1);
            // Each cell's recency/value-class color is computed once and reused for both the
            // Hex and Ascii spans.
            let (hex_spans, ascii_spans): (Vec<_>, Vec<_>) = cells
                .iter()
                .enumerate()
                .map(|(i, cell)| {
                    let cell_address = address.saturating_add((i as u16) * unit_per_cell);
                    let color = memory_cell_style(
                        kind.clone(),
                        cell_address,
                        unit_per_cell,
                        cell,
                        records,
                        now,
                    );
                    let style = ratatui::style::Style::default().fg(color);
                    let hex_text = if unit_per_cell == 8 {
                        format!("{:02x}", cell.value)
                    } else {
                        format!("{:04x}", cell.value)
                    };
                    let hex_text = if i == last {
                        hex_text
                    } else {
                        format!("{hex_text} ")
                    };
                    (
                        (hex_text, style),
                        (memory_cell_char(cell).to_string(), style),
                    )
                })
                .unzip();
            MemoryRow {
                kind: kind.to_string(),
                address: format!("{address:04x}"),
                hex,
                ascii,
                hex_spans,
                ascii_spans,
            }
        })
        .collect()
}

#[focusable]
#[derive(Focus)]
pub struct ModbusMonitorModuleView {
    module: ModbusMonitorModule,
    spec: ModuleSpec,
    device: MonitorDeviceConfig,
    /// Every unit id observed so far, sorted, refreshed each tick (UI-R-060).
    unit_ids: Vec<UnitId>,
    /// Index into `unit_ids` of the left panel's current selection.
    selected: usize,
    /// Units panel table, rebuilt live in `render()` from `unit_ids`; `selected`
    /// remains the single source of truth for which row is selected (unchanged, everything else
    /// already keys off it) — this only drives which row the table highlights.
    #[focus]
    units_table: UnitsTable,
    /// UI-R-062 Messages table for the selected unit id, re-derived each tick from
    /// `module.records()`.
    #[focus]
    messages_table: MessageTable,
    /// Memory-layout table (MB-R-144/UI-R-063) for the selected unit id, rebuilt live
    /// in `render()` — it depends on `module.records()`'s recency markers, which change
    /// independent of any `refresh()` tick.
    #[focus]
    memory_table: MemoryTable,
    /// UI-R-064 Resolved-registers table for the selected unit id, re-derived each tick.
    #[focus(when = self.resolved_focusable())]
    resolved_table: ResolvedTable,
    overlay: MonitorOverlay,
    /// `:compact` toggle (tui/api-contract.md §2.1) — a real row-margin toggle on
    /// `resolved_table.widget` (mirrors `TableView::set_compact`), not string formatting.
    compact: bool,
    /// `:order [col] [asc|desc]` (tui/api-contract.md §2.1) — sorts the resolved-registers
    /// table by column *index* (resolved from the name once, at `:order` parse time, via
    /// `column_index_for::<ResolvedHeader, 9>`); `None` is definition order. `bool` is
    /// "descending". Re-applied every `refresh()` tick (not just once at `:order` time) so newly
    /// added interpretations stay correctly ordered — a deliberate implementation choice, not
    /// spec-mandated (either satisfies UI-R-064/tui/api-contract.md).
    sort: Option<(usize, bool)>,
    /// MB-R-150 — the session-wide serial-path registry attached via `set_serial_paths`, kept so
    /// a rebuilt `self.module` (`:reload`, `confirm_edit`) can be reattached to the same registry
    /// instead of silently falling back to a private default.
    serial_paths: crate::module::modbus::SerialPathRegistry,
    /// UI-R-062 perf: which unit id `messages_table`'s rows were last built for, which
    /// `SharedRecordLog` instance they came from (Arc identity — `self.module.records()` returns
    /// a fresh clone of a *different* underlying log after `:reload`/`confirm_edit` rebuild
    /// `self.module`, even if the new log's generation coincidentally matches or exceeds the old
    /// one; comparing generations alone can't tell those two cases apart), and the generation
    /// observed at that time. `refresh()` skips rebuilding entirely when all three are unchanged,
    /// and formats only the newly-arrived tail when just the generation moved on the same log.
    /// `None`/`0` before the first refresh.
    cached_messages_unit: Option<UnitId>,
    cached_messages_log: Option<SharedRecordLog>,
    cached_messages_generation: u64,
}

impl ModbusMonitorModuleView {
    pub fn new(module: ModbusMonitorModule, spec: ModuleSpec, device: MonitorDeviceConfig) -> Self {
        Self {
            module,
            spec,
            device,
            unit_ids: Vec::new(),
            selected: 0,
            units_table: new_units_table(),
            messages_table: new_message_table(),
            memory_table: new_memory_table(),
            resolved_table: new_resolved_table(),
            overlay: MonitorOverlay::None,
            view_focused: false,
            focus: ModbusMonitorModuleViewFocus::UnitsTable,
            compact: true,
            sort: None,
            serial_paths: crate::module::modbus::SerialPathRegistry::default(),
            cached_messages_unit: None,
            cached_messages_log: None,
            cached_messages_generation: 0,
        }
    }

    fn selected_unit(&self) -> Option<UnitId> {
        self.unit_ids.get(self.selected).copied()
    }

    /// Validate the open `Add` overlay's dialog, scope it to the selected unit id (UI-R-061:
    /// "the dialog never asks for it"), and add it to the module's interpretations (MB-R-145).
    /// No-op if the overlay isn't open, the dialog is invalid, or nothing is selected.
    fn confirm_add(&mut self) {
        let MonitorOverlay::Add(overlay) = &self.overlay else {
            return;
        };
        let Ok((name, def)) = overlay.apply() else {
            return;
        };
        let Some(unit) = self.selected_unit() else {
            return;
        };
        self.module.add_interpretation(unit, name, def);
        // Keep
        // `self.device.definitions` (the on-disk list `:write` saves verbatim) in sync
        // with the module's live interpretations, mirroring `ModbusModule::apply_add`'s own
        // `self.device.definitions.insert(...)`.
        self.device.definitions = self.module.definitions();
        self.overlay.close();
    }

    /// Resolve the open `Edit` overlay's dialog and rebuild `spec`/`device`/`module` from it
    /// (MB-R-140), mirroring `:reload`'s "stop the old task, build a fresh module" shape but
    /// without an implicit restart — the user starts the monitor explicitly, same as after any
    /// other setup edit. No-op if the overlay isn't open or the dialog doesn't resolve.
    fn confirm_edit(&mut self) {
        let MonitorOverlay::EditSetup(dialog) = &self.overlay else {
            return;
        };
        let Ok(outcome) = dialog.resolve() else {
            return;
        };
        // `outcome.device` is only ever `Some` in New mode (`resolve()` never re-loads
        // the device config file from a possibly-edited path in Edit mode, matching the full
        // client/server module's own edit-confirm); the device-path *field* itself must
        // still apply on every edit-confirm regardless, so it comes from `values.config_path`
        // unconditionally rather than being silently dropped whenever `outcome.device` is `None`.
        self.spec.device = outcome.values.config_path.clone();
        let mut device = outcome
            .device
            .map_or_else(|| self.device.clone(), |(_, d)| d);
        self.spec.name = outcome.values.name;
        self.spec.endpoint = outcome.values.endpoint;
        device.reconnect = Some(outcome.values.reconnect);
        // `reconfigure` carries the running module's accumulated `table`/
        // `records`/`log`/`interpretations` over instead of `ModbusMonitorModule::new()`'s
        // always-fresh-and-empty construction.
        let placeholder = ModbusMonitorModule::new(&self.spec, &device);
        self.module =
            std::mem::replace(&mut self.module, placeholder).reconfigure(&self.spec, &device);
        // MB-R-150 — the resulting module is what a later `:start` actually runs (not a
        // throwaway preview instance), so it must carry the session-wide registry forward too.
        self.module.set_serial_paths(self.serial_paths.clone());
        // Resync `definitions`
        // from the reconfigured module's own live map (kept in sync at every `:add`/edit/delete,
        // `confirm_add`) rather than trusting whatever `device.definitions` was seeded with
        // above, so a runtime-added interpretation never regresses to a stale on-disk snapshot.
        device.definitions = self.module.definitions();
        self.device = device;
        self.overlay.close();
    }

    /// MB-R-148 — `Enter` on a Resolved-registers row (with the panel focused) opens the
    /// edit/delete dialog prefilled from that row. No-op if nothing is selected in either table.
    fn open_edit_interpretation(&mut self) {
        let Some(unit) = self.selected_unit() else {
            return;
        };
        let Some(idx) = self.resolved_table.state.table_state().selected() else {
            return;
        };
        // UI-R-064's `:order` may have sorted `resolved_table` out of `interpretations_for`'s
        // definition order, so look up by the selected row's own `name`, not by `idx` into
        // `interpretations_for` directly.
        let Some(row) = self.resolved_table.state.values().get(idx) else {
            return;
        };
        let name = row.name.clone();
        let interpretations = self.interpretations_for(unit);
        let Some((_, def)) = interpretations.iter().find(|(n, _)| **n == name) else {
            return;
        };
        let dialog = EditInterpretationDialog::from_interpretation(&name, def);
        self.overlay = MonitorOverlay::EditInterpretation(Box::new(InterpretationEdit {
            dialog,
            original_name: name,
        }));
    }

    /// MB-R-148 — apply the open `EditInterpretation` overlay's Confirm: edit the interpretation
    /// in place under its (possibly new) name. Never touches `module.table()`: this operation
    /// does not write to the bus or otherwise touch the slave's observed-value table.
    /// No-op if the overlay isn't open, the dialog is invalid, or nothing is selected.
    fn confirm_edit_interpretation(&mut self) {
        let MonitorOverlay::EditInterpretation(edit) = &self.overlay else {
            return;
        };
        let original_name = edit.original_name.clone();
        let Ok((new_name, def)) = edit.dialog.apply() else {
            return;
        };
        let Some(unit) = self.selected_unit() else {
            return;
        };
        self.module
            .edit_interpretation(unit, &original_name, new_name, def);
        // See `confirm_add`.
        self.device.definitions = self.module.definitions();
        self.overlay.close();
    }

    /// MB-R-148 — apply the open `EditInterpretation` overlay's confirmed Delete: remove the
    /// interpretation outright. Never touches `module.table()`, same as `confirm_edit_interpretation`.
    /// No-op if the overlay isn't open or nothing is selected.
    fn delete_interpretation(&mut self) {
        let MonitorOverlay::EditInterpretation(edit) = &self.overlay else {
            return;
        };
        let Some(unit) = self.selected_unit() else {
            return;
        };
        self.module.remove_interpretation(unit, &edit.original_name);
        // See `confirm_add`.
        self.device.definitions = self.module.definitions();
        self.overlay.close();
    }

    /// Interpretations defined for `unit` (MB-R-145), by name, in definition order — the
    /// Resolved-registers table's own real column sort (UI-R-064) applies separately, on the
    /// built `ResolvedRow`s (see `resolved_rows`/`apply_order`).
    fn interpretations_for(&self, unit: UnitId) -> Vec<(&String, &MonitorRegisterDef)> {
        self.module
            .interpretations_for(unit)
            .iter()
            .map(|(name, def)| (name, def))
            .collect()
    }

    /// Whether `unit` has at least one interpretation (MB-R-145) — drives both the
    /// Resolved-registers section's visibility (UI-R-061) and whether the Resolved-registers
    /// panel is reachable in the Tab cycle (UI-R-065).
    fn has_interpretation(&self, unit: UnitId) -> bool {
        !self.interpretations_for(unit).is_empty()
    }

    /// UI-R-065 — whether the Resolved-registers panel is currently in the Tab cycle: it is
    /// shown, and therefore focusable, only while the selected unit id has at least one
    /// interpretation (UI-R-060).
    fn resolved_focusable(&self) -> bool {
        self.selected_unit()
            .is_some_and(|u| self.has_interpretation(u))
    }

    /// The selected unit id's `ResolvedRow`s (UI-R-064), definition order unless `self.sort` is
    /// set, in which case sorted by that column index via `cmp_table_entry`.
    fn resolved_rows(&self, unit: UnitId) -> Vec<ResolvedRow> {
        let table = self.module.table();
        let table = table.read();
        let mut rows: Vec<ResolvedRow> = self
            .interpretations_for(unit)
            .into_iter()
            .map(|(name, def)| resolved_row(name, def, &table, unit))
            .collect();
        if let Some((column, descending)) = self.sort {
            rows.sort_by(|a, b| {
                crate::module::modbus::table::cmp_table_entry(a, b, column, descending)
            });
        }
        rows
    }

    /// Validate `:order`'s optional column argument (tui/api-contract.md §2.1) against the
    /// Resolved-registers table's real columns and set the sort; `Err` for an unrecognized
    /// column name.
    fn apply_order(&mut self, col: &str, desc: bool) -> Result<(), String> {
        match crate::module::modbus::table::column_index_for::<ResolvedHeader, 9>(col) {
            Some(idx) => {
                self.sort = Some((idx, desc));
                Ok(())
            }
            None => Err(format!(
                "unknown column '{col}' (expected one of: {})",
                ResolvedHeader::header().join(", ")
            )),
        }
    }

    /// Memory layout for `unit`, grouped by table kind (MB-R-144), non-empty kinds only.
    fn memory_rows(&self, unit: UnitId) -> Vec<(Kind, Vec<(u16, u16)>)> {
        let table = self.module.table();
        let table = table.read();
        TABLE_KINDS
            .iter()
            .filter_map(|kind| {
                let key = Key::new(SlaveKey {
                    slave_id: unit,
                    kind: kind.clone(),
                });
                let dump = table.dump(&key);
                if dump.is_empty() {
                    None
                } else {
                    Some((kind.clone(), dump))
                }
            })
            .collect()
    }
}

impl ModuleView for ModbusMonitorModuleView {
    fn name(&self) -> String {
        self.spec.name.clone()
    }

    fn is_overlay_active(&self) -> bool {
        self.overlay.is_active()
    }

    fn render(&mut self, frame: &mut Frame, area: Rect) {
        use ratatui::layout::{Constraint, Layout};

        // A connection-status bar (CONNECTED/RECONNECTING/DISCONNECTED), same shape and position
        // as `ModbusModuleView`'s own: one line tall, below the module's own panels
        // (`content_area`). The app-level compositor then draws the shared log pane below that.
        let [content_area, status_area] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);

        // Every Tab-cyclable panel is a `Table` widget whose own border color tracks
        // `state.focused()` (`ferrowl_ui::style::TableStyle`'s `border`/`general`).
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Length(10), Constraint::Min(1)]).areas(content_area);

        let buf = frame.buffer_mut();

        self.units_table.state.set_values(
            self.unit_ids
                .iter()
                .map(|unit| UnitRow {
                    unit: unit.to_string(),
                })
                .collect(),
        );
        self.units_table.state.select_index(self.selected);
        StatefulWidget::render(
            &self.units_table.widget,
            left_area,
            buf,
            &mut self.units_table.state,
        );

        let selected = self.selected_unit();
        let has_interpretation = selected.is_some_and(|u| self.has_interpretation(u));

        let sections: Vec<Constraint> = if has_interpretation {
            vec![
                Constraint::Percentage(34),
                Constraint::Percentage(33),
                Constraint::Percentage(33),
            ]
        } else {
            vec![Constraint::Percentage(50), Constraint::Percentage(50)]
        };
        let section_areas = Layout::vertical(sections).split(right_area);

        // Message table (MB-R-143/146, UI-R-062).
        StatefulWidget::render(
            &self.messages_table.widget,
            section_areas[0],
            buf,
            &mut self.messages_table.state,
        );

        // Memory layout, grouped by table kind (MB-R-144; a real Table widget).
        if let Some(unit) = selected {
            let kind_rows = self.memory_rows(unit);
            let lines = memory_layout_lines(&kind_rows);
            let records = self.module.records().read().records_for(unit);
            self.memory_table.state.set_values(memory_table_rows(
                &lines,
                &records,
                std::time::Instant::now(),
            ));
        } else {
            self.memory_table.state.set_values(Vec::new());
        }
        StatefulWidget::render(
            &self.memory_table.widget,
            section_areas[1],
            buf,
            &mut self.memory_table.state,
        );

        // Resolved registers (MB-R-145, UI-R-064) — omitted entirely when no interpretation
        // exists for the selected unit id (UI-R-061). Rebuilt live here (not cached in
        // `refresh()`, unlike the Messages table) so a newly added interpretation shows up
        // immediately, without a fresh refresh tick — same invariant the previous
        // `Paragraph`-based render held.
        if has_interpretation && let Some(unit) = selected {
            self.resolved_table
                .state
                .set_values(self.resolved_rows(unit));
            StatefulWidget::render(
                &self.resolved_table.widget,
                section_areas[2],
                buf,
                &mut self.resolved_table.state,
            );
        }

        // MB-R-152 — `ModbusMonitorModule` has no `bound_addr` (RTU/ASCII monitor, not a TCP
        // server), so `render_status_bar` is passed `None` unconditionally.
        let status = self.module.connection_status();
        crate::view::status_bar::render_status_bar(status, None, status_area, buf);
    }

    fn render_overlay(&mut self, frame: &mut Frame, _area: Rect) {
        let full_area = frame.area();
        match &mut self.overlay {
            MonitorOverlay::Add(overlay) => overlay.render(full_area, frame.buffer_mut()),
            MonitorOverlay::EditSetup(dialog) => dialog.render(full_area, frame.buffer_mut()),
            MonitorOverlay::EditInterpretation(edit) => {
                edit.dialog.render(full_area, frame.buffer_mut())
            }
            MonitorOverlay::None => {}
        }
    }

    fn handle_events(&mut self, modifiers: KeyModifiers, code: KeyCode) -> EventResult {
        if let MonitorOverlay::EditSetup(dialog) = &mut self.overlay {
            match code {
                KeyCode::Enter => {
                    // Offer Enter to the dialog first, so a focused
                    // completion popup (config-path/serial-path) gets to accept its highlighted
                    // suggestion (UI-R-026) instead of Enter unconditionally confirming the whole
                    // dialog; only treat it as confirm once the dialog itself leaves it unhandled
                    // (mirrors the full modbus module's own `ModbusModuleView::handle_events`).
                    if let EventResult::Unhandled(..) = dialog.handle_events(modifiers, code) {
                        self.confirm_edit();
                    }
                }
                _ => {
                    let _ = dialog.handle_events(modifiers, code);
                }
            }
            if let MonitorOverlay::EditSetup(dialog) = &mut self.overlay
                && dialog.take_close_request()
            {
                self.overlay = MonitorOverlay::None;
            }
            return EventResult::Consumed;
        }
        let interpretation_dialog = match &mut self.overlay {
            // The `:add` dialog is never `deletable` (UI-R-061: nothing exists yet to delete),
            // so `confirm_delete` never actually opens here — the sub-popup gate is shared
            // anyway to mirror `MonitorOverlay::EditInterpretation`'s own shape exactly, rather
            // than special-casing Add's routing.
            MonitorOverlay::Add(overlay) => Some(overlay.as_mut()),
            MonitorOverlay::EditInterpretation(edit) => Some(&mut edit.dialog),
            _ => None,
        };
        if let Some(dialog) = interpretation_dialog {
            match route_interpretation_subpopups(dialog, modifiers, code) {
                Some(SubPopupOutcome::Delete) => {
                    self.delete_interpretation();
                    return EventResult::Consumed;
                }
                Some(SubPopupOutcome::Close) => {
                    self.overlay = MonitorOverlay::None;
                    return EventResult::Consumed;
                }
                Some(SubPopupOutcome::Consumed) => return EventResult::Consumed,
                None => {}
            }
            match self.overlay.route_keys(modifiers, code) {
                ferrowl_ui::traits::OverlayRoute::Closed
                | ferrowl_ui::traits::OverlayRoute::Cycled => return EventResult::Consumed,
                ferrowl_ui::traits::OverlayRoute::Unhandled => {}
            }
            // `route_keys` only ever mutates `self.overlay` on `Closed`, which already
            // returned above, so the variant is still `Add`/`EditInterpretation` here.
            let (dialog, is_add) = match &mut self.overlay {
                MonitorOverlay::Add(overlay) => (overlay.as_mut(), true),
                MonitorOverlay::EditInterpretation(edit) => (&mut edit.dialog, false),
                _ => unreachable!(
                    "route_keys left self.overlay as Add/EditInterpretation on Unhandled"
                ),
            };
            if route_interpretation_body(dialog, modifiers, code) {
                if is_add {
                    self.confirm_add();
                } else {
                    self.confirm_edit_interpretation();
                }
            }
            return EventResult::Consumed;
        }
        match code {
            KeyCode::Tab if modifiers == KeyModifiers::NONE => {
                self.focus_next();
                EventResult::Consumed
            }
            KeyCode::BackTab => {
                self.focus_previous();
                EventResult::Consumed
            }
            // UI-R-065 — Units shares the same `TableState::handle_events` navigation as
            // every other panel (Up/Down/PageUp/PageDown/Home/End/left-right scroll, whatever
            // the shared widget supports). `selected` stays the source of truth other panels key
            // off (which unit id's data they show), but input flows table -> selected: the table
            // handles the keypress first, then `selected` picks up the resulting
            // `table_state().selected()` index (the reverse of render's existing
            // `select_index(self.selected)` sync).
            _ if self.focus == ModbusMonitorModuleViewFocus::UnitsTable => {
                let result = self.units_table.state.handle_events(modifiers, code);
                if let Some(idx) = self.units_table.state.table_state().selected() {
                    self.selected = idx;
                }
                result
            }
            _ if self.focus == ModbusMonitorModuleViewFocus::MessagesTable => {
                self.messages_table.state.handle_events(modifiers, code)
            }
            KeyCode::Enter if self.focus == ModbusMonitorModuleViewFocus::ResolvedTable => {
                self.open_edit_interpretation();
                EventResult::Consumed
            }
            _ if self.focus == ModbusMonitorModuleViewFocus::ResolvedTable => {
                self.resolved_table.state.handle_events(modifiers, code)
            }
            _ if self.focus == ModbusMonitorModuleViewFocus::MemoryTable => {
                self.memory_table.state.handle_events(modifiers, code)
            }
            _ => EventResult::Unhandled(modifiers, code),
        }
    }

    fn refresh<'a>(&'a mut self) -> RefreshFuture<'a> {
        Box::pin(async move {
            {
                let table = self.module.table();
                let table = table.read();
                self.unit_ids = table.unit_ids();
            }
            if self.selected >= self.unit_ids.len() {
                self.selected = self.unit_ids.len().saturating_sub(1);
            }

            match self.selected_unit() {
                None => {
                    if self.cached_messages_unit.is_some() {
                        self.messages_table.state.set_values(Vec::new());
                        self.cached_messages_unit = None;
                        self.cached_messages_log = None;
                        self.cached_messages_generation = 0;
                    }
                }
                Some(unit) => {
                    let records_log = self.module.records();
                    // The cache is only valid for the exact same (unit, underlying RecordLog
                    // instance) it was last built from. A `:reload`/`confirm_edit` rebuilds
                    // `self.module`, so `self.module.records()` returns a clone of a *different*
                    // Arc afterward — its generation can coincidentally land at or above the old
                    // cached value even though every record it holds is new, so identity (`Arc::
                    // ptr_eq`), not just the generation number, is what must gate the cache.
                    let same_source = self.cached_messages_unit == Some(unit)
                        && self
                            .cached_messages_log
                            .as_ref()
                            .is_some_and(|prev| std::sync::Arc::ptr_eq(prev, &records_log));

                    // A single read guard spans both the generation read and the records read
                    // below (whichever branch runs) — the modbus receive-loop task pushes to this
                    // same `RecordLog` concurrently (it's on the app's multi-threaded runtime), so
                    // two separate `.read()` acquisitions would leave a window for a push to land
                    // between them: the generation observed would then be stale-low relative to
                    // the rows already fetched (which already include that push), producing a
                    // duplicate row once the next tick's delta re-fetches it. One guard held
                    // across both reads makes them see the same snapshot, atomically.
                    let guard = records_log.read();
                    let generation = guard.generation_for(unit);
                    let full_rebuild = !same_source || generation < self.cached_messages_generation;
                    if full_rebuild {
                        let now = std::time::Instant::now();
                        let wall_now = std::time::SystemTime::now();
                        let mut records = guard.records_for(unit);
                        drop(guard);
                        records.reverse(); // most-recent-first (UI-R-062)
                        let rows: Vec<MessageRow> = records
                            .iter()
                            .map(|record| message_row(unit, record, now, wall_now))
                            .collect();
                        self.messages_table.state.set_values(rows);
                    } else {
                        let delta = generation - self.cached_messages_generation;
                        if delta > 0 {
                            if delta as usize >= RECORD_RING_CAPACITY {
                                // Bigger than the whole ring since last tick — equivalent to a
                                // full rebuild (every currently-cached row would be stale/evicted
                                // anyway).
                                let now = std::time::Instant::now();
                                let wall_now = std::time::SystemTime::now();
                                let mut records = guard.records_for(unit);
                                drop(guard);
                                records.reverse();
                                let rows: Vec<MessageRow> = records
                                    .iter()
                                    .map(|record| message_row(unit, record, now, wall_now))
                                    .collect();
                                self.messages_table.state.set_values(rows);
                            } else {
                                let now = std::time::Instant::now();
                                let wall_now = std::time::SystemTime::now();
                                let mut new_records = guard.recent_for(unit, delta as usize);
                                drop(guard);
                                // most-recent-first, matching the cached rows' own order.
                                new_records.reverse();
                                let mut rows: Vec<MessageRow> = new_records
                                    .iter()
                                    .map(|record| message_row(unit, record, now, wall_now))
                                    .collect();
                                rows.extend(self.messages_table.state.values().iter().cloned());
                                rows.truncate(RECORD_RING_CAPACITY);
                                self.messages_table.state.set_values(rows);
                            }
                        } else {
                            drop(guard);
                        }
                        // delta == 0: nothing changed since the last tick — skip entirely, zero
                        // `message_row`/`format!` calls. This is the fix's whole point for an
                        // idle tab.
                    }
                    self.cached_messages_unit = Some(unit);
                    self.cached_messages_log = Some(records_log);
                    self.cached_messages_generation = generation;
                }
            }
        })
    }

    fn handle_command<'a>(&'a mut self, cmd: &'a str) -> CommandFuture<'a> {
        let Some(parsed) = parse_command(&MONITOR_COMMAND_SPECS, cmd) else {
            return Box::pin(std::future::ready(CommandResult::Unhandled));
        };

        match parsed {
            ModbusMonitorCmd::Start => Box::pin(async move {
                let endpoint = self.spec.endpoint.to_string();
                match self
                    .module
                    .start(move |_s: String| async {}, move |_s: String| async {})
                    .await
                {
                    Ok(()) => CommandResult::Handled(Some((
                        Level::Info,
                        format!("Started monitor on {endpoint}"),
                    ))),
                    Err(e) => CommandResult::Handled(Some((
                        Level::Error,
                        format!("Start monitor failed: {e}"),
                    ))),
                }
            }),

            ModbusMonitorCmd::Stop => Box::pin(async move {
                match self.module.stop().await {
                    Ok(()) => CommandResult::Handled(Some((Level::Info, "Stopped monitor".into()))),
                    Err(e) => CommandResult::Handled(Some((
                        Level::Error,
                        format!("Stop monitor failed: {e}"),
                    ))),
                }
            }),

            ModbusMonitorCmd::Restart => Box::pin(async move {
                let endpoint = self.spec.endpoint.to_string();
                let _ = self.module.stop().await;
                match self
                    .module
                    .start(move |_s: String| async {}, move |_s: String| async {})
                    .await
                {
                    Ok(()) => CommandResult::Handled(Some((
                        Level::Info,
                        format!("Restarted monitor on {endpoint}"),
                    ))),
                    Err(e) => CommandResult::Handled(Some((
                        Level::Error,
                        format!("Restart monitor failed: {e}"),
                    ))),
                }
            }),

            ModbusMonitorCmd::Reload => Box::pin(async move {
                if self.spec.device.is_empty() {
                    return CommandResult::Handled(Some((
                        Level::Warning,
                        "No configuration file path configured. Reload aborted.".into(),
                    )));
                }
                let path = self.spec.device.clone();
                let device: MonitorDeviceConfig = match crate::config::load_monitor_device(&path) {
                    Ok(d) => d,
                    Err(e) => {
                        return CommandResult::Handled(Some((
                            Level::Error,
                            format!(":reload failed to load '{path}': {e}"),
                        )));
                    }
                };
                let _ = self.module.stop().await;
                let new_module = ModbusMonitorModule::new(&self.spec, &device);
                self.module = new_module;
                self.device = device;
                // MB-R-150 — the fresh module's `serial_paths` defaults to a private registry
                // (`ModbusMonitorModule::new`); reattach the session-wide one so an in-progress
                // conflict survives `:reload` instead of silently clearing.
                self.module.set_serial_paths(self.serial_paths.clone());
                if let Err(e) = self
                    .module
                    .start(move |_s: String| async {}, move |_s: String| async {})
                    .await
                {
                    return CommandResult::Handled(Some((
                        Level::Error,
                        format!(":reload start error: {e}"),
                    )));
                }
                CommandResult::Handled(Some((Level::Info, format!(":reload done — '{path}'"))))
            }),

            ModbusMonitorCmd::Edit => {
                let dialog = MonitorSetupDialog::edit(&self.spec.name, &self.spec, &self.device);
                self.overlay = MonitorOverlay::EditSetup(Box::new(dialog));
                Box::pin(std::future::ready(CommandResult::Handled(None)))
            }

            // UI-R-061 — no unit id discovered yet means there's nothing to scope a new
            // interpretation to; reject with a warning instead of opening the dialog.
            ModbusMonitorCmd::Add => Box::pin(std::future::ready(if self.unit_ids.is_empty() {
                CommandResult::Handled(Some((
                    Level::Warning,
                    "Unable to add interpretations to unknown unit id.".into(),
                )))
            } else {
                self.overlay = MonitorOverlay::Add(Box::new(EditInterpretationDialog::new()));
                CommandResult::Handled(None)
            })),

            ModbusMonitorCmd::Compact => {
                self.compact = !self.compact;
                // Real row-padding toggle (UI-R-064), mirroring `TableView::set_compact`: one
                // row of vertical margin when expanded, none when compact.
                let vertical = if self.compact { 0 } else { 1 };
                self.resolved_table
                    .widget
                    .set_row_margin(ratatui::layout::Margin {
                        vertical,
                        horizontal: 0,
                    });
                Box::pin(std::future::ready(CommandResult::Handled(None)))
            }

            ModbusMonitorCmd::WriteDevice(rest) => {
                let path = rest.unwrap_or_else(|| self.spec.device.clone());
                if path.is_empty() {
                    return Box::pin(std::future::ready(CommandResult::Handled(Some((
                        Level::Warning,
                        "No configuration file path configured.".into(),
                    )))));
                }
                let result = save_device_to(&self.device, &path);
                Box::pin(std::future::ready(result))
            }

            ModbusMonitorCmd::Log(None) => Box::pin(std::future::ready(CommandResult::Unhandled)),

            ModbusMonitorCmd::Log(Some(file)) => {
                self.device.log_file = Some(file.clone());
                Box::pin(std::future::ready(CommandResult::Handled(Some((
                    Level::Info,
                    format!("Logging to files based on {file} (':wd' to persist)"),
                )))))
            }

            ModbusMonitorCmd::Order(rest) => {
                let parts: Vec<&str> = rest.as_deref().unwrap_or("").split_whitespace().collect();
                let result = match parts.as_slice() {
                    [] => {
                        self.sort = None;
                        CommandResult::Handled(Some((Level::Info, "Order cleared".to_string())))
                    }
                    [col] | [col, "asc"] => match self.apply_order(col, false) {
                        Ok(()) => CommandResult::Handled(None),
                        Err(e) => CommandResult::Handled(Some((Level::Warning, e))),
                    },
                    [col, "desc"] => match self.apply_order(col, true) {
                        Ok(()) => CommandResult::Handled(None),
                        Err(e) => CommandResult::Handled(Some((Level::Warning, e))),
                    },
                    _ => CommandResult::Unhandled,
                };
                Box::pin(std::future::ready(result))
            }
        }
    }

    fn commands(&self) -> &[CommandDescriptor] {
        static DESCRIPTORS: std::sync::OnceLock<Vec<CommandDescriptor>> =
            std::sync::OnceLock::new();
        DESCRIPTORS.get_or_init(|| MONITOR_COMMAND_SPECS.iter().map(|s| s.descriptor).collect())
    }

    fn log(&self) -> SharedLog {
        self.module.log()
    }

    fn session_spec(&self) -> Option<serde_json::Value> {
        let mut v = serde_json::to_value(&self.spec).ok()?;
        v.as_object_mut()?.insert("type".into(), "modbus".into());
        Some(v)
    }

    fn set_serial_paths(&mut self, registry: crate::module::modbus::SerialPathRegistry) {
        self.module.set_serial_paths(registry.clone());
        self.serial_paths = registry;
    }
}

/// The parsed form of every command this view accepts; produced by [`parse_command`] over
/// [`MONITOR_COMMAND_SPECS`]. `:set`/`:script` are simply absent from the table — `parse_command`
/// already falls through to `CommandResult::Unhandled` for anything not listed, which is exactly
/// tui/api-contract.md's "both are unrecognized on this role rather than erroring".
enum ModbusMonitorCmd {
    Start,
    Stop,
    Restart,
    Reload,
    Edit,
    Add,
    Compact,
    WriteDevice(Option<String>),
    Log(Option<String>),
    Order(Option<String>),
}

/// Single source for this view's commands: aliases, help row, and parse target per entry.
static MONITOR_COMMAND_SPECS: [CommandSpec<ModbusMonitorCmd>; 10] = [
    CommandSpec {
        aliases: &["e", "edit"],
        descriptor: CommandDescriptor {
            name: ":e | :edit",
            description: "edit module setup",
        },
        build: |_| ModbusMonitorCmd::Edit,
    },
    CommandSpec {
        aliases: &["a", "add"],
        descriptor: CommandDescriptor {
            name: ":a | :add",
            description: "add register interpretation",
        },
        build: |_| ModbusMonitorCmd::Add,
    },
    CommandSpec {
        aliases: &["start"],
        descriptor: CommandDescriptor {
            name: ":start",
            description: "start module",
        },
        build: |_| ModbusMonitorCmd::Start,
    },
    CommandSpec {
        aliases: &["stop"],
        descriptor: CommandDescriptor {
            name: ":stop",
            description: "stop module",
        },
        build: |_| ModbusMonitorCmd::Stop,
    },
    CommandSpec {
        aliases: &["restart"],
        descriptor: CommandDescriptor {
            name: ":restart",
            description: "restart module",
        },
        build: |_| ModbusMonitorCmd::Restart,
    },
    CommandSpec {
        aliases: &["reload"],
        descriptor: CommandDescriptor {
            name: ":reload",
            description: "reload device config",
        },
        build: |_| ModbusMonitorCmd::Reload,
    },
    CommandSpec {
        aliases: &["compact"],
        descriptor: CommandDescriptor {
            name: ":compact",
            description: "toggle compact mode",
        },
        build: |_| ModbusMonitorCmd::Compact,
    },
    CommandSpec {
        aliases: &["wd", "write-device"],
        descriptor: CommandDescriptor {
            name: ":wd | :write-device [path]",
            description: "save device config",
        },
        build: |rest| ModbusMonitorCmd::WriteDevice(rest.map(str::to_string)),
    },
    CommandSpec {
        aliases: &["log"],
        descriptor: CommandDescriptor {
            name: ":log <file>",
            description: "set log file",
        },
        build: |rest| ModbusMonitorCmd::Log(rest.map(str::to_string)),
    },
    CommandSpec {
        aliases: &["order"],
        descriptor: CommandDescriptor {
            name: ":order [col] [asc|desc]",
            description: "sort table by column",
        },
        build: |rest| ModbusMonitorCmd::Order(rest.map(str::to_string)),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Endpoint, Role};
    use ferrowl_modbus::UnitId;
    use ferrowl_test_support::reserve_temp_dir;
    use ferrowl_ui::traits::SetFocus;
    use ferrowl_ui::widgets::TableEntry as TableEntryTrait;

    fn spec() -> ModuleSpec {
        ModuleSpec {
            name: "mon1".to_string(),
            device: String::new(),
            role: Role::Monitor,
            endpoint: Endpoint::Rtu {
                path: "/dev/none".to_string(),
                baud_rate: 9600,
                parity: None,
                data_bits: None,
                stop_bits: None,
            },
        }
    }

    fn device() -> MonitorDeviceConfig {
        MonitorDeviceConfig::default()
    }

    fn view() -> ModbusMonitorModuleView {
        let module = ModbusMonitorModule::new(&spec(), &device());
        ModbusMonitorModuleView::new(module, spec(), device())
    }

    /// UI-R-060 — the left panel's `unit_ids` refresh from the observed table, live.
    #[tokio::test]
    async fn ut_refresh_populates_unit_ids_from_observed_table() {
        let mut v = view();
        {
            let table = v.module.table();
            let mut table = table.write();
            table.write_words(
                Key::new(SlaveKey {
                    slave_id: UnitId(3),
                    kind: Kind::HoldingRegister,
                }),
                0,
                &[1],
            );
        }
        v.refresh().await;
        assert_eq!(v.unit_ids, vec![UnitId(3)]);
    }

    /// The Units panel is a real Table/TableEntry: exactly 1 column ("Unit"), rows
    /// track `unit_ids`, and the highlighted row tracks `selected` (`selected` itself remains
    /// the single source of truth — the table only mirrors it for rendering).
    #[test]
    fn ut_units_panel_is_a_real_table_tracking_unit_ids_and_selected() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        assert_eq!(UnitHeader::header(), ["Unit".to_string()]);

        let mut v = view();
        v.unit_ids = vec![UnitId(1), UnitId(3)];
        v.selected = 1;

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                v.render(frame, area);
            })
            .unwrap();

        let rows = v.units_table.state.values();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].unit, "1");
        assert_eq!(rows[1].unit, "3");
        assert_eq!(v.units_table.state.table_state().selected(), Some(1));
    }

    /// The Units panel disables the shared `Table` widget's selection bar
    /// (its one narrow "Unit" column already conveys focus via the selected row's own
    /// background/foreground change).
    #[test]
    fn ut_units_panel_disables_selection_marker() {
        let v = view();
        assert!(!v.units_table.widget.show_selection_marker());
    }

    /// UI-R-065 — Units shares the same `TableState::handle_events` navigation keys as every
    /// other panel; Down, `G` and Home are the ones exercised here. `selected` (which drives
    /// which unit id's data the other panels show) follows the table's own resulting selection
    /// index.
    #[test]
    fn ut_units_panel_navigation_matches_other_panels_table_state_keys() {
        let mut v = view();
        v.set_focused(true);
        v.unit_ids = vec![UnitId(1), UnitId(3), UnitId(5)];
        v.selected = 0;
        v.focus = ModbusMonitorModuleViewFocus::UnitsTable;
        // Render once so `units_table.state.values()` is populated (mirrors real event loop:
        // render() -> handle_events() -> render() ...).
        {
            use ratatui::Terminal;
            use ratatui::backend::TestBackend;
            let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
            terminal
                .draw(|frame| v.render(frame, frame.area()))
                .unwrap();
        }

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Down);
        assert_eq!(
            v.units_table.state.table_state().selected(),
            Some(1),
            "Down must move the table's own selection"
        );
        assert_eq!(
            v.selected, 1,
            "selected must follow the table's resulting index"
        );

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Char('G'));
        assert_eq!(
            v.units_table.state.table_state().selected(),
            Some(2),
            "'G' (move_to_bottom) must be handled just like on Messages/Resolved"
        );
        assert_eq!(v.selected, 2);

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Home);
        // Home is `move_to_left` (horizontal scroll), not a row-selection key — it must still be
        // Consumed by the table (not fall through to Unhandled), and must leave `selected` (a
        // row index) untouched.
        assert_eq!(v.selected, 2);
    }

    /// UI-R-065 — Memory delegates navigation straight to its own `TableState`, same as
    /// Messages/Resolved; `Down` is the key exercised here.
    #[test]
    fn ut_memory_panel_navigation_delegates_to_its_own_table_state() {
        let mut v = view();
        v.set_focused(true);
        v.unit_ids = vec![UnitId(1)];
        v.selected = 0;
        v.focus = ModbusMonitorModuleViewFocus::MemoryTable;
        // Two writes far enough apart to span multiple 8-address hex-editor lines (MB-R-144), so
        // the Memory table has more than one row to navigate.
        v.module.table().write().write_words(
            Key::new(SlaveKey {
                slave_id: UnitId(1),
                kind: Kind::HoldingRegister,
            }),
            0,
            &[1, 2, 3],
        );
        v.module.table().write().write_words(
            Key::new(SlaveKey {
                slave_id: UnitId(1),
                kind: Kind::HoldingRegister,
            }),
            20,
            &[4, 5, 6],
        );
        {
            use ratatui::Terminal;
            use ratatui::backend::TestBackend;
            let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
            terminal
                .draw(|frame| v.render(frame, frame.area()))
                .unwrap();
        }

        let before = v.memory_table.state.table_state().selected();
        let result = ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Down);
        assert!(
            matches!(result, EventResult::Consumed),
            "Memory must consume Down instead of falling through to Unhandled"
        );
        assert_ne!(
            v.memory_table.state.table_state().selected(),
            before,
            "Down must move the Memory table's own selection"
        );
    }

    /// A CONNECTED/RECONNECTING/DISCONNECTED status bar,
    /// same shape as `ModbusModuleView`'s own (`module/modbus/view/mod.rs`): centered,
    /// one line tall, `COLOR_SCHEME.success`/`warning`/`error` background, positioned below the
    /// module's own panels (which the outer app-level compositor then draws the shared log pane
    /// below in turn, `app/render.rs`'s `[view_area, log_area]` split — no extra coordination
    /// needed here). `ModbusMonitorModule` has no `bound_addr` (RTU/ASCII monitor, not a TCP
    /// server), so this drives off `connection_status()` instead and always shows the bare
    /// label (no address to append).
    #[test]
    fn ut_render_shows_disconnected_status_bar() {
        use ferrowl_ui::COLOR_SCHEME;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut v = view();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| v.render(frame, frame.area()))
            .unwrap();
        let last_row = terminal.backend().buffer().area.height - 1;
        let contents: String = (0..120)
            .map(|x| {
                terminal.backend().buffer()[(x, last_row)]
                    .symbol()
                    .to_string()
            })
            .collect();
        assert!(
            contents.contains("DISCONNECTED"),
            "not-yet-started monitor must show DISCONNECTED: {contents:?}"
        );
        assert_eq!(
            terminal.backend().buffer()[(0, last_row)].bg,
            COLOR_SCHEME.error,
            "DISCONNECTED row must use the error background"
        );
    }

    /// MB-R-152 — a monitor whose serial port never opens (bad path, `reconnect: true`) shows
    /// RECONNECTING (not DISCONNECTED) while its task keeps retrying, mirroring `module.rs`'s
    /// own `ut_monitor_module_start_stop_lifecycle` fixture.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn it_monitor_view_shows_reconnecting_while_port_open_fails() {
        use ferrowl_ui::COLOR_SCHEME;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut device = device();
        device.reconnect = Some(true);
        let module = ModbusMonitorModule::new(&spec(), &device);
        let mut v = ModbusMonitorModuleView::new(module, spec(), device);
        v.module
            .start(|_: String| async {}, |_: String| async {})
            .await
            .expect("start always succeeds for a valid transport");
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| v.render(frame, frame.area()))
            .unwrap();
        let last_row = terminal.backend().buffer().area.height - 1;
        let contents: String = (0..120)
            .map(|x| {
                terminal.backend().buffer()[(x, last_row)]
                    .symbol()
                    .to_string()
            })
            .collect();
        assert!(
            contents.contains("RECONNECTING"),
            "backing-off monitor must show RECONNECTING: {contents:?}"
        );
        assert_eq!(
            terminal.backend().buffer()[(0, last_row)].bg,
            COLOR_SCHEME.warning,
            "RECONNECTING row must use the warning background"
        );

        v.module.stop().await.expect("cleanup stop");
    }

    /// UI-R-061, UI-R-100 — the resolved-registers section is omitted entirely from the rendered buffer
    /// when no interpretation exists for the selected unit id, and reappears once one does.
    #[test]
    fn ut_resolved_registers_section_hidden_when_no_interpretation_for_selected_unit() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;

        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                v.render(frame, area);
            })
            .unwrap();
        let contents =
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .fold(String::new(), |mut acc, cell| {
                    acc.push_str(cell.symbol());
                    acc
                });
        assert!(!contents.contains("Resolved registers"));

        v.module.add_interpretation(
            UnitId(3),
            "power".to_string(),
            MonitorRegisterDef {
                name: "power".to_string(),
                slave_id: 3,
                kind: Kind::HoldingRegister,
                address: Some(0),
                is_virtual: false,
                value_type: crate::config::device::ValueType::U16,
                endian: Default::default(),
                word_order: Default::default(),
                resolution: 1.0,
                bitmask: None,
                length: 1,
                alignment: Default::default(),
                values: vec![],
                description: String::new(),
                default: None,
            },
        );

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                v.render(frame, area);
            })
            .unwrap();
        let contents =
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .fold(String::new(), |mut acc, cell| {
                    acc.push_str(cell.symbol());
                    acc
                });
        assert!(contents.contains("Resolved registers"));
    }

    /// Panel titles carry no requirement IDs — a spec ID in the rendered UI means nothing to
    /// the application's user.
    #[test]
    fn ut_panel_titles_have_no_requirement_ids() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, "Active power draw"));
        let contents = buffer_text(&mut v);
        assert!(contents.contains("Messages"));
        assert!(contents.contains("Memory layout"));
        assert!(contents.contains("Resolved registers"));
        assert!(!contents.contains("MB-R-143"));
        assert!(!contents.contains("MB-R-144"));
        assert!(!contents.contains("MB-R-145"));
    }

    /// A panel's border tracks the view's own focus, switching between `COLOR_SCHEME::border`
    /// and `COLOR_SCHEME::hi`; the Units panel's corner is the cell asserted here.
    #[test]
    fn ut_panel_borders_track_view_focus() {
        use ferrowl_ui::COLOR_SCHEME;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut v = view();
        v.set_focused(false);
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                v.render(frame, area);
            })
            .unwrap();
        assert_eq!(
            terminal.backend().buffer()[(0, 0)].fg,
            COLOR_SCHEME.border,
            "unfocused panel border must use the unfocused border color"
        );

        v.set_focused(true);
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                v.render(frame, area);
            })
            .unwrap();
        assert_eq!(
            terminal.backend().buffer()[(0, 0)].fg,
            COLOR_SCHEME.hi,
            "focused panel border must use the highlight color"
        );
    }

    /// UI-R-065 — a freshly constructed, unfocused view highlights neither the Units nor the
    /// Messages panel; once focused, only Units is highlighted, and Messages must not carry a
    /// stale `focused` default into that first render.
    #[test]
    fn ut_fresh_view_highlights_neither_units_nor_messages_until_focused() {
        use ferrowl_ui::COLOR_SCHEME;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut v = view();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                v.render(frame, area);
            })
            .unwrap();
        assert_eq!(
            terminal.backend().buffer()[(0, 0)].fg,
            COLOR_SCHEME.border,
            "an unfocused fresh view must not highlight the Units panel"
        );
        assert_eq!(
            terminal.backend().buffer()[(10, 0)].fg,
            COLOR_SCHEME.border,
            "an unfocused fresh view must not highlight the Messages panel"
        );

        v.set_focused(true);
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                v.render(frame, area);
            })
            .unwrap();
        assert_eq!(
            terminal.backend().buffer()[(0, 0)].fg,
            COLOR_SCHEME.hi,
            "Units panel is the initially-focused pane once the view is focused"
        );
        assert_eq!(
            terminal.backend().buffer()[(10, 0)].fg,
            COLOR_SCHEME.border,
            "Messages panel must not be highlighted before Tab ever moves focus onto it"
        );
    }

    /// Exactly one Tab-cyclable panel is highlighted at a time, defaulting to
    /// Units, and Tab/BackTab cycle Units -> Messages -> Memory -> Units (and back).
    #[test]
    fn ut_tab_cycles_panel_focus_units_messages_memory_and_back() {
        use ferrowl_ui::COLOR_SCHEME;

        let mut v = view();
        v.set_focused(true);
        assert_eq!(v.focus, ModbusMonitorModuleViewFocus::UnitsTable);

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        assert_eq!(v.focus, ModbusMonitorModuleViewFocus::MessagesTable);

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        assert_eq!(v.focus, ModbusMonitorModuleViewFocus::MemoryTable);

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        assert_eq!(
            v.focus,
            ModbusMonitorModuleViewFocus::UnitsTable,
            "Tab must wrap back to Units"
        );

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::BackTab);
        assert_eq!(
            v.focus,
            ModbusMonitorModuleViewFocus::MemoryTable,
            "BackTab must cycle in reverse"
        );

        // The currently panel-focused block's top-left border cell is highlighted; a
        // non-focused panel's is not.
        v.set_focused(false);
        v.focus = ModbusMonitorModuleViewFocus::UnitsTable;
        v.set_focused(true);
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                v.render(frame, area);
            })
            .unwrap();
        assert_eq!(
            terminal.backend().buffer()[(0, 0)].fg,
            COLOR_SCHEME.hi,
            "Units panel border must be highlighted while it is panel-focused"
        );
        assert_eq!(
            terminal.backend().buffer()[(10, 0)].fg,
            COLOR_SCHEME.border,
            "Messages panel border must not be highlighted while Units is panel-focused"
        );
    }

    /// tui/api-contract.md's monitor command table — `:set`/`:script` are simply absent, so
    /// both fall through `parse_command` to `Unhandled` rather than erroring.
    #[tokio::test]
    async fn ut_set_and_script_commands_unrecognized() {
        let mut v = view();
        assert!(matches!(
            v.handle_command("set foo 1").await,
            CommandResult::Unhandled
        ));
        assert!(matches!(
            v.handle_command("script").await,
            CommandResult::Unhandled
        ));
    }

    /// Every entry in `MONITOR_COMMAND_SPECS` routes through `handle_command` without panicking.
    #[tokio::test]
    async fn ut_every_monitor_command_spec_has_matching_handler() {
        let mut v = view();
        for spec in &MONITOR_COMMAND_SPECS {
            for alias in spec.aliases {
                let _ = v.handle_command(alias).await;
            }
        }
    }

    /// `commands()`/`log()` delegate to the command table and the module's own log ring.
    #[test]
    fn ut_monitor_view_commands_and_log_delegate() {
        let v = view();
        assert_eq!(v.commands().len(), MONITOR_COMMAND_SPECS.len());
        let _log: SharedLog = v.log();
    }

    #[test]
    /// UI-R-101 — the monitor tab's log pane is the monitor module's own ring, not a copy.
    fn ut_monitor_view_log_is_the_modules_own_ring() {
        let v = view();
        assert!(
            std::sync::Arc::ptr_eq(&v.log(), &v.module.log()),
            "the tab's log pane must show the monitor module's own ring, not a copy"
        );
    }

    /// MB-R-148 — once "Add predefined" opens the named-value sub-popup, keyboard input (typed
    /// characters, Tab/BackTab) reaches the sub-popup's own fields, not the parent
    /// `EditInterpretationDialog`'s (mirrors the modbus module's own `RegisterDialog`
    /// sub-dialog routing via its `overlay.has_sub_dialog()` gate).
    #[tokio::test]
    async fn ut_add_predefined_popup_receives_keyboard_focus() {
        let mut v = view();
        v.unit_ids.push(UnitId(1));
        v.handle_command("add").await;
        let MonitorOverlay::Add(overlay) = &mut v.overlay else {
            panic!(":add did not open the interpretation dialog");
        };
        let dialog = overlay.as_mut();
        dialog.open_add_dialog();
        assert!(dialog.add_dialog.is_some());
        let parent_label_before = dialog.label.state.input().to_string();

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Char('x'));

        let MonitorOverlay::Add(overlay) = &mut v.overlay else {
            panic!("overlay changed unexpectedly");
        };
        let dialog = overlay.as_mut();
        assert_eq!(
            dialog
                .add_dialog
                .as_ref()
                .expect("sub-popup stays open")
                .label
                .state
                .input(),
            "x",
            "the typed character reaches the sub-popup's own label field"
        );
        assert_eq!(
            dialog.label.state.input(),
            &parent_label_before,
            "the parent dialog's own label field is untouched while the sub-popup is open"
        );

        // Tab/BackTab route into the sub-popup too, not the parent dialog's own
        // `#[derive(Focus)]` cycle.
        assert!(dialog.label.state.focused());
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        let MonitorOverlay::Add(overlay) = &v.overlay else {
            panic!("overlay changed unexpectedly");
        };
        let sub = overlay.add_dialog.as_ref().expect("sub-popup stays open");
        assert!(
            sub.value.state.focused(),
            "Tab must move focus inside the sub-popup, onto its value field"
        );
        assert!(
            !overlay.description.state.focused(),
            "the parent dialog's own Tab cycle must not advance while the sub-popup is open"
        );

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::BackTab);
        let MonitorOverlay::Add(overlay) = &v.overlay else {
            panic!("overlay changed unexpectedly");
        };
        let sub = overlay.add_dialog.as_ref().expect("sub-popup stays open");
        assert!(
            sub.label.state.focused(),
            "BackTab must cycle focus back onto the sub-popup's label field"
        );
    }

    /// UI-R-112, UI-R-113, UI-E-061 — `Esc` on a freshly opened (nothing typed into it)
    /// `:add` interpretation dialog opens its close-confirm popup instead of closing the
    /// overlay outright; the confirmation is unconditional (no dirty-tracking).
    #[tokio::test]
    async fn ut_esc_on_add_interpretation_dialog_opens_close_confirm() {
        let mut v = view();
        v.unit_ids.push(UnitId(1));
        v.handle_command("add").await;
        assert!(matches!(v.overlay, MonitorOverlay::Add(_)));

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);

        let MonitorOverlay::Add(overlay) = &v.overlay else {
            panic!("Esc must not close the :add interpretation overlay directly");
        };
        assert!(overlay.close_confirm.is_some());
    }

    /// UI-R-112, UI-R-113 — same as above, for the edit/delete interpretation overlay.
    #[tokio::test]
    async fn ut_esc_on_edit_interpretation_dialog_opens_close_confirm() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, ""));
        buffer_text(&mut v);
        v.open_edit_interpretation();
        assert!(matches!(v.overlay, MonitorOverlay::EditInterpretation(..)));

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);

        let MonitorOverlay::EditInterpretation(edit) = &v.overlay else {
            panic!("Esc must not close the edit/delete interpretation overlay directly");
        };
        assert!(edit.dialog.close_confirm.is_some());
    }

    /// UI-R-112, UI-R-023 — confirming the close-confirm popup (`Enter`) closes the `:add`
    /// interpretation overlay.
    #[tokio::test]
    async fn ut_close_confirm_enter_closes_add_interpretation_overlay() {
        let mut v = view();
        v.unit_ids.push(UnitId(1));
        v.handle_command("add").await;
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        assert!(matches!(v.overlay, MonitorOverlay::None));
    }

    /// UI-R-112, UI-R-023 — dismissing the close-confirm popup (`Esc`) returns to the still-open
    /// `:add` interpretation dialog.
    #[tokio::test]
    async fn ut_esc_in_close_confirm_returns_to_add_interpretation_dialog() {
        let mut v = view();
        v.unit_ids.push(UnitId(1));
        v.handle_command("add").await;
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);

        let MonitorOverlay::Add(overlay) = &v.overlay else {
            panic!("Esc in the close-confirm popup must not close the overlay");
        };
        assert!(overlay.close_confirm.is_none());
    }

    /// UI-R-112, UI-R-023 — same as above, for the edit/delete interpretation overlay.
    #[tokio::test]
    async fn ut_close_confirm_enter_closes_edit_interpretation_overlay() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, ""));
        buffer_text(&mut v);
        v.open_edit_interpretation();
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        assert!(matches!(v.overlay, MonitorOverlay::None));
    }

    /// UI-R-112, UI-R-023 — same as above, for the edit/delete interpretation overlay.
    #[tokio::test]
    async fn ut_esc_in_close_confirm_returns_to_edit_interpretation_dialog() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, ""));
        buffer_text(&mut v);
        v.open_edit_interpretation();
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);

        let MonitorOverlay::EditInterpretation(edit) = &v.overlay else {
            panic!("Esc in the close-confirm popup must not close the overlay");
        };
        assert!(edit.dialog.close_confirm.is_none());
    }

    /// UI-R-112 — `Esc` on the monitor setup-edit overlay opens its close-confirm popup (`CLOSE`
    /// button rendered) instead of closing the overlay directly.
    #[tokio::test]
    async fn ut_esc_on_monitor_setup_dialog_opens_close_confirm() {
        let mut v = view();
        v.handle_command("edit").await;
        assert!(matches!(v.overlay, MonitorOverlay::EditSetup(_)));

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);

        assert!(
            matches!(v.overlay, MonitorOverlay::EditSetup(_)),
            "Esc must not close the setup-edit overlay directly"
        );
        let text = overlay_text(&mut v);
        assert!(
            text.contains("CLOSE"),
            "close-confirm popup not rendered:\n{text}"
        );
    }

    /// UI-R-112, UI-R-023 — confirming the close-confirm popup (`Enter`) closes the setup-edit
    /// overlay.
    #[tokio::test]
    async fn ut_close_confirm_enter_closes_monitor_setup_overlay() {
        let mut v = view();
        v.handle_command("edit").await;
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        assert!(matches!(v.overlay, MonitorOverlay::None));
    }

    /// UI-R-112, UI-R-023 — dismissing the close-confirm popup (`Esc`) returns to the still-open
    /// setup-edit dialog.
    #[tokio::test]
    async fn ut_esc_in_close_confirm_returns_to_monitor_setup_dialog() {
        let mut v = view();
        v.handle_command("edit").await;
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);

        assert!(matches!(v.overlay, MonitorOverlay::EditSetup(_)));
        let text = overlay_text(&mut v);
        assert!(
            !text.contains("CLOSE"),
            "close-confirm popup must be dismissed:\n{text}"
        );
    }

    /// UI-R-113 — the interpretation dialog's close-confirm popup renders over the dialog
    /// itself (drawn last).
    #[tokio::test]
    async fn ut_close_confirm_renders_over_interpretation_dialog() {
        let mut v = view();
        v.unit_ids.push(UnitId(1));
        v.handle_command("add").await;
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);

        let text = overlay_text(&mut v);
        assert!(
            text.contains("CLOSE"),
            "close-confirm popup not rendered:\n{text}"
        );
    }

    /// UI-R-022 — within the `:add` interpretation dialog, `Tab` advances focus to the next
    /// field and `BackTab` retreats, cycling.
    #[tokio::test]
    async fn ut_tab_cycles_focus_inside_add_interpretation_dialog() {
        let mut v = view();
        v.unit_ids.push(UnitId(1));
        v.handle_command("add").await;
        let MonitorOverlay::Add(overlay) = &v.overlay else {
            panic!(":add did not open the interpretation dialog");
        };
        assert!(overlay.label.state.focused());
        assert!(!overlay.description.state.focused());

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        let MonitorOverlay::Add(overlay) = &v.overlay else {
            panic!("overlay changed unexpectedly");
        };
        assert!(!overlay.label.state.focused());
        assert!(overlay.description.state.focused());

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::BackTab);
        let MonitorOverlay::Add(overlay) = &v.overlay else {
            panic!("overlay changed unexpectedly");
        };
        assert!(overlay.label.state.focused());
        assert!(!overlay.description.state.focused());
    }

    /// UI-R-061, UI-E-062 — while the "Add predefined" sub-popup is open, `Esc` dismisses only
    /// the popup; the `:add` interpretation dialog behind it stays open.
    #[tokio::test]
    async fn ut_esc_in_add_predefined_popup_closes_only_the_popup() {
        let mut v = view();
        v.unit_ids.push(UnitId(1));
        v.handle_command("add").await;
        let MonitorOverlay::Add(overlay) = &mut v.overlay else {
            panic!(":add did not open the interpretation dialog");
        };
        overlay.open_add_dialog();
        assert!(overlay.add_dialog.is_some());

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Esc);

        let MonitorOverlay::Add(overlay) = &v.overlay else {
            panic!("Esc in the sub-popup must not close the add dialog");
        };
        assert!(overlay.add_dialog.is_none());
    }

    /// UI-R-061 — with no unit id discovered yet, `:add` is rejected (Warning) instead of
    /// opening the dialog: there is no unit id to scope a new interpretation to.
    #[tokio::test]
    async fn ut_add_command_rejects_when_no_unit_id_discovered() {
        let mut v = view();
        assert!(v.unit_ids.is_empty());

        let result = v.handle_command("add").await;

        assert!(matches!(
            result,
            CommandResult::Handled(Some((Level::Warning, _)))
        ));
        assert!(
            matches!(v.overlay, MonitorOverlay::None),
            ":add must not open the dialog with no unit id to scope it to"
        );
    }

    /// UI-R-061 — `:add` scopes the new interpretation to the currently selected unit id; the
    /// dialog never asks for one, and other unit ids' interpretation sets are untouched. Also
    /// confirms the dialog opens as a fresh, non-`deletable` `EditInterpretationDialog` —
    /// the crux of "the add dialog matches the edit dialog except for prefill".
    #[tokio::test]
    async fn ut_add_command_scopes_new_interpretation_to_selected_unit_id() {
        use crate::module::modbus::dialog::set_input;

        let mut v = view();
        v.unit_ids = vec![UnitId(1), UnitId(3)];
        v.selected = 1; // unit 3

        v.handle_command("add").await;
        let MonitorOverlay::Add(overlay) = &mut v.overlay else {
            panic!(":add did not open the interpretation dialog");
        };
        let dialog = overlay.as_mut();
        assert!(
            !dialog.deletable,
            "the :add dialog must open with deletable == false"
        );
        set_input(&mut dialog.label, "power");
        set_input(&mut dialog.address, "10");
        v.confirm_add();

        assert_eq!(v.interpretations_for(UnitId(3)).len(), 1);
        assert_eq!(v.interpretations_for(UnitId(3))[0].0, "power");
        assert!(v.interpretations_for(UnitId(1)).is_empty());
        assert!(matches!(v.overlay, MonitorOverlay::None));
    }

    /// `:add` must sync the
    /// new interpretation into `self.device.definitions` too, not just `self.module`'s live
    /// map, or `:write` silently drops it (it saves `self.device` verbatim).
    #[tokio::test]
    async fn ut_confirm_add_syncs_device_definitions() {
        use crate::module::modbus::dialog::set_input;

        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;

        v.handle_command("add").await;
        let MonitorOverlay::Add(overlay) = &mut v.overlay else {
            panic!(":add did not open the interpretation dialog");
        };
        let dialog = overlay.as_mut();
        set_input(&mut dialog.label, "power");
        set_input(&mut dialog.address, "10");
        v.confirm_add();

        assert!(
            v.device.definitions.iter().any(|d| d.name == "power"),
            "device.definitions must be kept in sync so ':write' persists it"
        );
    }

    /// UI-R-061 — `Enter` on the `:add` dialog's focused Confirm button, routed as a real key
    /// through `ModuleView::handle_events` (not `v.confirm_add()` called directly), must commit
    /// the dialog.
    #[tokio::test]
    async fn ut_enter_on_add_confirm_button_commits_via_handle_events() {
        use crate::module::modbus::dialog::set_input;

        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;

        v.handle_command("add").await;
        let MonitorOverlay::Add(overlay) = &mut v.overlay else {
            panic!(":add did not open the interpretation dialog");
        };
        let dialog = overlay.as_mut();
        set_input(&mut dialog.label, "power");
        set_input(&mut dialog.address, "10");

        // Tab to the Confirm button, wherever it sits in the (kind-dependent) focus cycle.
        for _ in 0..30 {
            let MonitorOverlay::Add(overlay) = &v.overlay else {
                panic!("overlay changed unexpectedly");
            };
            if overlay.is_confirm_button_focused() {
                break;
            }
            ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        }
        let MonitorOverlay::Add(overlay) = &v.overlay else {
            panic!("overlay changed unexpectedly");
        };
        assert!(
            overlay.is_confirm_button_focused(),
            "Tab must be able to reach the Confirm button"
        );

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        assert!(
            matches!(v.overlay, MonitorOverlay::None),
            "Enter on the focused Confirm button must commit :add and close the overlay"
        );
        assert_eq!(v.interpretations_for(UnitId(3))[0].0, "power");
    }

    /// UI-R-022 — a key that is neither `Enter` nor `Space` (and no sub-popup is open) falls
    /// through to the focused field itself, routed as a real key through
    /// `ModuleView::handle_events` — it must not commit or close the `:add` overlay.
    #[tokio::test]
    async fn ut_typed_char_in_add_dialog_reaches_focused_field_without_committing() {
        let mut v = view();
        v.unit_ids.push(UnitId(1));
        v.handle_command("add").await;
        let MonitorOverlay::Add(overlay) = &v.overlay else {
            panic!(":add did not open the interpretation dialog");
        };
        assert!(
            overlay.label.state.focused(),
            "the dialog opens with Label focused"
        );

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Char('p'));

        let MonitorOverlay::Add(overlay) = &v.overlay else {
            panic!("overlay changed unexpectedly");
        };
        assert_eq!(
            overlay.label.state.input(),
            "p",
            "the typed character must reach the focused Label field"
        );
        assert!(
            matches!(v.overlay, MonitorOverlay::Add(_)),
            "a plain typed character must not commit or close the :add overlay"
        );
    }

    /// MB-R-148's edit/delete
    /// must also keep `self.device.definitions` in sync (rename moves the key, delete removes
    /// it), same parity requirement as `:add`.
    #[tokio::test]
    async fn ut_confirm_edit_interpretation_syncs_device_definitions() {
        use crate::module::modbus::dialog::set_input;

        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, ""));
        v.device.definitions.push(MonitorRegisterDef {
            name: "power".to_string(),
            ..def(10, "")
        });
        buffer_text(&mut v);

        v.open_edit_interpretation();
        let MonitorOverlay::EditInterpretation(edit) = &mut v.overlay else {
            panic!("Enter did not open the edit/delete dialog")
        };
        set_input(&mut edit.dialog.label, "power2");
        v.confirm_edit_interpretation();

        assert!(
            !v.device.definitions.iter().any(|d| d.name == "power"),
            "the old name must not linger in device.definitions after a rename"
        );
        assert!(
            v.device.definitions.iter().any(|d| d.name == "power2"),
            "the renamed interpretation must be present in device.definitions"
        );
    }

    /// MB-R-148 — `Enter` on the edit/delete dialog's focused Confirm button, routed as a real
    /// key through `ModuleView::handle_events` (not `v.confirm_edit_interpretation()` called
    /// directly), must commit the rename in place.
    #[tokio::test]
    async fn ut_enter_on_edit_interpretation_confirm_button_commits_via_handle_events() {
        use crate::module::modbus::dialog::set_input;

        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, ""));
        buffer_text(&mut v);

        v.open_edit_interpretation();
        let MonitorOverlay::EditInterpretation(edit) = &mut v.overlay else {
            panic!("Enter did not open the edit/delete dialog")
        };
        set_input(&mut edit.dialog.label, "power2");

        for _ in 0..30 {
            let MonitorOverlay::EditInterpretation(edit) = &v.overlay else {
                panic!("overlay changed unexpectedly");
            };
            if edit.dialog.is_confirm_button_focused() {
                break;
            }
            ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        }
        let MonitorOverlay::EditInterpretation(edit) = &v.overlay else {
            panic!("overlay changed unexpectedly");
        };
        assert!(
            edit.dialog.is_confirm_button_focused(),
            "Tab must be able to reach the Confirm button"
        );

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        assert!(
            matches!(v.overlay, MonitorOverlay::None),
            "Enter on the focused Confirm button must commit the rename and close the overlay"
        );
        assert_eq!(v.module.interpretations_for(UnitId(3))[0].0, "power2");
    }

    /// Deleting an
    /// interpretation must remove it from `self.device.definitions`, not just the module's
    /// in-memory map, or a stale copy resurfaces on the next `:write`.
    #[tokio::test]
    async fn ut_delete_interpretation_syncs_device_definitions() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, ""));
        v.device.definitions.push(MonitorRegisterDef {
            name: "power".to_string(),
            ..def(10, "")
        });
        buffer_text(&mut v);

        v.open_edit_interpretation();
        v.delete_interpretation();

        assert!(
            !v.device.definitions.iter().any(|d| d.name == "power"),
            "device.definitions must drop the deleted interpretation too"
        );
    }

    /// End to end: an
    /// interpretation added purely at runtime (never in the file `:o`/`:oc` originally loaded)
    /// must actually appear in the file after `:write`, not be silently dropped.
    #[tokio::test]
    async fn ut_write_device_command_persists_runtime_added_interpretation() {
        use crate::module::modbus::dialog::set_input;
        use ferrowl_util::convert::{Converter, FileType};

        let dir = reserve_temp_dir("ferrowl_modbus_monitor_view");
        let path = dir.join("write-test.toml");
        let path_str = path.to_str().unwrap().to_string();

        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;

        v.handle_command("add").await;
        let MonitorOverlay::Add(overlay) = &mut v.overlay else {
            panic!(":add did not open the interpretation dialog");
        };
        let dialog = overlay.as_mut();
        set_input(&mut dialog.label, "power");
        set_input(&mut dialog.address, "10");
        v.confirm_add();

        v.handle_command(&format!("wd {path_str}")).await;

        let loaded: MonitorDeviceConfig =
            Converter::load(&path_str, FileType::Toml).expect("save must succeed");
        assert!(
            loaded.definitions.iter().any(|d| d.name == "power"),
            "an interpretation added purely at runtime must be persisted by :write"
        );
    }

    /// MB-R-150 — `:reload` rebuilds `self.module` fresh; the session-wide serial-path registry
    /// attached via `set_serial_paths` must carry over to that fresh instance, not reset to a
    /// private default (which would silently drop an in-progress conflict on reload).
    #[tokio::test]
    async fn ut_reload_carries_serial_paths_registry_to_new_module() {
        use crate::module::modbus::SerialPathRegistry;

        let dir = reserve_temp_dir("ferrowl_modbus_monitor_view");
        let path = dir.join("reload-serial-paths.toml");
        let p = path.to_str().expect("temp path is valid UTF-8").to_string();

        // Produce a loadable device config file via the already-tested :write-device path.
        let mut writer = view();
        let _ = writer.handle_command(&format!("write-device {p}")).await;

        let serial_path = "/nonexistent/mb-r-150-monitor-reload";
        let mut s = spec();
        s.name = "A".into();
        s.device = p.clone();
        s.endpoint = Endpoint::Rtu {
            path: serial_path.into(),
            baud_rate: 9600,
            parity: None,
            data_bits: None,
            stop_bits: None,
        };
        let module = ModbusMonitorModule::new(&s, &device());
        let mut v = ModbusMonitorModuleView::new(module, s, device());

        // Another instance ("B") already claims the same serial path in a session-wide registry.
        let registry = SerialPathRegistry::new();
        registry.claim("B", serial_path);
        v.set_serial_paths(registry.clone());

        let _ = v.handle_command("reload").await;
        assert_eq!(
            registry.conflict("B", serial_path),
            Some("A".to_string()),
            "reload's fresh module lost the session-wide registry (claimed on a private default \
             instead)"
        );

        let _ = v.handle_command("stop").await;
    }

    /// MB-R-150 — the Edit-confirm reconfigure path (`confirm_edit`) also rebuilds `self.module`
    /// fresh; it must carry the session-wide registry over too, since the resulting module is
    /// the one a later `:start` runs (not merely a throwaway preview instance). A claim is only
    /// recorded on `start()` (mirroring `ModbusMonitorModule`'s own contract), so this test
    /// confirm-edits and then explicitly starts, same as a real user would.
    #[tokio::test]
    async fn ut_confirm_edit_carries_serial_paths_registry_to_new_module() {
        use crate::module::modbus::SerialPathRegistry;

        // `spec()`'s own Rtu endpoint path doubles as the conflicting serial path; the edit
        // dialog is confirmed unedited (same name/endpoint), matching a no-op re-confirm.
        let s = spec();
        let serial_path = match &s.endpoint {
            Endpoint::Rtu { path, .. } => path.clone(),
            other => panic!("fixture spec() must be Rtu, got {other:?}"),
        };
        let module = ModbusMonitorModule::new(&s, &device());
        let mut v = ModbusMonitorModuleView::new(module, s.clone(), device());

        let registry = SerialPathRegistry::new();
        registry.claim("B", &serial_path);
        v.set_serial_paths(registry.clone());

        v.overlay =
            MonitorOverlay::EditSetup(Box::new(MonitorSetupDialog::edit(&s.name, &s, &device())));
        v.confirm_edit();
        let _ = v.handle_command("start").await;

        assert_eq!(
            registry.conflict("B", &serial_path),
            Some(s.name.clone()),
            "confirm_edit's fresh module lost the session-wide registry (claimed on a private \
             default instead)"
        );

        let _ = v.handle_command("stop").await;
    }

    /// MB-R-145 — a newly added interpretation is reflected in the resolved-registers section
    /// immediately, without a fresh unit-id selection round-trip.
    #[tokio::test]
    async fn ut_resolved_registers_table_reflects_new_interpretation_immediately() {
        use crate::module::modbus::dialog::set_input;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;

        v.handle_command("add").await;
        let MonitorOverlay::Add(overlay) = &mut v.overlay else {
            panic!(":add did not open the interpretation dialog");
        };
        let dialog = overlay.as_mut();
        set_input(&mut dialog.label, "power");
        set_input(&mut dialog.address, "10");
        v.confirm_add();

        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                v.render(frame, area);
            })
            .unwrap();
        let contents =
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .fold(String::new(), |mut acc, cell| {
                    acc.push_str(cell.symbol());
                    acc
                });
        assert!(contents.contains("Resolved registers"));
        assert!(contents.contains("power"));
    }

    /// MB-R-140 — `:edit` opens the setup dialog prefilled from the current spec, and confirming
    /// it rebuilds `spec`/`device`/`module` from the resolved values.
    #[tokio::test]
    async fn ut_edit_command_opens_setup_dialog_prefilled_and_reconfigures_on_confirm() {
        let mut v = view();
        v.handle_command("edit").await;
        let MonitorOverlay::EditSetup(dialog) = &v.overlay else {
            panic!(":edit did not open the setup dialog");
        };
        assert_eq!(dialog.path.state.input(), "/dev/none");

        let MonitorOverlay::EditSetup(dialog) = &mut v.overlay else {
            unreachable!()
        };
        crate::dialog::widgets::set_input(&mut dialog.name, "renamed");
        v.confirm_edit();

        assert_eq!(v.spec.name, "renamed");
        assert!(matches!(v.overlay, MonitorOverlay::None));
    }

    /// `confirm_edit` carries the typed device-config path through to `spec.device`.
    #[tokio::test]
    async fn ut_edit_confirm_applies_device_path_field() {
        let mut v = view();
        v.handle_command("edit").await;
        let MonitorOverlay::EditSetup(dialog) = &mut v.overlay else {
            panic!(":edit did not open the setup dialog");
        };
        crate::dialog::widgets::set_suggest_input(&mut dialog.config_path, "new-device.toml");
        v.confirm_edit();

        assert_eq!(v.spec.device, "new-device.toml");
    }

    /// UI-R-026, UI-R-080 — `Enter` while the config-path field's completion popup is open must
    /// accept the highlighted suggestion, not unconditionally confirm the whole setup
    /// dialog; mirrors the modbus module's own `ModbusModuleView::handle_events`, which offers
    /// `Enter` to `setup.handle_events` before falling back to its own confirm handling.
    #[tokio::test]
    async fn ut_edit_setup_enter_accepts_suggestion_instead_of_confirming_when_popup_open() {
        let mut v = view();
        v.handle_command("edit").await;
        let MonitorOverlay::EditSetup(dialog) = &mut v.overlay else {
            panic!(":edit did not open the setup dialog");
        };
        dialog.focus_next(); // Name -> ConfigPath
        dialog
            .config_path
            .state
            .handle_events(KeyModifiers::NONE, KeyCode::Char('s'));
        assert!(dialog.config_path.state.suggestions_open());

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        let MonitorOverlay::EditSetup(dialog) = &v.overlay else {
            panic!("dialog must stay open, unconfirmed, while the popup handles Enter");
        };
        // The `ferrowl` crate root (test cwd) has an `src/` dir, a partial match for "s" that
        // re-queries and stays open (UI-R-026) — were `Enter` routed straight to `confirm_edit`,
        // the field would never see it and would stay at "s".
        assert_eq!(
            dialog.config_path.state.input(),
            "src/",
            "Enter must be offered to the dialog first so it can accept the suggestion"
        );
    }

    /// Editing setup must not reset the running monitor's
    /// accumulated state: known unit ids' observed table, message records, and interpretations
    /// added at runtime (MB-R-148's `:add`/`:edit`) all survive an edit-confirm.
    #[tokio::test]
    async fn ut_edit_confirm_preserves_accumulated_monitor_state() {
        let mut v = view();
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, ""));
        let table_before = v.module.table();
        let records_before = v.module.records();

        v.handle_command("edit").await;
        let MonitorOverlay::EditSetup(dialog) = &mut v.overlay else {
            panic!(":edit did not open the setup dialog");
        };
        crate::dialog::widgets::set_input(&mut dialog.name, "renamed");
        v.confirm_edit();

        assert!(
            std::sync::Arc::ptr_eq(&table_before, &v.module.table()),
            "table must be the same shared instance across an edit-confirm"
        );
        assert!(
            std::sync::Arc::ptr_eq(&records_before, &v.module.records()),
            "records must be the same shared instance across an edit-confirm"
        );
        assert_eq!(
            v.module.interpretations_for(UnitId(3)).len(),
            1,
            "an interpretation added at runtime must survive an edit-confirm"
        );
    }

    fn def(address: u16, description: &str) -> MonitorRegisterDef {
        MonitorRegisterDef {
            name: String::new(),
            slave_id: 3,
            kind: Kind::HoldingRegister,
            address: Some(address),
            is_virtual: false,
            value_type: crate::config::device::ValueType::U16,
            endian: Default::default(),
            word_order: Default::default(),
            resolution: 1.0,
            bitmask: None,
            length: 1,
            alignment: Default::default(),
            values: vec![],
            description: description.to_string(),
            default: None,
        }
    }

    fn buffer_text(v: &mut ModbusMonitorModuleView) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                v.render(frame, area);
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .fold(String::new(), |mut acc, cell| {
                acc.push_str(cell.symbol());
                acc
            })
    }

    fn overlay_text(v: &mut ModbusMonitorModuleView) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                ModuleView::render_overlay(v, frame, area);
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .fold(String::new(), |mut acc, cell| {
                acc.push_str(cell.symbol());
                acc
            })
    }

    /// tui/api-contract.md §2.1 — `:compact` toggles the resolved-registers table's row padding:
    /// no vertical row margin when compact, one row of it when expanded.
    #[tokio::test]
    async fn ut_compact_command_toggles_resolved_table_row_margin() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, "Active power draw"));

        assert!(v.compact);
        assert_eq!(v.resolved_table.widget.row_margin().vertical, 0);

        v.handle_command("compact").await;
        assert!(!v.compact);
        assert_eq!(v.resolved_table.widget.row_margin().vertical, 1);

        v.handle_command("compact").await;
        assert!(v.compact);
        assert_eq!(v.resolved_table.widget.row_margin().vertical, 0);
    }

    /// tui/api-contract.md §2.1/UI-R-064 — `:order <col> [asc|desc]` sorts the
    /// Resolved-registers table by the real column index; bare `:order` clears the sort back to
    /// definition order.
    #[tokio::test]
    async fn ut_order_command_sorts_resolved_table_by_column_index() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "low".to_string(), def(1, "low addr"));
        v.module
            .add_interpretation(UnitId(3), "high".to_string(), def(99, "high addr"));

        v.handle_command("order address desc").await;
        buffer_text(&mut v); // renders, which rebuilds `resolved_table`'s rows under the sort
        let names: Vec<String> = v
            .resolved_table
            .state
            .values()
            .iter()
            .map(|r| r.values()[0].clone())
            .collect();
        assert_eq!(names, vec!["high".to_string(), "low".to_string()]);

        v.handle_command("order address asc").await;
        buffer_text(&mut v);
        let names: Vec<String> = v
            .resolved_table
            .state
            .values()
            .iter()
            .map(|r| r.values()[0].clone())
            .collect();
        assert_eq!(names, vec!["low".to_string(), "high".to_string()]);

        v.handle_command("order").await;
        assert!(v.sort.is_none());
    }

    /// tui/api-contract.md §2.1 — an unrecognized `:order` column is reported (Warning), not
    /// silently accepted, and leaves the current sort untouched.
    #[tokio::test]
    async fn ut_order_command_rejects_unknown_column() {
        let mut v = view();
        let result = v.handle_command("order bogus").await;
        assert!(matches!(
            result,
            CommandResult::Handled(Some((Level::Warning, _)))
        ));
        assert!(v.sort.is_none());
    }

    /// Write a word into `unit`'s observed table so `refresh()`'s own `unit_ids` re-derivation
    /// (which runs before the Messages table re-derivation) keeps `unit` selectable, matching
    /// `ut_refresh_populates_unit_ids_from_observed_table`'s own fixture pattern.
    fn seed_unit(v: &ModbusMonitorModuleView, unit: UnitId) {
        v.module.table().write().write_words(
            Key::new(SlaveKey {
                slave_id: unit,
                kind: Kind::HoldingRegister,
            }),
            0,
            &[0],
        );
    }

    fn shaped_record(
        status: RecordStatus,
        operation: ferrowl_modbus::FunctionCode,
        shape: Option<ferrowl_modbus::monitor::TableShape>,
        age: std::time::Duration,
    ) -> MonitorRecord {
        MonitorRecord {
            timestamp: std::time::Instant::now() - age,
            status,
            operation,
            shape,
        }
    }

    fn shape(
        kind: Kind,
        address: u16,
        quantity: u16,
        write_address: Option<u16>,
        write_quantity: Option<u16>,
        values: Vec<u16>,
    ) -> ferrowl_modbus::monitor::TableShape {
        ferrowl_modbus::monitor::TableShape {
            kind,
            address,
            quantity,
            write_address,
            write_quantity,
            values,
        }
    }

    /// UI-R-062 — one row per captured record, most-recent-first, in the fixed 7-column order
    /// (Time, Status, Slave, Operation, Address, Quantity, Values/Payload).
    #[tokio::test]
    async fn ut_messages_table_renders_records_most_recent_first_in_fixed_column_order() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        seed_unit(&v, UnitId(3));
        let older = shaped_record(
            RecordStatus::Ok,
            ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
            Some(shape(Kind::HoldingRegister, 10, 1, None, None, vec![7])),
            std::time::Duration::from_secs(5),
        );
        let newer = shaped_record(
            RecordStatus::Ok,
            ferrowl_modbus::FunctionCode::WriteSingleRegister,
            Some(shape(Kind::HoldingRegister, 20, 1, None, None, vec![9])),
            std::time::Duration::from_secs(1),
        );
        v.module.records().write().push(UnitId(3), older);
        v.module.records().write().push(UnitId(3), newer);

        v.refresh().await;

        let rows = v.messages_table.state.values();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].values()[3], "WriteSingleRegister");
        assert_eq!(rows[0].values()[4], "20");
        assert_eq!(rows[1].values()[3], "ReadHoldingRegisters");
        assert_eq!(rows[1].values()[4], "10");
    }

    /// Visual improvement — the Status column colors "OK" green and any exception red, so the
    /// two are distinguishable at a glance, not just by reading the text. `Unmatched` gets no
    /// override (default text color) — not asked for.
    #[test]
    fn ut_message_row_styles_colors_ok_green_and_exception_red() {
        use ferrowl_ui::COLOR_SCHEME;
        use ratatui::style::Style;

        let now = std::time::Instant::now();
        let wall_now = std::time::SystemTime::now();

        let ok_row = message_row(
            UnitId(3),
            &shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
                None,
                std::time::Duration::from_secs(1),
            ),
            now,
            wall_now,
        );
        let styles = message_row_styles(&ok_row);
        assert_eq!(styles[1], Some(Style::default().fg(COLOR_SCHEME.success)));
        assert!(
            styles
                .iter()
                .enumerate()
                .all(|(i, s)| i == 1 || s.is_none()),
            "only the Status column should carry an override style"
        );

        let exception_row = message_row(
            UnitId(3),
            &shaped_record(
                RecordStatus::Exception(ferrowl_modbus::ExceptionCode::IllegalDataAddress),
                ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
                None,
                std::time::Duration::from_secs(1),
            ),
            now,
            wall_now,
        );
        assert_eq!(
            message_row_styles(&exception_row)[1],
            Some(Style::default().fg(COLOR_SCHEME.error))
        );

        let unmatched_row = message_row(
            UnitId(3),
            &shaped_record(
                RecordStatus::Unmatched,
                ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
                None,
                std::time::Duration::from_secs(1),
            ),
            now,
            wall_now,
        );
        assert_eq!(message_row_styles(&unmatched_row)[1], None);
    }

    /// UI-R-102 — a register-shaped record's Values/Payload renders 4-digit lowercase hex per
    /// word.
    #[tokio::test]
    async fn ut_messages_table_formats_register_payload_as_hex_words() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        seed_unit(&v, UnitId(3));
        v.module.records().write().push(
            UnitId(3),
            shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
                Some(shape(
                    Kind::HoldingRegister,
                    0,
                    2,
                    None,
                    None,
                    vec![0x00AB, 0x1234],
                )),
                std::time::Duration::from_millis(100),
            ),
        );

        v.refresh().await;

        let rows = v.messages_table.state.values();
        assert_eq!(rows[0].values()[6], "[00ab 1234]");
    }

    /// UI-R-102 — a coil-shaped record's Values/Payload renders one digit per bit.
    #[tokio::test]
    async fn ut_messages_table_formats_coil_payload_as_bit_digits() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        seed_unit(&v, UnitId(3));
        v.module.records().write().push(
            UnitId(3),
            shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::ReadCoils,
                Some(shape(Kind::Coil, 0, 3, None, None, vec![1, 0, 1])),
                std::time::Duration::from_millis(100),
            ),
        );

        v.refresh().await;

        let rows = v.messages_table.state.values();
        assert_eq!(rows[0].values()[6], "[1 0 1]");
    }

    /// (perf, no spec ID) — a second `refresh()` tick with no new records pushed since the first
    /// leaves the Messages table's rows unchanged (the generation-gated skip must not corrupt or
    /// duplicate rows).
    #[tokio::test]
    async fn ut_refresh_skips_rebuild_when_generation_unchanged() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        seed_unit(&v, UnitId(3));
        v.module.records().write().push(
            UnitId(3),
            shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
                Some(shape(Kind::HoldingRegister, 10, 1, None, None, vec![7])),
                std::time::Duration::from_millis(100),
            ),
        );

        v.refresh().await;
        let first: Vec<[String; 7]> = v
            .messages_table
            .state
            .values()
            .iter()
            .map(|r| r.values())
            .collect();
        v.refresh().await;
        let second: Vec<[String; 7]> = v
            .messages_table
            .state
            .values()
            .iter()
            .map(|r| r.values())
            .collect();
        assert_eq!(first, second);
    }

    /// (perf, no spec ID) — a `refresh()` tick after new records were pushed appends only the new
    /// rows (most-recent-first), leaving the previously-rendered rows' content untouched.
    #[tokio::test]
    async fn ut_refresh_incremental_appends_only_new_records_most_recent_first() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        seed_unit(&v, UnitId(3));
        v.module.records().write().push(
            UnitId(3),
            shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
                Some(shape(Kind::HoldingRegister, 10, 1, None, None, vec![7])),
                std::time::Duration::from_secs(5),
            ),
        );
        v.module.records().write().push(
            UnitId(3),
            shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::WriteSingleRegister,
                Some(shape(Kind::HoldingRegister, 20, 1, None, None, vec![9])),
                std::time::Duration::from_secs(1),
            ),
        );
        v.refresh().await;
        let rows = v.messages_table.state.values();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].values()[3], "WriteSingleRegister");
        assert_eq!(rows[1].values()[3], "ReadHoldingRegisters");
        let previous_rows: Vec<[String; 7]> = rows.iter().map(|r| r.values()).collect();

        v.module.records().write().push(
            UnitId(3),
            shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::ReadCoils,
                Some(shape(Kind::Coil, 0, 1, None, None, vec![1])),
                std::time::Duration::from_millis(100),
            ),
        );
        v.refresh().await;
        let rows = v.messages_table.state.values();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].values()[3], "ReadCoils", "newest record first");
        let trailing: Vec<[String; 7]> = rows[1..].iter().map(|r| r.values()).collect();
        assert_eq!(
            trailing, previous_rows,
            "previously-rendered rows must stay byte-identical, not reformatted"
        );
    }

    /// (perf, no spec ID) — switching the selected unit id forces a full rebuild (the cache is
    /// scoped to a single unit id at a time), rather than showing the previously selected unit's
    /// leftover cached rows.
    #[tokio::test]
    async fn ut_refresh_full_rebuild_when_selected_unit_changes() {
        let mut v = view();
        seed_unit(&v, UnitId(3));
        seed_unit(&v, UnitId(5));
        v.module.records().write().push(
            UnitId(3),
            shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
                Some(shape(Kind::HoldingRegister, 1, 1, None, None, vec![1])),
                std::time::Duration::from_millis(100),
            ),
        );
        v.module.records().write().push(
            UnitId(5),
            shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::ReadInputRegisters,
                Some(shape(Kind::InputRegister, 2, 1, None, None, vec![2])),
                std::time::Duration::from_millis(100),
            ),
        );
        v.selected = 0;
        v.refresh().await;
        assert_eq!(v.selected_unit(), Some(UnitId(3)));
        assert_eq!(
            v.messages_table.state.values()[0].values()[3],
            "ReadHoldingRegisters"
        );

        v.selected = 1;
        v.refresh().await;
        assert_eq!(v.selected_unit(), Some(UnitId(5)));
        let rows = v.messages_table.state.values();
        assert_eq!(
            rows.len(),
            1,
            "unit 5's own single record, not unit 3's leftover cache"
        );
        assert_eq!(rows[0].values()[3], "ReadInputRegisters");
    }

    /// (perf, no spec ID) — pushing more records than the ring's own capacity between two
    /// `refresh()` ticks forces a full rebuild (the incremental delta path is unsafe once the
    /// delta meets/exceeds `RECORD_RING_CAPACITY` — every cached row would be stale/evicted
    /// anyway); the ring's own 200-cap (MB-R-146) is still respected.
    #[tokio::test]
    async fn ut_refresh_full_rebuild_when_ring_exceeds_capacity_since_last_tick() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        seed_unit(&v, UnitId(3));
        v.module.records().write().push(
            UnitId(3),
            shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
                Some(shape(Kind::HoldingRegister, 0, 1, None, None, vec![0])),
                std::time::Duration::from_secs(10),
            ),
        );
        v.refresh().await;
        assert_eq!(v.messages_table.state.values().len(), 1);

        for i in 0..RECORD_RING_CAPACITY {
            v.module.records().write().push(
                UnitId(3),
                shaped_record(
                    RecordStatus::Ok,
                    ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
                    Some(shape(
                        Kind::HoldingRegister,
                        i as u16,
                        1,
                        None,
                        None,
                        vec![i as u16],
                    )),
                    std::time::Duration::from_millis(1),
                ),
            );
        }
        v.refresh().await;
        let rows = v.messages_table.state.values();
        assert_eq!(rows.len(), RECORD_RING_CAPACITY, "ring cap respected");
        assert_eq!(
            rows[0].values()[4],
            (RECORD_RING_CAPACITY - 1).to_string(),
            "newest pushed record renders first"
        );
    }

    /// (perf, no spec ID) — a module replacement (mirrors what `:reload`/`confirm_edit` do: a
    /// fresh `ModbusMonitorModule`, hence a fresh, empty `RecordLog` whose generation drops back
    /// to 0) must not panic (no `u64` underflow computing the generation delta) and must show
    /// only the fresh module's own records, not a stale mix with the old cache.
    #[tokio::test]
    async fn ut_refresh_handles_generation_drop_after_module_replacement_without_panicking() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        seed_unit(&v, UnitId(3));
        v.module.records().write().push(
            UnitId(3),
            shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
                Some(shape(Kind::HoldingRegister, 0, 1, None, None, vec![0])),
                std::time::Duration::from_millis(100),
            ),
        );
        v.refresh().await;
        assert_eq!(v.messages_table.state.values().len(), 1);

        v.module = ModbusMonitorModule::new(&spec(), &device());
        seed_unit(&v, UnitId(3));
        v.module.records().write().push(
            UnitId(3),
            shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::ReadCoils,
                Some(shape(Kind::Coil, 0, 1, None, None, vec![1])),
                std::time::Duration::from_millis(100),
            ),
        );
        v.refresh().await;
        let rows = v.messages_table.state.values();
        assert_eq!(
            rows.len(),
            1,
            "fresh module's own single record, not a stale mix"
        );
        assert_eq!(rows[0].values()[3], "ReadCoils");
    }

    /// (perf, no spec ID) — `refresh()` must read a unit's generation and its
    /// records/recent-tail off exactly *one* `RecordLog` read guard, not two separate
    /// acquisitions. Two separate acquisitions leave a window where a concurrent push (the
    /// modbus receive-loop task runs on the same multi-threaded runtime and pushes independently
    /// of the view's tick) can land between them: the cached generation observed would then be
    /// stale-low relative to the rows already fetched (which already include that push), so the
    /// next tick's delta computation re-fetches and prepends a record that's already present —
    /// a duplicate row lingering until ring-cap eviction. Drives real concurrent pushes from a
    /// spawned OS thread (not a cooperative async task — the actual race is at the
    /// `parking_lot::RwLock` level, independent of the tokio scheduler) while repeatedly calling
    /// `refresh()`, and asserts no duplicate address ever appears in the final Messages table.
    #[tokio::test]
    async fn ut_refresh_never_duplicates_rows_under_concurrent_pushes() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        seed_unit(&v, UnitId(3));

        const PUSHES: u16 = 2000;
        let records_log = v.module.records();
        let pusher = std::thread::spawn(move || {
            for i in 0..PUSHES {
                records_log.write().push(
                    UnitId(3),
                    shaped_record(
                        RecordStatus::Ok,
                        ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
                        Some(shape(Kind::HoldingRegister, i, 1, None, None, vec![i])),
                        std::time::Duration::from_millis(0),
                    ),
                );
            }
        });

        for _ in 0..PUSHES {
            v.refresh().await;
        }
        pusher.join().unwrap();
        v.refresh().await;

        let rows = v.messages_table.state.values();
        let addresses: Vec<String> = rows.iter().map(|r| r.values()[4].clone()).collect();
        let unique: std::collections::HashSet<&String> = addresses.iter().collect();
        assert_eq!(
            addresses.len(),
            unique.len(),
            "no duplicate address should ever appear in the Messages table despite concurrent \
             pushes racing refresh()'s reads: {addresses:?}"
        );
    }

    /// The Time column renders the full wall-clock timestamp (same
    /// `crate::view::log::format_timestamp` format the log pane already uses), not a relative
    /// "Xs ago" string.
    #[test]
    fn ut_message_row_time_renders_full_timestamp_not_relative_ago() {
        let now = std::time::Instant::now();
        let wall_now = std::time::SystemTime::now();
        let record = shaped_record(
            RecordStatus::Ok,
            ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
            None,
            std::time::Duration::from_secs(5),
        );
        let row = message_row(UnitId(3), &record, now, wall_now);

        let elapsed = now.duration_since(record.timestamp);
        let expected_wall = wall_now
            .checked_sub(elapsed)
            .expect("test elapsed fits in SystemTime");
        let ms = expected_wall
            .duration_since(std::time::UNIX_EPOCH)
            .expect("post-epoch")
            .as_millis() as u64;
        let expected = crate::view::log::format_timestamp(ms);

        assert_eq!(row.values()[0], expected);
        assert!(!row.values()[0].contains("ago"));
    }

    /// The Time column's `#[column(min, max)]` bounds are wide enough for
    /// `format_timestamp`'s full `"YYYY-MM-DD HH:MM:SS.mmm"` output (23 chars) to render on one
    /// line, not wrap/truncate.
    #[test]
    fn ut_message_time_column_width_fits_full_timestamp() {
        let full_timestamp_len = crate::view::log::format_timestamp(0).chars().count();
        let time_width = &MessageHeader::widths()[0];
        assert!(
            time_width.min >= full_timestamp_len && time_width.max >= full_timestamp_len,
            "Time column (min={}, max={}) too narrow for a {}-char timestamp",
            time_width.min,
            time_width.max,
            full_timestamp_len
        );
    }

    /// The Status column's exception case renders just the bare `ExceptionCode`
    /// variant name (`IllegalDataAddress`), not the Debug-derived `Exception(...)` wrapper.
    #[test]
    fn ut_message_row_exception_status_renders_bare_variant_name() {
        let now = std::time::Instant::now();
        let wall_now = std::time::SystemTime::now();
        let record = shaped_record(
            RecordStatus::Exception(ferrowl_modbus::ExceptionCode::IllegalDataAddress),
            ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
            None,
            std::time::Duration::from_millis(10),
        );
        let row = message_row(UnitId(3), &record, now, wall_now);
        assert_eq!(row.values()[1], "IllegalDataAddress");
    }

    /// UI-R-102 — a record whose operation isn't one of the 9 table-shaping ops renders empty
    /// Address/Quantity/Values-Payload columns.
    #[tokio::test]
    async fn ut_messages_table_non_table_shaping_operation_has_empty_shape_columns() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        seed_unit(&v, UnitId(3));
        v.module.records().write().push(
            UnitId(3),
            shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::MaskWriteRegister,
                None,
                std::time::Duration::from_millis(100),
            ),
        );

        v.refresh().await;

        let rows = v.messages_table.state.values();
        assert_eq!(rows[0].values()[4], "");
        assert_eq!(rows[0].values()[5], "");
        assert_eq!(rows[0].values()[6], "");
    }

    /// Edge-cases.md's Monitor boundaries row — `ReadWriteMultipleRegisters`'s shape carries both
    /// its own read and write address/quantity pairs; the table renders them slash-separated.
    #[tokio::test]
    async fn ut_messages_table_read_write_multiple_registers_renders_slash_separated_address_and_quantity()
     {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        seed_unit(&v, UnitId(3));
        v.module.records().write().push(
            UnitId(3),
            shaped_record(
                RecordStatus::Ok,
                ferrowl_modbus::FunctionCode::ReadWriteMultipleRegisters,
                Some(shape(
                    Kind::HoldingRegister,
                    10,
                    2,
                    Some(50),
                    Some(2),
                    vec![11, 22],
                )),
                std::time::Duration::from_millis(100),
            ),
        );

        v.refresh().await;

        let rows = v.messages_table.state.values();
        assert_eq!(rows[0].values()[4], "10/50");
        assert_eq!(rows[0].values()[5], "2/2");
    }

    /// UI-R-103 (horizontal-overflow scrolling reuses `TableState::handle_events` unchanged)
    /// — `Left`/`Right` reach the Messages table's own scroll handling when it's the
    /// panel-focused one, not the view's own unhandled fallback.
    #[test]
    fn ut_messages_panel_routes_left_right_to_table_horizontal_scroll() {
        let mut v = view();
        v.focus = ModbusMonitorModuleViewFocus::MessagesTable;
        let result = ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Right);
        assert!(matches!(result, EventResult::Consumed));
    }

    /// UI-R-105 — two far-apart observed addresses produce only their own two lines, not every
    /// unobserved line in between.
    #[test]
    fn ut_memory_layout_omits_unobserved_lines_renders_observed_ones() {
        let lines = memory_lines(Kind::HoldingRegister, &[(0, 1), (20, 2)]);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, 0);
        assert_eq!(lines[1].0, 16); // address 20's line starts at cell 16 (8 words/line)
    }

    /// UI-R-105 — within an otherwise-observed line, a cell with no observed constituent
    /// bit/word renders unobserved, not a silent zero.
    #[test]
    fn ut_memory_layout_unobserved_cell_renders_as_dim_placeholder_not_zero() {
        let lines = memory_lines(Kind::HoldingRegister, &[(0, 7)]);
        let cells = &lines[0].1;
        assert!(cells[0].observed);
        assert_eq!(cells[0].value, 7);
        assert!(!cells[3].observed, "address 3 was never observed");
        assert_eq!(cells[3].value, 0);
        assert_eq!(
            memory_cell_value_style(&cells[3]),
            ferrowl_ui::COLOR_SCHEME.placeholder
        );
    }

    /// UI-R-106 — value-class coloring: unobserved/observed-zero is neutral, observed
    /// printable-ASCII is normal text, any other observed non-zero value is flagged.
    #[test]
    fn ut_memory_layout_value_class_coloring_zero_ascii_other() {
        use ferrowl_ui::COLOR_SCHEME;
        let unobserved = MemoryCell {
            observed: false,
            value: 0,
        };
        let zero = MemoryCell {
            observed: true,
            value: 0,
        };
        let ascii = MemoryCell {
            observed: true,
            value: b'A' as u16,
        };
        let other = MemoryCell {
            observed: true,
            value: 0x0100, // low byte 0x00, not printable
        };
        assert_eq!(
            memory_cell_value_style(&unobserved),
            COLOR_SCHEME.placeholder
        );
        assert_eq!(memory_cell_value_style(&zero), COLOR_SCHEME.placeholder);
        assert_eq!(memory_cell_value_style(&ascii), COLOR_SCHEME.text);
        assert_eq!(memory_cell_value_style(&other), COLOR_SCHEME.warning);
    }

    /// UI-R-104 — the Memory table has exactly 4 columns, Kind/Address/Hex/Ascii, in order.
    #[test]
    fn ut_memory_table_has_exactly_4_columns_kind_address_hex_ascii() {
        assert_eq!(
            MemoryHeader::header(),
            [
                "Kind".to_string(),
                "Address".to_string(),
                "Hex".to_string(),
                "Ascii".to_string()
            ]
        );
    }

    /// UI-R-104 — a real line's Kind cell renders the line's table kind with the same `Kind`
    /// `Display` naming the modbus module's own register table uses.
    #[test]
    fn ut_memory_table_kind_column_shows_line_kind() {
        let lines = memory_layout_lines(&[(Kind::HoldingRegister, vec![(0, 1), (20, 2)])]);
        let rows = memory_table_rows(&lines, &[], std::time::Instant::now());
        assert_eq!(
            rows.len(),
            2,
            "two non-adjacent lines, no gap row between them"
        );
        assert_eq!(rows[0].kind, "Holding Register");
        assert_eq!(rows[1].kind, "Holding Register");
    }

    /// MB-R-147, UI-R-063, UI-R-107 — recency colouring is per cell, not per row: the observed cell's Hex
    /// span renders `hi` while its marker is active, and falls back to its ordinary value-class
    /// colour (`warning` here) once the marker lapses (>2s old).
    #[test]
    fn ut_memory_layout_recency_marker_overrides_value_class_color_while_active() {
        use ferrowl_ui::COLOR_SCHEME;
        use ratatui::style::Style;

        let lines = memory_layout_lines(&[(Kind::HoldingRegister, vec![(0, 1)])]);
        let now = std::time::Instant::now();

        let fresh = shaped_record(
            RecordStatus::Ok,
            ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
            Some(shape(Kind::HoldingRegister, 0, 1, None, None, vec![1])),
            std::time::Duration::from_millis(100),
        );
        let rows = memory_table_rows(&lines, std::slice::from_ref(&fresh), now);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].hex_spans[0].1, Style::default().fg(COLOR_SCHEME.hi));

        let stale = shaped_record(
            RecordStatus::Ok,
            ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
            Some(shape(Kind::HoldingRegister, 0, 1, None, None, vec![1])),
            std::time::Duration::from_secs(3),
        );
        let rows = memory_table_rows(&lines, std::slice::from_ref(&stale), now);
        assert_eq!(
            rows[0].hex_spans[0].1,
            Style::default().fg(COLOR_SCHEME.warning)
        );
    }

    /// (perf, no spec ID) — `memory_table_rows` computes each cell's recency/value-class color
    /// once and reuses it for both the Hex and Ascii spans (was computed twice, independently,
    /// per cell); a mix of unobserved, observed-zero, observed-printable, and observed
    /// -non-printable cells all still agree between the two columns.
    #[test]
    fn ut_memory_table_rows_hex_and_ascii_spans_share_the_same_color_per_cell() {
        // Register kind: 8 cells/line. Addresses 0,1,2 observed (zero, printable 'A', non
        // -printable), addresses 3..8 left unobserved.
        let lines =
            memory_layout_lines(&[(Kind::HoldingRegister, vec![(0, 0), (1, 65), (2, 300)])]);
        let rows = memory_table_rows(&lines, &[], std::time::Instant::now());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].hex_spans.len(), 8);
        for i in 0..8 {
            assert_eq!(
                rows[0].hex_spans[i].1, rows[0].ascii_spans[i].1,
                "cell {i}'s hex and ascii spans must share the same computed color"
            );
        }
    }

    /// UI-R-063 — two non-address-contiguous lines render back-to-back, with no separator row
    /// inserted between them.
    #[test]
    fn ut_memory_layout_no_separator_row_between_non_adjacent_lines() {
        let lines = memory_layout_lines(&[(Kind::HoldingRegister, vec![(0, 1), (20, 2)])]);
        assert_eq!(lines.len(), 2, "two non-adjacent lines, no gap inserted");

        let rows = memory_table_rows(&lines, &[], std::time::Instant::now());
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].address, "");
        assert_ne!(rows[1].address, "");
    }

    /// UI-R-106 — true per-cell granularity via `TableEntry::cell_spans`: two adjacent cells on
    /// the same line with different value classes (printable-ASCII `text` vs. non-printable
    /// `warning`) render as two separately-colored spans within the Hex column's single cell, not
    /// a single line-wide color.
    #[test]
    fn ut_memory_layout_two_adjacent_cells_render_with_distinct_per_cell_colors() {
        use ferrowl_ui::COLOR_SCHEME;
        use ferrowl_ui::widgets::TableEntry;
        use ratatui::style::Style;

        let lines =
            memory_layout_lines(&[(Kind::HoldingRegister, vec![(0, b'A' as u16), (1, 0x0100)])]);
        let rows = memory_table_rows(&lines, &[], std::time::Instant::now());
        assert_eq!(rows.len(), 1);

        let spans = TableEntry::<4>::cell_spans(&rows[0]);
        let hex_spans = spans[2]
            .as_ref()
            .expect("Hex column carries per-cell spans for a real (non-gap) line");
        assert_eq!(
            hex_spans[0].1,
            Style::default().fg(COLOR_SCHEME.text),
            "cell 0 (0x0041, printable low byte 'A') is `text`-colored"
        );
        assert_eq!(
            hex_spans[1].1,
            Style::default().fg(COLOR_SCHEME.warning),
            "cell 1 (0x0100, non-printable low byte) is `warning`-colored"
        );
        assert_ne!(
            hex_spans[0].1, hex_spans[1].1,
            "two adjacent cells on one line render with two different colors"
        );
    }

    /// UI-R-063 — a table kind with no
    /// observed traffic for the selected unit is omitted entirely, not rendered as an empty
    /// section.
    #[tokio::test]
    async fn ut_memory_layout_omits_table_kind_with_no_traffic_entirely() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        seed_unit(&v, UnitId(3));
        let kind_rows = v.memory_rows(UnitId(3));
        assert_eq!(kind_rows.len(), 1);
        assert_eq!(kind_rows[0].0, Kind::HoldingRegister);
    }

    /// UI-R-063 — coil-family kinds pack 8 bits per byte-cell, MSB-first, 16 bytes per line.
    #[test]
    fn ut_memory_layout_coil_packs_8_bits_per_byte_16_bytes_per_line() {
        // Bits 0 and 7 of the first byte set; bit 8 (second byte) also set.
        let pairs = vec![(0, 1), (7, 1), (8, 1)];
        let lines = memory_lines(Kind::Coil, &pairs);
        assert_eq!(lines.len(), 1);
        let (start, cells) = &lines[0];
        assert_eq!(*start, 0);
        assert_eq!(cells.len(), 16, "16 bytes per coil-family line");
        // bit 0 -> MSB (0x80), bit 7 -> LSB (0x01)
        assert_eq!(cells[0].value, 0b1000_0001);
        assert!(cells[0].observed);
        assert_eq!(cells[1].value, 0b1000_0000);
        assert!(cells[1].observed);
        assert!(!cells[2].observed);
    }

    /// UI-R-063 — register-family kinds are 1 word per cell, 8 words per line.
    #[test]
    fn ut_memory_layout_register_8_addresses_per_line() {
        let lines = memory_lines(Kind::HoldingRegister, &[(0, 10), (7, 20)]);
        assert_eq!(lines.len(), 1);
        let (start, cells) = &lines[0];
        assert_eq!(*start, 0);
        assert_eq!(cells.len(), 8, "8 words per register-family line");
        assert_eq!(cells[0].value, 10);
        assert_eq!(cells[7].value, 20);
        assert!(!cells[3].observed);
    }

    /// UI-R-064 — the Resolved-registers table has exactly `TableHeader`'s 9 non-Slave-ID/Access
    /// columns, in order.
    #[test]
    fn ut_resolved_registers_table_has_expected_9_columns_in_order() {
        assert_eq!(
            ResolvedHeader::header(),
            [
                "Name",
                "Description",
                "Address",
                "Kind",
                "Format",
                "Length",
                "Resolution",
                "Value",
                "Raw Value",
            ]
        );
    }

    /// UI-R-064 — the Raw Value column renders 4-digit lowercase hex per observed word, same
    /// convention as the Messages table's payload column.
    #[tokio::test]
    async fn ut_resolved_registers_table_raw_value_renders_hex_words() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module.table().write().write_words(
            Key::new(SlaveKey {
                slave_id: UnitId(3),
                kind: Kind::HoldingRegister,
            }),
            10,
            &[5],
        );
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, "Active power draw"));

        buffer_text(&mut v);
        let rows = v.resolved_table.state.values();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].values()[8], "[0005]");
    }

    /// UI-R-064 — the Value column renders the *decoded* reading (same decode path
    /// `ferrowl::module::modbus::table::Definition`'s own Value column uses), not a raw
    /// `{words:?}` debug dump, and with no surrounding `[...]` brackets (Raw Value
    /// keeps those). A resolution of `0.5` on a raw word of `100` proves the scaling actually
    /// ran, not just a hex reformat.
    #[tokio::test]
    async fn ut_resolved_registers_table_value_renders_decoded_scaled_reading() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module.table().write().write_words(
            Key::new(SlaveKey {
                slave_id: UnitId(3),
                kind: Kind::HoldingRegister,
            }),
            10,
            &[100],
        );
        let mut scaled = def(10, "Scaled reading");
        scaled.resolution = 0.5;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), scaled);

        buffer_text(&mut v);
        let rows = v.resolved_table.state.values();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].values()[7], "50");
        assert_eq!(rows[0].values()[8], "[0064]");
    }

    /// When the decoded value exactly matches one of the
    /// interpretation's named values, the Value column shows the label alone (not
    /// "label (value)", unlike the full modbus module's own `Definition::values`),
    /// using the same `Scalar::Int`-vs-raw-int-or-string matching logic.
    #[tokio::test]
    async fn ut_resolved_registers_table_value_shows_named_value_label_when_matched() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module.table().write().write_words(
            Key::new(SlaveKey {
                slave_id: UnitId(3),
                kind: Kind::HoldingRegister,
            }),
            10,
            &[1],
        );
        let mut named = def(10, "Kettle state");
        named.values = vec![crate::config::device::NamedValue {
            name: "kettle-on".to_string(),
            value: crate::config::device::Scalar::Int(1),
        }];
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), named);

        buffer_text(&mut v);
        let rows = v.resolved_table.state.values();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].values()[7],
            "kettle-on",
            "the label alone must render, not 'kettle-on (1)' or the raw '1'"
        );
    }

    /// No match means the decoded value renders as-is, unchanged.
    #[tokio::test]
    async fn ut_resolved_registers_table_value_renders_decoded_when_no_named_value_matches() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module.table().write().write_words(
            Key::new(SlaveKey {
                slave_id: UnitId(3),
                kind: Kind::HoldingRegister,
            }),
            10,
            &[2],
        );
        let mut named = def(10, "Kettle state");
        named.values = vec![crate::config::device::NamedValue {
            name: "kettle-on".to_string(),
            value: crate::config::device::Scalar::Int(1),
        }];
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), named);

        buffer_text(&mut v);
        let rows = v.resolved_table.state.values();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].values()[7], "2");
    }

    /// UI-R-065 — Tab cycles all 4 panels (Units→Messages→Memory→Resolved→Units) when the
    /// selected unit id has an interpretation.
    #[test]
    fn ut_tab_cycles_all_4_panels_when_resolved_registers_visible() {
        let mut v = view();
        v.set_focused(true);
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, "Active power draw"));

        assert_eq!(v.focus, ModbusMonitorModuleViewFocus::UnitsTable);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        assert_eq!(v.focus, ModbusMonitorModuleViewFocus::MessagesTable);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        assert_eq!(v.focus, ModbusMonitorModuleViewFocus::MemoryTable);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        assert_eq!(v.focus, ModbusMonitorModuleViewFocus::ResolvedTable);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        assert_eq!(
            v.focus,
            ModbusMonitorModuleViewFocus::UnitsTable,
            "Tab must wrap back to Units"
        );
    }

    /// UI-R-065 — the Resolved-registers panel is skipped in the Tab cycle when the selected
    /// unit id has no interpretation (hidden, per UI-R-061).
    #[test]
    fn ut_tab_skips_resolved_registers_panel_when_hidden() {
        let mut v = view();
        v.set_focused(true);
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        // No interpretation added: Resolved stays hidden.

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab); // -> Messages
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab); // -> Memory
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab); // would be Resolved, skipped -> Units
        assert_eq!(v.focus, ModbusMonitorModuleViewFocus::UnitsTable);
    }

    /// UI-R-065 — the Resolved-registers panel leaves the Tab cycle whenever it is hidden, so
    /// re-focusing the view while it is both focused and hidden re-homes focus onto Units rather
    /// than leaving keys routed to an invisible table.
    #[test]
    fn ut_refocusing_view_leaves_hidden_resolved_panel() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.focus = ModbusMonitorModuleViewFocus::ResolvedTable;
        v.set_focused(true);
        assert_eq!(v.focus, ModbusMonitorModuleViewFocus::UnitsTable);
    }

    /// UI-R-065 — each panel's selection/scroll is independent: switching panel focus away and
    /// back leaves the Messages table's own row selection untouched.
    #[tokio::test]
    async fn ut_panel_focus_switch_preserves_each_panels_own_selection() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        seed_unit(&v, UnitId(3));
        for i in 0..3 {
            v.module.records().write().push(
                UnitId(3),
                shaped_record(
                    RecordStatus::Ok,
                    ferrowl_modbus::FunctionCode::ReadHoldingRegisters,
                    Some(shape(Kind::HoldingRegister, i, 1, None, None, vec![i])),
                    std::time::Duration::from_secs(1),
                ),
            );
        }
        v.refresh().await;
        v.focus = ModbusMonitorModuleViewFocus::MessagesTable;
        v.messages_table.state.select_index(2);
        assert_eq!(v.messages_table.state.table_state().selected(), Some(2));

        v.focus = ModbusMonitorModuleViewFocus::MemoryTable;
        v.focus = ModbusMonitorModuleViewFocus::MessagesTable;

        assert_eq!(
            v.messages_table.state.table_state().selected(),
            Some(2),
            "switching panel focus away and back must not disturb the Messages table's own selection"
        );
    }

    /// MB-R-148, UI-R-108 — `Enter` on the Resolved-registers panel, with a row selected, opens
    /// `MonitorOverlay::EditInterpretation` prefilled from that row (by name, not raw table
    /// index, since `:order` may have reordered the displayed rows away from definition order).
    #[tokio::test]
    async fn ut_enter_on_resolved_row_opens_edit_dialog_prefilled() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, "Active power draw"));
        buffer_text(&mut v); // renders, which populates `resolved_table`'s rows
        v.focus = ModbusMonitorModuleViewFocus::ResolvedTable;

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        let MonitorOverlay::EditInterpretation(edit) = &v.overlay else {
            panic!("Enter on the Resolved panel did not open the edit-interpretation dialog");
        };
        assert_eq!(edit.original_name, "power");
        let dialog = &edit.dialog;
        assert_eq!(dialog.label.state.input(), "power");
        assert_eq!(dialog.description.state.input(), "Active power draw");
        assert_eq!(dialog.address.state.input(), "10");
    }

    /// The same "Add predefined" sub-popup routing as
    /// `ut_add_predefined_popup_receives_keyboard_focus`, but reached from
    /// `EditInterpretationDialog`'s own flow (`:edit`/Enter-on-Resolved-row), not `:add`'s.
    #[tokio::test]
    async fn ut_edit_interpretation_add_predefined_popup_receives_keyboard_focus() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, "Active power draw"));
        buffer_text(&mut v);
        v.focus = ModbusMonitorModuleViewFocus::ResolvedTable;
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        let MonitorOverlay::EditInterpretation(edit) = &mut v.overlay else {
            panic!("Enter on the Resolved panel did not open the edit-interpretation dialog");
        };
        let dialog = &mut edit.dialog;
        dialog.open_add_dialog();
        assert!(dialog.add_dialog.is_some());
        let parent_label_before = dialog.label.state.input().to_string();

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Char('x'));

        let MonitorOverlay::EditInterpretation(edit) = &mut v.overlay else {
            panic!("overlay changed unexpectedly");
        };
        let dialog = &mut edit.dialog;
        assert_eq!(
            dialog
                .add_dialog
                .as_ref()
                .expect("sub-popup stays open")
                .label
                .state
                .input(),
            "x",
            "the typed character reaches the sub-popup's own label field"
        );
        assert_eq!(
            dialog.label.state.input(),
            &parent_label_before,
            "the parent dialog's own label field is untouched while the sub-popup is open"
        );
    }

    /// MB-R-148 — confirming the edit dialog replaces the interpretation in place, under a
    /// (possibly new) name, mirroring the module-level `edit_interpretation` test one layer
    /// up, through the dialog/view.
    #[tokio::test]
    async fn ut_confirm_edit_interpretation_replaces_in_place_under_new_name() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, "Active power draw"));
        buffer_text(&mut v);
        v.focus = ModbusMonitorModuleViewFocus::ResolvedTable;
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        {
            let MonitorOverlay::EditInterpretation(edit) = &mut v.overlay else {
                panic!("edit-interpretation dialog did not open");
            };
            crate::dialog::widgets::set_input(&mut edit.dialog.label, "power2");
        }
        // Same as `ut_add_command_scopes_new_interpretation_to_selected_unit_id`'s own
        // `v.confirm_add()`: call the confirming method directly rather than routing the
        // Confirm-button focus/keypress through `handle_events`.
        v.confirm_edit_interpretation();

        assert!(matches!(v.overlay, MonitorOverlay::None));
        let interpretations = v.module.interpretations_for(UnitId(3));
        assert_eq!(interpretations.len(), 1);
        assert_eq!(interpretations[0].0, "power2");
        assert_eq!(interpretations[0].1.description, "Active power draw");
    }

    /// MB-R-148, UI-R-108 — the Delete flow removes the interpretation outright, gated by the dialog's own
    /// `confirm_delete` popup (Space on Delete opens it, Enter on its DELETE button confirms).
    #[tokio::test]
    async fn ut_delete_interpretation_removes_via_confirm_delete_flow() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, "Active power draw"));
        buffer_text(&mut v);
        v.focus = ModbusMonitorModuleViewFocus::ResolvedTable;
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        {
            let MonitorOverlay::EditInterpretation(edit) = &mut v.overlay else {
                panic!("edit-interpretation dialog did not open");
            };
            let dialog = &mut edit.dialog;
            dialog.open_confirm_delete();
            assert!(dialog.confirm_delete.is_some());
        }
        // The confirm-delete popup defaults to CANCEL focused; Tab to DELETE, then Enter.
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        assert!(matches!(v.overlay, MonitorOverlay::None));
        assert!(v.module.interpretations_for(UnitId(3)).is_empty());
    }

    /// MB-R-148 — neither edit nor delete ever touches `module.table()` (the slave's observed-
    /// value table): a value written there survives both operations unchanged.
    #[tokio::test]
    async fn ut_edit_and_delete_interpretation_never_touch_observed_table() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, "Active power draw"));
        let key = Key::new(SlaveKey {
            slave_id: UnitId(3),
            kind: Kind::HoldingRegister,
        });
        v.module.table().write().write_words(key.clone(), 10, &[42]);
        let observed_before = v.module.table().read().read_words(&key, 10, 1);

        buffer_text(&mut v);
        v.focus = ModbusMonitorModuleViewFocus::ResolvedTable;
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);
        {
            let MonitorOverlay::EditInterpretation(edit) = &mut v.overlay else {
                panic!("edit-interpretation dialog did not open");
            };
            crate::dialog::widgets::set_input(&mut edit.dialog.label, "power2");
        }
        v.confirm_edit_interpretation();
        assert!(
            matches!(v.overlay, MonitorOverlay::None),
            "edit did not confirm"
        );
        assert_eq!(v.module.interpretations_for(UnitId(3))[0].0, "power2");
        assert_eq!(
            v.module.table().read().read_words(&key, 10, 1),
            observed_before
        );

        buffer_text(&mut v);
        v.focus = ModbusMonitorModuleViewFocus::ResolvedTable;
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);
        {
            let MonitorOverlay::EditInterpretation(edit) = &mut v.overlay else {
                panic!("edit-interpretation dialog did not open");
            };
            edit.dialog.open_confirm_delete();
        }
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);
        assert!(v.module.interpretations_for(UnitId(3)).is_empty());
        assert_eq!(
            v.module.table().read().read_words(&key, 10, 1),
            observed_before
        );
    }

    /// An interpretation that already has aliases defined opens
    /// focused directly on the alias list (`value`), not `Address` (see
    /// `EditInterpretationDialog::from_interpretation`).
    #[tokio::test]
    async fn ut_enter_on_resolved_row_with_aliases_opens_dialog_focused_on_value() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        let mut named = def(10, "Kettle state");
        named.values = vec![crate::config::device::NamedValue {
            name: "kettle-on".to_string(),
            value: crate::config::device::Scalar::Int(1),
        }];
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), named);
        buffer_text(&mut v);
        v.focus = ModbusMonitorModuleViewFocus::ResolvedTable;

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        let MonitorOverlay::EditInterpretation(edit) = &v.overlay else {
            panic!("Enter on the Resolved panel did not open the edit-interpretation dialog");
        };
        assert!(
            !edit.dialog.value.state.values().is_empty(),
            "an interpretation with aliases must open with the alias list populated"
        );
        assert!(
            edit.dialog.value.state.focused(),
            "an interpretation with aliases must open focused directly on the alias list"
        );
    }

    /// Adding the first alias through the dialog's own "ADD ALIAS" sub-popup re-homes focus
    /// onto the alias list (`value`), which has just become focusable (see
    /// `EditInterpretationDialog::confirm_add_dialog`).
    #[tokio::test]
    async fn ut_edit_interpretation_add_first_alias_focuses_value() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), def(10, "Active power draw"));
        buffer_text(&mut v);
        v.focus = ModbusMonitorModuleViewFocus::ResolvedTable;
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        {
            let MonitorOverlay::EditInterpretation(edit) = &v.overlay else {
                panic!("edit-interpretation dialog did not open");
            };
            assert!(edit.dialog.value.state.values().is_empty());
        }

        let MonitorOverlay::EditInterpretation(edit) = &mut v.overlay else {
            panic!("edit-interpretation dialog did not open");
        };
        let dialog = &mut edit.dialog;
        dialog.open_add_dialog();
        let sub = dialog.add_dialog.as_mut().unwrap();
        crate::dialog::widgets::set_input(&mut sub.label, "kettle-on");
        crate::dialog::widgets::set_input(&mut sub.value, "1");

        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        let MonitorOverlay::EditInterpretation(edit) = &v.overlay else {
            panic!("overlay changed unexpectedly");
        };
        assert!(
            !edit.dialog.value.state.values().is_empty(),
            "adding the first alias must populate the alias list"
        );
        assert!(
            edit.dialog.value.state.focused(),
            "adding the first alias must re-home focus onto the alias list"
        );
    }

    /// Deleting the last remaining alias re-homes focus onto `Label`, since the alias list has
    /// just become unfocusable (see `EditInterpretationDialog::delete_selected_named_value`).
    #[tokio::test]
    async fn ut_edit_interpretation_delete_last_alias_focuses_label() {
        let mut v = view();
        v.unit_ids = vec![UnitId(3)];
        v.selected = 0;
        let mut named = def(10, "Kettle state");
        named.values = vec![crate::config::device::NamedValue {
            name: "kettle-on".to_string(),
            value: crate::config::device::Scalar::Int(1),
        }];
        v.module
            .add_interpretation(UnitId(3), "power".to_string(), named);
        buffer_text(&mut v);
        v.focus = ModbusMonitorModuleViewFocus::ResolvedTable;
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Enter);

        {
            let MonitorOverlay::EditInterpretation(edit) = &v.overlay else {
                panic!("edit-interpretation dialog did not open");
            };
            assert!(!edit.dialog.value.state.values().is_empty());
        }

        // The dialog opens with `Value` focused (see `from_interpretation`); Tab twice
        // reaches `DeleteValueButton` (Value -> AddButton -> DeleteValueButton).
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Tab);
        ModuleView::handle_events(&mut v, KeyModifiers::NONE, KeyCode::Char(' '));

        let MonitorOverlay::EditInterpretation(edit) = &v.overlay else {
            panic!("overlay changed unexpectedly");
        };
        assert!(
            edit.dialog.value.state.values().is_empty(),
            "deleting the last alias must empty the alias list"
        );
        assert!(
            edit.dialog.label.state.focused(),
            "deleting the last alias must re-home focus onto Label"
        );
    }
}
