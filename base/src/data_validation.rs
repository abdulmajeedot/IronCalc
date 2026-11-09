//! Data validation: which values a cell accepts and the dropdown it offers.
//!
//! The rules are kept in the workbook next to the sheets, like pivots and
//! tables. Formulas are stored as the xlsx format has them: in English with no
//! leading `=`, a literal list as `"a,b,c"`, and relative references anchored
//! at the top left of the rule's cells.

use bitcode::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::actions::{displace_cf_col, displace_cf_formula_str, displace_cf_row};
use crate::calc_result::CalcResult;
use crate::cell::CellValue;
use crate::conditional_formatting::parse_sqref;
use crate::constants::{LAST_COLUMN, LAST_ROW};
use crate::expressions::parser::stringify::{
    rename_sheet_in_node, to_english_string, DisplaceData,
};
use crate::expressions::types::{CellReferenceIndex, CellReferenceRC};
use crate::expressions::utils::number_to_column;
use crate::language::get_default_language;
use crate::locale::get_default_locale;
use crate::model::Model;

/// What kind of value a rule allows.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum ValidationKind {
    /// Anything goes. Such a rule can still show an input message.
    #[default]
    #[serde(rename = "none")]
    Any,
    /// Whole numbers
    Whole,
    /// Any number
    Decimal,
    /// One of a list of values
    List,
    /// A date
    Date,
    /// A time
    Time,
    /// A text of a given length
    TextLength,
    /// Whatever makes a formula true
    Custom,
}

/// How a value is compared with a rule's bounds.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum ValidationOperator {
    /// Between two values, both included
    #[default]
    Between,
    /// Outside two values
    NotBetween,
    /// Equal to a value
    Equal,
    /// Different from a value
    NotEqual,
    /// Greater than a value
    GreaterThan,
    /// Less than a value
    LessThan,
    /// Greater than or equal to a value
    GreaterThanOrEqual,
    /// Less than or equal to a value
    LessThanOrEqual,
}

impl ValidationOperator {
    /// Whether the operator takes two values.
    pub fn takes_two_values(self) -> bool {
        matches!(self, Self::Between | Self::NotBetween)
    }
}

/// What happens when a value is rejected.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum ValidationAlertStyle {
    /// The value can't be kept
    #[default]
    Stop,
    /// The user is asked whether to keep it
    Warning,
    /// The user is told, and can keep it
    Information,
}

/// A rule, without the cells it applies to.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct ValidationSettings {
    /// What kind of value is allowed
    #[serde(default)]
    pub kind: ValidationKind,
    /// How the value is compared with the bounds
    #[serde(default)]
    pub operator: ValidationOperator,
    /// The first bound, the list or the custom formula
    #[serde(default)]
    pub formula1: String,
    /// The second bound
    #[serde(default)]
    pub formula2: String,
    /// Whether a blank is allowed
    #[serde(default)]
    pub ignore_blank: bool,
    /// Whether a list shows a dropdown
    #[serde(default)]
    pub in_cell_dropdown: bool,
    /// Whether the input message is shown
    #[serde(default)]
    pub show_input: bool,
    /// The title of the input message
    #[serde(default)]
    pub input_title: String,
    /// The text of the input message
    #[serde(default)]
    pub input_message: String,
    /// Whether wrong values are rejected at all
    #[serde(default)]
    pub show_error: bool,
    /// What happens to a wrong value
    #[serde(default)]
    pub error_style: ValidationAlertStyle,
    /// The title of the alert
    #[serde(default)]
    pub error_title: String,
    /// The text of the alert
    #[serde(default)]
    pub error_message: String,
}

impl Default for ValidationSettings {
    fn default() -> Self {
        ValidationSettings {
            kind: ValidationKind::Any,
            operator: ValidationOperator::Between,
            formula1: String::new(),
            formula2: String::new(),
            ignore_blank: true,
            in_cell_dropdown: true,
            show_input: true,
            input_title: String::new(),
            input_message: String::new(),
            show_error: true,
            error_style: ValidationAlertStyle::Stop,
            error_title: String::new(),
            error_message: String::new(),
        }
    }
}

impl ValidationSettings {
    /// A rule that neither restricts anything nor says anything is no rule.
    pub fn is_empty(&self) -> bool {
        self.kind == ValidationKind::Any
            && self.input_title.is_empty()
            && self.input_message.is_empty()
    }

    pub(crate) fn formulas_mut(&mut self) -> [&mut String; 2] {
        [&mut self.formula1, &mut self.formula2]
    }
}

/// A rule and the cells it applies to.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone)]
pub struct DataValidation {
    /// The id of the sheet the rule is on
    pub sheet_id: u32,
    /// The cells, as space-separated ranges: `A1:A5 C3`
    pub sqref: String,
    /// What the rule says
    pub settings: ValidationSettings,
}

/// Why the value of a cell was rejected.
#[derive(Serialize, Debug, PartialEq, Eq, Clone)]
pub struct ValidationFailure {
    /// The title the rule gives its alert
    pub title: String,
    /// The text the rule gives its alert
    pub message: String,
    /// What should happen to the value
    pub style: ValidationAlertStyle,
}

/// The top left of the bounding box of a rule's cells: what its relative
/// references are measured from.
pub(crate) fn sqref_anchor(ranges: &[(i32, i32, i32, i32)]) -> Option<(i32, i32)> {
    ranges
        .iter()
        .map(|&(row, column, _, _)| (row, column))
        .reduce(|(r, c), (row, column)| (r.min(row), c.min(column)))
}

pub(crate) fn range_name(range: (i32, i32, i32, i32)) -> Option<String> {
    let (first_row, first_column, last_row, last_column) = range;
    let first = number_to_column(first_column)?;
    if (first_row, first_column) == (last_row, last_column) {
        return Some(format!("{first}{first_row}"));
    }
    let last = number_to_column(last_column)?;
    Some(format!("{first}{first_row}:{last}{last_row}"))
}

pub(crate) fn sqref_of(ranges: &[(i32, i32, i32, i32)]) -> String {
    ranges
        .iter()
        .filter_map(|range| range_name(*range))
        .collect::<Vec<_>>()
        .join(" ")
}

/// What is left of `range` once `hole` is cut out of it.
pub(crate) fn subtract_range(
    range: (i32, i32, i32, i32),
    hole: (i32, i32, i32, i32),
) -> Vec<(i32, i32, i32, i32)> {
    let (r1, c1, r2, c2) = range;
    let (h1, k1, h2, k2) = hole;
    if h1 > r2 || h2 < r1 || k1 > c2 || k2 < c1 {
        return vec![range];
    }
    let mut pieces = Vec::new();
    if r1 < h1 {
        pieces.push((r1, c1, h1 - 1, c2));
    }
    let (top, bottom) = (r1.max(h1), r2.min(h2));
    if c1 < k1 {
        pieces.push((top, c1, bottom, k1 - 1));
    }
    if k2 < c2 {
        pieces.push((top, k2 + 1, bottom, c2));
    }
    if h2 < r2 {
        pieces.push((h2 + 1, c1, r2, c2));
    }
    pieces
}

/// A range split where a block of cells moved away from under it.
pub(crate) fn ranges_after_block(
    range: (i32, i32, i32, i32),
    data: &DisplaceData,
) -> Vec<(i32, i32, i32, i32)> {
    let (zone, down, delta) = match *data {
        DisplaceData::BlockDown {
            row,
            first_column,
            last_column,
            delta,
            ..
        } => ((row, first_column, LAST_ROW, last_column), true, delta),
        DisplaceData::BlockRight {
            column,
            first_row,
            last_row,
            delta,
            ..
        } => ((first_row, column, last_row, LAST_COLUMN), false, delta),
        _ => return vec![range],
    };
    let mut pieces = subtract_range(range, zone);
    let inside = (
        range.0.max(zone.0),
        range.1.max(zone.1),
        range.2.min(zone.2),
        range.3.min(zone.3),
    );
    if inside.0 > inside.2 || inside.1 > inside.3 {
        return pieces;
    }
    // Going back, the cells of the hole are gone
    let survivor = if delta >= 0 {
        Some(inside)
    } else if down {
        let first = inside.0.max(zone.0 - delta);
        (first <= inside.2).then_some((first, inside.1, inside.2, inside.3))
    } else {
        let first = inside.1.max(zone.1 - delta);
        (first <= inside.3).then_some((inside.0, first, inside.2, inside.3))
    };
    if let Some((a, b, c, d)) = survivor {
        pieces.push(if down {
            (a + delta, b, (c + delta).min(LAST_ROW), d)
        } else {
            (a, b + delta, c, (d + delta).min(LAST_COLUMN))
        });
    }
    pieces
}

fn inside(range: &(i32, i32, i32, i32), row: i32, column: i32) -> bool {
    (range.0..=range.2).contains(&row) && (range.1..=range.3).contains(&column)
}

/// A span of rows or columns after a displacement, `None` if it is all gone.
fn shifted_span(first: i32, last: i32, at: i32, delta: i32) -> Option<(i32, i32)> {
    if delta >= 0 {
        let moved = |line: i32| if line >= at { line + delta } else { line };
        return Some((moved(first), moved(last)));
    }
    let end = at - delta - 1;
    let new_first = if first < at {
        first
    } else if first <= end {
        at
    } else {
        first + delta
    };
    let new_last = if last < at {
        last
    } else if last <= end {
        at - 1
    } else {
        last + delta
    };
    (new_first <= new_last).then_some((new_first, new_last))
}

pub(crate) fn displaced_range(
    range: (i32, i32, i32, i32),
    data: &DisplaceData,
    sheet: u32,
) -> Option<(i32, i32, i32, i32)> {
    let (r1, c1, r2, c2) = range;
    match *data {
        DisplaceData::BlockDown { .. } | DisplaceData::BlockRight { .. } => Some(range),
        DisplaceData::Row { row, delta, .. } => {
            let (first, last) = shifted_span(r1, r2, row, delta)?;
            Some((first, c1, last, c2))
        }
        DisplaceData::Column { column, delta, .. } => {
            let (first, last) = shifted_span(c1, c2, column, delta)?;
            Some((r1, first, r2, last))
        }
        _ => {
            let row1 = displace_cf_row(r1, data, sheet).unwrap_or(r1);
            let row2 = displace_cf_row(r2, data, sheet).unwrap_or(r2);
            let column1 = displace_cf_col(c1, data, sheet).unwrap_or(c1);
            let column2 = displace_cf_col(c2, data, sheet).unwrap_or(c2);
            Some((
                row1.min(row2),
                column1.min(column2),
                row1.max(row2),
                column1.max(column2),
            ))
        }
    }
}

impl Model<'_> {
    /// The rule on a cell, if it has one.
    pub(crate) fn data_validation_at(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Option<&DataValidation> {
        let sheet_id = self.workbook.worksheets.get(sheet as usize)?.sheet_id;
        self.workbook.data_validations.iter().find(|rule| {
            rule.sheet_id == sheet_id
                && parse_sqref(&rule.sqref)
                    .iter()
                    .any(|range| inside(range, row, column))
        })
    }

    fn evaluate_validation_formula(
        &mut self,
        sheet: u32,
        rule: &DataValidation,
        formula: &str,
        row: i32,
        column: i32,
    ) -> CalcResult {
        let anchor = sqref_anchor(&parse_sqref(&rule.sqref)).unwrap_or((row, column));
        let sheet_name = self
            .workbook
            .worksheets
            .get(sheet as usize)
            .map(|worksheet| worksheet.get_name())
            .unwrap_or_default();
        let context = CellReferenceRC {
            sheet: sheet_name,
            row: anchor.0,
            column: anchor.1,
        };
        let node = self.parse_internal_formula(formula, &context);
        let cell = CellReferenceIndex { sheet, row, column };
        match self.evaluate_node_in_context(&node, cell) {
            CalcResult::Range { left, right } if left == right => self.evaluate_cell(left),
            result => result,
        }
    }

    /// The values a list rule offers. Empty for any other kind of rule.
    pub(crate) fn data_validation_items(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Vec<String>, String> {
        let Some(rule) = self.data_validation_at(sheet, row, column).cloned() else {
            return Ok(vec![]);
        };
        if rule.settings.kind != ValidationKind::List {
            return Ok(vec![]);
        }
        if let Some(items) = literal_list(&rule.settings.formula1) {
            return Ok(items);
        }
        let formula = rule.settings.formula1.clone();
        Ok(
            match self.evaluate_validation_formula(sheet, &rule, &formula, row, column) {
                CalcResult::Range { left, right } => {
                    let dimension = self.workbook.worksheet(left.sheet)?.dimension();
                    let mut items = Vec::new();
                    for r in left.row..=right.row.min(dimension.max_row) {
                        for c in left.column..=right.column.min(dimension.max_column) {
                            let text = self.get_formatted_cell_value(left.sheet, r, c)?;
                            if !text.is_empty() {
                                items.push(text);
                            }
                        }
                    }
                    items
                }
                CalcResult::String(text) => vec![text],
                CalcResult::Number(number) => vec![number.to_string()],
                _ => vec![],
            },
        )
    }

    /// Checks the value now in a cell against the cell's rule.
    pub(crate) fn data_validation_failure(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<ValidationFailure>, String> {
        let Some(rule) = self.data_validation_at(sheet, row, column).cloned() else {
            return Ok(None);
        };
        let settings = &rule.settings;
        if !settings.show_error || settings.kind == ValidationKind::Any {
            return Ok(None);
        }
        let value = self.get_cell_value_by_index(sheet, row, column)?;
        let is_blank = match &value {
            CellValue::None => true,
            CellValue::String(text) => text.is_empty(),
            _ => false,
        };
        let accepted = if is_blank {
            settings.ignore_blank
        } else {
            self.value_is_accepted(sheet, &rule, &value, row, column)?
        };
        Ok((!accepted).then(|| ValidationFailure {
            title: settings.error_title.clone(),
            message: settings.error_message.clone(),
            style: settings.error_style,
        }))
    }

    fn value_is_accepted(
        &mut self,
        sheet: u32,
        rule: &DataValidation,
        value: &CellValue,
        row: i32,
        column: i32,
    ) -> Result<bool, String> {
        let settings = &rule.settings;
        let number = match (settings.kind, value) {
            (ValidationKind::List, _) => {
                let text = self.get_formatted_cell_value(sheet, row, column)?;
                let items = self.data_validation_items(sheet, row, column)?;
                // A typed list is case sensitive and a list from cells isn't, as in Excel
                return Ok(if settings.formula1.starts_with('"') {
                    items.contains(&text)
                } else {
                    items.iter().any(|item| item.eq_ignore_ascii_case(&text))
                });
            }
            (ValidationKind::Custom, _) => {
                let formula = settings.formula1.clone();
                return Ok(
                    match self.evaluate_validation_formula(sheet, rule, &formula, row, column) {
                        CalcResult::Boolean(result) => result,
                        CalcResult::Number(result) => result != 0.0,
                        _ => false,
                    },
                );
            }
            (ValidationKind::TextLength, _) => self
                .get_formatted_cell_value(sheet, row, column)?
                .chars()
                .count() as f64,
            (ValidationKind::Whole, CellValue::Number(number)) if number.fract() == 0.0 => *number,
            (
                ValidationKind::Decimal | ValidationKind::Date | ValidationKind::Time,
                CellValue::Number(number),
            ) => *number,
            _ => return Ok(false),
        };
        let first = self.validation_bound(sheet, rule, &settings.formula1, row, column);
        let second = if settings.operator.takes_two_values() {
            self.validation_bound(sheet, rule, &settings.formula2, row, column)
        } else {
            Bound::Number(0.0)
        };
        let (first, second) = match (first, second) {
            (Bound::Number(first), Bound::Number(second)) => (first, second),
            // A bound in an empty cell: Ignore blank lets everything through
            (Bound::Blank, _) | (_, Bound::Blank) if settings.ignore_blank => return Ok(true),
            (Bound::Blank, _) | (_, Bound::Blank) => (0.0, 0.0),
            _ => return Ok(true),
        };
        Ok(match settings.operator {
            ValidationOperator::Between => first <= number && number <= second,
            ValidationOperator::NotBetween => number < first || second < number,
            ValidationOperator::Equal => number == first,
            ValidationOperator::NotEqual => number != first,
            ValidationOperator::GreaterThan => number > first,
            ValidationOperator::LessThan => number < first,
            ValidationOperator::GreaterThanOrEqual => number >= first,
            ValidationOperator::LessThanOrEqual => number <= first,
        })
    }

    fn validation_bound(
        &mut self,
        sheet: u32,
        rule: &DataValidation,
        formula: &str,
        row: i32,
        column: i32,
    ) -> Bound {
        match self.evaluate_validation_formula(sheet, rule, formula, row, column) {
            CalcResult::Number(number) => Bound::Number(number),
            CalcResult::EmptyCell | CalcResult::EmptyArg => Bound::Blank,
            _ => Bound::Unusable,
        }
    }

    /// Keeps the rules on their cells, and their formulas on theirs, when rows
    /// or columns move. A rule whose cells are all deleted goes with them.
    pub(crate) fn displace_data_validations(&mut self, sheet: u32, data: &DisplaceData) {
        if self.workbook.data_validations.is_empty() {
            return;
        }
        let locale = self.locale;
        let language = self.language;
        self.parser.set_locale(get_default_locale());
        self.parser.set_language(get_default_language());
        let rules = std::mem::take(&mut self.workbook.data_validations);
        let mut kept = Vec::with_capacity(rules.len());
        for mut rule in rules {
            let index = self
                .workbook
                .worksheets
                .iter()
                .position(|worksheet| worksheet.sheet_id == rule.sheet_id);
            let ranges = parse_sqref(&rule.sqref);
            let (Some(index), Some(anchor)) = (index, sqref_anchor(&ranges)) else {
                kept.push(rule);
                continue;
            };
            let context = CellReferenceRC {
                sheet: self.workbook.worksheets[index].get_name(),
                row: anchor.0,
                column: anchor.1,
            };
            for formula in rule.settings.formulas_mut() {
                if !formula.is_empty() {
                    *formula = displace_cf_formula_str(&mut self.parser, formula, &context, data);
                }
            }
            if index as u32 == sheet {
                let moved: Vec<_> = ranges
                    .into_iter()
                    .flat_map(|range| ranges_after_block(range, data))
                    .filter_map(|range| displaced_range(range, data, sheet))
                    .collect();
                if moved.is_empty() {
                    continue;
                }
                rule.sqref = sqref_of(&moved);
            }
            kept.push(rule);
        }
        self.workbook.data_validations = kept;
        self.parser.set_locale(locale);
        self.parser.set_language(language);
    }

    /// Follows a renamed sheet. Called before the sheet takes its new name.
    pub(crate) fn rename_sheet_in_data_validations(&mut self, sheet: u32, new_name: &str) {
        let mut rules = std::mem::take(&mut self.workbook.data_validations);
        for rule in &mut rules {
            let name = self
                .workbook
                .worksheets
                .iter()
                .find(|worksheet| worksheet.sheet_id == rule.sheet_id)
                .map(|worksheet| worksheet.get_name());
            let anchor = sqref_anchor(&parse_sqref(&rule.sqref));
            let (Some(name), Some(anchor)) = (name, anchor) else {
                continue;
            };
            let context = CellReferenceRC {
                sheet: name,
                row: anchor.0,
                column: anchor.1,
            };
            for formula in rule.settings.formulas_mut() {
                if !formula.is_empty() {
                    let mut node = self.parse_internal_formula(formula, &context);
                    rename_sheet_in_node(&mut node, sheet, new_name);
                    *formula = to_english_string(&node, &context);
                }
            }
        }
        self.workbook.data_validations = rules;
    }
}

enum Bound {
    Number(f64),
    Blank,
    Unusable,
}

/// The items of a list typed into the rule, `None` if the list is a reference.
pub(crate) fn literal_list(formula: &str) -> Option<Vec<String>> {
    let inner = formula.strip_prefix('"')?.strip_suffix('"')?;
    Some(
        inner
            .replace("\"\"", "\"")
            .split(',')
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect(),
    )
}
