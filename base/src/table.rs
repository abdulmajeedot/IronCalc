//! Tables and slicers: what a table looks like, how its filters decide which
//! rows show, and what stays attached to it when rows and columns move.
//!
//! A table's look is not stored in its cells. It is laid over them when they
//! are drawn, so banding follows filters and a cell's own format always wins,
//! as in Excel. Filters are applied once, by hiding rows, as in Excel.

use std::cmp::Ordering;
use std::collections::HashSet;

use bitcode::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::actions::displace_cf_col;
use crate::cell::CellValue;
use crate::data_validation::displaced_range;
use crate::expressions::parser::parse_range;
use crate::expressions::parser::stringify::DisplaceData;
use crate::expressions::utils::column_to_number;
use crate::expressions::utils::number_to_column;
use crate::model::Model;
use crate::pivot::{wildcard_match, PivotArea};
use crate::types::{BorderItem, BorderStyle, Cell, Color, Style, Table, TableColumn, Theme};

/// How empty cells are listed in a filter.
pub const BLANK_FILTER_LABEL: &str = "(Blanks)";
pub const DEFAULT_TABLE_STYLE: &str = "TableStyleMedium2";
pub const DEFAULT_SLICER_STYLE: &str = "SlicerStyleLight1";

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum TableFilterOperator {
    Equals,
    NotEquals,
    BeginsWith,
    NotBeginsWith,
    EndsWith,
    NotEndsWith,
    Contains,
    NotContains,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Between,
    NotBetween,
    AboveAverage,
    BelowAverage,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TableFilterRule {
    /// Only rows showing one of these labels stay. Values that appear later
    /// stay hidden until the filter changes, as in Excel.
    Values { visible: Vec<String> },
    Condition {
        operator: TableFilterOperator,
        value: String,
        #[serde(default)]
        value2: String,
    },
    Top {
        count: f64,
        bottom: bool,
        percent: bool,
    },
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Clone)]
pub struct TableColumnFilter {
    pub column: String,
    pub rule: TableFilterRule,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct TableSort {
    pub column: String,
    pub descending: bool,
}

/// The filters and sort of one table.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Clone)]
pub struct TableFilterState {
    pub table: String,
    pub columns: Vec<TableColumnFilter>,
    #[serde(default)]
    pub sort: Option<TableSort>,
}

/// One entry of a filter list or slicer.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct TableFilterItem {
    pub label: String,
    /// Passes this column's own filter.
    pub visible: bool,
    /// Has a row that passes the filters of the other columns.
    pub has_data: bool,
}

/// The colours of a table style, for its picture in the style gallery.
#[derive(Serialize, Debug, PartialEq, Clone, Default)]
pub struct TableStyleSwatch {
    pub header_fill: Option<String>,
    pub header_color: Option<String>,
    pub body_fill: Option<String>,
    pub stripe_fill: Option<String>,
    pub line: Option<String>,
}

/// A filter button on a table's header.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct TableMarker {
    pub table: String,
    pub column: String,
    /// What the user calls the column
    pub label: String,
    pub row: i32,
    pub col: i32,
    pub filtered: bool,
    pub sort_descending: Option<bool>,
}

#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct TableColumnEntry {
    pub name: String,
    pub totals_function: Option<String>,
    pub totals_label: Option<String>,
    pub filtered: bool,
}

/// A table as listed to the UI.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct TableEntry {
    pub name: String,
    pub sheet: u32,
    pub area: PivotArea,
    pub reference: String,
    pub columns: Vec<TableColumnEntry>,
    pub options: TableOptions,
    pub filtered: bool,
    pub sort: Option<TableSort>,
}

/// Everything the Table Design panel can switch.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct TableOptions {
    pub name: String,
    /// `None` is a table without a style.
    pub style_name: Option<String>,
    pub header_row: bool,
    pub total_row: bool,
    pub banded_rows: bool,
    pub banded_columns: bool,
    pub first_column: bool,
    pub last_column: bool,
    pub filter_button: bool,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct SlicerPivot {
    pub name: String,
    pub sheet_id: u32,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SlicerSource {
    Table {
        table: String,
        column: String,
    },
    Pivot {
        pivots: Vec<SlicerPivot>,
        field: String,
    },
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum SlicerPlacement {
    MoveAndSize,
    Move,
    Free,
}

/// A slicer floating over a sheet. Which of its buttons are on is not stored:
/// it is read from the filter of the table or pivot it controls.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Clone)]
pub struct Slicer {
    pub name: String,
    pub caption: String,
    pub show_header: bool,
    pub sheet_id: u32,
    pub source: SlicerSource,
    /// The cell its top left corner sits in, and the distance into that cell.
    pub row: i32,
    pub column: i32,
    pub offset_x: f64,
    pub offset_y: f64,
    pub width: f64,
    pub height: f64,
    pub button_columns: u32,
    pub button_height: f64,
    pub placement: SlicerPlacement,
    /// Whether moving and resizing it is disabled.
    pub locked: bool,
    pub print: bool,
    pub style: String,
    pub alt_text: String,
    pub multi_select: bool,
}

#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct SlicerConnection {
    pub name: String,
    pub sheet_id: u32,
    pub sheet_name: String,
    pub connected: bool,
}

/// A slicer as listed to the UI.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct SlicerEntry {
    pub slicer: Slicer,
    pub sheet: u32,
    pub items: Vec<TableFilterItem>,
    pub filtered: bool,
    /// Such as `Sheet2!A1:D9 [Item]`.
    pub source_description: String,
    pub connections: Vec<SlicerConnection>,
}

/// The area of a table, or `None` for a damaged reference.
pub fn table_area(table: &Table) -> Option<PivotArea> {
    let (first_column, first_row, last_column, last_row) = parse_range(&table.reference).ok()?;
    Some(PivotArea {
        first_row,
        first_column,
        last_row,
        last_column,
    })
}

pub fn area_reference(area: &PivotArea) -> String {
    format!(
        "{}{}:{}{}",
        number_to_column(area.first_column).unwrap_or_default(),
        area.first_row,
        number_to_column(area.last_column).unwrap_or_default(),
        area.last_row
    )
}

/// The rows holding a table's data, without its header and total rows.
pub fn table_data_rows(table: &Table, area: &PivotArea) -> (i32, i32) {
    (
        area.first_row + table.header_row_count as i32,
        area.last_row - table.totals_row_count as i32,
    )
}

/// Filter names that start with this belong to a sheet, not to a table. A
/// table's name can't contain `#`.
const SHEET_FILTER_PREFIX: &str = "#sheet:";

/// The filter buttons on a plain range of a sheet.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct SheetFilter {
    /// The id of the sheet
    pub sheet_id: u32,
    /// The range, its first row being the one with the buttons
    pub area: PivotArea,
}

/// The name a sheet's filter goes by in the filter calls.
pub fn sheet_filter_name(sheet_id: u32) -> String {
    format!("{SHEET_FILTER_PREFIX}{sheet_id}")
}

/// What a sheet's filter says about itself.
#[derive(Serialize, Debug, PartialEq, Eq, Clone)]
pub struct SheetFilterInfo {
    /// The name to use in the filter calls
    pub name: String,
    /// The range, its first row being the one with the buttons
    pub area: PivotArea,
    /// Whether any column is being filtered
    pub filtered: bool,
}

/// What a filter works on, be it a table's or a sheet's.
pub(crate) struct FilterScope {
    pub(crate) sheet: u32,
    pub(crate) area: PivotArea,
    pub(crate) first: i32,
    pub(crate) last: i32,
    /// The name of each column, in order. A sheet's columns go by their letter.
    pub(crate) columns: Vec<String>,
}

impl FilterScope {
    pub(crate) fn index(&self, column: &str) -> Option<usize> {
        self.columns.iter().position(|c| same_name(c, column))
    }
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

pub fn table_column_index(table: &Table, column: &str) -> Option<usize> {
    table
        .columns
        .iter()
        .position(|c| same_name(&c.name, column))
}

fn wildcard(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    wildcard_match(&pattern, &text)
}

fn compare_to(label: &str, number: Option<f64>, target: &str) -> Ordering {
    if let (Some(number), Ok(target)) = (number, target.trim().parse::<f64>()) {
        return number.total_cmp(&target);
    }
    label.to_lowercase().cmp(&target.to_lowercase())
}

/// One cell of a filtered column.
#[derive(Clone)]
pub(crate) struct FilterCell {
    pub(crate) label: String,
    pub(crate) number: Option<f64>,
}

fn condition_keeps(
    operator: TableFilterOperator,
    value: &str,
    value2: &str,
    cell: &FilterCell,
    average: f64,
) -> bool {
    use TableFilterOperator as O;
    let label = cell.label.as_str();
    let blank = label == BLANK_FILTER_LABEL;
    let shown = if blank { "" } else { label };
    let order = || compare_to(shown, cell.number, value);
    let inside = || {
        let (low, high) = match (value.trim().parse::<f64>(), value2.trim().parse::<f64>()) {
            (Ok(a), Ok(b)) if a > b => (value2, value),
            _ => (value, value2),
        };
        compare_to(shown, cell.number, low) != Ordering::Less
            && compare_to(shown, cell.number, high) != Ordering::Greater
    };
    let measurable = !blank;
    match operator {
        O::Equals => wildcard(value, shown),
        O::NotEquals => !wildcard(value, shown),
        O::BeginsWith => wildcard(&format!("{value}*"), shown),
        O::NotBeginsWith => !wildcard(&format!("{value}*"), shown),
        O::EndsWith => wildcard(&format!("*{value}"), shown),
        O::NotEndsWith => !wildcard(&format!("*{value}"), shown),
        O::Contains => wildcard(&format!("*{value}*"), shown),
        O::NotContains => !wildcard(&format!("*{value}*"), shown),
        O::GreaterThan => measurable && order() == Ordering::Greater,
        O::GreaterThanOrEqual => measurable && order() != Ordering::Less,
        O::LessThan => measurable && order() == Ordering::Less,
        O::LessThanOrEqual => measurable && order() != Ordering::Greater,
        O::Between => measurable && inside(),
        O::NotBetween => measurable && !inside(),
        O::AboveAverage => cell.number.is_some_and(|n| n > average),
        O::BelowAverage => cell.number.is_some_and(|n| n < average),
    }
}

/// For each cell of a column, whether `rule` keeps it.
pub(crate) fn rule_keeps(rule: &TableFilterRule, cells: &[FilterCell]) -> Vec<bool> {
    let numbers: Vec<f64> = cells.iter().filter_map(|c| c.number).collect();
    match rule {
        TableFilterRule::Values { visible } => {
            let visible: HashSet<String> = visible.iter().map(|v| v.to_lowercase()).collect();
            cells
                .iter()
                .map(|c| visible.contains(&c.label.to_lowercase()))
                .collect()
        }
        TableFilterRule::Condition {
            operator,
            value,
            value2,
        } => {
            let average = numbers.iter().sum::<f64>() / numbers.len().max(1) as f64;
            cells
                .iter()
                .map(|c| condition_keeps(*operator, value, value2, c, average))
                .collect()
        }
        TableFilterRule::Top {
            count,
            bottom,
            percent,
        } => {
            let mut sorted = numbers.clone();
            sorted.sort_by(|a, b| {
                if *bottom {
                    a.total_cmp(b)
                } else {
                    b.total_cmp(a)
                }
            });
            let wanted = if *percent {
                (sorted.len() as f64 * count / 100.0).ceil()
            } else {
                *count
            };
            let wanted = (wanted.max(1.0) as usize).min(sorted.len());
            let Some(limit) = wanted.checked_sub(1).and_then(|i| sorted.get(i)).copied() else {
                return vec![false; cells.len()];
            };
            cells
                .iter()
                .map(|c| {
                    c.number
                        .is_some_and(|n| if *bottom { n <= limit } else { n >= limit })
                })
                .collect()
        }
    }
}

/// Numbers first, then text, with blanks at the end whichever way it sorts.
pub(crate) fn compare_filter_cells(a: &FilterCell, b: &FilterCell, descending: bool) -> Ordering {
    let blank = |c: &FilterCell| c.label == BLANK_FILTER_LABEL;
    match (blank(a), blank(b)) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    let order = match (a.number, b.number) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => a.label.to_lowercase().cmp(&b.label.to_lowercase()),
    };
    if descending {
        order.reverse()
    } else {
        order
    }
}

type Line = Option<(BorderStyle, String)>;

/// The formatting of one region of a table.
#[derive(Clone, Default, Debug)]
struct TablePart {
    fill: Option<String>,
    bold: bool,
    color: Option<String>,
    top: Line,
    bottom: Line,
    left: Line,
    right: Line,
}

/// A table style resolved against the workbook theme.
#[derive(Clone, Debug, Default)]
pub(crate) struct TableLook {
    whole: TablePart,
    inner_horizontal: Line,
    inner_vertical: Line,
    header: TablePart,
    total: TablePart,
    stripe: TablePart,
    column_stripe: TablePart,
    first_column: TablePart,
    last_column: TablePart,
}

const THEME_DARK: i32 = 1;
const THEME_LIGHT: i32 = 0;
const THEME_ACCENT: i32 = 4;

fn thin(color: &str) -> Line {
    Some((BorderStyle::Thin, color.to_string()))
}

fn medium(color: &str) -> Line {
    Some((BorderStyle::Medium, color.to_string()))
}

fn double(color: &str) -> Line {
    Some((BorderStyle::Double, color.to_string()))
}

fn outline(part: &mut TablePart, line: &Line) {
    part.top = line.clone();
    part.bottom = line.clone();
    part.left = line.clone();
    part.right = line.clone();
}

/// Builds Excel's built-in styles. They come in families of seven, one for
/// black and one for each accent colour.
pub(crate) fn table_look(name: &str, theme: &Theme) -> Option<TableLook> {
    let rest = name.strip_prefix("TableStyle")?;
    let (kind, number) = if let Some(n) = rest.strip_prefix("Light") {
        ("light", n)
    } else if let Some(n) = rest.strip_prefix("Medium") {
        ("medium", n)
    } else {
        ("dark", rest.strip_prefix("Dark")?)
    };
    let number: usize = number.parse().ok()?;
    if number == 0 {
        return None;
    }
    let family = (number - 1) / 7;
    let tone = (number - 1) % 7;
    let index = if tone == 0 {
        THEME_DARK
    } else {
        THEME_ACCENT + tone as i32 - 1
    };
    let main = theme.resolve(index, 0.0);
    let tint = |t: f64| theme.resolve(index, t);
    let pale = if tone == 0 { 0.85 } else { 0.8 };
    let white = theme.resolve(THEME_LIGHT, 0.0);
    let black = theme.resolve(THEME_DARK, 0.0);
    let mut look = TableLook::default();
    look.first_column.bold = true;
    look.last_column.bold = true;
    look.total.bold = true;
    look.header.bold = true;
    match (kind, family) {
        ("light", 0) => {
            look.whole.top = thin(&main);
            look.whole.bottom = thin(&main);
            if tone != 0 {
                look.whole.color = Some(tint(-0.25));
            }
            look.header.bottom = thin(&main);
            look.total.top = thin(&main);
            look.stripe.fill = Some(tint(pale));
            look.column_stripe.fill = Some(tint(pale));
        }
        ("light", 1) => {
            outline(&mut look.whole, &thin(&main));
            look.header.fill = Some(main.clone());
            look.header.color = Some(white);
            look.total.top = double(&main);
            look.stripe.top = thin(&main);
            look.stripe.bottom = thin(&main);
            look.column_stripe.left = thin(&main);
            look.column_stripe.right = thin(&main);
        }
        ("light", 2) => {
            outline(&mut look.whole, &thin(&main));
            look.inner_horizontal = thin(&main);
            look.inner_vertical = thin(&main);
            look.header.bottom = medium(&main);
            look.total.top = double(&main);
            look.stripe.fill = Some(tint(pale));
            look.column_stripe.fill = Some(tint(pale));
        }
        ("medium", 0) => {
            let line = if tone == 0 { black } else { tint(0.4) };
            outline(&mut look.whole, &thin(&line));
            look.inner_horizontal = thin(&line);
            look.header.fill = Some(main.clone());
            look.header.color = Some(white);
            look.total.top = double(&main);
            look.stripe.fill = Some(tint(pale));
            look.column_stripe.fill = Some(tint(pale));
        }
        ("medium", 1) => {
            outline(&mut look.whole, &thin(&white));
            look.inner_horizontal = thin(&white);
            look.inner_vertical = thin(&white);
            look.whole.fill = Some(tint(pale));
            look.whole.color = Some(black);
            look.header.fill = Some(main.clone());
            look.header.color = Some(white.clone());
            look.header.bottom = medium(&white);
            look.total.fill = Some(main.clone());
            look.total.color = Some(white.clone());
            look.total.top = medium(&white);
            look.first_column.fill = Some(main.clone());
            look.first_column.color = Some(white.clone());
            look.last_column.fill = Some(main.clone());
            look.last_column.color = Some(white);
            look.stripe.fill = Some(tint(if tone == 0 { 0.65 } else { 0.6 }));
            look.column_stripe.fill = Some(tint(if tone == 0 { 0.65 } else { 0.6 }));
        }
        ("medium", 2) => {
            look.whole.top = medium(&black);
            look.whole.bottom = medium(&black);
            look.inner_horizontal = thin(&black);
            look.header.fill = Some(main.clone());
            look.header.color = Some(white);
            look.header.bottom = medium(&black);
            look.total.top = double(&black);
            look.stripe.fill = Some(if tone == 0 {
                theme.resolve(THEME_DARK, 0.85)
            } else {
                tint(pale)
            });
            look.column_stripe.fill = look.stripe.fill.clone();
        }
        ("medium", 3) => {
            let line = if tone == 0 { black } else { tint(0.4) };
            outline(&mut look.whole, &thin(&line));
            look.inner_horizontal = thin(&line);
            look.inner_vertical = thin(&line);
            look.whole.fill = Some(tint(pale));
            look.header.bottom = medium(&main);
            look.total.top = double(&main);
            look.stripe.fill = Some(tint(if tone == 0 { 0.65 } else { 0.6 }));
            look.column_stripe.fill = look.stripe.fill.clone();
        }
        ("dark", 0) => {
            let body = if tone == 0 {
                theme.resolve(THEME_DARK, 0.35)
            } else {
                tint(-0.25)
            };
            let band = if tone == 0 {
                theme.resolve(THEME_DARK, 0.25)
            } else {
                tint(-0.5)
            };
            look.whole.fill = Some(body);
            look.whole.color = Some(white.clone());
            look.header.fill = Some(black);
            look.header.color = Some(white.clone());
            look.header.bottom = medium(&white);
            look.total.fill = Some(band.clone());
            look.total.color = Some(white);
            look.stripe.fill = Some(band.clone());
            look.column_stripe.fill = Some(band);
        }
        ("dark", 1) => {
            look.whole.fill = Some(tint(pale));
            look.header.fill = Some(main.clone());
            look.header.color = Some(white);
            look.total.top = double(&main);
            look.stripe.fill = Some(tint(if tone == 0 { 0.65 } else { 0.6 }));
            look.column_stripe.fill = look.stripe.fill.clone();
        }
        _ => return None,
    }
    Some(look)
}

/// The colours a built-in style would use with this theme.
pub fn table_style_swatch(name: &str, theme: &Theme) -> Option<TableStyleSwatch> {
    let look = table_look(name, theme)?;
    let color = |line: &Line| line.as_ref().map(|(_, color)| color.clone());
    Some(TableStyleSwatch {
        header_fill: look.header.fill.clone(),
        header_color: look.header.color.clone(),
        body_fill: look.whole.fill.clone(),
        stripe_fill: look.stripe.fill.clone(),
        line: color(&look.inner_horizontal)
            .or_else(|| color(&look.stripe.top))
            .or_else(|| color(&look.whole.top))
            .or_else(|| color(&look.header.bottom)),
    })
}

/// What drawing needs to know about a table, gathered once per evaluation so
/// that drawing a cell stays cheap.
#[derive(Clone, Debug)]
pub(crate) struct TableRender {
    sheet: u32,
    area: PivotArea,
    has_header: bool,
    has_totals: bool,
    banded_rows: bool,
    banded_columns: bool,
    first_column: bool,
    last_column: bool,
    look: Option<TableLook>,
    default_color: Color,
    black_colors: [Color; 2],
    /// The hidden rows of its data, in order.
    hidden_rows: Vec<i32>,
}

impl TableRender {
    fn apply(&self, row: i32, column: i32, style: &mut Style) {
        let Some(look) = &self.look else {
            return;
        };
        let area = &self.area;
        let is_header = self.has_header && row == area.first_row;
        let is_total = self.has_totals && row == area.last_row;
        let mut parts: Vec<&TablePart> = vec![&look.whole];
        if !is_header && !is_total {
            if self.banded_columns && (column - area.first_column) % 2 == 0 {
                parts.push(&look.column_stripe);
            }
            if self.banded_rows {
                let first_data = area.first_row + i32::from(self.has_header);
                let hidden_above = self.hidden_rows.partition_point(|r| *r < row) as i32;
                if (row - first_data - hidden_above) % 2 == 0 {
                    parts.push(&look.stripe);
                }
            }
        }
        if self.first_column && column == area.first_column {
            parts.push(&look.first_column);
        }
        if self.last_column && column == area.last_column {
            parts.push(&look.last_column);
        }
        if is_header {
            parts.push(&look.header);
        }
        if is_total {
            parts.push(&look.total);
        }

        let mut merged = TablePart::default();
        for part in &parts {
            merged.fill = part.fill.clone().or(merged.fill);
            merged.color = part.color.clone().or(merged.color);
            merged.bold |= part.bold;
        }
        let whole = &look.whole;
        merged.top = if row == area.first_row {
            whole.top.clone()
        } else {
            look.inner_horizontal.clone()
        };
        merged.bottom = if row == area.last_row {
            whole.bottom.clone()
        } else {
            look.inner_horizontal.clone()
        };
        merged.left = if column == area.first_column {
            whole.left.clone()
        } else {
            look.inner_vertical.clone()
        };
        merged.right = if column == area.last_column {
            whole.right.clone()
        } else {
            look.inner_vertical.clone()
        };
        for part in &parts[1..] {
            merged.top = part.top.clone().or(merged.top);
            merged.bottom = part.bottom.clone().or(merged.bottom);
            merged.left = part.left.clone().or(merged.left);
            merged.right = part.right.clone().or(merged.right);
        }
        // The row under the header and the row above the totals share a line
        if self.has_header && row == area.first_row + 1 {
            merged.top = look.header.bottom.clone().or(merged.top);
        }
        if self.has_totals && row == area.last_row - 1 {
            merged.bottom = look.total.top.clone().or(merged.bottom);
        }

        if style.fill.color == Color::None {
            if let Some(fill) = merged.fill {
                style.fill.color = Color::Rgb(fill);
            }
        }
        style.font.b |= merged.bold;
        // Black is what text is unless someone chose otherwise
        let plain = style.font.color == Color::None
            || style.font.color == self.default_color
            || self.black_colors.contains(&style.font.color);
        if plain {
            if let Some(color) = merged.color {
                style.font.color = Color::Rgb(color);
            }
        }
        let lay = |slot: &mut Option<BorderItem>, line: Line| {
            if slot.is_none() {
                *slot = line.map(|(style, color)| BorderItem {
                    style,
                    color: Color::Rgb(color),
                });
            }
        };
        lay(&mut style.border.top, merged.top);
        lay(&mut style.border.bottom, merged.bottom);
        lay(&mut style.border.left, merged.left);
        lay(&mut style.border.right, merged.right);
    }
}

impl Model<'_> {
    /// The index of the sheet holding a table, if that sheet still exists.
    pub fn table_sheet(&self, table: &Table) -> Option<u32> {
        self.get_sheet_index_by_name(&table.sheet_name)
    }

    /// Lays a table's look over a cell's own format.
    pub(crate) fn apply_table_style(&self, sheet: u32, row: i32, column: i32, style: &mut Style) {
        for render in &self.table_render {
            if render.sheet == sheet && render.area.contains(row, column) {
                render.apply(row, column, style);
                return;
            }
        }
    }

    /// Whether a table may change how this cell looks.
    pub(crate) fn table_covers(&self, sheet: u32, row: i32, column: i32) -> bool {
        self.table_render
            .iter()
            .any(|r| r.sheet == sheet && r.area.contains(row, column))
    }

    pub(crate) fn sync_table_parser(&mut self) {
        self.parser.set_tables(self.workbook.tables.clone());
    }

    /// Keeps the headers and column names of tables in step, whatever changed
    /// them, then gathers what drawing needs.
    pub(crate) fn sync_tables(&mut self) {
        if self.workbook.tables.is_empty() {
            self.table_render.clear();
            return;
        }
        let mut names: Vec<String> = self.workbook.tables.keys().cloned().collect();
        names.sort();
        let mut changed = false;
        for name in &names {
            changed |= self.sync_table_header(name).unwrap_or(false);
        }
        if changed {
            self.sync_table_parser();
        }
        self.rebuild_table_render();
    }

    /// A table's header row holds its column names: unique, and always text.
    fn sync_table_header(&mut self, name: &str) -> Result<bool, String> {
        let Some(table) = self.workbook.tables.get(name) else {
            return Ok(false);
        };
        let Some(area) = table_area(table) else {
            return Ok(false);
        };
        let Some(sheet) = self.table_sheet(table) else {
            return Ok(false);
        };
        let width = (area.last_column - area.first_column + 1) as usize;
        let old = table.columns.clone();
        let has_header = table.header_row_count > 0;
        let mut next_id = old.iter().map(|c| c.id).max().unwrap_or(0);
        let row = area.first_row;

        // What each header currently says
        let mut texts = Vec::with_capacity(width);
        for offset in 0..width {
            let column = area.first_column + offset as i32;
            let text = if has_header {
                let cell = self.workbook.worksheet(sheet)?.cell(row, column);
                let is_empty = cell
                    .as_deref()
                    .is_none_or(|c| matches!(c, Cell::EmptyCell { .. }));
                if is_empty {
                    String::new()
                } else {
                    self.get_formatted_cell_value(sheet, row, column)?
                }
            } else {
                old.get(offset).map(|c| c.name.clone()).unwrap_or_default()
            };
            texts.push(text);
        }
        let mut used: HashSet<String> = HashSet::new();
        let reserved: HashSet<String> = texts.iter().map(|t| t.to_lowercase()).collect();
        let mut columns = Vec::with_capacity(width);
        let mut number = 1;
        for (offset, text) in texts.iter().enumerate() {
            let mut unique = text.clone();
            if unique.is_empty() {
                while reserved.contains(&format!("column{number}"))
                    || used.contains(&format!("column{number}"))
                {
                    number += 1;
                }
                unique = format!("Column{number}");
            } else if used.contains(&unique.to_lowercase()) {
                let mut suffix = 2;
                while used.contains(&format!("{}{suffix}", unique.to_lowercase()))
                    || reserved.contains(&format!("{}{suffix}", unique.to_lowercase()))
                {
                    suffix += 1;
                }
                unique = format!("{unique}{suffix}");
            }
            used.insert(unique.to_lowercase());
            let column = match old.get(offset) {
                Some(existing) if existing.id != 0 => TableColumn {
                    name: unique.clone(),
                    ..existing.clone()
                },
                _ => {
                    next_id += 1;
                    TableColumn {
                        id: next_id,
                        name: unique.clone(),
                        ..Default::default()
                    }
                }
            };
            columns.push(column);
            if has_header {
                let at = area.first_column + offset as i32;
                let is_text = matches!(
                    self.workbook.worksheet(sheet)?.cell(row, at).as_deref(),
                    Some(Cell::SharedString { .. })
                );
                if !is_text || unique != *text {
                    let style = self.get_cell_style_index(sheet, row, at)?;
                    self.set_cell_with_string(sheet, row, at, &unique, style)?;
                }
            }
        }
        if columns == old {
            return Ok(false);
        }
        // A rename carries the filters and slicers of the column along
        let renames: Vec<(String, String)> = old
            .iter()
            .filter_map(|o| {
                let new = columns.iter().find(|c| c.id == o.id && c.id != 0)?;
                (new.name != o.name).then(|| (o.name.clone(), new.name.clone()))
            })
            .collect();
        for (from, to) in &renames {
            self.rename_table_column_references(name, from, to);
        }
        if let Some(table) = self.workbook.tables.get_mut(name) {
            table.columns = columns;
        }
        Ok(true)
    }

    fn rename_table_column_references(&mut self, table: &str, from: &str, to: &str) {
        for state in self
            .workbook
            .table_filters
            .iter_mut()
            .filter(|s| s.table == table)
        {
            for filter in state.columns.iter_mut().filter(|f| f.column == from) {
                filter.column = to.to_string();
            }
            if let Some(sort) = state.sort.as_mut().filter(|s| s.column == from) {
                sort.column = to.to_string();
            }
        }
        for slicer in &mut self.workbook.slicers {
            if let SlicerSource::Table { table: t, column } = &mut slicer.source {
                if t == table && column == from {
                    *column = to.to_string();
                }
            }
        }
    }

    fn rebuild_table_render(&mut self) {
        let mut renders = Vec::with_capacity(self.workbook.tables.len());
        let default_color = self
            .workbook
            .styles
            .get_style(0)
            .map(|s| s.font.color)
            .unwrap_or_default();
        for table in self.workbook.tables.values() {
            let (Some(area), Some(sheet)) = (table_area(table), self.table_sheet(table)) else {
                continue;
            };
            let (first, last) = table_data_rows(table, &area);
            let mut hidden_rows: Vec<i32> = self
                .workbook
                .worksheets
                .get(sheet as usize)
                .map(|w| {
                    w.rows
                        .iter()
                        .filter(|r| r.hidden && r.r >= first && r.r <= last)
                        .map(|r| r.r)
                        .collect()
                })
                .unwrap_or_default();
            hidden_rows.sort_unstable();
            let info = &table.style_info;
            renders.push(TableRender {
                sheet,
                area,
                has_header: table.header_row_count > 0,
                has_totals: table.totals_row_count > 0,
                banded_rows: info.show_row_stripes,
                banded_columns: info.show_column_stripes,
                first_column: info.show_first_column,
                last_column: info.show_last_column,
                look: info
                    .name
                    .as_deref()
                    .and_then(|name| table_look(name, &self.workbook.theme)),
                default_color: default_color.clone(),
                black_colors: [
                    Color::Theme(THEME_DARK, 0.0),
                    Color::Rgb("#000000".to_string()),
                ],
                hidden_rows,
            });
        }
        self.table_render = renders;
    }

    /// Moves tables and slicers along with inserted (`delta > 0`) or deleted
    /// (`delta < 0`) rows or columns of a sheet.
    pub(crate) fn displace_tables(&mut self, sheet: u32, is_row: bool, at: i32, delta: i32) {
        let Some(worksheet) = self.workbook.worksheets.get(sheet as usize) else {
            return;
        };
        let sheet_id = worksheet.sheet_id;
        let sheet_name = worksheet.get_name();
        let mut gone = Vec::new();
        let mut changed = false;
        for (name, table) in &mut self.workbook.tables {
            if table.sheet_name != sheet_name {
                continue;
            }
            let Some(area) = table_area(table) else {
                continue;
            };
            let (first, last) = if is_row {
                (area.first_row, area.last_row)
            } else {
                (area.first_column, area.last_column)
            };
            let (new_first, new_last);
            if delta > 0 {
                new_first = if at <= first { first + delta } else { first };
                new_last = if at <= last { last + delta } else { last };
                if !is_row && at > first && at <= last {
                    let position = (at - first) as usize;
                    for _ in 0..delta {
                        table.columns.insert(
                            position,
                            TableColumn {
                                id: 0,
                                name: String::new(),
                                ..Default::default()
                            },
                        );
                    }
                }
            } else {
                let count = -delta;
                let end = at + count - 1;
                if first >= at && last <= end {
                    gone.push(name.clone());
                    continue;
                }
                new_first = if first < at {
                    first
                } else if first > end {
                    first - count
                } else {
                    at
                };
                new_last = if last < at {
                    last
                } else if last > end {
                    last - count
                } else {
                    at - 1
                };
                if is_row {
                    if last >= at && last <= end {
                        table.totals_row_count = 0;
                    }
                } else {
                    let from = (at - first).max(0) as usize;
                    let to = ((end - first + 1).max(0) as usize).min(table.columns.len());
                    if from < to {
                        table.columns.drain(from..to);
                    }
                }
            }
            if (new_first, new_last) == (first, last) {
                continue;
            }
            let mut moved = area;
            if is_row {
                moved.first_row = new_first;
                moved.last_row = new_last;
            } else {
                moved.first_column = new_first;
                moved.last_column = new_last;
            }
            table.reference = area_reference(&moved);
            changed = true;
        }
        for name in gone {
            self.workbook.tables.remove(&name);
            self.workbook.table_filters.retain(|s| s.table != name);
            changed = true;
        }
        for slicer in &mut self.workbook.slicers {
            if slicer.sheet_id != sheet_id || slicer.placement == SlicerPlacement::Free {
                continue;
            }
            let anchor = if is_row {
                &mut slicer.row
            } else {
                &mut slicer.column
            };
            if delta > 0 {
                if *anchor >= at {
                    *anchor += delta;
                }
            } else if *anchor >= at {
                *anchor = (*anchor + delta).max(at);
            }
        }
        if changed {
            self.sync_table_parser();
        }
    }

    /// The range of a sheet's filter, with the rows typed right below it.
    pub(crate) fn sheet_filter_area(&self, filter: &SheetFilter) -> Result<PivotArea, String> {
        let worksheet = self
            .workbook
            .worksheets
            .iter()
            .find(|worksheet| worksheet.sheet_id == filter.sheet_id)
            .ok_or("Sheet not found")?;
        let mut area = filter.area;
        let filled = |row: i32| {
            (area.first_column..=area.last_column).any(|column| {
                worksheet
                    .cell(row, column)
                    .is_some_and(|cell| !matches!(cell.as_ref(), Cell::EmptyCell { .. }))
            })
        };
        let mut last_row = area.last_row;
        while last_row < crate::constants::LAST_ROW && filled(last_row + 1) {
            last_row += 1;
        }
        area.last_row = last_row;
        Ok(area)
    }

    /// The range a sheet's filter covers now.
    pub fn sheet_filter_extent(&self, filter: &SheetFilter) -> Result<PivotArea, String> {
        self.sheet_filter_area(filter)
    }

    pub(crate) fn sheet_filter_of(&self, sheet: u32) -> Option<&SheetFilter> {
        let sheet_id = self.workbook.worksheets.get(sheet as usize)?.sheet_id;
        self.workbook
            .sheet_filters
            .iter()
            .find(|filter| filter.sheet_id == sheet_id)
    }

    /// What the filter of that name works on.
    pub(crate) fn filter_scope(&self, name: &str) -> Result<FilterScope, String> {
        if let Some(id) = name.strip_prefix(SHEET_FILTER_PREFIX) {
            let filter = self
                .workbook
                .sheet_filters
                .iter()
                .find(|filter| id.parse() == Ok(filter.sheet_id))
                .ok_or("This sheet has no filter")?;
            let sheet = self
                .workbook
                .worksheets
                .iter()
                .position(|worksheet| worksheet.sheet_id == filter.sheet_id)
                .ok_or("Sheet not found")? as u32;
            let area = self.sheet_filter_area(filter)?;
            return Ok(FilterScope {
                sheet,
                area,
                first: area.first_row + 1,
                last: area.last_row,
                columns: (area.first_column..=area.last_column)
                    .filter_map(number_to_column)
                    .collect(),
            });
        }
        let table = self
            .workbook
            .tables
            .get(name)
            .ok_or_else(|| format!("Table not found: {name}"))?;
        let area = table_area(table).ok_or("Invalid table range")?;
        let sheet = self.table_sheet(table).ok_or("Sheet not found")?;
        let (first, last) = table_data_rows(table, &area);
        Ok(FilterScope {
            sheet,
            area,
            first,
            last,
            columns: table.columns.iter().map(|c| c.name.clone()).collect(),
        })
    }

    /// Keeps a sheet's filter on its data when rows or columns move.
    pub(crate) fn displace_sheet_filters(&mut self, sheet: u32, data: &DisplaceData) {
        let Some(sheet_id) = self
            .workbook
            .worksheets
            .get(sheet as usize)
            .map(|w| w.sheet_id)
        else {
            return;
        };
        let Some(index) = self
            .workbook
            .sheet_filters
            .iter()
            .position(|filter| filter.sheet_id == sheet_id)
        else {
            return;
        };
        let name = sheet_filter_name(sheet_id);
        let area = self.workbook.sheet_filters[index].area;
        let header_deleted = matches!(
            data,
            DisplaceData::Row { row, delta, .. }
                if *delta < 0 && (*row..*row - *delta).contains(&area.first_row)
        );
        let moved = displaced_range(
            (
                area.first_row,
                area.first_column,
                area.last_row,
                area.last_column,
            ),
            data,
            sheet,
        );
        let Some((first_row, first_column, last_row, last_column)) =
            moved.filter(|_| !header_deleted)
        else {
            self.workbook.sheet_filters.remove(index);
            self.workbook.table_filters.retain(|s| s.table != name);
            return;
        };
        self.workbook.sheet_filters[index].area = PivotArea {
            first_row,
            first_column,
            last_row,
            last_column,
        };
        // The columns go by their letter
        let moved_key = |key: &str| {
            let column = column_to_number(key).ok()?;
            number_to_column(displace_cf_col(column, data, sheet)?)
        };
        for state in self
            .workbook
            .table_filters
            .iter_mut()
            .filter(|s| s.table == name)
        {
            state
                .columns
                .retain_mut(|filter| match moved_key(&filter.column) {
                    Some(key) => {
                        filter.column = key;
                        true
                    }
                    None => false,
                });
            state.sort = state.sort.take().and_then(|mut sort| {
                sort.column = moved_key(&sort.column)?;
                Some(sort)
            });
        }
        self.workbook
            .table_filters
            .retain(|s| s.table != name || !s.columns.is_empty() || s.sort.is_some());
    }

    /// Reads the cells of one table column, in row order.
    pub(crate) fn table_filter_cells(
        &self,
        sheet: u32,
        column: i32,
        first_row: i32,
        last_row: i32,
    ) -> Result<Vec<FilterCell>, String> {
        let mut cells = Vec::with_capacity((last_row - first_row + 1).max(0) as usize);
        for row in first_row..=last_row {
            let value = self.get_cell_value_by_index(sheet, row, column)?;
            let number = match value {
                CellValue::Number(number) => Some(number),
                _ => None,
            };
            let label = if matches!(value, CellValue::None) {
                BLANK_FILTER_LABEL.to_string()
            } else {
                let text = self.get_formatted_cell_value(sheet, row, column)?;
                if text.is_empty() {
                    BLANK_FILTER_LABEL.to_string()
                } else {
                    text
                }
            };
            cells.push(FilterCell { label, number });
        }
        Ok(cells)
    }

    pub fn table_filter_state(&self, table: &str) -> Option<&TableFilterState> {
        self.workbook
            .table_filters
            .iter()
            .find(|s| s.table == table)
    }

    /// Per filtered column, which rows of the table's data its filter keeps.
    fn table_filter_masks(&self, name: &str) -> Result<Vec<(String, Vec<bool>)>, String> {
        let scope = self.filter_scope(name)?;
        let mut masks = Vec::new();
        if let Some(state) = self.table_filter_state(name) {
            for filter in &state.columns {
                let Some(index) = scope.index(&filter.column) else {
                    continue;
                };
                let column = scope.area.first_column + index as i32;
                let cells =
                    self.table_filter_cells(scope.sheet, column, scope.first, scope.last)?;
                masks.push((filter.column.clone(), rule_keeps(&filter.rule, &cells)));
            }
        }
        Ok(masks)
    }

    /// `(row, hidden)` for each row of a table's data under its current filters.
    pub fn table_row_visibility(&self, name: &str) -> Result<Vec<(i32, bool)>, String> {
        let scope = self.filter_scope(name)?;
        let masks = self.table_filter_masks(name)?;
        Ok((scope.first..=scope.last)
            .enumerate()
            .map(|(offset, row)| (row, masks.iter().any(|(_, m)| !m[offset])))
            .collect())
    }

    /// The distinct values of a table column, as a filter list or slicer shows them.
    pub fn table_filter_items(
        &self,
        name: &str,
        column_name: &str,
    ) -> Result<Vec<TableFilterItem>, String> {
        let scope = self.filter_scope(name)?;
        let index = scope
            .index(column_name)
            .ok_or_else(|| format!("Column not found: {column_name}"))?;
        let cells = self.table_filter_cells(
            scope.sheet,
            scope.area.first_column + index as i32,
            scope.first,
            scope.last,
        )?;
        let masks = self.table_filter_masks(name)?;
        let own = masks
            .iter()
            .find(|(c, _)| same_name(c, column_name))
            .map(|(_, m)| m);
        let others: Vec<&Vec<bool>> = masks
            .iter()
            .filter(|(c, _)| !same_name(c, column_name))
            .map(|(_, m)| m)
            .collect();

        let mut order: Vec<usize> = (0..cells.len()).collect();
        order.sort_by(|a, b| compare_filter_cells(&cells[*a], &cells[*b], false));
        let mut items: Vec<TableFilterItem> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for position in &order {
            let key = cells[*position].label.to_lowercase();
            if seen.insert(key) {
                items.push(TableFilterItem {
                    label: cells[*position].label.clone(),
                    visible: false,
                    has_data: false,
                });
            }
        }
        for (position, cell) in cells.iter().enumerate() {
            let Some(item) = items.iter_mut().find(|i| same_name(&i.label, &cell.label)) else {
                continue;
            };
            item.visible |= own.is_none_or(|m| m[position]);
            item.has_data |= others.iter().all(|m| m[position]);
        }
        Ok(items)
    }
}
