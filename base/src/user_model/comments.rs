//! Cell notes: what Excel calls a note, and the xlsx format calls a comment.

use crate::user_model::protection::Permission;
use serde::Serialize;

use crate::expressions::types::Area;
use crate::expressions::utils::{is_valid_column_number, is_valid_row};
use crate::expressions::utils::{number_to_column, parse_reference_a1};
use crate::pivot::PivotArea;
use crate::types::Comment;

use super::{common::UserModel, history::Diff};

/// A note and the cell it is on.
#[derive(Serialize, Debug, PartialEq, Eq, Clone)]
pub struct CellComment {
    /// Row of the cell
    pub row: i32,
    /// Column of the cell
    pub column: i32,
    /// The text of the note
    pub text: String,
    /// Who wrote it, or empty if that is not known
    pub author: String,
}

fn comment_cell(comment: &Comment) -> Option<(i32, i32)> {
    let cell = parse_reference_a1(&comment.cell_ref)?;
    Some((cell.row, cell.column))
}

fn cell_name(row: i32, column: i32) -> Result<String, String> {
    if !is_valid_row(row) {
        return Err(format!("Invalid row: '{row}'"));
    }
    if !is_valid_column_number(column) {
        return Err(format!("Invalid column: '{column}'"));
    }
    let name = number_to_column(column).ok_or_else(|| format!("Invalid column: '{column}'"))?;
    Ok(format!("{name}{row}"))
}

impl UserModel<'_> {
    /// Returns the notes of a sheet, sorted by row and then column.
    pub fn get_comments(&self, sheet: u32) -> Result<Vec<CellComment>, String> {
        let worksheet = self.model.workbook.worksheet(sheet)?;
        let mut comments: Vec<CellComment> = worksheet
            .comments
            .iter()
            .filter_map(|comment| {
                let (row, column) = comment_cell(comment)?;
                Some(CellComment {
                    row,
                    column,
                    text: comment.text.clone(),
                    author: comment.author_name.clone(),
                })
            })
            .collect();
        comments.sort_by_key(|comment| (comment.row, comment.column));
        Ok(comments)
    }

    /// Returns the note of a cell. A merged cell's note is on its first cell.
    pub fn get_comment(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<CellComment>, String> {
        let (row, column) = self.merge_anchor(sheet, row, column)?;
        Ok(self
            .find_comment(sheet, row, column)?
            .map(|comment| CellComment {
                row,
                column,
                text: comment.text,
                author: comment.author_name,
            }))
    }

    /// Writes the note of a cell. Editing a note keeps its author.
    pub fn set_comment(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        text: &str,
    ) -> Result<(), String> {
        self.guard(sheet, Permission::Objects)?;
        let (row, column) = self.merge_anchor(sheet, row, column)?;
        let old_value = self.find_comment(sheet, row, column)?;
        let new_value = Comment {
            text: text.to_string(),
            author_name: old_value
                .as_ref()
                .map(|comment| comment.author_name.clone())
                .unwrap_or_default(),
            author_id: old_value
                .as_ref()
                .and_then(|comment| comment.author_id.clone()),
            cell_ref: cell_name(row, column)?,
        };
        if old_value.as_ref().map(|comment| comment.text.as_str()) == Some(text) {
            return Ok(());
        }
        self.put_comment(sheet, row, column, Some(new_value.clone()))?;
        self.push_diff_list(vec![Diff::SetCellComment {
            sheet,
            row,
            column,
            old_value: Box::new(old_value),
            new_value: Box::new(Some(new_value)),
        }]);
        Ok(())
    }

    /// Removes the note of a cell. It is not an error if there is none.
    pub fn delete_comment(&mut self, sheet: u32, row: i32, column: i32) -> Result<(), String> {
        self.guard(sheet, Permission::Objects)?;
        let (row, column) = self.merge_anchor(sheet, row, column)?;
        let Some(old_value) = self.find_comment(sheet, row, column)? else {
            return Ok(());
        };
        self.put_comment(sheet, row, column, None)?;
        self.push_diff_list(vec![Diff::SetCellComment {
            sheet,
            row,
            column,
            old_value: Box::new(Some(old_value)),
            new_value: Box::new(None),
        }]);
        Ok(())
    }

    pub(super) fn find_comment(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> Result<Option<Comment>, String> {
        Ok(self
            .model
            .workbook
            .worksheet(sheet)?
            .comments
            .iter()
            .find(|comment| comment_cell(comment) == Some((row, column)))
            .cloned())
    }

    /// Replaces whatever note the cell has with `comment`.
    pub(super) fn put_comment(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        comment: Option<Comment>,
    ) -> Result<(), String> {
        let cell_ref = cell_name(row, column)?;
        let worksheet = self.model.workbook.worksheet_mut(sheet)?;
        worksheet
            .comments
            .retain(|existing| comment_cell(existing) != Some((row, column)));
        if let Some(comment) = comment {
            worksheet.comments.push(Comment {
                cell_ref,
                ..comment
            });
        }
        Ok(())
    }

    /// Removes the notes in `range` and returns the steps to bring them back.
    pub(super) fn remove_range_comments(&mut self, range: &Area) -> Result<Vec<Diff>, String> {
        let inside = |row: i32, column: i32| {
            (range.row..range.row + range.height).contains(&row)
                && (range.column..range.column + range.width).contains(&column)
        };
        let worksheet = self.model.workbook.worksheet_mut(range.sheet)?;
        let mut diffs = Vec::new();
        worksheet.comments.retain(|comment| {
            let Some((row, column)) = comment_cell(comment).filter(|(r, c)| inside(*r, *c)) else {
                return true;
            };
            diffs.push(Diff::SetCellComment {
                sheet: range.sheet,
                row,
                column,
                old_value: Box::new(Some(comment.clone())),
                new_value: Box::new(None),
            });
            false
        });
        Ok(diffs)
    }

    /// Moves the notes inside `area` along with cells moved by a table edit.
    /// `destination` gives a cell's new place, or `None` if it is gone.
    pub(super) fn remap_comments(
        &mut self,
        sheet: u32,
        area: &PivotArea,
        destination: impl Fn(i32, i32) -> Option<(i32, i32)>,
    ) -> Result<(), String> {
        let worksheet = self.model.workbook.worksheet_mut(sheet)?;
        worksheet.comments.retain_mut(|comment| {
            let Some((row, column)) = comment_cell(comment) else {
                return true;
            };
            if !(area.first_row..=area.last_row).contains(&row)
                || !(area.first_column..=area.last_column).contains(&column)
            {
                return true;
            }
            let Some((new_row, new_column)) = destination(row, column) else {
                return false;
            };
            if let Ok(name) = cell_name(new_row, new_column) {
                comment.cell_ref = name;
            }
            true
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::expressions::types::Area;
    use crate::pivot::PivotArea;
    use crate::UserModel;

    fn model() -> UserModel<'static> {
        UserModel::new_empty("book", "en", "UTC", "en").unwrap()
    }

    fn places(model: &UserModel) -> Vec<(i32, i32, String)> {
        model
            .get_comments(0)
            .unwrap()
            .into_iter()
            .map(|c| (c.row, c.column, c.text))
            .collect()
    }

    fn note(row: i32, column: i32, text: &str) -> (i32, i32, String) {
        (row, column, text.to_string())
    }

    #[test]
    fn add_edit_delete_with_undo_and_redo() {
        let mut model = model();
        model.set_comment(0, 2, 3, "first").unwrap();
        assert_eq!(places(&model), [note(2, 3, "first")]);
        model.set_comment(0, 2, 3, "second").unwrap();
        assert_eq!(model.get_comment(0, 2, 3).unwrap().unwrap().text, "second");
        model.delete_comment(0, 2, 3).unwrap();
        assert_eq!(model.get_comment(0, 2, 3).unwrap(), None);

        model.undo().unwrap();
        assert_eq!(places(&model), [note(2, 3, "second")]);
        model.undo().unwrap();
        assert_eq!(places(&model), [note(2, 3, "first")]);
        model.undo().unwrap();
        assert!(places(&model).is_empty());
        model.redo().unwrap();
        model.redo().unwrap();
        model.redo().unwrap();
        assert!(places(&model).is_empty());
    }

    #[test]
    fn saving_the_same_text_adds_no_undo_step() {
        let mut model = model();
        model.set_comment(0, 1, 1, "same").unwrap();
        model.set_comment(0, 1, 1, "same").unwrap();
        model.undo().unwrap();
        assert!(places(&model).is_empty());
    }

    #[test]
    fn editing_keeps_the_author() {
        let mut model = model();
        model.set_comment(0, 1, 1, "first").unwrap();
        model.model.workbook.worksheets[0].comments[0].author_name = "Ann".to_string();
        model.set_comment(0, 1, 1, "changed").unwrap();
        let note = model.get_comment(0, 1, 1).unwrap().unwrap();
        assert_eq!(note.author, "Ann");
        assert_eq!(note.text, "changed");
    }

    #[test]
    fn a_note_on_a_merged_cell_is_on_its_first_cell() {
        let mut model = model();
        model
            .merge_cells(&Area {
                sheet: 0,
                row: 2,
                column: 2,
                width: 2,
                height: 2,
            })
            .unwrap();
        model.set_comment(0, 3, 3, "merged").unwrap();
        assert_eq!(places(&model), [note(2, 2, "merged")]);
        assert_eq!(model.get_comment(0, 3, 2).unwrap().unwrap().text, "merged");
    }

    #[test]
    fn invalid_cells_are_rejected() {
        let mut model = model();
        assert!(model.set_comment(0, 0, 1, "x").is_err());
        assert!(model.set_comment(9, 1, 1, "x").is_err());
    }

    #[test]
    fn notes_are_saved() {
        let mut model = model();
        model.set_comment(0, 4, 2, "kept").unwrap();
        let reopened = UserModel::from_bytes(&model.to_bytes(), "en").unwrap();
        assert_eq!(places(&reopened), [note(4, 2, "kept")]);
    }

    #[test]
    fn copy_keeps_the_note_and_cut_moves_it() {
        let mut model = model();
        model.set_user_input(0, 1, 1, "a").unwrap();
        model.set_comment(0, 1, 1, "note").unwrap();

        model.set_selected_cell(1, 1).unwrap();
        let clipboard = model.copy_to_clipboard().unwrap();
        model.set_selected_cell(3, 3).unwrap();
        model
            .paste_from_clipboard(0, (1, 1, 1, 1), &clipboard.data, false)
            .unwrap();
        assert_eq!(places(&model), [note(1, 1, "note"), note(3, 3, "note")]);
        model.undo().unwrap();
        assert_eq!(places(&model), [note(1, 1, "note")]);

        model.set_selected_cell(5, 5).unwrap();
        model
            .paste_from_clipboard(0, (1, 1, 1, 1), &clipboard.data, true)
            .unwrap();
        assert_eq!(places(&model), [note(5, 5, "note")]);
        model.undo().unwrap();
        assert_eq!(places(&model), [note(1, 1, "note")]);
    }

    #[test]
    fn clear_contents_keeps_notes_and_clear_all_removes_them() {
        let mut model = model();
        model.set_user_input(0, 2, 2, "x").unwrap();
        model.set_comment(0, 2, 2, "note").unwrap();
        let area = Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 3,
            height: 3,
        };
        model.range_clear_contents(&area).unwrap();
        assert_eq!(places(&model), [note(2, 2, "note")]);
        model.range_clear_all(&area).unwrap();
        assert!(places(&model).is_empty());
        model.undo().unwrap();
        assert_eq!(places(&model), [note(2, 2, "note")]);
    }

    fn table_model() -> UserModel<'static> {
        let mut model = model();
        for (row, (name, quantity)) in
            [("Item", "Qty"), ("Pear", "3"), ("Apple", "1"), ("Fig", "2")]
                .iter()
                .enumerate()
        {
            model.set_user_input(0, row as i32 + 1, 1, name).unwrap();
            model
                .set_user_input(0, row as i32 + 1, 2, quantity)
                .unwrap();
        }
        model
            .create_table(
                0,
                PivotArea {
                    first_row: 1,
                    first_column: 1,
                    last_row: 4,
                    last_column: 2,
                },
                true,
                None,
            )
            .unwrap();
        model
    }

    fn table_name(model: &UserModel) -> String {
        model.get_tables(0)[0].name.clone()
    }

    #[test]
    fn sorting_a_table_carries_its_notes() {
        let mut model = table_model();
        let name = table_name(&model);
        model.set_comment(0, 2, 1, "on pear").unwrap();
        model.set_comment(0, 8, 1, "outside").unwrap();
        model.sort_table(&name, "Qty", false).unwrap();
        assert_eq!(model.get_cell_content(0, 4, 1).unwrap(), "Pear");
        assert_eq!(
            places(&model),
            [note(4, 1, "on pear"), note(8, 1, "outside")]
        );
        model.undo().unwrap();
        assert_eq!(
            places(&model),
            [note(2, 1, "on pear"), note(8, 1, "outside")]
        );
    }

    #[test]
    fn deleting_a_table_row_or_column_moves_its_notes() {
        let mut model = table_model();
        let name = table_name(&model);
        model.set_comment(0, 3, 1, "gone").unwrap();
        model.set_comment(0, 4, 1, "moves up").unwrap();
        model.delete_table_row(&name, 3).unwrap();
        assert_eq!(places(&model), [note(3, 1, "moves up")]);
        model.undo().unwrap();
        assert_eq!(places(&model), [note(3, 1, "gone"), note(4, 1, "moves up")]);

        model.set_comment(0, 2, 2, "in B").unwrap();
        model.delete_table_column(&name, 1).unwrap();
        assert_eq!(places(&model), [note(2, 1, "in B")]);
    }
}
