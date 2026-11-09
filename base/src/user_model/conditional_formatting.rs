use crate::{
    cf_types::{CfRule, CfRuleInput, ConditionalFormatting, ConditionalFormattingView},
    conditional_formatting::parse_sqref,
    data_validation::{sqref_anchor, sqref_of, subtract_range},
    expressions::{parser::stringify::to_english_string, types::CellReferenceRC},
    pivot::PivotArea,
    types::Dxf,
};

use super::{common::UserModel, history::Diff};
use crate::conditional_formatting::canonical_sqref;

impl<'a> UserModel<'a> {
    /// Removes rules from an area, or from the whole sheet with `None`. Rules
    /// reaching outside the area keep their other cells.
    pub fn clear_conditional_formatting(
        &mut self,
        sheet: u32,
        area: Option<PivotArea>,
    ) -> Result<(), String> {
        let old_value = self
            .model
            .workbook
            .worksheet(sheet)?
            .conditional_formatting
            .clone();
        let Some(area) = area else {
            return self.set_conditional_formats(sheet, old_value, vec![]);
        };
        let hole = (
            area.first_row,
            area.first_column,
            area.last_row,
            area.last_column,
        );
        let mut new_value = Vec::with_capacity(old_value.len());
        for rule in &old_value {
            let ranges = parse_sqref(&rule.range);
            let pieces: Vec<_> = ranges
                .iter()
                .flat_map(|range| subtract_range(*range, hole))
                .collect();
            if pieces.is_empty() {
                continue;
            }
            let mut rule = rule.clone();
            if pieces != ranges {
                if sqref_anchor(&pieces) != sqref_anchor(&ranges) {
                    self.move_rule_anchor(sheet, &mut rule.cf_rule, &ranges, &pieces)?;
                }
                rule.range = sqref_of(&pieces);
            }
            new_value.push(rule);
        }
        self.set_conditional_formats(sheet, old_value, new_value)
    }

    fn set_conditional_formats(
        &mut self,
        sheet: u32,
        old_value: Vec<ConditionalFormatting>,
        new_value: Vec<ConditionalFormatting>,
    ) -> Result<(), String> {
        if old_value == new_value {
            return Ok(());
        }
        self.model
            .workbook
            .worksheet_mut(sheet)?
            .conditional_formatting = new_value.clone();
        self.push_diff_list(vec![Diff::SetConditionalFormats {
            sheet,
            old_value,
            new_value,
        }]);
        self.evaluate_if_not_paused();
        Ok(())
    }

    /// Relative references are read from the top left of a rule's cells.
    fn move_rule_anchor(
        &mut self,
        sheet: u32,
        rule: &mut CfRule,
        before: &[(i32, i32, i32, i32)],
        after: &[(i32, i32, i32, i32)],
    ) -> Result<(), String> {
        let (Some(old), Some(new)) = (sqref_anchor(before), sqref_anchor(after)) else {
            return Ok(());
        };
        let name = self.model.workbook.worksheet(sheet)?.get_name();
        let context = |(row, column): (i32, i32)| CellReferenceRC {
            sheet: name.clone(),
            row,
            column,
        };
        let (old_context, new_context) = (context(old), context(new));
        let mut move_formula = |formula: &mut String| {
            let trimmed = formula.trim();
            let (prefix, body) = match trimmed.strip_prefix('=') {
                Some(body) => ("=", body),
                None => ("", trimmed),
            };
            let node = self.model.parse_internal_formula(body, &old_context);
            *formula = format!("{prefix}{}", to_english_string(&node, &new_context));
        };
        match rule {
            CfRule::CellIs {
                formula, formula2, ..
            } => {
                move_formula(formula);
                if let Some(second) = formula2 {
                    move_formula(second);
                }
            }
            CfRule::Formula { formula, .. } => move_formula(formula),
            _ => {}
        }
        Ok(())
    }

    /// Returns all CF rules for the given sheet, sorted by priority descending.
    ///
    /// Each entry carries its storage `index`; see
    /// [`crate::Model::get_conditional_formatting_list`].
    pub fn get_conditional_formatting_list(
        &self,
        sheet: u32,
    ) -> Result<Vec<ConditionalFormattingView>, String> {
        self.model.get_conditional_formatting_list(sheet)
    }

    /// Returns the differential format (Dxf) for the CF rule at `index` on `sheet`,
    /// or `None` if the rule type has no associated dxf (e.g. ColorScale, DataBar).
    pub fn get_dxf_for_conditional_formatting(
        &self,
        sheet: u32,
        index: u32,
    ) -> Result<Option<Dxf>, String> {
        self.model
            .get_dxf_for_conditional_formatting(sheet, index as usize)
    }

    /// Adds a new CF rule to `sheet`.
    pub fn add_conditional_formatting(
        &mut self,
        sheet: u32,
        range: &str,
        rule: CfRuleInput,
    ) -> Result<(), String> {
        let range = &canonical_sqref(range)?;
        let priority = self.model.add_conditional_formatting(sheet, range, rule)?;
        // Read back the stored entry so the Diff contains the dxf_id that was assigned.
        let stored_rule = self
            .model
            .workbook
            .worksheet(sheet)
            .ok()
            .and_then(|ws| {
                ws.conditional_formatting
                    .iter()
                    .find(|cf| cf.priority == priority)
                    .map(|cf| cf.cf_rule.clone())
            })
            .unwrap_or(CfRule::DuplicateValues {
                dxf_id: 0,
                stop_if_true: false,
            });
        self.push_diff_list(vec![Diff::AddConditionalFormatting {
            sheet,
            range: range.to_string(),
            rule: Box::new(stored_rule),
            priority,
        }]);
        self.evaluate_if_not_paused();
        Ok(())
    }

    /// Removes the CF rule at `index` from `sheet`.
    pub fn delete_conditional_formatting(&mut self, sheet: u32, index: u32) -> Result<(), String> {
        let old = self
            .model
            .delete_conditional_formatting(sheet, index as usize)?;
        self.push_diff_list(vec![Diff::DeleteConditionalFormatting {
            sheet,
            index,
            old_range: old.range,
            old_rule: Box::new(old.cf_rule),
            old_priority: old.priority,
        }]);
        self.evaluate_if_not_paused();
        Ok(())
    }

    /// Replaces the range and rule of the CF entry at `index` on `sheet`.
    pub fn update_conditional_formatting(
        &mut self,
        sheet: u32,
        index: u32,
        new_range: &str,
        new_rule: CfRuleInput,
    ) -> Result<(), String> {
        let new_range = &canonical_sqref(new_range)?;
        let old =
            self.model
                .update_conditional_formatting(sheet, index as usize, new_range, new_rule)?;
        // Read back the stored entry so the Diff contains the dxf_id that was assigned.
        let stored_rule = self
            .model
            .workbook
            .worksheet(sheet)
            .ok()
            .and_then(|ws| {
                ws.conditional_formatting
                    .get(index as usize)
                    .map(|cf| cf.cf_rule.clone())
            })
            .unwrap_or(CfRule::DuplicateValues {
                dxf_id: 0,
                stop_if_true: false,
            });
        self.push_diff_list(vec![Diff::UpdateConditionalFormatting {
            sheet,
            index,
            old_range: old.range,
            old_rule: Box::new(old.cf_rule),
            old_priority: old.priority,
            new_range: new_range.to_string(),
            new_rule: Box::new(stored_rule),
        }]);
        self.evaluate_if_not_paused();
        Ok(())
    }

    /// Raises the priority of the CF rule at `index` on `sheet` by swapping its
    /// priority with the next-higher-priority rule. No-op if it is already the
    /// highest-priority rule.
    pub fn raise_conditional_formatting_priority(
        &mut self,
        sheet: u32,
        index: u32,
    ) -> Result<(), String> {
        self.swap_conditional_formatting_priority(sheet, index, true)
    }

    /// Lowers the priority of the CF rule at `index` on `sheet` by swapping its
    /// priority with the next-lower-priority rule. No-op if it is already the
    /// lowest-priority rule.
    pub fn lower_conditional_formatting_priority(
        &mut self,
        sheet: u32,
        index: u32,
    ) -> Result<(), String> {
        self.swap_conditional_formatting_priority(sheet, index, false)
    }

    /// Shared implementation for raising/lowering CF priority. Snapshots the
    /// priorities before and after delegating to the base model, then records a
    /// `SwapConditionalFormattingPriority` diff for the (at most two) rules whose
    /// priority changed so the operation can be undone/redone.
    fn swap_conditional_formatting_priority(
        &mut self,
        sheet: u32,
        index: u32,
        raise: bool,
    ) -> Result<(), String> {
        let before: Vec<u32> = self
            .model
            .workbook
            .worksheet(sheet)?
            .conditional_formatting
            .iter()
            .map(|cf| cf.priority)
            .collect();
        if raise {
            self.model
                .raise_conditional_formatting_priority(sheet, index as usize)?;
        } else {
            self.model
                .lower_conditional_formatting_priority(sheet, index as usize)?;
        }
        let after: Vec<u32> = self
            .model
            .workbook
            .worksheet(sheet)?
            .conditional_formatting
            .iter()
            .map(|cf| cf.priority)
            .collect();
        // A successful swap touches exactly two rules; if nothing changed (the
        // rule was already at the boundary) there is nothing to record.
        let changed: Vec<usize> = (0..before.len())
            .filter(|&i| before[i] != after[i])
            .collect();
        if let [a, b] = changed[..] {
            self.push_diff_list(vec![Diff::SwapConditionalFormattingPriority {
                sheet,
                index_a: a as u32,
                index_b: b as u32,
                priority_a: before[a],
                priority_b: before[b],
            }]);
        }
        self.evaluate_if_not_paused();
        Ok(())
    }
}

#[cfg(test)]
mod clear_tests {
    #![allow(clippy::unwrap_used)]

    use crate::cf_types::CfRuleInput;
    use crate::pivot::PivotArea;
    use crate::types::Dxf;
    use crate::UserModel;

    fn model() -> UserModel<'static> {
        UserModel::new_empty("book", "en", "UTC", "en").unwrap()
    }

    fn formula(text: &str) -> CfRuleInput {
        CfRuleInput::Formula {
            formula: text.to_string(),
            format: Dxf::default(),
            stop_if_true: false,
        }
    }

    fn area(first_row: i32, first_column: i32, last_row: i32, last_column: i32) -> PivotArea {
        PivotArea {
            first_row,
            first_column,
            last_row,
            last_column,
        }
    }

    fn ranges(model: &UserModel) -> Vec<String> {
        model.model.workbook.worksheets[0]
            .conditional_formatting
            .iter()
            .map(|cf| cf.range.clone())
            .collect()
    }

    #[test]
    fn clearing_part_of_a_rule_leaves_the_rest_and_undo_restores_it() {
        let mut model = model();
        model
            .add_conditional_formatting(0, "A1:B3", formula("=C1>0"))
            .unwrap();
        model
            .clear_conditional_formatting(0, Some(area(1, 1, 1, 2)))
            .unwrap();
        assert_eq!(ranges(&model), ["A2:B3"]);
        match &model.model.workbook.worksheets[0].conditional_formatting[0].cf_rule {
            crate::cf_types::CfRule::Formula { formula, .. } => assert_eq!(formula, "=C2>0"),
            other => panic!("unexpected rule {other:?}"),
        }
        model.undo().unwrap();
        assert_eq!(ranges(&model), ["A1:B3"]);
        model.redo().unwrap();
        assert_eq!(ranges(&model), ["A2:B3"]);
    }

    #[test]
    fn clearing_the_sheet_removes_everything_in_one_undo() {
        let mut model = model();
        model
            .add_conditional_formatting(0, "A1", formula("=TRUE"))
            .unwrap();
        model
            .add_conditional_formatting(0, "B1", formula("=TRUE"))
            .unwrap();
        model.clear_conditional_formatting(0, None).unwrap();
        assert!(ranges(&model).is_empty());
        model.undo().unwrap();
        assert_eq!(ranges(&model).len(), 2);
    }

    #[test]
    fn rules_outside_the_area_are_untouched() {
        let mut model = model();
        model
            .add_conditional_formatting(0, "D4", formula("=TRUE"))
            .unwrap();
        model
            .clear_conditional_formatting(0, Some(area(1, 1, 2, 2)))
            .unwrap();
        assert_eq!(ranges(&model), ["D4"]);
    }

    #[test]
    fn rules_move_with_inserted_cells_and_undo_restores_them() {
        let mut model = model();
        model
            .add_conditional_formatting(0, "A1:A4", formula("=TRUE"))
            .unwrap();
        model.insert_cells(0, area(2, 1, 2, 1), true).unwrap();
        assert_eq!(ranges(&model), ["A1 A3:A5"]);
        model.undo().unwrap();
        assert_eq!(ranges(&model), ["A1:A4"]);
    }

    #[test]
    fn excels_icon_sets_have_its_bands() {
        let set = crate::cf_types::icon_set_thresholds("3Arrows").unwrap();
        let bands: Vec<_> = set.iter().map(|t| t.cfvo.clone()).collect();
        assert_eq!(
            bands,
            [
                crate::cf_types::Cfvo::Percent(0.0),
                crate::cf_types::Cfvo::Percent(33.0),
                crate::cf_types::Cfvo::Percent(67.0)
            ]
        );
        assert!(crate::cf_types::icon_set_thresholds("nonsense").is_none());
    }
}
