//! Editing data validation rules, and finding the data around a cell.

use crate::conditional_formatting::parse_sqref;
use crate::constants::{LAST_COLUMN, LAST_ROW};
use crate::data_validation::{
    literal_list, sqref_anchor, sqref_of, DataValidation, ValidationFailure, ValidationKind,
    ValidationSettings,
};
use crate::expressions::parser::stringify::{to_english_string, to_rc_format};
use crate::expressions::parser::Node;
use crate::expressions::types::CellReferenceRC;
use crate::formatter::format::{format_number, parse_formatted_number};
use crate::pivot::PivotArea;
use crate::types::Cell;

use serde::{Deserialize, Serialize};

use super::{common::UserModel, history::Diff};

const LIST_LIMIT: usize = 255;

type Range = (i32, i32, i32, i32);

/// What is left of `range` once `hole` is cut out of it.
fn subtract(range: Range, hole: Range) -> Vec<Range> {
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

/// Joins ranges that touch, so a rule stays short after many single-cell pastes.
fn merge_ranges(mut ranges: Vec<Range>) -> Vec<Range> {
    fn join(ranges: Vec<Range>, touching: impl Fn(&Range, &Range) -> bool) -> Vec<Range> {
        let mut joined: Vec<Range> = Vec::with_capacity(ranges.len());
        for range in ranges {
            match joined.last_mut() {
                Some(last) if touching(last, &range) => {
                    last.2 = last.2.max(range.2);
                    last.3 = last.3.max(range.3);
                }
                _ => joined.push(range),
            }
        }
        joined
    }
    for _ in 0..2 {
        ranges.sort_by_key(|&(r1, c1, r2, c2)| (c1, c2, r1, r2));
        ranges = join(ranges, |a, b| a.1 == b.1 && a.3 == b.3 && a.2 + 1 == b.0);
        ranges.sort_by_key(|&(r1, c1, r2, c2)| (r1, r2, c1, c2));
        ranges = join(ranges, |a, b| a.0 == b.0 && a.2 == b.2 && a.3 + 1 == b.1);
    }
    ranges
}

/// A rule as it travels with a copied cell.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct CopiedValidation {
    settings: ValidationSettings,
    /// The cell the rule's relative references are measured from
    anchor: (i32, i32),
}

/// Cells getting the same rule, relative references and all.
struct PastedRule {
    relative: ValidationSettings,
    source: CopiedValidation,
    ranges: Vec<Range>,
}

/// How far rules may stretch a copy past the used part of a sheet.
const COPY_REACH_LIMIT: i32 = 20_000;

fn is_number(text: &str) -> bool {
    text.parse::<f64>().is_ok_and(f64::is_finite)
}

impl UserModel<'_> {
    fn validation_context(
        &self,
        sheet: u32,
        anchor: (i32, i32),
    ) -> Result<CellReferenceRC, String> {
        Ok(CellReferenceRC {
            sheet: self.model.workbook.worksheet(sheet)?.get_name(),
            row: anchor.0,
            column: anchor.1,
        })
    }

    fn validation_formula(
        &mut self,
        text: &str,
        context: &CellReferenceRC,
    ) -> Result<String, String> {
        let internal = self.model.user_formula_to_internal(text, context)?;
        Ok(internal.trim_start_matches('=').to_string())
    }

    fn validation_input_to_internal(
        &mut self,
        kind: ValidationKind,
        text: &str,
        context: &CellReferenceRC,
    ) -> Result<String, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(String::new());
        }
        if text.starts_with('=') || kind == ValidationKind::Custom {
            return self.validation_formula(text, context);
        }
        match kind {
            ValidationKind::List => {
                let literal = format!("\"{}\"", text.replace('"', "\"\""));
                if literal.len() > LIST_LIMIT {
                    return Err(format!(
                        "A typed list can't be longer than {LIST_LIMIT} characters. Put the values in cells instead."
                    ));
                }
                Ok(literal)
            }
            _ => {
                let currencies: [&str; 0] = [];
                let (value, _) = parse_formatted_number(text, &currencies, self.model.locale)
                    .map_err(|_| format!("'{text}' is not a valid value"))?;
                Ok(value.to_string())
            }
        }
    }

    fn validation_internal_to_display(
        &self,
        kind: ValidationKind,
        formula: &str,
        context: &CellReferenceRC,
    ) -> String {
        if formula.is_empty() {
            return String::new();
        }
        if kind == ValidationKind::List {
            if let Some(items) = literal_list(formula) {
                return items.join(",");
            }
        }
        if is_number(formula) {
            let pattern = match kind {
                ValidationKind::Date => "yyyy-mm-dd",
                ValidationKind::Time => "hh:mm:ss",
                _ => return formula.to_string(),
            };
            let value = formula.parse::<f64>().unwrap_or_default();
            return format_number(value, pattern, self.model.locale).text;
        }
        format!(
            "={}",
            self.model.internal_formula_to_display(formula, context)
        )
    }

    /// The rule on a cell, with its formulas as a person would type them.
    pub fn get_data_validation(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<ValidationSettings>, String> {
        let Some(rule) = self.model.data_validation_at(sheet, row, column).cloned() else {
            return Ok(None);
        };
        let anchor = sqref_anchor(&parse_sqref(&rule.sqref)).unwrap_or((row, column));
        let anchor_context = self.validation_context(sheet, anchor)?;
        // As in Excel, relative references read as they apply to this cell
        let context = self.validation_context(sheet, (row, column))?;
        let mut settings = rule.settings;
        let kind = settings.kind;
        for formula in settings.formulas_mut() {
            if !formula.is_empty() && anchor != (row, column) {
                let node = self.model.parse_internal_formula(formula, &anchor_context);
                *formula = to_english_string(&node, &context);
            }
            *formula = self.validation_internal_to_display(kind, formula, &context);
        }
        Ok(Some(settings))
    }

    /// Replaces whatever rules `area` has with `settings`. Rules that reach
    /// outside `area` keep their other cells.
    pub fn set_data_validation(
        &mut self,
        sheet: u32,
        area: PivotArea,
        mut settings: ValidationSettings,
    ) -> Result<(), String> {
        let sheet_id = self.model.workbook.worksheet(sheet)?.sheet_id;
        let hole = (
            area.first_row,
            area.first_column,
            area.last_row,
            area.last_column,
        );
        // What is typed reads from the selected cell, wherever it is in the area
        let view = self.get_selected_view();
        let selected_inside = view.sheet == sheet
            && (hole.0..=hole.2).contains(&view.row)
            && (hole.1..=hole.3).contains(&view.column);
        let anchor = (hole.0, hole.1);
        let entry = if selected_inside {
            (view.row, view.column)
        } else {
            anchor
        };
        let context = self.validation_context(sheet, entry)?;
        self.check_validation_settings(&settings)?;
        let kind = settings.kind;
        let first = settings.formula1.clone();
        let second = settings.formula2.clone();
        settings.formula1 = self.validation_input_to_internal(kind, &first, &context)?;
        settings.formula2 = if settings.operator.takes_two_values() {
            self.validation_input_to_internal(kind, &second, &context)?
        } else {
            String::new()
        };
        if entry != anchor {
            let anchor_context = self.validation_context(sheet, anchor)?;
            for formula in settings.formulas_mut() {
                if !formula.is_empty() {
                    let node = self.model.parse_internal_formula(formula, &context);
                    *formula = to_english_string(&node, &anchor_context);
                }
            }
        }
        if is_number(&settings.formula1)
            && is_number(&settings.formula2)
            && settings.formula1.parse::<f64>().ok() > settings.formula2.parse::<f64>().ok()
        {
            return Err("The Maximum must be greater than or equal to the Minimum.".to_string());
        }

        let old_value = self.model.workbook.data_validations.clone();
        let mut new_value = self.rules_without_area(old_value.clone(), sheet, hole)?;
        if !settings.is_empty() {
            new_value.push(DataValidation {
                sheet_id,
                sqref: sqref_of(&[hole]),
                settings,
            });
        }
        if new_value == old_value {
            return Ok(());
        }
        self.model.workbook.data_validations = new_value.clone();
        self.push_diff_list(vec![Diff::SetDataValidations {
            old_value,
            new_value,
        }]);
        Ok(())
    }

    /// `rules` with the cells of `hole` on `sheet` taken out of them.
    fn rules_without_area(
        &mut self,
        rules: Vec<DataValidation>,
        sheet: u32,
        hole: Range,
    ) -> Result<Vec<DataValidation>, String> {
        let sheet_id = self.model.workbook.worksheet(sheet)?.sheet_id;
        let mut kept = Vec::with_capacity(rules.len() + 1);
        for mut rule in rules {
            if rule.sheet_id != sheet_id {
                kept.push(rule);
                continue;
            }
            let ranges = parse_sqref(&rule.sqref);
            let pieces: Vec<Range> = ranges
                .iter()
                .flat_map(|range| subtract(*range, hole))
                .collect();
            if pieces.is_empty() {
                continue;
            }
            // Relative references are measured from the top left of the rule
            if sqref_anchor(&pieces) != sqref_anchor(&ranges) {
                self.move_validation_anchor(&mut rule, sheet, &ranges, &pieces)?;
            }
            rule.sqref = sqref_of(&pieces);
            kept.push(rule);
        }
        Ok(kept)
    }

    /// The rule on a cell, ready to go along with a copy of the cell.
    pub(super) fn copied_validation(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Option<CopiedValidation> {
        let rule = self.model.data_validation_at(sheet, row, column)?;
        Some(CopiedValidation {
            settings: rule.settings.clone(),
            anchor: sqref_anchor(&parse_sqref(&rule.sqref))?,
        })
    }

    /// The last row and column of `range` that have a rule. A copy has to
    /// reach them even where the cells are still empty.
    pub(super) fn validation_reach(&self, sheet: u32, range: Range) -> (i32, i32) {
        let (row_start, column_start, row_end, column_end) = range;
        let Some(sheet_id) = self
            .model
            .workbook
            .worksheets
            .get(sheet as usize)
            .map(|worksheet| worksheet.sheet_id)
        else {
            return (0, 0);
        };
        let mut reach = (0, 0);
        for rule in &self.model.workbook.data_validations {
            if rule.sheet_id != sheet_id {
                continue;
            }
            for (r1, c1, r2, c2) in parse_sqref(&rule.sqref) {
                if r1 <= row_end && r2 >= row_start && c1 <= column_end && c2 >= column_start {
                    reach.0 = i32::max(reach.0, r2.min(row_end).min(COPY_REACH_LIMIT));
                    reach.1 = i32::max(reach.1, c2.min(column_end));
                }
            }
        }
        reach
    }

    /// The rule's formulas in a form that doesn't depend on where it is, so
    /// rules can be compared. `None` if a formula no longer makes sense here,
    /// such as a reference to a sheet that is gone.
    fn relative_validation(
        &mut self,
        copied: &CopiedValidation,
        sheet: u32,
    ) -> Result<Option<ValidationSettings>, String> {
        let context = self.validation_context(sheet, copied.anchor)?;
        let mut relative = copied.settings.clone();
        for formula in relative.formulas_mut() {
            if !formula.is_empty() {
                let node = self.model.parse_internal_formula(formula, &context);
                if matches!(node, Node::ParseErrorKind { .. }) {
                    return Ok(None);
                }
                *formula = to_rc_format(&node);
            }
        }
        Ok(Some(relative))
    }

    /// Replaces the rules of `hole` with those copied onto its cells. A cell
    /// in `hole` that got no rule ends up with none, as in Excel. `cut` is
    /// what a cut leaves behind.
    pub(super) fn place_copied_validations(
        &mut self,
        sheet: u32,
        hole: Range,
        cells: Vec<(i32, i32, CopiedValidation)>,
        cut: Option<(u32, Range)>,
    ) -> Result<Option<Diff>, String> {
        if cells.is_empty() && self.model.workbook.data_validations.is_empty() {
            return Ok(None);
        }
        let sheet_id = self.model.workbook.worksheet(sheet)?.sheet_id;
        let old_value = self.model.workbook.data_validations.clone();
        let mut rules = self.rules_without_area(old_value.clone(), sheet, hole)?;
        if let Some((cut_sheet, cut_range)) = cut {
            rules = self.rules_without_area(rules, cut_sheet, cut_range)?;
        }

        let mut pasted: Vec<PastedRule> = Vec::new();
        let mut known: Vec<(CopiedValidation, Option<usize>)> = Vec::new();
        for (row, column, copied) in cells {
            let group = match known.iter().find(|(seen, _)| *seen == copied) {
                Some((_, group)) => *group,
                None => {
                    let group = self.relative_validation(&copied, sheet)?.map(|relative| {
                        match pasted.iter().position(|p| p.relative == relative) {
                            Some(index) => index,
                            None => {
                                pasted.push(PastedRule {
                                    relative,
                                    source: copied.clone(),
                                    ranges: vec![],
                                });
                                pasted.len() - 1
                            }
                        }
                    });
                    known.push((copied, group));
                    group
                }
            };
            if let Some(group) = group {
                pasted[group].ranges.push((row, column, row, column));
            }
        }

        // A cell pasted next to its own rule joins that rule
        let mut new_value = Vec::with_capacity(rules.len() + pasted.len());
        for rule in rules {
            let ranges = parse_sqref(&rule.sqref);
            let copied = sqref_anchor(&ranges).map(|anchor| CopiedValidation {
                settings: rule.settings.clone(),
                anchor,
            });
            let joins = match copied {
                Some(copied) if rule.sheet_id == sheet_id => self
                    .relative_validation(&copied, sheet)?
                    .and_then(|relative| pasted.iter().position(|p| p.relative == relative)),
                _ => None,
            };
            match joins {
                Some(group) => pasted[group].ranges.extend(ranges),
                None => new_value.push(rule),
            }
        }
        for group in pasted {
            let ranges = merge_ranges(group.ranges);
            let Some(anchor) = sqref_anchor(&ranges) else {
                continue;
            };
            let old_context = self.validation_context(sheet, group.source.anchor)?;
            let new_context = self.validation_context(sheet, anchor)?;
            let mut settings = group.source.settings;
            for formula in settings.formulas_mut() {
                if !formula.is_empty() {
                    let node = self.model.parse_internal_formula(formula, &old_context);
                    *formula = to_english_string(&node, &new_context);
                }
            }
            new_value.push(DataValidation {
                sheet_id,
                sqref: sqref_of(&ranges),
                settings,
            });
        }
        if new_value == old_value {
            return Ok(None);
        }
        self.model.workbook.data_validations = new_value.clone();
        Ok(Some(Diff::SetDataValidations {
            old_value,
            new_value,
        }))
    }

    /// Repeats the rules of `source` over `target`, for the fill handle.
    pub(super) fn fill_data_validations(
        &mut self,
        sheet: u32,
        source: Range,
        target: Range,
        diff_list: &mut Vec<Diff>,
    ) -> Result<(), String> {
        if self.model.workbook.data_validations.is_empty() {
            return Ok(());
        }
        let (row1, column1, row2, column2) = source;
        let height = row2 - row1 + 1;
        let width = column2 - column1 + 1;
        let mut cells = Vec::new();
        for row in target.0..=target.2 {
            for column in target.1..=target.3 {
                let source_row = row1 + (row - row1).rem_euclid(height);
                let source_column = column1 + (column - column1).rem_euclid(width);
                if let Some(copied) = self.copied_validation(sheet, source_row, source_column) {
                    cells.push((row, column, copied));
                }
            }
        }
        diff_list.extend(self.place_copied_validations(sheet, target, cells, None)?);
        Ok(())
    }

    fn check_validation_settings(&self, settings: &ValidationSettings) -> Result<(), String> {
        let needs_first = settings.kind != ValidationKind::Any;
        let needs_second = matches!(
            settings.kind,
            ValidationKind::Whole
                | ValidationKind::Decimal
                | ValidationKind::Date
                | ValidationKind::Time
                | ValidationKind::TextLength
        ) && settings.operator.takes_two_values();
        if (needs_first && settings.formula1.trim().is_empty())
            || (needs_second && settings.formula2.trim().is_empty())
        {
            return Err("Fill in every value the rule needs.".to_string());
        }
        Ok(())
    }

    fn move_validation_anchor(
        &mut self,
        rule: &mut DataValidation,
        sheet: u32,
        before: &[Range],
        after: &[Range],
    ) -> Result<(), String> {
        let (Some(old), Some(new)) = (sqref_anchor(before), sqref_anchor(after)) else {
            return Ok(());
        };
        let old_context = self.validation_context(sheet, old)?;
        let new_context = self.validation_context(sheet, new)?;
        for formula in rule.settings.formulas_mut() {
            if !formula.is_empty() {
                let node = self.model.parse_internal_formula(formula, &old_context);
                *formula = to_english_string(&node, &new_context);
            }
        }
        Ok(())
    }

    /// The values a list rule offers on a cell.
    pub fn get_data_validation_items(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Vec<String>, String> {
        self.model.data_validation_items(sheet, row, column)
    }

    /// Checks the value now in a cell against the cell's rule.
    pub fn get_data_validation_failure(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<ValidationFailure>, String> {
        self.model.data_validation_failure(sheet, row, column)
    }

    /// The block of data around a cell, up to the first empty rows and columns.
    pub fn get_current_region(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<PivotArea, String> {
        let worksheet = self.model.workbook.worksheet(sheet)?;
        let filled = |r: i32, c: i32| {
            worksheet
                .cell(r, c)
                .is_some_and(|cell| !matches!(cell.as_ref(), Cell::EmptyCell { .. }))
        };
        let (mut top, mut left, mut bottom, mut right) = (row, column, row, column);
        loop {
            let before = (top, left, bottom, right);
            // Rows first, then columns: an edge is only rechecked if the other
            // direction has grown.
            while bottom < LAST_ROW && (left - 1..=right + 1).any(|c| filled(bottom + 1, c)) {
                bottom += 1;
            }
            while top > 1 && (left - 1..=right + 1).any(|c| filled(top - 1, c)) {
                top -= 1;
            }
            while right < LAST_COLUMN && (top - 1..=bottom + 1).any(|r| filled(r, right + 1)) {
                right += 1;
            }
            while left > 1 && (top - 1..=bottom + 1).any(|r| filled(r, left - 1)) {
                left -= 1;
            }
            if before == (top, left, bottom, right) {
                break;
            }
        }
        Ok(PivotArea {
            first_row: top,
            first_column: left,
            last_row: bottom,
            last_column: right,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::data_validation::{ValidationAlertStyle, ValidationOperator};
    use crate::expressions::types::Area;
    use crate::UserModel;

    fn model() -> UserModel<'static> {
        UserModel::new_empty("book", "en", "UTC", "en").unwrap()
    }

    fn area(first_row: i32, first_column: i32, last_row: i32, last_column: i32) -> PivotArea {
        PivotArea {
            first_row,
            first_column,
            last_row,
            last_column,
        }
    }

    fn list(source: &str) -> ValidationSettings {
        ValidationSettings {
            kind: ValidationKind::List,
            formula1: source.to_string(),
            ..Default::default()
        }
    }

    fn whole(operator: ValidationOperator, first: &str, second: &str) -> ValidationSettings {
        ValidationSettings {
            kind: ValidationKind::Whole,
            operator,
            formula1: first.to_string(),
            formula2: second.to_string(),
            ..Default::default()
        }
    }

    fn entered(model: &mut UserModel, row: i32, column: i32, text: &str) -> bool {
        model.set_user_input(0, row, column, text).unwrap();
        model
            .get_data_validation_failure(0, row, column)
            .unwrap()
            .is_none()
    }

    fn sqrefs(model: &UserModel) -> Vec<String> {
        model
            .model
            .workbook
            .data_validations
            .iter()
            .map(|rule| rule.sqref.clone())
            .collect()
    }

    #[test]
    fn typed_list() {
        let mut model = model();
        model
            .set_data_validation(0, area(1, 1, 3, 1), list("cat, dog,bird"))
            .unwrap();
        assert_eq!(
            model.get_data_validation_items(0, 2, 1).unwrap(),
            ["cat", "dog", "bird"]
        );
        assert!(entered(&mut model, 2, 1, "dog"));
        assert!(!entered(&mut model, 2, 1, "Dog"));
        assert!(!entered(&mut model, 2, 1, "fish"));
        assert_eq!(model.get_data_validation_items(0, 9, 1).unwrap().len(), 0);
        let shown = model.get_data_validation(0, 1, 1).unwrap().unwrap();
        assert_eq!(shown.formula1, "cat,dog,bird");
    }

    #[test]
    fn list_from_cells_ignores_case_and_shows_the_reference() {
        let mut model = model();
        model.new_sheet().unwrap();
        model.set_selected_sheet(0).unwrap();
        for (row, value) in ["North", "South", "", "West"].iter().enumerate() {
            model.set_user_input(1, row as i32 + 1, 1, value).unwrap();
        }
        model
            .set_data_validation(0, area(1, 1, 1, 1), list("=Sheet2!$A$1:$A$4"))
            .unwrap();
        assert_eq!(
            model.get_data_validation_items(0, 1, 1).unwrap(),
            ["North", "South", "West"]
        );
        assert!(entered(&mut model, 1, 1, "north"));
        assert!(!entered(&mut model, 1, 1, "East"));
        let shown = model.get_data_validation(0, 1, 1).unwrap().unwrap();
        assert_eq!(shown.formula1, "=Sheet2!$A$1:$A$4");
    }

    #[test]
    fn numbers_and_operators() {
        let mut model = model();
        model
            .set_data_validation(
                0,
                area(1, 1, 1, 1),
                whole(ValidationOperator::Between, "1", "10"),
            )
            .unwrap();
        assert!(entered(&mut model, 1, 1, "10"));
        assert!(!entered(&mut model, 1, 1, "11"));
        assert!(!entered(&mut model, 1, 1, "2.5"));
        assert!(!entered(&mut model, 1, 1, "text"));

        for (operator, value, good, bad) in [
            (ValidationOperator::NotBetween, "5", "11", "5"),
            (ValidationOperator::Equal, "5", "5", "6"),
            (ValidationOperator::NotEqual, "5", "6", "5"),
            (ValidationOperator::GreaterThan, "5", "6", "5"),
            (ValidationOperator::LessThan, "5", "4", "5"),
            (ValidationOperator::GreaterThanOrEqual, "5", "5", "4"),
            (ValidationOperator::LessThanOrEqual, "5", "5", "6"),
        ] {
            model
                .set_data_validation(0, area(1, 1, 1, 1), whole(operator, "1", "10"))
                .unwrap();
            if operator != ValidationOperator::NotBetween {
                model
                    .set_data_validation(0, area(1, 1, 1, 1), whole(operator, value, ""))
                    .unwrap();
            }
            assert!(entered(&mut model, 1, 1, good), "{operator:?} {good}");
            assert!(!entered(&mut model, 1, 1, bad) || operator == ValidationOperator::NotBetween);
        }
    }

    #[test]
    fn dates_text_length_and_custom() {
        let mut model = model();
        let date = ValidationSettings {
            kind: ValidationKind::Date,
            operator: ValidationOperator::GreaterThan,
            formula1: "2026-01-01".to_string(),
            ..Default::default()
        };
        model
            .set_data_validation(0, area(1, 1, 1, 1), date)
            .unwrap();
        assert_eq!(
            model
                .get_data_validation(0, 1, 1)
                .unwrap()
                .unwrap()
                .formula1,
            "2026-01-01"
        );
        assert!(entered(&mut model, 1, 1, "2026-02-01"));
        assert!(!entered(&mut model, 1, 1, "2025-02-01"));

        let length = ValidationSettings {
            kind: ValidationKind::TextLength,
            operator: ValidationOperator::LessThanOrEqual,
            formula1: "3".to_string(),
            ..Default::default()
        };
        model
            .set_data_validation(0, area(2, 1, 2, 1), length)
            .unwrap();
        assert!(entered(&mut model, 2, 1, "abc"));
        assert!(!entered(&mut model, 2, 1, "abcd"));

        // Relative to each cell of the rule
        let custom = ValidationSettings {
            kind: ValidationKind::Custom,
            formula1: "=C5>D5".to_string(),
            ..Default::default()
        };
        model.set_user_input(0, 6, 4, "1").unwrap();
        model
            .set_data_validation(0, area(5, 3, 6, 3), custom)
            .unwrap();
        assert!(entered(&mut model, 6, 3, "2"));
        assert!(!entered(&mut model, 6, 3, "1"));
    }

    #[test]
    fn blanks_alerts_and_show_error() {
        let mut model = model();
        let mut settings = whole(ValidationOperator::GreaterThan, "1", "");
        settings.error_style = ValidationAlertStyle::Warning;
        settings.error_title = "Too small".to_string();
        settings.error_message = "Bigger please".to_string();
        model
            .set_data_validation(0, area(1, 1, 1, 1), settings.clone())
            .unwrap();
        assert!(entered(&mut model, 1, 1, ""));
        model.set_user_input(0, 1, 1, "0").unwrap();
        let failure = model.get_data_validation_failure(0, 1, 1).unwrap().unwrap();
        assert_eq!(failure.title, "Too small");
        assert_eq!(failure.message, "Bigger please");
        assert_eq!(failure.style, ValidationAlertStyle::Warning);

        settings.ignore_blank = false;
        model
            .set_data_validation(0, area(1, 1, 1, 1), settings.clone())
            .unwrap();
        assert!(!entered(&mut model, 1, 1, ""));

        settings.show_error = false;
        model
            .set_data_validation(0, area(1, 1, 1, 1), settings)
            .unwrap();
        assert!(entered(&mut model, 1, 1, "0"));
    }

    #[test]
    fn a_new_rule_cuts_the_old_one_and_undo_restores_it() {
        let mut model = model();
        model
            .set_data_validation(0, area(1, 1, 5, 2), list("a,b"))
            .unwrap();
        model
            .set_data_validation(0, area(2, 1, 3, 1), list("c"))
            .unwrap();
        let mut found = sqrefs(&model);
        found.sort();
        assert_eq!(found, ["A1:B1 B2:B3 A4:B5", "A2:A3"]);
        model.undo().unwrap();
        assert_eq!(sqrefs(&model), ["A1:B5"]);
        model.redo().unwrap();
        assert_eq!(sqrefs(&model).len(), 2);
        // Any value with no messages clears
        model
            .set_data_validation(0, area(1, 1, 9, 9), ValidationSettings::default())
            .unwrap();
        assert!(sqrefs(&model).is_empty());
    }

    #[test]
    fn cutting_the_top_left_keeps_relative_references_right() {
        let mut model = model();
        let custom = ValidationSettings {
            kind: ValidationKind::Custom,
            formula1: "=C1>0".to_string(),
            ..Default::default()
        };
        model
            .set_data_validation(0, area(1, 1, 3, 1), custom)
            .unwrap();
        model
            .set_data_validation(0, area(1, 1, 1, 1), ValidationSettings::default())
            .unwrap();
        assert_eq!(sqrefs(&model), ["A2:A3"]);
        assert_eq!(
            model
                .get_data_validation(0, 2, 1)
                .unwrap()
                .unwrap()
                .formula1,
            "=C2>0"
        );
    }

    #[test]
    fn bad_settings_are_refused() {
        let mut model = model();
        let target = area(1, 1, 1, 1);
        assert!(model.set_data_validation(0, target, list("")).is_err());
        assert!(model
            .set_data_validation(0, target, list(&"x,".repeat(200)))
            .is_err());
        assert!(model
            .set_data_validation(0, target, whole(ValidationOperator::Between, "9", "1"))
            .is_err());
        assert!(model
            .set_data_validation(0, target, whole(ValidationOperator::Equal, "abc", ""))
            .is_err());
        assert!(sqrefs(&model).is_empty());
    }

    #[test]
    fn rules_follow_rows_and_their_sources() {
        let mut model = model();
        model.new_sheet().unwrap();
        model.set_selected_sheet(0).unwrap();
        model
            .set_data_validation(0, area(3, 1, 5, 1), list("=Sheet2!$A$2:$A$4"))
            .unwrap();
        model.insert_rows(0, 1, 2).unwrap();
        assert_eq!(sqrefs(&model), ["A5:A7"]);
        model.insert_rows(1, 1, 1).unwrap();
        assert_eq!(
            model
                .get_data_validation(0, 5, 1)
                .unwrap()
                .unwrap()
                .formula1,
            "=Sheet2!$A$3:$A$5"
        );
        model.delete_rows(0, 6, 1).unwrap();
        assert_eq!(sqrefs(&model), ["A5:A6"]);
        model.delete_rows(0, 5, 2).unwrap();
        assert!(sqrefs(&model).is_empty());
        model.undo().unwrap();
        assert_eq!(sqrefs(&model), ["A5:A6"]);
    }

    #[test]
    fn rules_follow_a_renamed_sheet() {
        let mut model = model();
        model.new_sheet().unwrap();
        model.set_selected_sheet(0).unwrap();
        model
            .set_data_validation(0, area(1, 1, 1, 1), list("=Sheet2!$A$1:$A$2"))
            .unwrap();
        model.rename_sheet(1, "Lists").unwrap();
        assert_eq!(
            model
                .get_data_validation(0, 1, 1)
                .unwrap()
                .unwrap()
                .formula1,
            "=Lists!$A$1:$A$2"
        );
    }

    #[test]
    fn rules_are_saved() {
        let mut model = model();
        model
            .set_data_validation(0, area(1, 1, 2, 1), list("a,b"))
            .unwrap();
        let mut reopened = UserModel::from_bytes(&model.to_bytes(), "en").unwrap();
        assert_eq!(
            reopened
                .get_data_validation(0, 2, 1)
                .unwrap()
                .unwrap()
                .formula1,
            "a,b"
        );
    }

    #[test]
    fn current_region_stops_at_empty_rows_and_columns() {
        let mut model = model();
        for (row, column) in [(2, 2), (2, 3), (3, 2), (4, 4), (9, 9)] {
            model.set_user_input(0, row, column, "x").unwrap();
        }
        assert_eq!(model.get_current_region(0, 2, 2).unwrap(), area(2, 2, 4, 4),);
        assert_eq!(model.get_current_region(0, 9, 9).unwrap(), area(9, 9, 9, 9));
    }

    fn copy_cells(
        model: &mut UserModel,
        range: [i32; 4],
    ) -> crate::user_model::clipboard::Clipboard {
        model.set_selected_cell(range[0], range[1]).unwrap();
        model
            .set_selected_range(range[0], range[1], range[2], range[3])
            .unwrap();
        model.copy_to_clipboard().unwrap()
    }

    fn paste(
        model: &mut UserModel,
        clipboard: &crate::user_model::clipboard::Clipboard,
        row: i32,
        column: i32,
        is_cut: bool,
    ) {
        model.set_selected_cell(row, column).unwrap();
        model
            .paste_from_clipboard(0, clipboard.range, &clipboard.data, is_cut)
            .unwrap();
    }

    #[test]
    fn copy_brings_the_rule_and_undo_takes_it_back() {
        let mut model = model();
        model
            .set_data_validation(0, area(1, 1, 1, 1), list("a,b"))
            .unwrap();
        let clipboard = copy_cells(&mut model, [1, 1, 1, 1]);
        paste(&mut model, &clipboard, 4, 3, false);
        assert_eq!(
            model.get_data_validation_items(0, 4, 3).unwrap(),
            ["a", "b"]
        );
        assert_eq!(
            model.get_data_validation_items(0, 1, 1).unwrap(),
            ["a", "b"]
        );
        model.undo().unwrap();
        assert_eq!(model.get_data_validation(0, 4, 3).unwrap(), None);
        model.redo().unwrap();
        assert!(model.get_data_validation(0, 4, 3).unwrap().is_some());
    }

    #[test]
    fn pasting_next_to_the_rule_joins_it() {
        let mut model = model();
        model
            .set_data_validation(0, area(1, 1, 1, 1), list("a,b"))
            .unwrap();
        let clipboard = copy_cells(&mut model, [1, 1, 1, 1]);
        paste(&mut model, &clipboard, 2, 1, false);
        assert_eq!(sqrefs(&model), ["A1:A2"]);
    }

    #[test]
    fn a_copied_range_keeps_each_cells_rule_on_an_empty_sheet() {
        let mut model = model();
        model
            .set_data_validation(0, area(1, 1, 3, 1), list("a,b"))
            .unwrap();
        let clipboard = copy_cells(&mut model, [1, 1, 3, 1]);
        paste(&mut model, &clipboard, 1, 3, false);
        let mut found = sqrefs(&model);
        found.sort();
        assert_eq!(found, ["A1:A3 C1:C3"]);
    }

    #[test]
    fn relative_references_move_with_the_paste() {
        let mut model = model();
        let custom = ValidationSettings {
            kind: ValidationKind::Custom,
            formula1: "=B1>$D$1".to_string(),
            ..Default::default()
        };
        model
            .set_data_validation(0, area(1, 1, 2, 1), custom)
            .unwrap();
        let clipboard = copy_cells(&mut model, [2, 1, 2, 1]);
        paste(&mut model, &clipboard, 8, 3, false);
        assert_eq!(
            model
                .get_data_validation(0, 8, 3)
                .unwrap()
                .unwrap()
                .formula1,
            "=D8>$D$1"
        );
        assert_eq!(
            model
                .get_data_validation(0, 1, 1)
                .unwrap()
                .unwrap()
                .formula1,
            "=B1>$D$1"
        );
    }

    #[test]
    fn pasting_a_plain_cell_clears_the_rule_there() {
        let mut model = model();
        model.set_user_input(0, 2, 1, "plain").unwrap();
        model
            .set_data_validation(0, area(1, 3, 3, 3), list("a,b"))
            .unwrap();
        let clipboard = copy_cells(&mut model, [2, 1, 2, 1]);
        paste(&mut model, &clipboard, 2, 3, false);
        assert_eq!(model.get_data_validation(0, 2, 3).unwrap(), None);
        let mut found = sqrefs(&model);
        found.sort();
        assert_eq!(found, ["C1 C3"]);
    }

    #[test]
    fn a_formula_typed_with_the_active_cell_lower_down_reads_from_it() {
        let mut model = model();
        model.set_selected_cell(3, 1).unwrap();
        model.set_selected_range(3, 1, 1, 1).unwrap();
        assert_eq!(model.get_selected_view().row, 3);
        let custom = ValidationSettings {
            kind: ValidationKind::Custom,
            formula1: "=B3>0".to_string(),
            ..Default::default()
        };
        model
            .set_data_validation(0, area(1, 1, 3, 1), custom)
            .unwrap();
        assert_eq!(
            model
                .get_data_validation(0, 1, 1)
                .unwrap()
                .unwrap()
                .formula1,
            "=B1>0"
        );
        assert_eq!(
            model
                .get_data_validation(0, 3, 1)
                .unwrap()
                .unwrap()
                .formula1,
            "=B3>0"
        );
    }

    #[test]
    fn cut_moves_the_rule() {
        let mut model = model();
        model.set_user_input(0, 1, 1, "a").unwrap();
        model
            .set_data_validation(0, area(1, 1, 1, 1), list("a,b"))
            .unwrap();
        let clipboard = copy_cells(&mut model, [1, 1, 1, 1]);
        paste(&mut model, &clipboard, 5, 5, true);
        assert_eq!(sqrefs(&model), ["E5"]);
        model.undo().unwrap();
        assert_eq!(sqrefs(&model), ["A1"]);
    }

    #[test]
    fn a_rule_can_be_pasted_on_another_sheet() {
        let mut model = model();
        model.new_sheet().unwrap();
        model.set_selected_sheet(0).unwrap();
        model
            .set_data_validation(0, area(1, 1, 1, 1), list("=$C$1:$C$2"))
            .unwrap();
        let clipboard = copy_cells(&mut model, [1, 1, 1, 1]);
        model.set_selected_sheet(1).unwrap();
        model.set_selected_cell(2, 2).unwrap();
        model
            .paste_from_clipboard(0, clipboard.range, &clipboard.data, false)
            .unwrap();
        assert_eq!(
            model
                .get_data_validation(1, 2, 2)
                .unwrap()
                .unwrap()
                .formula1,
            "=$C$1:$C$2"
        );
        assert!(model.get_data_validation(0, 1, 1).unwrap().is_some());
    }

    #[test]
    fn the_fill_handle_repeats_rules() {
        let mut model = model();
        model.set_user_input(0, 1, 1, "a").unwrap();
        model.set_user_input(0, 2, 1, "b").unwrap();
        model
            .set_data_validation(0, area(1, 1, 1, 1), list("a,b"))
            .unwrap();
        let source = Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 1,
            height: 2,
        };
        model.auto_fill_rows(&source, 4).unwrap();
        assert!(model.get_data_validation(0, 3, 1).unwrap().is_some());
        assert_eq!(model.get_data_validation(0, 4, 1).unwrap(), None);
        model.undo().unwrap();
        assert_eq!(model.get_data_validation(0, 3, 1).unwrap(), None);
        model.auto_fill_columns(&source, 3).unwrap();
        assert!(model.get_data_validation(0, 1, 3).unwrap().is_some());
        assert_eq!(model.get_data_validation(0, 2, 2).unwrap(), None);
    }

    fn duplicates() -> UserModel<'static> {
        let mut model = model();
        for (row, values) in [
            ["Name", "Qty"],
            ["pear", "1"],
            ["Pear", "1"],
            ["fig", "1"],
            ["pear", "2"],
        ]
        .iter()
        .enumerate()
        {
            for (column, value) in values.iter().enumerate() {
                model
                    .set_user_input(0, row as i32 + 1, column as i32 + 1, value)
                    .unwrap();
            }
        }
        model.set_user_input(0, 3, 4, "outside").unwrap();
        model
    }

    #[test]
    fn remove_duplicates_on_all_columns() {
        let mut model = duplicates();
        model.set_comment(0, 4, 1, "on fig").unwrap();
        let result = model
            .remove_duplicates(0, area(1, 1, 5, 2), &[1, 2], true)
            .unwrap();
        assert_eq!(result, (1, 3));
        assert_eq!(model.get_cell_content(0, 3, 1).unwrap(), "fig");
        assert_eq!(model.get_cell_content(0, 4, 1).unwrap(), "pear");
        assert_eq!(model.get_cell_content(0, 5, 1).unwrap(), "");
        assert_eq!(model.get_cell_content(0, 3, 4).unwrap(), "outside");
        assert_eq!(model.get_comment(0, 3, 1).unwrap().unwrap().text, "on fig");
        model.undo().unwrap();
        assert_eq!(model.get_cell_content(0, 3, 1).unwrap(), "Pear");
        assert_eq!(model.get_comment(0, 4, 1).unwrap().unwrap().text, "on fig");
    }

    #[test]
    fn remove_duplicates_on_some_columns_and_without_headers() {
        let mut model = duplicates();
        assert_eq!(
            model
                .remove_duplicates(0, area(1, 1, 5, 2), &[2], false)
                .unwrap(),
            (2, 3)
        );
        assert_eq!(model.get_cell_content(0, 2, 1).unwrap(), "pear");
        assert_eq!(model.get_cell_content(0, 3, 1).unwrap(), "pear");
    }

    #[test]
    fn remove_duplicates_with_nothing_to_remove_adds_no_undo_step() {
        let mut model = duplicates();
        model.set_user_input(0, 9, 9, "last").unwrap();
        assert_eq!(
            model
                .remove_duplicates(0, area(4, 1, 5, 2), &[1, 2], false)
                .unwrap(),
            (0, 2)
        );
        model.undo().unwrap();
        assert_eq!(model.get_cell_content(0, 9, 9).unwrap(), "");
    }

    #[test]
    fn remove_duplicates_is_refused_where_it_would_do_damage() {
        let mut model = duplicates();
        assert!(model
            .remove_duplicates(0, area(1, 1, 5, 2), &[], true)
            .is_err());
        model.create_table(0, area(1, 1, 5, 2), true, None).unwrap();
        assert!(model
            .remove_duplicates(0, area(1, 1, 5, 2), &[1], true)
            .is_err());
    }
}
