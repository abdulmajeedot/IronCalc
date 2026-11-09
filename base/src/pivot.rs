//! PivotTables: the definition stored in a workbook and the calculation that
//! turns it into a grid of cells.
//!
//! A pivot is a definition plus ordinary cells. Its output is written to the
//! sheet as static values and only changes when the pivot is edited or
//! refreshed, the same way a spreadsheet pivot reads a snapshot of its source.
//!
//! Item values are compared case-insensitively and sorted numbers first, then
//! text, booleans, errors and blanks.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use bitcode::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::cell::CellValue;
use crate::model::Model;
use crate::types::{
    Border, BorderItem, BorderStyle, Cell, CellType, Color, Fill, SavedView, Style, Workbook,
};

const HEADER_FILL: &str = "#E3E8F0";
const TOTAL_FILL: &str = "#F1F4F8";
const RULE_COLOR: &str = "#6B7280";
/// One level of a label in a one-column layout.
const INDENT: &str = "  ";
const PERCENT_FORMAT: &str = "0.00%";

/// Returned as the error of a pivot edit that would replace cells outside its
/// current output. The caller can ask the user and retry with `overwrite`.
pub const PIVOT_OVERWRITE_ERROR: &str = "PIVOT_OVERWRITE";

/// Marks saved bytes that carry pivots. A workbook without pivots is saved in
/// the plain format, so older files and pivot-free files are byte compatible.
const PIVOT_MAGIC: &[u8; 8] = b"ICPIVOT2";
/// Adds the filters of tables and the slicers, as JSON, after the pivots.
const EXTRAS_MAGIC: &[u8; 8] = b"ICSHEET3";
/// The first format, whose pivots cannot be read any more. Their workbooks
/// still load, with the pivots' cells as plain values.
const OLD_PIVOT_MAGIC: &[u8; 8] = b"ICPIVOT1";

/// The label of an empty item.
pub const BLANK_ITEM_LABEL: &str = "(blank)";

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum PivotAggregation {
    Sum,
    Count,
    Average,
    Max,
    Min,
    Product,
    CountNumbers,
    StdDev,
    StdDevP,
    Var,
    VarP,
}

impl PivotAggregation {
    /// The word used in a default value name such as `Sum of Sales`.
    pub fn caption(&self) -> &'static str {
        match self {
            PivotAggregation::Sum => "Sum",
            PivotAggregation::Count => "Count",
            PivotAggregation::Average => "Average",
            PivotAggregation::Max => "Max",
            PivotAggregation::Min => "Min",
            PivotAggregation::Product => "Product",
            PivotAggregation::CountNumbers => "Count Numbers",
            PivotAggregation::StdDev => "StdDev",
            PivotAggregation::StdDevP => "StdDevp",
            PivotAggregation::Var => "Var",
            PivotAggregation::VarP => "Varp",
        }
    }
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum PivotSubtotals {
    None,
    Top,
    Bottom,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum PivotLayout {
    /// All row fields share one indented column.
    Compact,
    /// Each row field has its own column.
    Tabular,
}

/// A field placed on rows, columns or filters.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Clone, Default)]
pub struct PivotField {
    pub field: String,
    /// Labels of the items left out of the pivot.
    #[serde(default)]
    pub hidden_items: Vec<String>,
    #[serde(default)]
    pub sort: PivotSort,
    /// Labels in display order, used when the sort is manual.
    #[serde(default)]
    pub item_order: Vec<String>,
    #[serde(default)]
    pub label_filter: Option<PivotLabelFilter>,
    #[serde(default)]
    pub value_filter: Option<PivotValueFilter>,
    /// Labels of the items showing only their total.
    #[serde(default)]
    pub collapsed_items: Vec<String>,
    /// Overrides the pivot's subtotal setting for this field.
    #[serde(default)]
    pub subtotal: Option<PivotSubtotals>,
    /// Overrides the pivot's repeat-labels setting for this field.
    #[serde(default)]
    pub repeat_labels: Option<bool>,
    #[serde(default)]
    pub blank_line: bool,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum PivotSortOrder {
    #[default]
    Ascending,
    Descending,
    Manual,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Default)]
pub struct PivotSort {
    #[serde(default)]
    pub order: PivotSortOrder,
    /// The caption of the value to sort by, instead of by the item labels.
    #[serde(default)]
    pub by_value: Option<String>,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum PivotLabelCondition {
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
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct PivotLabelFilter {
    pub condition: PivotLabelCondition,
    pub value: String,
    /// The upper bound of Between and Not Between.
    #[serde(default)]
    pub value2: String,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum PivotValueCondition {
    Equals,
    NotEquals,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Between,
    NotBetween,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum PivotTopUnit {
    Items,
    Percent,
    Sum,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PivotValueRule {
    Compare {
        condition: PivotValueCondition,
        value: f64,
        #[serde(default)]
        value2: f64,
    },
    Top {
        count: f64,
        bottom: bool,
        unit: PivotTopUnit,
    },
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Clone)]
pub struct PivotValueFilter {
    /// The caption of the value the rule tests.
    pub value_name: String,
    pub rule: PivotValueRule,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum PivotShowAs {
    #[default]
    Normal,
    PercentOfTotal,
    PercentOfColumn,
    PercentOfRow,
    PercentOfParentRow,
    PercentOfParentColumn,
    PercentOfParent,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct PivotValue {
    pub field: String,
    pub aggregation: PivotAggregation,
    /// The custom caption. Empty means the default, such as `Sum of Sales`.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub show_as: PivotShowAs,
    /// The field `PercentOfParent` measures against.
    #[serde(default)]
    pub base_field: String,
}

impl PivotValue {
    pub fn caption(&self) -> String {
        if self.name.is_empty() {
            format!("{} of {}", self.aggregation.caption(), self.field)
        } else {
            self.name.clone()
        }
    }
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy)]
pub struct PivotArea {
    pub first_row: i32,
    pub first_column: i32,
    pub last_row: i32,
    pub last_column: i32,
}

impl PivotArea {
    pub fn contains(&self, row: i32, column: i32) -> bool {
        row >= self.first_row
            && row <= self.last_row
            && column >= self.first_column
            && column <= self.last_column
    }

    pub(crate) fn intersects(&self, other: &PivotArea) -> bool {
        self.first_row <= other.last_row
            && other.first_row <= self.last_row
            && self.first_column <= other.last_column
            && other.first_column <= self.last_column
    }
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy)]
pub struct PivotOptions {
    /// A grand total at the end of each row (the rightmost column).
    pub grand_totals_rows: bool,
    /// A grand total at the end of each column (the bottom row).
    pub grand_totals_columns: bool,
    pub subtotals: PivotSubtotals,
    pub layout: PivotLayout,
    /// Repeats outer labels on every line of a tabular pivot.
    pub repeat_labels: bool,
}

impl Default for PivotOptions {
    fn default() -> Self {
        PivotOptions {
            grand_totals_rows: true,
            grand_totals_columns: true,
            subtotals: PivotSubtotals::Top,
            layout: PivotLayout::Compact,
            repeat_labels: false,
        }
    }
}

/// The XML parts of a pivot read from an xlsx file. They are written back as
/// they came until the pivot is edited, which keeps everything IronCalc does
/// not model.
#[derive(Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct PivotRaw {
    pub table_xml: String,
    pub cache_xml: String,
    pub records_xml: Option<String>,
    /// Rows between the top of the output and the pivot's body.
    pub page_rows: i32,
}

/// A line of the output as saved: what each row or column on the sheet shows.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct PivotOutputLine {
    pub kind: PivotOutputLineKind,
    /// Item labels from the outermost field down to this line's level.
    pub path: Vec<String>,
    /// The levels of `path` that the previous line already shows.
    pub shared: u32,
    /// The value shown by a column line, as an index in `values`.
    pub value: Option<u32>,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum PivotOutputLineKind {
    Item,
    Total,
    Grand,
    Blank,
}

/// How the cells last written map to the pivot's fields and items. Menus and
/// on-sheet buttons read this instead of recalculating, so they always match
/// what is on the sheet even when the source has changed since.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Default)]
pub struct PivotOutputMap {
    pub filter_rows: i32,
    pub header_rows: i32,
    pub label_columns: i32,
    pub rows: Vec<PivotOutputLine>,
    pub columns: Vec<PivotOutputLine>,
}

#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Clone)]
pub struct PivotTable {
    pub name: String,
    /// The sheet holding the output.
    pub sheet_id: u32,
    /// The top left cell of the output, filters included.
    pub row: i32,
    pub column: i32,
    pub source_sheet_id: u32,
    /// The source data. Its first row holds the field names.
    pub source: PivotArea,
    pub rows: Vec<PivotField>,
    pub columns: Vec<PivotField>,
    pub values: Vec<PivotValue>,
    pub filters: Vec<PivotField>,
    pub options: PivotOptions,
    /// The cells last written. None while the pivot has no fields.
    #[serde(default)]
    pub output: Option<PivotArea>,
    #[serde(default)]
    pub output_map: Option<PivotOutputMap>,
    #[serde(default)]
    pub style_name: String,
    /// True when an imported pivot uses features that cannot be edited here.
    #[serde(default)]
    pub read_only: bool,
    /// Fields filtered by a slicer without being part of the layout.
    #[serde(default)]
    pub slicer_filters: Vec<PivotField>,
    #[serde(skip)]
    pub raw: Option<Box<PivotRaw>>,
}

/// A pivot as listed to the UI, with its sheets as current indexes.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct PivotTableEntry {
    pub index: u32,
    pub sheet: u32,
    pub source_sheet: Option<u32>,
    pub pivot: PivotTable,
}

/// What a cell of a pivot is, for its right-click menu.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct PivotCellInfo {
    pub index: u32,
    pub zone: PivotCellZone,
    /// The field the cell belongs to, for filters, headers and labels.
    pub field: Option<String>,
    pub level: u32,
    pub item: Option<String>,
    pub has_children: bool,
    pub collapsed: bool,
    pub value_index: Option<u32>,
    pub row_path: Vec<String>,
    pub column_path: Vec<String>,
    pub read_only: bool,
}

#[derive(Serialize, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum PivotCellZone {
    Filter,
    RowHeader,
    ColumnHeader,
    RowLabel,
    ColumnLabel,
    Value,
    Total,
}

/// A button drawn on the sheet: a field's filter arrow or an item's +/-.
#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct PivotMarker {
    pub index: u32,
    pub row: i32,
    pub column: i32,
    pub kind: PivotMarkerKind,
    pub field: String,
    pub item: Option<String>,
    /// Whether the field currently has any filter on it.
    pub filtered: bool,
}

#[derive(Serialize, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum PivotMarkerKind {
    Filter,
    Expand,
    Collapse,
}

#[derive(Serialize, Debug, PartialEq, Clone)]
pub struct PivotFieldInfo {
    pub name: String,
    /// True if every non-blank value is a number. Such a field defaults to Values.
    pub numeric: bool,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PivotItem {
    Number(f64),
    Text(String),
    Boolean(bool),
    Error(String),
    Blank,
}

#[derive(Debug, Clone)]
struct ItemKey(PivotItem);

impl ItemKey {
    fn of(item: &PivotItem) -> Self {
        ItemKey(match item {
            PivotItem::Text(text) => PivotItem::Text(text.to_lowercase()),
            other => other.clone(),
        })
    }

    fn class(&self) -> u8 {
        match self.0 {
            PivotItem::Number(_) => 0,
            PivotItem::Text(_) => 1,
            PivotItem::Boolean(_) => 2,
            PivotItem::Error(_) => 3,
            PivotItem::Blank => 4,
        }
    }
}

impl Ord for ItemKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.0, &other.0) {
            (PivotItem::Number(a), PivotItem::Number(b)) => a.total_cmp(b),
            (PivotItem::Text(a), PivotItem::Text(b)) => a.cmp(b),
            (PivotItem::Boolean(a), PivotItem::Boolean(b)) => a.cmp(b),
            (PivotItem::Error(a), PivotItem::Error(b)) => a.cmp(b),
            _ => self.class().cmp(&other.class()),
        }
    }
}

impl PartialOrd for ItemKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for ItemKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for ItemKey {}

/// One distinct value of a field.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldItem {
    pub value: PivotItem,
    pub label: String,
    pub number_format: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldData {
    pub name: String,
    /// Distinct values in display order.
    pub items: Vec<FieldItem>,
}

/// The source of a pivot as a table of item indexes.
#[derive(Debug, Clone, PartialEq)]
pub struct PivotData {
    pub fields: Vec<FieldData>,
    /// One entry per record, one item index per field.
    pub records: Vec<Vec<u32>>,
}

impl PivotData {
    pub fn field_index(&self, name: &str) -> Option<usize> {
        self.fields.iter().position(|f| f.name == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PivotCellKind {
    Header,
    Item,
    Value,
    Parent,
    Total,
    GrandTotal,
    Filter,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PivotGridCell {
    pub row: i32,
    pub column: i32,
    pub value: PivotItem,
    pub kind: PivotCellKind,
    /// Empty uses the general format.
    pub number_format: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PivotLineKind {
    Item,
    Total,
    Grand,
    Blank,
}

/// A line of the body: a row of the output, or a column of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotLine {
    pub kind: PivotLineKind,
    /// Item indexes from the outermost field down to this line's level.
    pub path: Vec<u32>,
    /// Whether the line shows numbers. A compact parent without a subtotal does not.
    pub show_values: bool,
    /// The levels of `path` shared with the previous leaf.
    pub shared: usize,
    /// The value shown by a column line, as an index in `values`.
    pub value: Option<usize>,
}

/// The result of laying out a pivot. Offsets are relative to the top left cell
/// of the output.
#[derive(Debug, Clone, PartialEq)]
pub struct PivotGrid {
    pub cells: Vec<PivotGridCell>,
    pub height: i32,
    pub width: i32,
    /// Rows above the body, taken by the filters and the gap below them.
    pub filter_rows: i32,
    pub label_columns: i32,
    pub header_rows: i32,
    pub row_lines: Vec<PivotLine>,
    pub column_lines: Vec<PivotLine>,
    pub map: PivotOutputMap,
    pub data: PivotData,
}

impl PivotGrid {
    pub fn area(&self, row: i32, column: i32) -> Option<PivotArea> {
        if self.height == 0 || self.width == 0 {
            return None;
        }
        Some(PivotArea {
            first_row: row,
            first_column: column,
            last_row: row + self.height - 1,
            last_column: column + self.width - 1,
        })
    }
}

#[derive(Clone, Default)]
struct Accumulator {
    count: u64,
    numbers: u64,
    sum: f64,
    product: f64,
    min: f64,
    max: f64,
    mean: f64,
    variance_sum: f64,
    error: Option<String>,
}

impl Accumulator {
    fn add(&mut self, item: &PivotItem) {
        match item {
            PivotItem::Blank => {}
            PivotItem::Number(value) => {
                self.count += 1;
                self.numbers += 1;
                if self.numbers == 1 {
                    self.product = *value;
                    self.min = *value;
                    self.max = *value;
                } else {
                    self.product *= value;
                    self.min = self.min.min(*value);
                    self.max = self.max.max(*value);
                }
                self.sum += value;
                let delta = value - self.mean;
                self.mean += delta / self.numbers as f64;
                self.variance_sum += delta * (value - self.mean);
            }
            PivotItem::Error(error) => {
                self.count += 1;
                self.error.get_or_insert_with(|| error.clone());
            }
            _ => self.count += 1,
        }
    }

    fn result(&self, aggregation: PivotAggregation) -> PivotItem {
        let div_zero = || PivotItem::Error("#DIV/0!".to_string());
        if let Some(error) = &self.error {
            if aggregation != PivotAggregation::Count {
                return PivotItem::Error(error.clone());
            }
        }
        let n = self.numbers as f64;
        match aggregation {
            PivotAggregation::Sum => PivotItem::Number(self.sum),
            PivotAggregation::Count => PivotItem::Number(self.count as f64),
            PivotAggregation::CountNumbers => PivotItem::Number(n),
            PivotAggregation::Average if self.numbers == 0 => div_zero(),
            PivotAggregation::Average => PivotItem::Number(self.sum / n),
            PivotAggregation::Max => {
                PivotItem::Number(if self.numbers == 0 { 0.0 } else { self.max })
            }
            PivotAggregation::Min => {
                PivotItem::Number(if self.numbers == 0 { 0.0 } else { self.min })
            }
            PivotAggregation::Product => {
                PivotItem::Number(if self.numbers == 0 { 0.0 } else { self.product })
            }
            PivotAggregation::Var | PivotAggregation::StdDev if self.numbers < 2 => div_zero(),
            PivotAggregation::VarP | PivotAggregation::StdDevP if self.numbers == 0 => div_zero(),
            PivotAggregation::Var => PivotItem::Number(self.variance_sum / (n - 1.0)),
            PivotAggregation::VarP => PivotItem::Number(self.variance_sum / n),
            PivotAggregation::StdDev => PivotItem::Number((self.variance_sum / (n - 1.0)).sqrt()),
            PivotAggregation::StdDevP => PivotItem::Number((self.variance_sum / n).sqrt()),
        }
    }
}

/// Adds the look of a kind of pivot cell to a style, keeping everything else.
pub(crate) fn apply_pivot_look(style: &mut Style, kind: PivotCellKind) {
    if !matches!(
        kind,
        PivotCellKind::Item | PivotCellKind::Value | PivotCellKind::Filter
    ) {
        style.font.b = true;
    }
    match kind {
        PivotCellKind::Header => {
            style.fill = Fill {
                color: Color::Rgb(HEADER_FILL.to_string()),
            }
        }
        PivotCellKind::Total => {
            style.fill = Fill {
                color: Color::Rgb(TOTAL_FILL.to_string()),
            }
        }
        PivotCellKind::GrandTotal => {
            style.border = Border {
                top: Some(BorderItem {
                    style: BorderStyle::Thin,
                    color: Color::Rgb(RULE_COLOR.to_string()),
                }),
                ..Border::default()
            }
        }
        _ => {}
    }
}

/// A cell of a pivot read from a file, and how it should look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotPaintCell {
    pub row: i32,
    pub column: i32,
    pub kind: PivotCellKind,
    /// The level of a label in a one-column layout. Zero leaves the text alone.
    pub indent: usize,
}

/// Gives the cells of an imported pivot the look of a pivot built here. A file
/// keeps a pivot's colours and indents in the pivot definition rather than in
/// its cells, so without this they arrive plain. Values are left as they are.
pub fn paint_pivot_cells(
    workbook: &mut Workbook,
    sheet_id: u32,
    cells: &[PivotPaintCell],
) -> Result<(), String> {
    let Workbook {
        worksheets,
        styles,
        shared_strings,
        ..
    } = workbook;
    let worksheet = worksheets
        .iter_mut()
        .find(|w| w.sheet_id == sheet_id)
        .ok_or_else(|| "The sheet of this PivotTable does not exist".to_string())?;
    let mut painted: HashMap<(i32, PivotCellKind), i32> = HashMap::new();
    let mut indented: HashMap<String, i32> = HashMap::new();
    for paint in cells {
        let old_style = worksheet.get_style(paint.row, paint.column);
        let new_style = match painted.get(&(old_style, paint.kind)) {
            Some(index) => *index,
            None => {
                let mut style = styles.get_style(old_style)?;
                apply_pivot_look(&mut style, paint.kind);
                let index = styles.get_style_index_or_create(&style);
                painted.insert((old_style, paint.kind), index);
                index
            }
        };
        worksheet.set_cell_style(paint.row, paint.column, new_style)?;
        if paint.indent == 0 {
            continue;
        }
        let Some(Cell::SharedString { si, s }) =
            worksheet.cell(paint.row, paint.column).map(Cow::into_owned)
        else {
            continue;
        };
        let Some(text) = usize::try_from(si).ok().and_then(|i| shared_strings.get(i)) else {
            continue;
        };
        // A file this app exported already has the indent in its text
        let text = format!("{}{}", INDENT.repeat(paint.indent), text.trim_start());
        let index = *indented.entry(text).or_insert_with_key(|text| {
            shared_strings.push(text.clone());
            shared_strings.len() as i32 - 1
        });
        worksheet.update_cell(paint.row, paint.column, Cell::new_string(index, s))?;
    }
    Ok(())
}

/// Pivots as saved. The definitions are JSON, so adding a field to them later
/// does not make older files unreadable.
#[derive(Encode, Decode)]
struct SavedPivots {
    definitions: String,
    raw: Vec<Option<Box<PivotRaw>>>,
}

/// Encodes a workbook for saving.
pub fn workbook_to_bytes(workbook: &Workbook) -> Vec<u8> {
    workbook_to_bytes_with_view(workbook, &workbook.saved_view)
}

/// Like [`workbook_to_bytes`], recording `view` as where the user is.
pub fn workbook_to_bytes_with_view(workbook: &Workbook, view: &SavedView) -> Vec<u8> {
    let body = bitcode::encode(workbook);
    if workbook.pivot_tables.is_empty()
        && workbook.table_filters.is_empty()
        && workbook.slicers.is_empty()
        && workbook.data_validations.is_empty()
        && workbook.sheet_filters.is_empty()
        && workbook.sheet_protections.is_empty()
        && *view == SavedView::default()
    {
        return body;
    }
    let saved = SavedPivots {
        definitions: serde_json::to_string(&workbook.pivot_tables).unwrap_or_default(),
        raw: workbook
            .pivot_tables
            .iter()
            .map(|p| p.raw.clone())
            .collect(),
    };
    let pivots = bitcode::encode(&saved);
    let extras = serde_json::to_string(&SavedExtras {
        table_filters: workbook.table_filters.clone(),
        slicers: workbook.slicers.clone(),
        view: view.clone(),
        data_validations: workbook.data_validations.clone(),
        sheet_filters: workbook.sheet_filters.clone(),
        sheet_protections: workbook.sheet_protections.clone(),
    })
    .unwrap_or_default();
    let mut bytes =
        Vec::with_capacity(EXTRAS_MAGIC.len() + 16 + body.len() + pivots.len() + extras.len());
    bytes.extend_from_slice(EXTRAS_MAGIC);
    bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&body);
    bytes.extend_from_slice(&(pivots.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&pivots);
    bytes.extend_from_slice(extras.as_bytes());
    bytes
}

#[derive(Serialize, Deserialize)]
struct SavedExtras {
    #[serde(default)]
    table_filters: Vec<crate::table::TableFilterState>,
    #[serde(default)]
    slicers: Vec<crate::table::Slicer>,
    #[serde(default)]
    view: SavedView,
    #[serde(default)]
    data_validations: Vec<crate::data_validation::DataValidation>,
    #[serde(default)]
    sheet_filters: Vec<crate::table::SheetFilter>,
    #[serde(default)]
    sheet_protections: Vec<crate::protection::SheetProtection>,
}

fn restore_pivots(workbook: &mut Workbook, pivots: &[u8]) -> Result<(), String> {
    let saved: SavedPivots =
        bitcode::decode(pivots).map_err(|e| format!("Error parsing workbook: {e}"))?;
    let mut definitions: Vec<PivotTable> = serde_json::from_str(&saved.definitions)
        .map_err(|e| format!("Error parsing workbook: {e}"))?;
    for (pivot, raw) in definitions.iter_mut().zip(saved.raw) {
        pivot.raw = raw;
    }
    workbook.pivot_tables = definitions;
    Ok(())
}

fn split_envelope(rest: &[u8]) -> Result<(&[u8], &[u8]), String> {
    let truncated = || "Error parsing workbook: truncated pivot data".to_string();
    let (length, rest) = rest.split_first_chunk::<8>().ok_or_else(truncated)?;
    let length = usize::try_from(u64::from_le_bytes(*length)).map_err(|_| truncated())?;
    if length > rest.len() {
        return Err(truncated());
    }
    Ok(rest.split_at(length))
}

/// Decodes saved bytes, in any format.
pub fn workbook_from_bytes(bytes: &[u8]) -> Result<Workbook, String> {
    let parse_error = |e: bitcode::Error| format!("Error parsing workbook: {e}");
    if let Some(rest) = bytes.strip_prefix(OLD_PIVOT_MAGIC) {
        let (body, _) = split_envelope(rest)?;
        return bitcode::decode(body).map_err(parse_error);
    }
    if let Some(rest) = bytes.strip_prefix(EXTRAS_MAGIC) {
        let (body, rest) = split_envelope(rest)?;
        let (pivots, extras) = split_envelope(rest)?;
        let mut workbook: Workbook = bitcode::decode(body).map_err(parse_error)?;
        restore_pivots(&mut workbook, pivots)?;
        let extras: SavedExtras =
            serde_json::from_slice(extras).map_err(|e| format!("Error parsing workbook: {e}"))?;
        workbook.table_filters = extras.table_filters;
        workbook.slicers = extras.slicers;
        workbook.saved_view = extras.view;
        workbook.data_validations = extras.data_validations;
        workbook.sheet_filters = extras.sheet_filters;
        workbook.sheet_protections = extras.sheet_protections;
        return Ok(workbook);
    }
    let Some(rest) = bytes.strip_prefix(PIVOT_MAGIC) else {
        return bitcode::decode(bytes).map_err(parse_error);
    };
    let (body, pivots) = split_envelope(rest)?;
    let mut workbook: Workbook = bitcode::decode(body).map_err(parse_error)?;
    restore_pivots(&mut workbook, pivots)?;
    Ok(workbook)
}

fn common_prefix(a: &[u32], b: &[u32]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn line(kind: PivotLineKind, path: Vec<u32>, show_values: bool, shared: usize) -> PivotLine {
    PivotLine {
        kind,
        path,
        show_values,
        shared,
        value: None,
    }
}

/// Everything the lines of one axis depend on.
struct AxisContext<'a> {
    fields: &'a [&'a PivotField],
    field_indexes: &'a [usize],
    data: &'a PivotData,
    children: &'a HashMap<Vec<u32>, BTreeSet<u32>>,
    values: &'a [&'a PivotValue],
    options: &'a PivotOptions,
    is_rows: bool,
    /// The total of a value for an item of this axis.
    total: &'a dyn Fn(&[u32], usize) -> Option<f64>,
}

impl AxisContext<'_> {
    fn items(&self, level: usize) -> &[FieldItem] {
        &self.data.fields[self.field_indexes[level]].items
    }

    fn sorted_children(&self, level: usize, prefix: &[u32]) -> Vec<u32> {
        let Some(children) = self.children.get(prefix) else {
            return vec![];
        };
        let mut sorted: Vec<u32> = children.iter().copied().collect();
        let sort = &self.fields[level].sort;
        let by_value = sort
            .by_value
            .as_ref()
            .and_then(|name| self.values.iter().position(|v| &v.caption() == name));
        if let Some(value) = by_value {
            let number = |item: u32| {
                let mut path = prefix.to_vec();
                path.push(item);
                (self.total)(&path, value).unwrap_or(f64::NEG_INFINITY)
            };
            sorted.sort_by(|a, b| number(*a).total_cmp(&number(*b)));
            if sort.order == PivotSortOrder::Descending {
                sorted.reverse();
            }
            return sorted;
        }
        match sort.order {
            PivotSortOrder::Ascending => {}
            PivotSortOrder::Descending => sorted.reverse(),
            PivotSortOrder::Manual => {
                let order = &self.fields[level].item_order;
                let items = self.items(level);
                sorted.sort_by_key(|item| {
                    let label = &items[*item as usize].label;
                    order.iter().position(|l| l == label).unwrap_or(order.len())
                });
            }
        }
        sorted
    }

    fn walk(
        &self,
        level: usize,
        prefix: &mut Vec<u32>,
        last_item: &mut Vec<u32>,
        lines: &mut Vec<PivotLine>,
    ) {
        let depth = self.fields.len();
        let field = self.fields[level];
        let subtotal = field.subtotal.unwrap_or(self.options.subtotals);
        let compact = self.options.layout == PivotLayout::Compact;
        for item in self.sorted_children(level, prefix) {
            prefix.push(item);
            let is_leaf = level + 1 == depth;
            let label = &self.items(level)[item as usize].label;
            let collapsed = !is_leaf && field.collapsed_items.contains(label);
            let mut item_line = |show_values: bool, last_item: &mut Vec<u32>| {
                let shared = common_prefix(prefix, last_item);
                *last_item = prefix.clone();
                lines.push(line(
                    PivotLineKind::Item,
                    prefix.clone(),
                    show_values,
                    shared,
                ));
            };
            if is_leaf || collapsed {
                item_line(true, last_item);
            } else {
                let top = subtotal == PivotSubtotals::Top;
                if self.is_rows && (compact || top) {
                    item_line(top, last_item);
                }
                self.walk(level + 1, prefix, last_item, lines);
                let total = if self.is_rows {
                    subtotal == PivotSubtotals::Bottom
                } else {
                    subtotal != PivotSubtotals::None
                };
                if total {
                    lines.push(line(PivotLineKind::Total, prefix.clone(), true, level));
                }
            }
            if self.is_rows && field.blank_line {
                lines.push(line(PivotLineKind::Blank, prefix.clone(), false, level));
            }
            prefix.pop();
        }
    }

    fn lines(&self) -> Vec<PivotLine> {
        let mut lines = Vec::new();
        self.walk(0, &mut Vec::new(), &mut Vec::new(), &mut lines);
        lines
    }
}

/// `*` and `?` work as in a spreadsheet filter. The text is already lowercase.
pub(crate) fn wildcard_match(pattern: &[char], text: &[char]) -> bool {
    match pattern.first() {
        None => text.is_empty(),
        Some('*') => {
            wildcard_match(&pattern[1..], text)
                || (!text.is_empty() && wildcard_match(pattern, &text[1..]))
        }
        Some('?') => !text.is_empty() && wildcard_match(&pattern[1..], &text[1..]),
        Some(c) => text.first() == Some(c) && wildcard_match(&pattern[1..], &text[1..]),
    }
}

fn compare_label(item: &FieldItem, target: &str) -> Ordering {
    if let (PivotItem::Number(number), Ok(target)) = (&item.value, target.trim().parse::<f64>()) {
        return number.total_cmp(&target);
    }
    item.label.to_lowercase().cmp(&target.to_lowercase())
}

/// Whether a label filter keeps an item.
pub fn label_filter_keeps(filter: &PivotLabelFilter, item: &FieldItem) -> bool {
    use PivotLabelCondition as C;
    let matches = |pattern: String| {
        let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
        let text: Vec<char> = item.label.to_lowercase().chars().collect();
        wildcard_match(&pattern, &text)
    };
    let value = &filter.value;
    match filter.condition {
        C::Equals => matches(value.clone()),
        C::NotEquals => !matches(value.clone()),
        C::BeginsWith => matches(format!("{value}*")),
        C::NotBeginsWith => !matches(format!("{value}*")),
        C::EndsWith => matches(format!("*{value}")),
        C::NotEndsWith => !matches(format!("*{value}")),
        C::Contains => matches(format!("*{value}*")),
        C::NotContains => !matches(format!("*{value}*")),
        C::GreaterThan => compare_label(item, value) == Ordering::Greater,
        C::GreaterThanOrEqual => compare_label(item, value) != Ordering::Less,
        C::LessThan => compare_label(item, value) == Ordering::Less,
        C::LessThanOrEqual => compare_label(item, value) != Ordering::Greater,
        C::Between => {
            compare_label(item, value) != Ordering::Less
                && compare_label(item, &filter.value2) != Ordering::Greater
        }
        C::NotBetween => {
            compare_label(item, value) == Ordering::Less
                || compare_label(item, &filter.value2) == Ordering::Greater
        }
    }
}

fn value_condition_keeps(condition: PivotValueCondition, number: f64, a: f64, b: f64) -> bool {
    use PivotValueCondition as C;
    match condition {
        C::Equals => number == a,
        C::NotEquals => number != a,
        C::GreaterThan => number > a,
        C::GreaterThanOrEqual => number >= a,
        C::LessThan => number < a,
        C::LessThanOrEqual => number <= a,
        C::Between => number >= a && number <= b,
        C::NotBetween => number < a || number > b,
    }
}

/// The items a value rule removes from one group. Items are `(item, total)`.
fn value_rule_removes(rule: &PivotValueRule, items: &[(u32, Option<f64>)]) -> Vec<u32> {
    match rule {
        PivotValueRule::Compare {
            condition,
            value,
            value2,
        } => items
            .iter()
            .filter(|(_, number)| {
                !number.is_some_and(|n| value_condition_keeps(*condition, n, *value, *value2))
            })
            .map(|(item, _)| *item)
            .collect(),
        PivotValueRule::Top {
            count,
            bottom,
            unit,
        } => {
            let mut ranked: Vec<(u32, f64)> = items
                .iter()
                .filter_map(|(item, number)| number.map(|n| (*item, n)))
                .collect();
            ranked.sort_by(|a, b| {
                if *bottom {
                    a.1.total_cmp(&b.1)
                } else {
                    b.1.total_cmp(&a.1)
                }
            });
            let keep = match unit {
                PivotTopUnit::Items => *count as usize,
                PivotTopUnit::Percent => {
                    ((ranked.len() as f64 * count / 100.0).ceil() as usize).max(1)
                }
                PivotTopUnit::Sum => {
                    let mut sum = 0.0;
                    ranked
                        .iter()
                        .take_while(|(_, number)| {
                            let inside = sum < *count;
                            sum += number;
                            inside
                        })
                        .count()
                }
            };
            let mut removed: Vec<u32> = ranked.iter().skip(keep).map(|(item, _)| *item).collect();
            removed.extend(
                items
                    .iter()
                    .filter(|(_, number)| number.is_none())
                    .map(|(item, _)| *item),
            );
            removed
        }
    }
}

fn output_lines(
    lines: &[PivotLine],
    field_indexes: &[usize],
    data: &PivotData,
) -> Vec<PivotOutputLine> {
    lines
        .iter()
        .map(|line| PivotOutputLine {
            kind: match line.kind {
                PivotLineKind::Item => PivotOutputLineKind::Item,
                PivotLineKind::Total => PivotOutputLineKind::Total,
                PivotLineKind::Grand => PivotOutputLineKind::Grand,
                PivotLineKind::Blank => PivotOutputLineKind::Blank,
            },
            path: line
                .path
                .iter()
                .zip(field_indexes)
                .map(|(item, field)| data.fields[*field].items[*item as usize].label.clone())
                .collect(),
            shared: line.shared as u32,
            value: line.value.map(|v| v as u32),
        })
        .collect()
}

/// Lays out a pivot over data that has already been read.
pub fn layout_pivot(pivot: &PivotTable, data: PivotData) -> PivotGrid {
    type CellKey = (Vec<u32>, Vec<u32>);
    let known = |f: &&PivotField| data.field_index(&f.field).is_some();
    let rows: Vec<&PivotField> = pivot.rows.iter().filter(known).collect();
    let columns: Vec<&PivotField> = pivot.columns.iter().filter(known).collect();
    let filters: Vec<&PivotField> = pivot.filters.iter().filter(known).collect();
    let values: Vec<&PivotValue> = pivot
        .values
        .iter()
        .filter(|v| data.field_index(&v.field).is_some())
        .collect();
    let options = &pivot.options;
    let index_of = |name: &str| data.field_index(name).unwrap_or(0);
    let row_fields: Vec<usize> = rows.iter().map(|f| index_of(&f.field)).collect();
    let column_fields: Vec<usize> = columns.iter().map(|f| index_of(&f.field)).collect();
    let value_fields: Vec<usize> = values.iter().map(|v| index_of(&v.field)).collect();

    let empty = rows.is_empty() && columns.is_empty() && values.is_empty();
    if empty && filters.is_empty() {
        return PivotGrid {
            cells: vec![],
            height: 0,
            width: 0,
            filter_rows: 0,
            label_columns: 0,
            header_rows: 0,
            row_lines: vec![],
            column_lines: vec![],
            map: PivotOutputMap::default(),
            data,
        };
    }

    // Items left out by any field of the layout
    let mut hidden: Vec<(usize, Vec<bool>)> = Vec::new();
    let slicer_only: Vec<&PivotField> = pivot.slicer_filters.iter().filter(known).collect();
    for field in rows
        .iter()
        .chain(&columns)
        .chain(&filters)
        .chain(&slicer_only)
    {
        if field.hidden_items.is_empty() && field.label_filter.is_none() {
            continue;
        }
        let index = index_of(&field.field);
        let mask = data.fields[index]
            .items
            .iter()
            .map(|item| {
                field.hidden_items.contains(&item.label)
                    || field
                        .label_filter
                        .as_ref()
                        .is_some_and(|filter| !label_filter_keeps(filter, item))
            })
            .collect();
        hidden.push((index, mask));
    }

    let tally = |excluded: &HashSet<(bool, Vec<u32>)>| {
        let mut totals: HashMap<CellKey, Vec<Accumulator>> = HashMap::new();
        let mut row_children: HashMap<Vec<u32>, BTreeSet<u32>> = HashMap::new();
        let mut column_children: HashMap<Vec<u32>, BTreeSet<u32>> = HashMap::new();
        for record in &data.records {
            if hidden
                .iter()
                .any(|(field, mask)| mask[record[*field] as usize])
            {
                continue;
            }
            let row_path: Vec<u32> = row_fields.iter().map(|f| record[*f]).collect();
            let column_path: Vec<u32> = column_fields.iter().map(|f| record[*f]).collect();
            let is_excluded = |is_rows: bool, path: &[u32]| {
                (1..=path.len()).any(|n| excluded.contains(&(is_rows, path[..n].to_vec())))
            };
            if is_excluded(true, &row_path) || is_excluded(false, &column_path) {
                continue;
            }
            for row_prefix in 0..=row_path.len() {
                for column_prefix in 0..=column_path.len() {
                    let entry = totals
                        .entry((
                            row_path[..row_prefix].to_vec(),
                            column_path[..column_prefix].to_vec(),
                        ))
                        .or_insert_with(|| vec![Accumulator::default(); values.len()]);
                    for (accumulator, field) in entry.iter_mut().zip(&value_fields) {
                        accumulator.add(&data.fields[*field].items[record[*field] as usize].value);
                    }
                }
            }
            for n in 0..row_path.len() {
                row_children
                    .entry(row_path[..n].to_vec())
                    .or_default()
                    .insert(row_path[n]);
            }
            for n in 0..column_path.len() {
                column_children
                    .entry(column_path[..n].to_vec())
                    .or_default()
                    .insert(column_path[n]);
            }
        }
        (totals, row_children, column_children)
    };

    let number_at =
        |totals: &HashMap<CellKey, Vec<Accumulator>>, key: CellKey, value: usize| match totals
            .get(&key)?[value]
            .result(values[value].aggregation)
        {
            PivotItem::Number(number) => Some(number),
            _ => None,
        };

    // Value filters test each item's total within its group, then every total
    // is worked out again from what is left.
    let (first_totals, first_rows, first_columns) = tally(&HashSet::new());
    let mut excluded: HashSet<(bool, Vec<u32>)> = HashSet::new();
    for (is_rows, axis, children) in [
        (true, &rows, &first_rows),
        (false, &columns, &first_columns),
    ] {
        for (level, field) in axis.iter().enumerate() {
            let Some(filter) = &field.value_filter else {
                continue;
            };
            let Some(value) = values.iter().position(|v| v.caption() == filter.value_name) else {
                continue;
            };
            for (prefix, items) in children.iter().filter(|(p, _)| p.len() == level) {
                let totals: Vec<(u32, Option<f64>)> = items
                    .iter()
                    .map(|item| {
                        let mut path = prefix.clone();
                        path.push(*item);
                        let key = if is_rows {
                            (path, vec![])
                        } else {
                            (vec![], path)
                        };
                        (*item, number_at(&first_totals, key, value))
                    })
                    .collect();
                for item in value_rule_removes(&filter.rule, &totals) {
                    let mut path = prefix.clone();
                    path.push(item);
                    excluded.insert((is_rows, path));
                }
            }
        }
    }
    let (totals, row_children, column_children) = if excluded.is_empty() {
        (first_totals, first_rows, first_columns)
    } else {
        tally(&excluded)
    };

    // The lines of the body
    let row_depth = rows.len();
    let column_depth = columns.len();
    let value_count = values.len();
    let row_total = |path: &[u32], value: usize| number_at(&totals, (path.to_vec(), vec![]), value);
    let column_total =
        |path: &[u32], value: usize| number_at(&totals, (vec![], path.to_vec()), value);
    let mut row_lines = if row_depth > 0 {
        AxisContext {
            fields: &rows,
            field_indexes: &row_fields,
            data: &data,
            children: &row_children,
            values: &values,
            options,
            is_rows: true,
            total: &row_total,
        }
        .lines()
    } else {
        vec![line(PivotLineKind::Grand, vec![], true, 0)]
    };
    if row_depth > 0 && options.grand_totals_columns {
        row_lines.push(line(PivotLineKind::Grand, vec![], true, 0));
    }
    let mut base_columns = if column_depth > 0 {
        AxisContext {
            fields: &columns,
            field_indexes: &column_fields,
            data: &data,
            children: &column_children,
            values: &values,
            options,
            is_rows: false,
            total: &column_total,
        }
        .lines()
    } else {
        vec![]
    };
    if column_depth > 0 && options.grand_totals_rows {
        base_columns.push(line(PivotLineKind::Grand, vec![], true, 0));
    }
    if column_depth == 0 && value_count > 0 {
        base_columns.push(line(PivotLineKind::Grand, vec![], true, 0));
    }
    let mut column_lines = Vec::new();
    for base in &base_columns {
        if value_count == 0 {
            column_lines.push(base.clone());
        }
        for value in 0..value_count {
            let mut expanded = base.clone();
            expanded.value = Some(value);
            column_lines.push(expanded);
        }
    }

    let compact = options.layout == PivotLayout::Compact;
    let label_columns = if row_depth > 0 {
        if compact {
            1
        } else {
            row_depth as i32
        }
    } else if column_depth > 0 {
        1
    } else {
        0
    };
    let values_row = column_depth > 0 && value_count > 1;
    let header_rows = if column_depth > 0 {
        1 + column_depth as i32 + i32::from(values_row)
    } else if value_count > 0 || row_depth > 0 {
        1
    } else {
        0
    };
    let filter_rows = if filters.is_empty() {
        0
    } else {
        filters.len() as i32 + 1
    };
    let body_lines = if empty { 0 } else { row_lines.len() as i32 };
    let height = filter_rows + if empty { -1 } else { header_rows + body_lines };
    let width =
        (label_columns + column_lines.len() as i32).max(if filters.is_empty() { 1 } else { 2 });
    let body_width = label_columns + column_lines.len() as i32;

    let mut grid: HashMap<(i32, i32), PivotGridCell> = HashMap::new();
    let mut put =
        |row: i32, column: i32, value: PivotItem, kind: PivotCellKind, number_format: &str| {
            grid.insert(
                (row, column),
                PivotGridCell {
                    row,
                    column,
                    value,
                    kind,
                    number_format: number_format.to_string(),
                },
            );
        };
    let text = |s: &str| PivotItem::Text(s.to_string());

    for (index, field) in filters.iter().enumerate() {
        let row = index as i32;
        put(row, 0, text(&field.field), PivotCellKind::Filter, "");
        let items = &data.fields[index_of(&field.field)].items;
        let visible: Vec<&FieldItem> = items
            .iter()
            .filter(|item| !field.hidden_items.contains(&item.label))
            .collect();
        let selection = if field.hidden_items.is_empty() {
            "(All)".to_string()
        } else if visible.len() == 1 {
            visible[0].label.clone()
        } else {
            "(Multiple Items)".to_string()
        };
        put(row, 1, PivotItem::Text(selection), PivotCellKind::Item, "");
    }

    if !empty {
        let top = filter_rows;
        for row in top..top + header_rows {
            for column in 0..body_width {
                put(row, column, PivotItem::Blank, PivotCellKind::Header, "");
            }
        }
        let header_label_row = top + header_rows - 1;
        if row_depth > 0 {
            if compact {
                put(
                    header_label_row,
                    0,
                    text("Row Labels"),
                    PivotCellKind::Header,
                    "",
                );
            } else {
                for (column, field) in rows.iter().enumerate() {
                    put(
                        header_label_row,
                        column as i32,
                        text(&field.field),
                        PivotCellKind::Header,
                        "",
                    );
                }
            }
        }
        if column_depth > 0 {
            if value_count == 1 {
                put(
                    top,
                    0,
                    PivotItem::Text(values[0].caption()),
                    PivotCellKind::Header,
                    "",
                );
            }
            if compact {
                put(
                    top,
                    label_columns,
                    text("Column Labels"),
                    PivotCellKind::Header,
                    "",
                );
            } else {
                for (level, field) in columns.iter().enumerate() {
                    put(
                        top,
                        label_columns + level as i32,
                        text(&field.field),
                        PivotCellKind::Header,
                        "",
                    );
                }
            }
        }

        // Column headers
        for (index, line) in column_lines.iter().enumerate() {
            let column = label_columns + index as i32;
            let value = line.value.map(|v| values[v].caption());
            if column_depth == 0 {
                if let Some(value) = value {
                    put(
                        top,
                        column,
                        PivotItem::Text(value),
                        PivotCellKind::Header,
                        "",
                    );
                }
                continue;
            }
            let first_of_group = line.value.unwrap_or(0) == 0;
            let value_row = top + column_depth as i32 + 1;
            match line.kind {
                PivotLineKind::Item => {
                    if first_of_group {
                        let levels = column_fields.iter().zip(&line.path).enumerate();
                        for (level, (field, position)) in levels.skip(line.shared) {
                            let item = &data.fields[*field].items[*position as usize];
                            put(
                                top + 1 + level as i32,
                                column,
                                item.value.clone(),
                                PivotCellKind::Header,
                                &item.number_format,
                            );
                        }
                    }
                    if values_row {
                        put(
                            value_row,
                            column,
                            PivotItem::Text(value.unwrap_or_default()),
                            PivotCellKind::Header,
                            "",
                        );
                    }
                }
                PivotLineKind::Total => {
                    let level = line.path.len() - 1;
                    let item = &data.fields[column_fields[level]].items[line.path[level] as usize];
                    if values_row {
                        let caption = format!("{} {}", item.label, value.unwrap_or_default());
                        put(
                            value_row,
                            column,
                            PivotItem::Text(caption),
                            PivotCellKind::Header,
                            "",
                        );
                    } else {
                        let caption = format!("{} Total", item.label);
                        put(
                            top + 1 + level as i32,
                            column,
                            PivotItem::Text(caption),
                            PivotCellKind::Header,
                            "",
                        );
                    }
                }
                PivotLineKind::Blank => {}
                PivotLineKind::Grand => {
                    if values_row {
                        let caption = format!("Total {}", value.unwrap_or_default());
                        put(
                            value_row,
                            column,
                            PivotItem::Text(caption),
                            PivotCellKind::Header,
                            "",
                        );
                    } else {
                        put(
                            top + 1,
                            column,
                            text("Grand Total"),
                            PivotCellKind::Header,
                            "",
                        );
                    }
                }
            }
        }

        // The body
        let first_body_row = top + header_rows;
        for (index, line) in row_lines.iter().enumerate() {
            let row = first_body_row + index as i32;
            if line.kind == PivotLineKind::Blank {
                continue;
            }
            let is_parent = line.kind == PivotLineKind::Item && line.path.len() < row_depth;
            let (label_kind, value_kind) = match line.kind {
                PivotLineKind::Grand => (PivotCellKind::GrandTotal, PivotCellKind::GrandTotal),
                PivotLineKind::Total => (PivotCellKind::Total, PivotCellKind::Total),
                PivotLineKind::Item if is_parent => (PivotCellKind::Parent, PivotCellKind::Parent),
                PivotLineKind::Item | PivotLineKind::Blank => {
                    (PivotCellKind::Item, PivotCellKind::Value)
                }
            };
            if matches!(line.kind, PivotLineKind::Grand | PivotLineKind::Total) {
                for column in 0..body_width {
                    put(row, column, PivotItem::Blank, value_kind, "");
                }
            }
            match line.kind {
                PivotLineKind::Blank => {}
                PivotLineKind::Grand => {
                    if label_columns > 0 {
                        let caption = if row_depth > 0 {
                            "Grand Total"
                        } else {
                            "Total"
                        };
                        put(row, 0, text(caption), label_kind, "");
                    }
                }
                PivotLineKind::Total => {
                    let level = line.path.len() - 1;
                    let item = &data.fields[row_fields[level]].items[line.path[level] as usize];
                    let indent = if compact {
                        INDENT.repeat(level)
                    } else {
                        String::new()
                    };
                    let column = if compact { 0 } else { level as i32 };
                    let caption = format!("{indent}{} Total", item.label);
                    put(row, column, PivotItem::Text(caption), label_kind, "");
                }
                PivotLineKind::Item if compact => {
                    let level = line.path.len() - 1;
                    let item = &data.fields[row_fields[level]].items[line.path[level] as usize];
                    let value = if level > 0 {
                        PivotItem::Text(format!("{}{}", INDENT.repeat(level), item.label))
                    } else {
                        item.value.clone()
                    };
                    put(
                        row,
                        0,
                        display_blank(value),
                        label_kind,
                        &item.number_format,
                    );
                }
                PivotLineKind::Item => {
                    let last = line.path.len() - 1;
                    let first_new = line.shared.min(last);
                    let levels = row_fields.iter().zip(&line.path).enumerate();
                    for (level, (field, position)) in levels {
                        let repeat = rows[level].repeat_labels.unwrap_or(options.repeat_labels);
                        if level < first_new && !repeat {
                            continue;
                        }
                        let item = &data.fields[*field].items[*position as usize];
                        let kind = if level < last || is_parent {
                            PivotCellKind::Parent
                        } else {
                            label_kind
                        };
                        put(
                            row,
                            level as i32,
                            display_blank(item.value.clone()),
                            kind,
                            &item.number_format,
                        );
                    }
                }
            }
            if !line.show_values {
                continue;
            }
            for (column_index, column_line) in column_lines.iter().enumerate() {
                let Some(value_index) = column_line.value else {
                    continue;
                };
                let key = (line.path.clone(), column_line.path.clone());
                if let Some(entry) = totals.get(&key) {
                    let value = values[value_index];
                    let result = entry[value_index].result(value.aggregation);
                    let number_format = if value.show_as == PivotShowAs::Normal {
                        ""
                    } else {
                        PERCENT_FORMAT
                    };
                    let base = key_for_show_as(value, &key, &rows, &columns);
                    let shown = match (value.show_as, &result) {
                        (PivotShowAs::Normal, _) | (_, PivotItem::Error(_)) => result,
                        (_, PivotItem::Number(number)) => {
                            match number_at(&totals, base, value_index) {
                                Some(total) if total != 0.0 => PivotItem::Number(number / total),
                                _ => PivotItem::Error("#DIV/0!".to_string()),
                            }
                        }
                        _ => result,
                    };
                    put(
                        row,
                        label_columns + column_index as i32,
                        shown,
                        value_kind,
                        number_format,
                    );
                }
            }
        }
    }

    let mut cells: Vec<PivotGridCell> = grid.into_values().collect();
    cells.sort_by_key(|c| (c.row, c.column));
    let map = PivotOutputMap {
        filter_rows,
        header_rows,
        label_columns,
        rows: output_lines(&row_lines, &row_fields, &data),
        columns: output_lines(&column_lines, &column_fields, &data),
    };
    PivotGrid {
        cells,
        height,
        width,
        filter_rows,
        label_columns,
        header_rows,
        row_lines,
        column_lines,
        map,
        data,
    }
}

/// The cell a value is measured against when it is shown as a percentage.
fn key_for_show_as(
    value: &PivotValue,
    key: &(Vec<u32>, Vec<u32>),
    rows: &[&PivotField],
    columns: &[&PivotField],
) -> (Vec<u32>, Vec<u32>) {
    let (row, column) = key;
    let parent = |path: &[u32]| path[..path.len().saturating_sub(1)].to_vec();
    match value.show_as {
        PivotShowAs::Normal => key.clone(),
        PivotShowAs::PercentOfTotal => (vec![], vec![]),
        PivotShowAs::PercentOfColumn => (vec![], column.clone()),
        PivotShowAs::PercentOfRow => (row.clone(), vec![]),
        PivotShowAs::PercentOfParentRow => (parent(row), column.clone()),
        PivotShowAs::PercentOfParentColumn => (row.clone(), parent(column)),
        PivotShowAs::PercentOfParent => {
            if let Some(level) = rows.iter().position(|f| f.field == value.base_field) {
                (row[..level.min(row.len())].to_vec(), column.clone())
            } else if let Some(level) = columns.iter().position(|f| f.field == value.base_field) {
                (row.clone(), column[..level.min(column.len())].to_vec())
            } else {
                key.clone()
            }
        }
    }
}

fn display_blank(value: PivotItem) -> PivotItem {
    match value {
        PivotItem::Blank => PivotItem::Text(BLANK_ITEM_LABEL.to_string()),
        other => other,
    }
}

impl Model<'_> {
    /// Reads the source of a pivot. The first row must name every column.
    pub fn read_pivot_data(&self, sheet: u32, area: &PivotArea) -> Result<PivotData, String> {
        if area.last_row <= area.first_row {
            return Err("The source needs a header row and at least one row of data".to_string());
        }
        let mut names: Vec<String> = Vec::new();
        for column in area.first_column..=area.last_column {
            let name = self.get_formatted_cell_value(sheet, area.first_row, column)?;
            let name = name.trim().to_string();
            if name.is_empty() {
                return Err("Every column of the source needs a name in its first row".to_string());
            }
            let mut unique = name.clone();
            let mut suffix = 2;
            while names.iter().any(|n| n.eq_ignore_ascii_case(&unique)) {
                unique = format!("{name}{suffix}");
                suffix += 1;
            }
            names.push(unique);
        }

        let width = names.len();
        let mut seen: Vec<BTreeMap<ItemKey, FieldItem>> = vec![BTreeMap::new(); width];
        let mut rows: Vec<Vec<ItemKey>> = Vec::new();
        for row in (area.first_row + 1)..=area.last_row {
            let mut record = Vec::with_capacity(width);
            for (offset, column) in (area.first_column..=area.last_column).enumerate() {
                let is_error = matches!(
                    self.get_cell_type(sheet, row, column)?,
                    CellType::ErrorValue
                );
                let value = match self.get_cell_value_by_index(sheet, row, column)? {
                    CellValue::None => PivotItem::Blank,
                    CellValue::Number(number) => PivotItem::Number(number),
                    CellValue::Boolean(boolean) => PivotItem::Boolean(boolean),
                    CellValue::String(text) if is_error => PivotItem::Error(text),
                    CellValue::String(text) if text.is_empty() => PivotItem::Blank,
                    CellValue::String(text) => PivotItem::Text(text),
                };
                let key = ItemKey::of(&value);
                if !seen[offset].contains_key(&key) {
                    let number_format = match value {
                        PivotItem::Number(_) => {
                            let format = self.get_style_for_cell(sheet, row, column)?.num_fmt;
                            if format.eq_ignore_ascii_case("general") {
                                String::new()
                            } else {
                                format
                            }
                        }
                        _ => String::new(),
                    };
                    let label = match value {
                        PivotItem::Blank => BLANK_ITEM_LABEL.to_string(),
                        _ => self.get_formatted_cell_value(sheet, row, column)?,
                    };
                    seen[offset].insert(
                        key.clone(),
                        FieldItem {
                            value,
                            label,
                            number_format,
                        },
                    );
                }
                record.push(key);
            }
            rows.push(record);
        }

        let mut fields = Vec::with_capacity(width);
        let mut positions: Vec<BTreeMap<&ItemKey, u32>> = Vec::with_capacity(width);
        for (name, items) in names.into_iter().zip(&seen) {
            fields.push(FieldData {
                name,
                items: items.values().cloned().collect(),
            });
            positions.push(items.keys().zip(0u32..).collect());
        }
        let records = rows
            .iter()
            .map(|record| {
                record
                    .iter()
                    .zip(&positions)
                    .map(|(key, position)| position.get(key).copied().unwrap_or(0))
                    .collect()
            })
            .collect();
        Ok(PivotData { fields, records })
    }

    /// Where a cell sits in a pivot, read from what was last written.
    pub fn pivot_cell_info(&self, sheet: u32, row: i32, column: i32) -> Option<PivotCellInfo> {
        let sheet_id = self.workbook.worksheets.get(sheet as usize)?.sheet_id;
        let (index, pivot) = self
            .workbook
            .pivot_tables
            .iter()
            .enumerate()
            .find(|(_, p)| {
                p.sheet_id == sheet_id && p.output.is_some_and(|o| o.contains(row, column))
            })?;
        let output = pivot.output?;
        let map = pivot.output_map.as_ref()?;
        let r = row - output.first_row;
        let c = column - output.first_column;
        let label_columns = map.label_columns;
        let compact = pivot.options.layout == PivotLayout::Compact;
        let mut info = PivotCellInfo {
            index: index as u32,
            zone: PivotCellZone::Total,
            field: None,
            level: 0,
            item: None,
            has_children: false,
            collapsed: false,
            value_index: None,
            row_path: vec![],
            column_path: vec![],
            read_only: pivot.read_only,
        };
        if r < map.filter_rows {
            info.zone = PivotCellZone::Filter;
            info.field = pivot.filters.get(r as usize).map(|f| f.field.clone());
            info.field.as_ref()?;
            return Some(info);
        }
        let body = r - map.filter_rows;
        if body < map.header_rows {
            if c < label_columns {
                if body == map.header_rows - 1 && !pivot.rows.is_empty() {
                    let level = if compact { 0 } else { c as usize };
                    info.zone = PivotCellZone::RowHeader;
                    info.level = level as u32;
                    info.field = pivot.rows.get(level).map(|f| f.field.clone());
                }
                return Some(info);
            }
            let line = map.columns.get((c - label_columns) as usize);
            info.value_index = line.and_then(|l| l.value);
            info.column_path = line.map(|l| l.path.clone()).unwrap_or_default();
            if pivot.columns.is_empty() {
                info.zone = PivotCellZone::Value;
                return Some(info);
            }
            if body == 0 {
                let level = if compact {
                    0
                } else {
                    (c - label_columns) as usize
                };
                info.zone = PivotCellZone::ColumnHeader;
                info.level = level as u32;
                info.field = pivot.columns.get(level).map(|f| f.field.clone());
                return Some(info);
            }
            let level = (body - 1) as usize;
            if level >= pivot.columns.len() {
                info.zone = PivotCellZone::Value;
                return Some(info);
            }
            let Some(line) = line else {
                return Some(info);
            };
            if line.kind != PivotOutputLineKind::Item || level >= line.path.len() {
                return Some(info);
            }
            info.zone = PivotCellZone::ColumnLabel;
            info.level = level as u32;
            info.field = Some(pivot.columns[level].field.clone());
            info.item = line.path.get(level).cloned();
            info.has_children = level + 1 < pivot.columns.len();
            info.collapsed = info
                .item
                .as_ref()
                .is_some_and(|item| pivot.columns[level].collapsed_items.contains(item));
            return Some(info);
        }
        let line = map.rows.get((body - map.header_rows) as usize)?;
        info.row_path = line.path.clone();
        if c >= label_columns {
            let column_line = map.columns.get((c - label_columns) as usize)?;
            if line.kind == PivotOutputLineKind::Blank {
                return Some(info);
            }
            info.zone = PivotCellZone::Value;
            info.value_index = column_line.value;
            info.column_path = column_line.path.clone();
            return Some(info);
        }
        if line.kind != PivotOutputLineKind::Item {
            if line.kind == PivotOutputLineKind::Total {
                info.level = line.path.len().saturating_sub(1) as u32;
                info.field = pivot.rows.get(info.level as usize).map(|f| f.field.clone());
            }
            return Some(info);
        }
        let level = if compact {
            line.path.len().saturating_sub(1)
        } else {
            (c as usize).min(line.path.len().saturating_sub(1))
        };
        let field = pivot.rows.get(level)?;
        info.zone = PivotCellZone::RowLabel;
        info.level = level as u32;
        info.field = Some(field.field.clone());
        info.item = line.path.get(level).cloned();
        info.row_path = line.path[..=level].to_vec();
        info.has_children = level + 1 < pivot.rows.len();
        info.collapsed = info
            .item
            .as_ref()
            .is_some_and(|item| field.collapsed_items.contains(item));
        Some(info)
    }

    /// The buttons to draw on a sheet.
    pub fn pivot_markers(&self, sheet: u32) -> Vec<PivotMarker> {
        let Some(sheet_id) = self
            .workbook
            .worksheets
            .get(sheet as usize)
            .map(|w| w.sheet_id)
        else {
            return vec![];
        };
        let mut markers = Vec::new();
        for (index, pivot) in self.workbook.pivot_tables.iter().enumerate() {
            let (Some(output), Some(map), false, true) = (
                pivot.output,
                pivot.output_map.as_ref(),
                pivot.read_only,
                pivot.sheet_id == sheet_id,
            ) else {
                continue;
            };
            let filtered = |f: &PivotField| {
                !f.hidden_items.is_empty() || f.label_filter.is_some() || f.value_filter.is_some()
            };
            let compact = pivot.options.layout == PivotLayout::Compact;
            let index = index as u32;
            let mut filter_marker = |row: i32, column: i32, field: &PivotField| {
                markers.push(PivotMarker {
                    index,
                    row,
                    column,
                    kind: PivotMarkerKind::Filter,
                    field: field.field.clone(),
                    item: None,
                    filtered: filtered(field),
                });
            };
            for (position, field) in pivot.filters.iter().enumerate() {
                filter_marker(
                    output.first_row + position as i32,
                    output.first_column + 1,
                    field,
                );
            }
            let top = output.first_row + map.filter_rows;
            let header_row = top + map.header_rows - 1;
            if compact {
                if let Some(field) = pivot.rows.first() {
                    filter_marker(header_row, output.first_column, field);
                }
                if let Some(field) = pivot.columns.first() {
                    filter_marker(top, output.first_column + map.label_columns, field);
                }
            } else {
                for (position, field) in pivot.rows.iter().enumerate() {
                    filter_marker(header_row, output.first_column + position as i32, field);
                }
                for (position, field) in pivot.columns.iter().enumerate() {
                    filter_marker(
                        top,
                        output.first_column + map.label_columns + position as i32,
                        field,
                    );
                }
            }
            let first_line = top + map.header_rows;
            for (offset, line) in map.rows.iter().enumerate() {
                if line.kind != PivotOutputLineKind::Item {
                    continue;
                }
                let row = first_line + offset as i32;
                let last = line.path.len().saturating_sub(1);
                let start = (line.shared as usize).min(last);
                for level in start..line.path.len() {
                    let Some(field) = pivot.rows.get(level) else {
                        continue;
                    };
                    if level + 1 >= pivot.rows.len() || (compact && level != last) {
                        continue;
                    }
                    let item = line.path[level].clone();
                    let kind = if field.collapsed_items.contains(&item) {
                        PivotMarkerKind::Expand
                    } else {
                        PivotMarkerKind::Collapse
                    };
                    let column = output.first_column + if compact { 0 } else { level as i32 };
                    markers.push(PivotMarker {
                        index,
                        row,
                        column,
                        kind,
                        field: field.field.clone(),
                        item: Some(item),
                        filtered: false,
                    });
                }
            }
        }
        markers
    }

    /// Computes the grid of a pivot from the current source values.
    pub fn compute_pivot(&self, pivot: &PivotTable) -> Result<PivotGrid, String> {
        let sheet = self
            .pivot_sheet_index(pivot.source_sheet_id)
            .ok_or_else(|| "The source sheet of this PivotTable no longer exists".to_string())?;
        let data = self.read_pivot_data(sheet, &pivot.source)?;
        Ok(layout_pivot(pivot, data))
    }

    /// The current index of a sheet given its stable id.
    pub fn pivot_sheet_index(&self, sheet_id: u32) -> Option<u32> {
        self.workbook
            .worksheets
            .iter()
            .position(|w| w.sheet_id == sheet_id)
            .map(|index| index as u32)
    }

    /// Moves pivots along with inserted (`delta > 0`) or deleted (`delta < 0`)
    /// rows or columns of a sheet.
    pub(crate) fn displace_pivots(&mut self, sheet: u32, is_row: bool, at: i32, delta: i32) {
        let Some(sheet_id) = self
            .workbook
            .worksheets
            .get(sheet as usize)
            .map(|w| w.sheet_id)
        else {
            return;
        };
        let first_of = |x: i32| {
            if delta > 0 {
                if x >= at {
                    x + delta
                } else {
                    x
                }
            } else if x < at {
                x
            } else {
                (x + delta).max(at)
            }
        };
        let last_of = |x: i32| {
            if delta > 0 {
                if x >= at {
                    x + delta
                } else {
                    x
                }
            } else if x < at {
                x
            } else if x >= at - delta {
                x + delta
            } else {
                at - 1
            }
        };
        let shift = |area: &PivotArea| {
            let mut moved = *area;
            if is_row {
                moved.first_row = first_of(area.first_row);
                moved.last_row = last_of(area.last_row);
            } else {
                moved.first_column = first_of(area.first_column);
                moved.last_column = last_of(area.last_column);
            }
            moved
        };
        let is_empty = |a: &PivotArea| a.first_row > a.last_row || a.first_column > a.last_column;
        for pivot in &mut self.workbook.pivot_tables {
            if pivot.sheet_id == sheet_id {
                if is_row {
                    pivot.row = first_of(pivot.row);
                } else {
                    pivot.column = first_of(pivot.column);
                }
                pivot.output = pivot.output.map(|o| shift(&o)).filter(|o| !is_empty(o));
            }
            if pivot.source_sheet_id == sheet_id {
                pivot.source = shift(&pivot.source);
            }
        }
    }

    /// True if `area` of the sheet collides with a pivot's source or another output.
    pub(crate) fn pivot_area_conflict(
        &self,
        pivot: &PivotTable,
        skip: Option<usize>,
        area: &PivotArea,
    ) -> Option<String> {
        if pivot.source_sheet_id == pivot.sheet_id && area.intersects(&pivot.source) {
            return Some("A PivotTable cannot overlap its source data".to_string());
        }
        for (index, other) in self.workbook.pivot_tables.iter().enumerate() {
            if Some(index) == skip || other.sheet_id != pivot.sheet_id {
                continue;
            }
            if other.output.as_ref().is_some_and(|o| o.intersects(area)) {
                return Some("A PivotTable cannot overlap another PivotTable".to_string());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]

    use super::*;

    fn data() -> PivotData {
        let item = |value: PivotItem, label: &str| FieldItem {
            value,
            label: label.to_string(),
            number_format: String::new(),
        };
        let text = |s: &str| item(PivotItem::Text(s.to_string()), s);
        let number = |n: f64| item(PivotItem::Number(n), &n.to_string());
        PivotData {
            fields: vec![
                FieldData {
                    name: "Region".to_string(),
                    items: vec![text("East"), text("West")],
                },
                FieldData {
                    name: "Product".to_string(),
                    items: vec![text("Chair"), text("Desk")],
                },
                FieldData {
                    name: "Units".to_string(),
                    items: vec![number(1.0), number(2.0), number(3.0)],
                },
            ],
            records: vec![vec![0, 0, 0], vec![0, 1, 1], vec![1, 0, 2], vec![1, 0, 0]],
        }
    }

    fn pivot() -> PivotTable {
        PivotTable {
            name: "PivotTable1".to_string(),
            sheet_id: 2,
            row: 3,
            column: 1,
            source_sheet_id: 1,
            source: PivotArea {
                first_row: 1,
                first_column: 1,
                last_row: 5,
                last_column: 3,
            },
            rows: vec![field("Region")],
            columns: vec![],
            values: vec![sum("Units")],
            filters: vec![],
            options: PivotOptions::default(),
            output: None,
            output_map: None,
            style_name: String::new(),
            read_only: false,
            slicer_filters: vec![],
            raw: None,
        }
    }

    fn field(name: &str) -> PivotField {
        PivotField {
            field: name.to_string(),
            hidden_items: vec![],
            ..Default::default()
        }
    }

    fn sum(name: &str) -> PivotValue {
        PivotValue {
            field: name.to_string(),
            aggregation: PivotAggregation::Sum,
            name: String::new(),
            show_as: Default::default(),
            base_field: String::new(),
        }
    }

    fn at(grid: &PivotGrid, row: i32, column: i32) -> PivotItem {
        grid.cells
            .iter()
            .find(|c| c.row == row && c.column == column)
            .map(|c| c.value.clone())
            .unwrap_or(PivotItem::Blank)
    }

    fn t(s: &str) -> PivotItem {
        PivotItem::Text(s.to_string())
    }

    #[test]
    fn single_row_field_with_grand_total() {
        let grid = layout_pivot(&pivot(), data());
        assert_eq!((grid.height, grid.width), (4, 2));
        assert_eq!(at(&grid, 0, 0), t("Row Labels"));
        assert_eq!(at(&grid, 0, 1), t("Sum of Units"));
        assert_eq!(at(&grid, 1, 0), t("East"));
        assert_eq!(at(&grid, 1, 1), PivotItem::Number(3.0));
        assert_eq!(at(&grid, 2, 1), PivotItem::Number(4.0));
        assert_eq!(at(&grid, 3, 0), t("Grand Total"));
        assert_eq!(at(&grid, 3, 1), PivotItem::Number(7.0));
    }

    #[test]
    fn nested_rows_compact_with_top_subtotals() {
        let mut p = pivot();
        p.rows.push(field("Product"));
        let grid = layout_pivot(&p, data());
        assert_eq!(at(&grid, 1, 0), t("East"));
        assert_eq!(at(&grid, 1, 1), PivotItem::Number(3.0));
        assert_eq!(at(&grid, 2, 0), t("  Chair"));
        assert_eq!(at(&grid, 2, 1), PivotItem::Number(1.0));
        assert_eq!(at(&grid, 3, 0), t("  Desk"));
        assert_eq!(at(&grid, 4, 0), t("West"));
        assert_eq!(at(&grid, 5, 1), PivotItem::Number(4.0));
        assert_eq!(at(&grid, 6, 1), PivotItem::Number(7.0));
    }

    #[test]
    fn nested_rows_tabular_with_bottom_subtotals() {
        let mut p = pivot();
        p.rows.push(field("Product"));
        p.options.layout = PivotLayout::Tabular;
        p.options.subtotals = PivotSubtotals::Bottom;
        let grid = layout_pivot(&p, data());
        assert_eq!(at(&grid, 0, 0), t("Region"));
        assert_eq!(at(&grid, 0, 1), t("Product"));
        assert_eq!(at(&grid, 1, 0), t("East"));
        assert_eq!(at(&grid, 1, 1), t("Chair"));
        assert_eq!(at(&grid, 2, 0), PivotItem::Blank);
        assert_eq!(at(&grid, 2, 1), t("Desk"));
        assert_eq!(at(&grid, 3, 0), t("East Total"));
        assert_eq!(at(&grid, 3, 2), PivotItem::Number(3.0));
    }

    #[test]
    fn columns_and_two_values() {
        let mut p = pivot();
        p.columns.push(field("Product"));
        p.values.push(PivotValue {
            field: "Units".to_string(),
            aggregation: PivotAggregation::Count,
            name: "Lines".to_string(),
            show_as: Default::default(),
            base_field: String::new(),
        });
        let grid = layout_pivot(&p, data());
        assert_eq!(grid.header_rows, 3);
        assert_eq!(at(&grid, 0, 1), t("Column Labels"));
        assert_eq!(at(&grid, 1, 1), t("Chair"));
        assert_eq!(at(&grid, 2, 1), t("Sum of Units"));
        assert_eq!(at(&grid, 2, 2), t("Lines"));
        assert_eq!(at(&grid, 2, 5), t("Total Sum of Units"));
        // East / Chair
        assert_eq!(at(&grid, 3, 1), PivotItem::Number(1.0));
        assert_eq!(at(&grid, 3, 2), PivotItem::Number(1.0));
        // Grand total of the two value columns
        assert_eq!(at(&grid, 5, 5), PivotItem::Number(7.0));
        assert_eq!(at(&grid, 5, 6), PivotItem::Number(4.0));
    }

    #[test]
    fn filters_sit_above_the_body_and_hide_records() {
        let mut p = pivot();
        p.filters.push(PivotField {
            field: "Product".to_string(),
            hidden_items: vec!["Desk".to_string()],
            ..Default::default()
        });
        let grid = layout_pivot(&p, data());
        assert_eq!(grid.filter_rows, 2);
        assert_eq!(at(&grid, 0, 0), t("Product"));
        assert_eq!(at(&grid, 0, 1), t("Chair"));
        assert_eq!(at(&grid, 3, 1), PivotItem::Number(1.0));
        assert_eq!(at(&grid, 5, 1), PivotItem::Number(5.0));
    }

    #[test]
    fn hidden_rows_leave_the_grand_total() {
        let mut p = pivot();
        p.rows[0].hidden_items.push("West".to_string());
        let grid = layout_pivot(&p, data());
        assert_eq!(grid.height, 3);
        assert_eq!(at(&grid, 2, 1), PivotItem::Number(3.0));
    }

    #[test]
    fn aggregations() {
        let mut accumulator = Accumulator::default();
        for n in [2.0, 4.0, 9.0] {
            accumulator.add(&PivotItem::Number(n));
        }
        accumulator.add(&t("text"));
        accumulator.add(&PivotItem::Blank);
        let n = |a| accumulator.result(a);
        assert_eq!(n(PivotAggregation::Sum), PivotItem::Number(15.0));
        assert_eq!(n(PivotAggregation::Count), PivotItem::Number(4.0));
        assert_eq!(n(PivotAggregation::CountNumbers), PivotItem::Number(3.0));
        assert_eq!(n(PivotAggregation::Average), PivotItem::Number(5.0));
        assert_eq!(n(PivotAggregation::Max), PivotItem::Number(9.0));
        assert_eq!(n(PivotAggregation::Min), PivotItem::Number(2.0));
        assert_eq!(n(PivotAggregation::Product), PivotItem::Number(72.0));
        assert_eq!(n(PivotAggregation::Var), PivotItem::Number(13.0));
    }

    #[test]
    fn average_of_nothing_is_an_error() {
        let accumulator = Accumulator::default();
        assert_eq!(
            accumulator.result(PivotAggregation::Average),
            PivotItem::Error("#DIV/0!".to_string())
        );
    }

    #[test]
    fn sorts_descending_and_by_value() {
        let mut p = pivot();
        p.rows[0].sort.order = PivotSortOrder::Descending;
        let grid = layout_pivot(&p, data());
        assert_eq!(at(&grid, 1, 0), t("West"));

        let mut p = pivot();
        p.rows[0].sort.by_value = Some("Sum of Units".to_string());
        p.rows[0].sort.order = PivotSortOrder::Ascending;
        let grid = layout_pivot(&p, data());
        // East totals 3 and West 4
        assert_eq!(at(&grid, 1, 0), t("East"));
        p.rows[0].sort.order = PivotSortOrder::Descending;
        let grid = layout_pivot(&p, data());
        assert_eq!(at(&grid, 1, 0), t("West"));
    }

    #[test]
    fn manual_order_moves_an_item() {
        let mut p = pivot();
        p.rows[0].sort.order = PivotSortOrder::Manual;
        p.rows[0].item_order = vec!["West".to_string(), "East".to_string()];
        let grid = layout_pivot(&p, data());
        assert_eq!(at(&grid, 1, 0), t("West"));
        assert_eq!(at(&grid, 2, 0), t("East"));
    }

    #[test]
    fn label_filters_cover_every_condition_family() {
        let check = |condition, value: &str, value2: &str, expected: &[&str]| {
            let mut p = pivot();
            p.rows[0].label_filter = Some(PivotLabelFilter {
                condition,
                value: value.to_string(),
                value2: value2.to_string(),
            });
            let grid = layout_pivot(&p, data());
            let shown: Vec<PivotItem> = (1..=expected.len() as i32)
                .map(|row| at(&grid, row, 0))
                .collect();
            let wanted: Vec<PivotItem> = expected.iter().map(|e| t(e)).collect();
            assert_eq!(shown, wanted, "{condition:?} {value}");
            assert_eq!(grid.height as usize, expected.len() + 2, "{condition:?}");
        };
        use PivotLabelCondition as C;
        check(C::Equals, "east", "", &["East"]);
        check(C::NotEquals, "East", "", &["West"]);
        check(C::BeginsWith, "W", "", &["West"]);
        check(C::NotBeginsWith, "W", "", &["East"]);
        check(C::EndsWith, "t", "", &["East", "West"]);
        check(C::NotEndsWith, "st", "", &[]);
        check(C::Contains, "as", "", &["East"]);
        check(C::NotContains, "as", "", &["West"]);
        check(C::GreaterThan, "East", "", &["West"]);
        check(C::GreaterThanOrEqual, "East", "", &["East", "West"]);
        check(C::LessThan, "West", "", &["East"]);
        check(C::LessThanOrEqual, "East", "", &["East"]);
        check(C::Between, "A", "F", &["East"]);
        check(C::NotBetween, "A", "F", &["West"]);
        check(C::Equals, "?ast", "", &["East"]);
    }

    fn value_filter(rule: PivotValueRule) -> PivotValueFilter {
        PivotValueFilter {
            value_name: "Sum of Units".to_string(),
            rule,
        }
    }

    #[test]
    fn value_filter_recomputes_the_grand_total() {
        let mut p = pivot();
        p.rows[0].value_filter = Some(value_filter(PivotValueRule::Compare {
            condition: PivotValueCondition::GreaterThan,
            value: 3.0,
            value2: 0.0,
        }));
        let grid = layout_pivot(&p, data());
        assert_eq!(grid.height, 3);
        assert_eq!(at(&grid, 1, 0), t("West"));
        assert_eq!(at(&grid, 2, 1), PivotItem::Number(4.0));
    }

    #[test]
    fn top_and_bottom_filters() {
        let mut p = pivot();
        let top = |bottom, count, unit| {
            value_filter(PivotValueRule::Top {
                count,
                bottom,
                unit,
            })
        };
        p.rows[0].value_filter = Some(top(false, 1.0, PivotTopUnit::Items));
        assert_eq!(at(&layout_pivot(&p, data()), 1, 0), t("West"));
        p.rows[0].value_filter = Some(top(true, 1.0, PivotTopUnit::Items));
        assert_eq!(at(&layout_pivot(&p, data()), 1, 0), t("East"));
        p.rows[0].value_filter = Some(top(false, 50.0, PivotTopUnit::Percent));
        assert_eq!(layout_pivot(&p, data()).height, 3);
        p.rows[0].value_filter = Some(top(false, 4.0, PivotTopUnit::Sum));
        assert_eq!(layout_pivot(&p, data()).height, 3);
    }

    #[test]
    fn collapsing_shows_only_the_total() {
        let mut p = pivot();
        p.rows.push(field("Product"));
        p.rows[0].collapsed_items.push("East".to_string());
        let grid = layout_pivot(&p, data());
        assert_eq!(at(&grid, 1, 0), t("East"));
        assert_eq!(at(&grid, 1, 1), PivotItem::Number(3.0));
        assert_eq!(at(&grid, 2, 0), t("West"));
        assert_eq!(at(&grid, 3, 0), t("  Chair"));
    }

    #[test]
    fn per_field_subtotal_and_blank_line() {
        let mut p = pivot();
        p.rows.push(field("Product"));
        p.options.layout = PivotLayout::Tabular;
        p.rows[0].subtotal = Some(PivotSubtotals::None);
        p.rows[0].blank_line = true;
        let grid = layout_pivot(&p, data());
        // East, Chair / Desk, a blank line, then West
        assert_eq!(at(&grid, 1, 0), t("East"));
        assert_eq!(at(&grid, 2, 1), t("Desk"));
        assert_eq!(at(&grid, 3, 0), PivotItem::Blank);
        assert_eq!(at(&grid, 3, 1), PivotItem::Blank);
        assert_eq!(at(&grid, 4, 0), t("West"));
    }

    #[test]
    fn repeat_labels_for_one_field() {
        let mut p = pivot();
        p.rows.push(field("Product"));
        p.options.layout = PivotLayout::Tabular;
        p.options.subtotals = PivotSubtotals::None;
        p.rows[0].repeat_labels = Some(true);
        let grid = layout_pivot(&p, data());
        assert_eq!(at(&grid, 2, 0), t("East"));
    }

    #[test]
    fn values_shown_as_percentages() {
        let check = |show_as, row, expected: f64| {
            let mut p = pivot();
            p.rows.push(field("Product"));
            p.values[0].show_as = show_as;
            let grid = layout_pivot(&p, data());
            let PivotItem::Number(number) = at(&grid, row, 1) else {
                panic!("not a number");
            };
            assert!((number - expected).abs() < 1e-9, "{show_as:?} {number}");
        };
        check(PivotShowAs::PercentOfTotal, 1, 3.0 / 7.0);
        check(PivotShowAs::PercentOfColumn, 1, 3.0 / 7.0);
        check(PivotShowAs::PercentOfRow, 1, 1.0);
        // Chair within East
        check(PivotShowAs::PercentOfParentRow, 2, 1.0 / 3.0);
    }

    #[test]
    fn the_output_map_names_every_line() {
        let mut p = pivot();
        p.rows.push(field("Product"));
        let grid = layout_pivot(&p, data());
        assert_eq!(grid.map.rows[0].path, vec!["East".to_string()]);
        assert_eq!(
            grid.map.rows[1].path,
            vec!["East".to_string(), "Chair".to_string()]
        );
        assert_eq!(
            grid.map.rows.last().unwrap().kind,
            PivotOutputLineKind::Grand
        );
    }
}
