//! Inserting blank cells, and Insert Copied Cells / Insert Cut Cells.

use crate::user_model::protection::Permission;
use serde::{Deserialize, Serialize};

use crate::constants::{LAST_COLUMN, LAST_ROW};
use crate::pivot::PivotArea;
use crate::table::table_area;
use crate::types::Cell;

use super::{
    clipboard::{ClipboardData, ClipboardTuple},
    common::UserModel,
    history::Diff,
};

type Range = (i32, i32, i32, i32);

/// How copied cells make room for themselves.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum InsertCopiedMode {
    /// The cells at the target move down
    Down,
    /// The cells at the target move right
    Right,
    /// Whole rows are inserted
    Rows,
    /// Whole columns are inserted
    Columns,
}

fn overlap_error() -> String {
    "The cut cells and the place to insert them overlap.".to_string()
}

impl UserModel<'_> {
    fn check_cell_shift(&self, sheet: u32, area: Range, down: bool) -> Result<(), String> {
        let (r1, c1, r2, c2) = area;
        let worksheet = self.model.workbook.worksheet(sheet)?;
        let zone = if down {
            PivotArea {
                first_row: r1,
                first_column: c1,
                last_row: LAST_ROW,
                last_column: c2,
            }
        } else {
            PivotArea {
                first_row: r1,
                first_column: c1,
                last_row: r2,
                last_column: LAST_COLUMN,
            }
        };
        let (height, width) = (r2 - r1 + 1, c2 - c1 + 1);
        for (row, column, cell) in worksheet.sheet_data.cells() {
            if !zone.contains(row, column) {
                continue;
            }
            if (down && row + height > LAST_ROW) || (!down && column + width > LAST_COLUMN) {
                return Err("That would push cells off the end of the sheet.".to_string());
            }
            if matches!(&*cell, Cell::ArrayFormula { .. } | Cell::SpillCell { .. }) {
                return Err("Can't shift the cells of an array formula.".to_string());
            }
            if self.get_pivot_table_at(sheet, row, column).is_some() {
                return Err("Can't shift the cells of a PivotTable.".to_string());
            }
        }
        for merge in &worksheet.merged_cells {
            let splits = if down {
                merge.last_column() >= c1
                    && merge.column <= c2
                    && merge.last_row() >= r1
                    && (merge.column < c1 || merge.last_column() > c2 || merge.row < r1)
            } else {
                merge.last_row() >= r1
                    && merge.row <= r2
                    && merge.last_column() >= c1
                    && (merge.row < r1 || merge.last_row() > r2 || merge.column < c1)
            };
            if splits {
                return Err("Can't shift only part of a merged cell.".to_string());
            }
        }
        let sheet_name = worksheet.get_name();
        let in_table = self.model.workbook.tables.values().any(|table| {
            table.sheet_name == sheet_name
                && table_area(table).is_some_and(|place| place.intersects(&zone))
        });
        if in_table {
            return Err("Can't shift the cells of a table.".to_string());
        }
        if self
            .get_sheet_filter(sheet)
            .is_some_and(|filter| filter.filtered && filter.area.intersects(&zone))
        {
            return Err("Clear the filter before shifting these cells.".to_string());
        }
        Ok(())
    }

    fn shift_cells_step(
        &mut self,
        sheet: u32,
        area: Range,
        down: bool,
        insert: bool,
    ) -> Result<(), String> {
        if insert {
            self.check_cell_shift(sheet, area, down)?;
        }
        let snapshot = self.table_snapshot_diff(sheet);
        self.model.shift_cells(sheet, area, down, insert)?;
        let mut diff_list: Vec<Diff> = snapshot.into_iter().collect();
        diff_list.push(Diff::ShiftCells {
            sheet,
            area,
            down,
            insert,
        });
        self.push_diff_list(diff_list);
        self.evaluate_if_not_paused();
        Ok(())
    }

    /// Inserts blank cells over `area`, moving what is there down or right.
    pub fn insert_cells(&mut self, sheet: u32, area: PivotArea, down: bool) -> Result<(), String> {
        self.guard(sheet, Permission::Never)?;
        self.shift_cells_step(
            sheet,
            (
                area.first_row,
                area.first_column,
                area.last_row,
                area.last_column,
            ),
            down,
            true,
        )
    }

    /// Inserts copied or cut cells at the selection, as one undo step.
    ///
    /// `count` is how many rows or columns the copy spans in those two modes,
    /// where the copied range itself stops at the used part of the sheet.
    pub fn insert_copied_cells(
        &mut self,
        source_sheet: u32,
        source_range: ClipboardTuple,
        clipboard: &ClipboardData,
        is_cut: bool,
        mode: InsertCopiedMode,
        count: i32,
    ) -> Result<(), String> {
        self.guard(self.view.sheet, Permission::Never)?;
        let depth = self.history.undo_stack.len();
        let result =
            self.insert_copied_steps(source_sheet, source_range, clipboard, is_cut, mode, count);
        if result.is_err() {
            while self.history.undo_stack.len() > depth {
                self.undo()?;
            }
            self.history.redo_stack.clear();
            return result;
        }
        let steps = self.history.undo_stack.split_off(depth);
        let merged: Vec<Diff> = steps.into_iter().flatten().collect();
        if !merged.is_empty() {
            self.history.undo_stack.push(merged);
        }
        Ok(())
    }

    fn paste_at(
        &mut self,
        row: i32,
        column: i32,
        source_sheet: u32,
        source_range: ClipboardTuple,
        clipboard: &ClipboardData,
        is_cut: bool,
    ) -> Result<(), String> {
        self.set_selected_cell(row, column)?;
        self.paste_from_clipboard(source_sheet, source_range, clipboard, is_cut)
    }

    /// Takes cut cells from where they are now, so that a shift they went
    /// through is accounted for.
    fn paste_cut_from(
        &mut self,
        source_sheet: u32,
        source: Range,
        target: (u32, i32, i32),
    ) -> Result<(), String> {
        let (r1, c1, r2, c2) = source;
        self.set_selected_sheet(source_sheet)?;
        self.set_selected_cell(r1, c1)?;
        self.set_selected_range(r1, c1, r2, c2)?;
        let cut = self.copy_to_clipboard()?;
        self.set_selected_sheet(target.0)?;
        self.paste_at(target.1, target.2, source_sheet, cut.range, &cut.data, true)
    }

    fn insert_copied_steps(
        &mut self,
        source_sheet: u32,
        source_range: ClipboardTuple,
        clipboard: &ClipboardData,
        is_cut: bool,
        mode: InsertCopiedMode,
        count: i32,
    ) -> Result<(), String> {
        let view = self.get_selected_view();
        let sheet = view.sheet;
        let row = view.range[0].min(view.range[2]);
        let column = view.range[1].min(view.range[3]);
        let (r1, c1, r2, c2) = source_range;
        let (height, width) = (r2 - r1 + 1, c2 - c1 + 1);
        let same_sheet = sheet == source_sheet;
        match mode {
            InsertCopiedMode::Rows => {
                let last = r1 + count - 1;
                if is_cut && same_sheet {
                    if (r1..=last + 1).contains(&row) {
                        return Err(overlap_error());
                    }
                    let delta = if row < r1 { row - r1 } else { row - 1 - last };
                    return self.move_rows_action(sheet, r1, count, delta);
                }
                self.insert_rows(sheet, row, count)?;
                self.paste_at(row, c1, source_sheet, source_range, clipboard, is_cut)?;
                if is_cut {
                    self.delete_rows(source_sheet, r1, count)?;
                }
                self.set_selected_cell(row, c1)
            }
            InsertCopiedMode::Columns => {
                let last = c1 + count - 1;
                if is_cut && same_sheet {
                    if (c1..=last + 1).contains(&column) {
                        return Err(overlap_error());
                    }
                    let delta = if column < c1 {
                        column - c1
                    } else {
                        column - 1 - last
                    };
                    return self.move_columns_action(sheet, c1, count, delta);
                }
                self.insert_columns(sheet, column, count)?;
                self.paste_at(r1, column, source_sheet, source_range, clipboard, is_cut)?;
                if is_cut {
                    self.delete_columns(source_sheet, c1, count)?;
                }
                self.set_selected_cell(r1, column)
            }
            InsertCopiedMode::Down | InsertCopiedMode::Right => {
                let down = mode == InsertCopiedMode::Down;
                let area = (row, column, row + height - 1, column + width - 1);
                if !is_cut {
                    self.shift_cells_step(sheet, area, down, true)?;
                    self.paste_at(row, column, source_sheet, source_range, clipboard, false)?;
                    return self.select_inserted(area);
                }
                self.insert_cut_cells(source_sheet, source_range, sheet, area, down)
            }
        }
    }

    fn select_inserted(&mut self, area: Range) -> Result<(), String> {
        self.set_selected_cell(area.0, area.1)?;
        self.set_selected_range(area.0, area.1, area.2, area.3)
    }

    fn insert_cut_cells(
        &mut self,
        source_sheet: u32,
        source: Range,
        sheet: u32,
        area: Range,
        down: bool,
    ) -> Result<(), String> {
        let (r1, c1, r2, c2) = source;
        let (height, width) = (r2 - r1 + 1, c2 - c1 + 1);
        let (row, column, ..) = area;
        let same_sheet = sheet == source_sheet;
        let overlaps = same_sheet && area.0 <= r2 && area.2 >= r1 && area.1 <= c2 && area.3 >= c1;
        // Moving along the cells' own line closes the gap they leave
        let closes_gap = same_sheet && if down { column == c1 } else { row == r1 };
        if overlaps
            || (closes_gap
                && if down {
                    row == r2 + 1
                } else {
                    column == c2 + 1
                })
        {
            return Err(overlap_error());
        }
        // The shift can carry the cut cells along, but must not tear them
        let size = if down { height } else { width };
        let mut moved = source;
        if same_sheet {
            let (in_line, past) = if down {
                (c1 <= area.3 && c2 >= area.1, r2 >= row)
            } else {
                (r1 <= area.2 && r2 >= area.0, c2 >= column)
            };
            if in_line && past {
                let whole = if down {
                    c1 >= area.1 && c2 <= area.3 && r1 >= row
                } else {
                    r1 >= area.0 && r2 <= area.2 && c1 >= column
                };
                if !whole {
                    return Err(overlap_error());
                }
                moved = if down {
                    (r1 + size, c1, r2 + size, c2)
                } else {
                    (r1, c1 + size, r2, c2 + size)
                };
            }
        }
        self.shift_cells_step(sheet, area, down, true)?;
        self.paste_cut_from(source_sheet, moved, (sheet, row, column))?;
        let mut landed = area;
        if closes_gap {
            self.shift_cells_step(source_sheet, moved, down, false)?;
            let gap_is_before = if down {
                moved.0 < row
            } else {
                moved.1 < column
            };
            if gap_is_before {
                landed = if down {
                    (area.0 - size, area.1, area.2 - size, area.3)
                } else {
                    (area.0, area.1 - size, area.2, area.3 - size)
                };
            }
        }
        self.set_selected_sheet(sheet)?;
        self.select_inserted(landed)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::InsertCopiedMode;
    use crate::expressions::types::Area;
    use crate::pivot::PivotArea;
    use crate::user_model::clipboard::Clipboard;
    use crate::UserModel;

    fn grid() -> UserModel<'static> {
        let mut model = UserModel::new_empty("book", "en", "UTC", "en").unwrap();
        for row in 1..=4 {
            for column in 1..=3 {
                let text = format!("{}{row}", ["a", "b", "c"][column as usize - 1]);
                model.set_user_input(0, row, column, &text).unwrap();
            }
        }
        model
    }

    fn area(first_row: i32, first_column: i32, last_row: i32, last_column: i32) -> PivotArea {
        PivotArea {
            first_row,
            first_column,
            last_row,
            last_column,
        }
    }

    fn text(model: &UserModel, row: i32, column: i32) -> String {
        model.get_formatted_cell_value(0, row, column).unwrap()
    }

    fn column_of(model: &UserModel, column: i32, rows: i32) -> Vec<String> {
        (1..=rows).map(|r| text(model, r, column)).collect()
    }

    fn row_of(model: &UserModel, row: i32, columns: i32) -> Vec<String> {
        (1..=columns).map(|c| text(model, row, c)).collect()
    }

    fn copy(model: &mut UserModel, range: [i32; 4]) -> Clipboard {
        model.set_selected_cell(range[0], range[1]).unwrap();
        model
            .set_selected_range(range[0], range[1], range[2], range[3])
            .unwrap();
        model.copy_to_clipboard().unwrap()
    }

    fn insert(
        model: &mut UserModel,
        clipboard: &Clipboard,
        at: (i32, i32),
        is_cut: bool,
        mode: InsertCopiedMode,
        count: i32,
    ) {
        model.set_selected_cell(at.0, at.1).unwrap();
        model
            .insert_copied_cells(0, clipboard.range, &clipboard.data, is_cut, mode, count)
            .unwrap();
    }

    #[test]
    fn blank_cells_push_only_their_own_columns() {
        let mut model = grid();
        model.insert_cells(0, area(2, 1, 3, 2), true).unwrap();
        assert_eq!(column_of(&model, 1, 6), ["a1", "", "", "a2", "a3", "a4"]);
        assert_eq!(column_of(&model, 2, 6), ["b1", "", "", "b2", "b3", "b4"]);
        assert_eq!(column_of(&model, 3, 4), ["c1", "c2", "c3", "c4"]);
        model.undo().unwrap();
        assert_eq!(column_of(&model, 1, 5), ["a1", "a2", "a3", "a4", ""]);
        model.redo().unwrap();
        assert_eq!(text(&model, 4, 1), "a2");
    }

    #[test]
    fn blank_cells_can_push_right() {
        let mut model = grid();
        model.insert_cells(0, area(2, 2, 2, 2), false).unwrap();
        assert_eq!(row_of(&model, 2, 4), ["a2", "", "b2", "c2"]);
        assert_eq!(row_of(&model, 1, 3), ["a1", "b1", "c1"]);
    }

    #[test]
    fn formulas_follow_the_cells_they_read() {
        let mut model = grid();
        model.set_user_input(0, 3, 5, "=A3").unwrap();
        model.set_user_input(0, 4, 5, "=B3").unwrap();
        model.set_user_input(0, 5, 5, "=COUNTA(A1:A4)").unwrap();
        model.set_user_input(0, 6, 5, "=COUNTA(A1:C2)").unwrap();
        model.insert_cells(0, area(2, 1, 2, 1), true).unwrap();
        assert_eq!(model.get_cell_content(0, 3, 5).unwrap(), "=A4");
        assert_eq!(model.get_cell_content(0, 4, 5).unwrap(), "=B3");
        assert_eq!(model.get_cell_content(0, 5, 5).unwrap(), "=COUNTA(A1:A5)");
        // A range wider than what moved stays as it is
        assert_eq!(model.get_cell_content(0, 6, 5).unwrap(), "=COUNTA(A1:C2)");
        model.undo().unwrap();
        assert_eq!(model.get_cell_content(0, 3, 5).unwrap(), "=A3");
        assert_eq!(model.get_cell_content(0, 5, 5).unwrap(), "=COUNTA(A1:A4)");
    }

    #[test]
    fn a_moved_formula_keeps_reading_what_it_read() {
        let mut model = grid();
        model.set_user_input(0, 4, 1, "=C1").unwrap();
        model.insert_cells(0, area(2, 1, 2, 1), true).unwrap();
        assert_eq!(model.get_cell_content(0, 5, 1).unwrap(), "=C1");
    }

    #[test]
    fn notes_and_rules_move_with_cells() {
        let mut model = grid();
        model.set_comment(0, 3, 1, "note").unwrap();
        let list = crate::data_validation::ValidationSettings {
            kind: crate::data_validation::ValidationKind::List,
            formula1: "x,y".to_string(),
            ..Default::default()
        };
        model
            .set_data_validation(0, area(2, 1, 4, 1), list)
            .unwrap();
        model.insert_cells(0, area(3, 1, 3, 1), true).unwrap();
        assert_eq!(model.get_comment(0, 4, 1).unwrap().unwrap().text, "note");
        assert!(model.get_data_validation(0, 2, 1).unwrap().is_some());
        assert_eq!(model.get_data_validation(0, 3, 1).unwrap(), None);
        assert!(model.get_data_validation(0, 5, 1).unwrap().is_some());
        model.undo().unwrap();
        assert_eq!(model.get_comment(0, 3, 1).unwrap().unwrap().text, "note");
        assert!(model.get_data_validation(0, 3, 1).unwrap().is_some());
    }

    #[test]
    fn insert_copied_shifts_then_pastes_as_one_undo() {
        let mut model = grid();
        let clipboard = copy(&mut model, [1, 1, 1, 2]);
        insert(
            &mut model,
            &clipboard,
            (3, 1),
            false,
            InsertCopiedMode::Down,
            0,
        );
        assert_eq!(column_of(&model, 1, 5), ["a1", "a2", "a1", "a3", "a4"]);
        assert_eq!(column_of(&model, 2, 5), ["b1", "b2", "b1", "b3", "b4"]);
        assert_eq!(column_of(&model, 3, 4), ["c1", "c2", "c3", "c4"]);
        assert_eq!(model.get_selected_view().range, [3, 1, 3, 2]);
        model.undo().unwrap();
        assert_eq!(column_of(&model, 1, 5), ["a1", "a2", "a3", "a4", ""]);
        model.redo().unwrap();
        assert_eq!(text(&model, 3, 1), "a1");
        assert_eq!(text(&model, 4, 1), "a3");
    }

    #[test]
    fn insert_copied_can_shift_right() {
        let mut model = grid();
        let clipboard = copy(&mut model, [1, 1, 2, 1]);
        insert(
            &mut model,
            &clipboard,
            (3, 2),
            false,
            InsertCopiedMode::Right,
            0,
        );
        assert_eq!(row_of(&model, 3, 4), ["a3", "a1", "b3", "c3"]);
        assert_eq!(row_of(&model, 4, 4), ["a4", "a2", "b4", "c4"]);
        assert_eq!(row_of(&model, 2, 3), ["a2", "b2", "c2"]);
    }

    #[test]
    fn whole_copied_rows_go_in_as_rows() {
        let mut model = grid();
        let clipboard = copy(&mut model, [1, 1, 1, crate::constants::LAST_COLUMN]);
        insert(
            &mut model,
            &clipboard,
            (3, 2),
            false,
            InsertCopiedMode::Rows,
            1,
        );
        assert_eq!(row_of(&model, 3, 3), ["a1", "b1", "c1"]);
        assert_eq!(row_of(&model, 4, 3), ["a3", "b3", "c3"]);
        model.undo().unwrap();
        assert_eq!(row_of(&model, 3, 3), ["a3", "b3", "c3"]);
    }

    #[test]
    fn whole_copied_columns_go_in_as_columns() {
        let mut model = grid();
        let clipboard = copy(&mut model, [1, 3, crate::constants::LAST_ROW, 3]);
        insert(
            &mut model,
            &clipboard,
            (2, 1),
            false,
            InsertCopiedMode::Columns,
            1,
        );
        assert_eq!(row_of(&model, 1, 4), ["c1", "a1", "b1", "c1"]);
    }

    #[test]
    fn cut_along_a_column_closes_the_gap() {
        let mut model = grid();
        let clipboard = copy(&mut model, [1, 1, 1, 1]);
        insert(
            &mut model,
            &clipboard,
            (4, 1),
            true,
            InsertCopiedMode::Down,
            0,
        );
        assert_eq!(column_of(&model, 1, 5), ["a2", "a3", "a1", "a4", ""]);
        assert_eq!(model.get_selected_view().range, [3, 1, 3, 1]);
        assert_eq!(column_of(&model, 2, 4), ["b1", "b2", "b3", "b4"]);
        model.undo().unwrap();
        assert_eq!(column_of(&model, 1, 5), ["a1", "a2", "a3", "a4", ""]);
        model.redo().unwrap();
        assert_eq!(column_of(&model, 1, 4), ["a2", "a3", "a1", "a4"]);
    }

    #[test]
    fn cut_upwards_closes_the_gap_too() {
        let mut model = grid();
        let clipboard = copy(&mut model, [4, 1, 4, 1]);
        insert(
            &mut model,
            &clipboard,
            (2, 1),
            true,
            InsertCopiedMode::Down,
            0,
        );
        assert_eq!(column_of(&model, 1, 5), ["a1", "a4", "a2", "a3", ""]);
    }

    #[test]
    fn cut_along_a_row_closes_the_gap() {
        let mut model = grid();
        let clipboard = copy(&mut model, [2, 3, 2, 3]);
        insert(
            &mut model,
            &clipboard,
            (2, 1),
            true,
            InsertCopiedMode::Right,
            0,
        );
        assert_eq!(row_of(&model, 2, 4), ["c2", "a2", "b2", ""]);
    }

    #[test]
    fn cut_elsewhere_leaves_the_source_empty() {
        let mut model = grid();
        let clipboard = copy(&mut model, [1, 1, 1, 1]);
        insert(
            &mut model,
            &clipboard,
            (2, 3),
            true,
            InsertCopiedMode::Down,
            0,
        );
        assert_eq!(column_of(&model, 3, 5), ["c1", "a1", "c2", "c3", "c4"]);
        assert_eq!(column_of(&model, 1, 2), ["", "a2"]);
    }

    #[test]
    fn cut_rows_are_moved() {
        let mut model = grid();
        let clipboard = copy(&mut model, [1, 1, 1, crate::constants::LAST_COLUMN]);
        insert(
            &mut model,
            &clipboard,
            (4, 1),
            true,
            InsertCopiedMode::Rows,
            1,
        );
        assert_eq!(column_of(&model, 1, 4), ["a2", "a3", "a1", "a4"]);
    }

    #[test]
    fn inserting_cut_cells_on_themselves_is_refused_and_changes_nothing() {
        let mut model = grid();
        let clipboard = copy(&mut model, [1, 1, 2, 1]);
        model.set_selected_cell(2, 1).unwrap();
        assert!(model
            .insert_copied_cells(
                0,
                clipboard.range,
                &clipboard.data,
                true,
                InsertCopiedMode::Down,
                0
            )
            .is_err());
        assert_eq!(column_of(&model, 1, 4), ["a1", "a2", "a3", "a4"]);
    }

    #[test]
    fn shifting_that_would_break_something_is_refused() {
        let mut model = grid();
        model
            .merge_cells(&Area {
                sheet: 0,
                row: 6,
                column: 1,
                width: 2,
                height: 1,
            })
            .unwrap();
        assert!(model.insert_cells(0, area(1, 1, 1, 1), true).is_err());
        assert_eq!(text(&model, 3, 1), "a3");

        let mut model = grid();
        model.create_table(0, area(1, 1, 4, 3), true, None).unwrap();
        assert!(model.insert_cells(0, area(2, 1, 2, 1), true).is_err());

        let mut model = grid();
        model
            .set_user_input(0, crate::constants::LAST_ROW, 1, "bottom")
            .unwrap();
        assert!(model.insert_cells(0, area(2, 1, 2, 1), true).is_err());
    }
}
