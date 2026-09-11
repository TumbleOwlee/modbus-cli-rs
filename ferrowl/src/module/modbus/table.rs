//! Main register table view: one row per register with its decoded value.

use crate::{
    config::device::{NamedValue, Scalar},
    dialog::parse_raw_value,
};
use derive_builder::Builder;
use ferrowl_codec::{Register, Value};
use ferrowl_ui::{
    Border, COLOR_SCHEME,
    state::{TableState, TableStateBuilder},
    style::TableStyleBuilder,
    widgets::{Header, Table, TableBuilder, TableEntry, Widget, Width},
};
use ferrowl_ui_derive::{Focus, focusable};
use ratatui::{
    buffer::Buffer,
    layout::{Margin, Rect},
    style::Style,
    widgets::StatefulWidget,
};
use std::fmt::Debug;
use std::time::{Duration, Instant};

pub const COLUMN_COUNT: usize = 11;

/// How long a row stays highlighted after a register change.
pub const CHANGE_HIGHLIGHT: Duration = Duration::from_secs(2);

/// Resolve a user-supplied column name to its index in `H::header()`. Matching is
/// case-insensitive and ignores spaces, so `slaveid`, `slave id`, and `Slave ID` all resolve to
/// the same column. Returns `None` if nothing matches. Generic over any [`Header`] so other
/// tables (e.g. the monitor module's resolved-registers table) can resolve `:order`'s column
/// argument the same way, without duplicating the normalize-and-match body.
pub fn column_index_for<H: Header<N>, const N: usize>(name: &str) -> Option<usize> {
    let normalize = |s: &str| {
        s.chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let target = normalize(name);
    H::header().iter().position(|h| normalize(h) == target)
}

/// Resolve a user-supplied column name to its index in [`TableHeader::header`]. See
/// [`column_index_for`] for the matching rule.
pub fn column_index(name: &str) -> Option<usize> {
    column_index_for::<TableHeader, COLUMN_COUNT>(name)
}

/// Compare two [`TableEntry`] rows by the given column for `:order`. Both sides are taken from
/// [`TableEntry::values`] (the displayed strings); numeric when both parse as `f64`, otherwise
/// case-insensitive lexicographic. `descending` reverses the result. Generic over any
/// `TableEntry<N>` so other tables can reuse the same sort rule.
pub fn cmp_table_entry<V: TableEntry<N>, const N: usize>(
    a: &V,
    b: &V,
    column: usize,
    descending: bool,
) -> std::cmp::Ordering {
    let va = a.values();
    let vb = b.values();
    let sa = va.get(column).map(String::as_str).unwrap_or_default();
    let sb = vb.get(column).map(String::as_str).unwrap_or_default();
    let ord = match (sa.parse::<f64>(), sb.parse::<f64>()) {
        (Ok(na), Ok(nb)) => na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal),
        _ => sa.to_lowercase().cmp(&sb.to_lowercase()),
    };
    if descending { ord.reverse() } else { ord }
}

/// Compare two register rows by the given column for `:order`. See [`cmp_table_entry`].
pub fn cmp_definitions(
    a: &Definition,
    b: &Definition,
    column: usize,
    descending: bool,
) -> std::cmp::Ordering {
    cmp_table_entry(a, b, column, descending)
}

#[derive(Clone, Debug)]
pub struct TableHeader {}

impl Header<COLUMN_COUNT> for TableHeader {
    fn header() -> [String; COLUMN_COUNT] {
        [
            "Name".into(),
            "Description".into(),
            "Slave ID".into(),
            "Address".into(),
            "Access".into(),
            "Kind".into(),
            "Format".into(),
            "Length".into(),
            "Resolution".into(),
            "Value".into(),
            "Raw Value".into(),
        ]
    }

    fn widths() -> [Width; COLUMN_COUNT] {
        [
            Width { min: 15, max: 30 },
            Width { min: 25, max: 40 },
            Width { min: 10, max: 20 },
            Width { min: 10, max: 20 },
            Width { min: 10, max: 20 },
            Width { min: 20, max: 30 },
            Width { min: 10, max: 20 },
            Width { min: 10, max: 20 },
            Width { min: 10, max: 20 },
            Width { min: 5, max: 40 },
            Width { min: 0, max: 800 },
        ]
    }
}

/// A register row. Metadata comes from `register`; `value`/`raw_value` are filled from a
/// memory snapshot each frame (see `App::refresh_snapshot`).
#[derive(Clone, Debug)]
pub struct Definition {
    pub name: String,
    pub description: String,
    pub register: Register,
    pub named_values: Vec<NamedValue>,
    pub value: Value,
    pub raw_value: String,
    /// When the decoded value last changed; drives the change highlight (see
    /// [`CHANGE_HIGHLIGHT`]). `None` until the first observed change.
    pub changed_at: Option<Instant>,
}

impl Definition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        register: Register,
        named_values: Vec<NamedValue>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            register,
            named_values,
            value: Value::Ascii(String::new()),
            raw_value: String::new(),
            changed_at: None,
        }
    }

    /// Whether the row is inside its post-change highlight window.
    fn highlight_active(&self) -> bool {
        self.changed_at
            .is_some_and(|at| at.elapsed() < CHANGE_HIGHLIGHT)
    }
}

impl TableEntry<COLUMN_COUNT> for Definition {
    fn values(&self) -> [String; COLUMN_COUNT] {
        let resolution = match self.register.format().resolution() {
            Some(v) => format!("{v}"),
            None => "None".into(),
        };

        let raw_value = &self.raw_value;
        let raw_int = parse_raw_value(raw_value);
        let mut value = self.value.to_string();
        if let Some(named) = self.named_values.iter().find(|nv| match &nv.value {
            Scalar::Int(v) => raw_int == Some(*v) || value == v.to_string(),
            other => value == other.to_string(),
        }) {
            value = format!("{} ({})", named.name, value);
        }

        let value: String = value
            .chars()
            .map(|c| {
                if c == ' ' || c.is_ascii_graphic() {
                    c
                } else {
                    '.'
                }
            })
            .collect();

        [
            self.name.clone(),
            self.description.clone(),
            format!("{}", self.register.slave_id()),
            format!("{}", self.register.address()),
            format!("{}", self.register.access()),
            format!("{}", self.register.kind()),
            format!("{}", self.register.format()),
            format!("{}", self.register.format().width()),
            resolution,
            value,
            self.raw_value.clone(),
        ]
    }

    fn height(&self) -> u16 {
        3
    }

    fn cell_styles(&self) -> [Option<Style>; COLUMN_COUNT] {
        if self.highlight_active() {
            [Some(Style::default().fg(COLOR_SCHEME.hi)); COLUMN_COUNT]
        } else {
            [None; COLUMN_COUNT]
        }
    }
}

#[focusable]
#[derive(Builder, Focus)]
pub struct TableView {
    #[focus]
    pub table:
        Widget<TableState<Definition, COLUMN_COUNT>, Table<Definition, TableHeader, COLUMN_COUNT>>,
    #[builder(default)]
    pub compact: bool,
}

impl TableView {
    pub fn new(values: Vec<Definition>) -> Self {
        TableViewBuilder::default()
            .table(Widget {
                state: TableStateBuilder::default()
                    .values(values)
                    .build()
                    .expect("all required builder fields are set"),
                widget: TableBuilder::default()
                    .border(Border::Full(Margin::new(1, 0)))
                    .title(Some("Register".into()))
                    .style(
                        TableStyleBuilder::default()
                            .build()
                            .expect("all required builder fields are set"),
                    )
                    .split_by_whitespace([
                        true, true, true, true, true, true, true, true, true, false, true,
                    ])
                    .row_margin(Margin {
                        vertical: 1,
                        horizontal: 0,
                    })
                    .build()
                    .expect("all required builder fields are set"),
            })
            .focus(TableViewFocus::Table)
            .build()
            .expect("all required builder fields are set")
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        StatefulWidget::render(&self.table.widget, area, buf, &mut self.table.state);
    }

    pub fn set_compact(&mut self, compact: bool) {
        self.compact = compact;
        let vertical = if compact { 0 } else { 1 };
        self.table.widget.set_row_margin(Margin {
            vertical,
            horizontal: 0,
        });
    }

    /// Current register rows.
    pub fn definitions(&self) -> &[Definition] {
        self.table.state.values()
    }

    /// Replace the register rows (keeps the table's selection/scroll state).
    pub fn set_definitions(&mut self, values: Vec<Definition>) {
        self.table.state.set_values(values);
    }

    /// Stable-sort the register rows by `column` (see [`cmp_definitions`]).
    pub fn sort_definitions(&mut self, column: usize, descending: bool) {
        let mut values = self.definitions().to_vec();
        values.sort_by(|a, b| cmp_definitions(a, b, column, descending));
        self.set_definitions(values);
    }

    /// The currently selected register row, if any.
    pub fn selected(&self) -> Option<&Definition> {
        let idx = self.table.state.table_state().selected()?;
        self.table.state.values().get(idx)
    }

    /// The index of the currently selected register row, if any.
    pub fn selected_index(&self) -> Option<usize> {
        self.table.state.table_state().selected()
    }

    /// Select the first register row (or clear the selection when the table is empty).
    pub fn select_first(&mut self) {
        self.table.state.move_to_top();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrowl_codec::format::{BitField, Endian, Format, Resolution, WordOrder};
    use ferrowl_codec::{Access, Address, Kind, RegisterBuilder};
    use ferrowl_modbus::UnitId;

    fn definition() -> Definition {
        let register = RegisterBuilder::default()
            .slave_id(UnitId(1))
            .access(Access::ReadWrite)
            .kind(Kind::HoldingRegister)
            .address(Address::Fixed(0))
            .format(Format::u16(
                Endian::Big,
                WordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ))
            .build()
            .unwrap();
        Definition::new("reg".to_string(), "d".to_string(), register, vec![])
    }

    #[test]
    /// UI-R-185 — an unchanged register row carries no change-highlight cell style.
    fn ut_no_change_has_no_cell_styles() {
        let d = definition();
        assert!(d.cell_styles().iter().all(Option::is_none));
    }

    #[test]
    /// UI-R-185 — a recently-changed register value highlights its row for a brief window.
    fn ut_recent_change_highlights_full_row() {
        let mut d = definition();
        d.changed_at = Some(Instant::now());
        assert!(d.cell_styles().iter().all(Option::is_some));
    }

    #[test]
    /// UI-R-185 — the change highlight expires after its window.
    fn ut_highlight_expires_after_window() {
        let mut d = definition();
        d.changed_at = Instant::now().checked_sub(CHANGE_HIGHLIGHT + Duration::from_secs(1));
        assert!(d.changed_at.is_some(), "back-dating must succeed");
        assert!(d.cell_styles().iter().all(Option::is_none));
    }

    #[test]
    /// UI-R-185 — the change-highlight window is 2 seconds.
    fn ut_change_highlight_window_is_two_seconds() {
        assert_eq!(CHANGE_HIGHLIGHT, Duration::from_secs(2));
    }

    fn named(name: &str, slave: u8) -> Definition {
        let register = RegisterBuilder::default()
            .slave_id(UnitId(slave))
            .access(Access::ReadWrite)
            .kind(Kind::HoldingRegister)
            .address(Address::Fixed(0))
            .format(Format::u16(
                Endian::Big,
                WordOrder::Normal,
                Resolution(1.0),
                BitField::default(),
            ))
            .build()
            .unwrap();
        Definition::new(name.to_string(), "d".to_string(), register, vec![])
    }

    #[test]
    fn ut_column_index_normalizes_name() {
        assert_eq!(column_index("Name"), Some(0));
        assert_eq!(column_index("slave id"), Some(2)); // case + whitespace insensitive
        assert_eq!(column_index("RAWVALUE"), Some(10));
        assert_eq!(column_index("nonsense"), None);
    }

    #[test]
    fn ut_cmp_definitions_numeric_and_lexical() {
        // Column 2 (Slave ID) parses numerically: 2 sorts after 10 only lexically, not numerically.
        let a = named("a", 2);
        let b = named("b", 10);
        assert_eq!(cmp_definitions(&a, &b, 2, false), std::cmp::Ordering::Less);
        assert_eq!(
            cmp_definitions(&a, &b, 2, true),
            std::cmp::Ordering::Greater
        );
        // Column 0 (Name) compares lexically, case-insensitively.
        let x = named("Alpha", 1);
        let y = named("beta", 1);
        assert_eq!(cmp_definitions(&x, &y, 0, false), std::cmp::Ordering::Less);
    }

    #[test]
    fn ut_table_view_selection_and_sort() {
        let mut t = TableView::new(vec![named("zeta", 9), named("alpha", 1)]);
        assert_eq!(t.definitions().len(), 2);
        t.select_first();
        assert_eq!(t.selected_index(), Some(0));
        assert_eq!(
            t.selected().map(|d| d.name.clone()),
            Some("zeta".to_string())
        );
        // Sorting by Name ascending brings "alpha" to the front.
        t.sort_definitions(0, false);
        assert_eq!(t.definitions()[0].name, "alpha");
        // set_definitions replaces the contents.
        t.set_definitions(vec![named("solo", 1)]);
        assert_eq!(t.definitions().len(), 1);
        t.set_compact(true);
    }
}
