use crate::{
    constants::LAST_COLUMN,
    pivot::PivotArea,
    protection::{
        add_range, remove_range, CellProtection, CellRange, ProtectionPassword,
        ProtectionPermissions, SheetProtection, SheetProtectionInfo, PROTECTED_SHEET_ERROR,
    },
    user_model::history::{Diff, DiffType, History, QueueDiffs},
};

use super::common::UserModel;

/// What a change needs from the protection of its sheet.
#[derive(Clone, Copy)]
pub(crate) enum Permission {
    /// Not possible while the sheet is protected.
    Never,
    FormatCells,
    FormatColumns,
    FormatRows,
    InsertColumns,
    InsertRows,
    InsertHyperlinks,
    DeleteColumns,
    DeleteRows,
    Sort,
    AutoFilter,
    PivotTables,
    Objects,
}

fn refused<T>() -> Result<T, String> {
    Err(PROTECTED_SHEET_ERROR.to_string())
}

impl UserModel<'_> {
    /// Fails if the protection of the sheet doesn't allow `permission`.
    pub(crate) fn guard(&self, sheet: u32, permission: Permission) -> Result<(), String> {
        let Some(protection) = self.model.active_protection(sheet) else {
            return Ok(());
        };
        let allowed = &protection.permissions;
        let granted = match permission {
            Permission::Never => false,
            Permission::FormatCells => allowed.format_cells,
            Permission::FormatColumns => allowed.format_columns,
            Permission::FormatRows => allowed.format_rows,
            Permission::InsertColumns => allowed.insert_columns,
            Permission::InsertRows => allowed.insert_rows,
            Permission::InsertHyperlinks => allowed.insert_hyperlinks,
            Permission::DeleteColumns => allowed.delete_columns,
            Permission::DeleteRows => allowed.delete_rows,
            Permission::Sort => allowed.sort,
            Permission::AutoFilter => allowed.auto_filter,
            Permission::PivotTables => allowed.pivot_tables,
            Permission::Objects => allowed.objects,
        };
        if granted {
            Ok(())
        } else {
            refused()
        }
    }

    /// Fails if any cell of `area` is locked on a protected sheet.
    pub(crate) fn guard_cells(&self, sheet: u32, area: CellRange) -> Result<(), String> {
        if self.model.is_area_editable(sheet, area) {
            Ok(())
        } else {
            refused()
        }
    }

    pub(crate) fn guard_table(&self, name: &str, permission: Permission) -> Result<(), String> {
        match self.model.filter_scope(name) {
            Ok(scope) => self.guard(scope.sheet, permission),
            Err(_) => Ok(()),
        }
    }

    pub(crate) fn guard_pivot(&self, index: u32, permission: Permission) -> Result<(), String> {
        let sheet_id = self
            .model
            .workbook
            .pivot_tables
            .get(index as usize)
            .map(|pivot| pivot.sheet_id);
        match sheet_id.and_then(|id| self.model.pivot_sheet_index(id)) {
            Some(sheet) => self.guard(sheet, permission),
            None => Ok(()),
        }
    }

    pub(crate) fn guard_slicer(&self, name: &str, permission: Permission) -> Result<(), String> {
        let sheet_id = self
            .model
            .workbook
            .slicers
            .iter()
            .find(|slicer| slicer.name == name)
            .map(|slicer| slicer.sheet_id);
        match sheet_id.and_then(|id| self.model.pivot_sheet_index(id)) {
            Some(sheet) => self.guard(sheet, permission),
            None => Ok(()),
        }
    }

    /// Whether rows or columns can be deleted: every cell in them is unlocked.
    pub(crate) fn guard_rows(&self, sheet: u32, row: i32, count: i32) -> Result<(), String> {
        self.guard_cells(sheet, (row, 1, row + count - 1, LAST_COLUMN))
    }

    pub(crate) fn guard_columns(&self, sheet: u32, column: i32, count: i32) -> Result<(), String> {
        self.guard_cells(
            sheet,
            (1, column, crate::constants::LAST_ROW, column + count - 1),
        )
    }

    /// Whether the protection of the sheet lets the selection land on a cell.
    pub(crate) fn can_select_cell(&self, sheet: u32, row: i32, column: i32) -> bool {
        let Some(protection) = self.model.active_protection(sheet) else {
            return true;
        };
        if self.model.get_cell_protection(sheet, row, column).locked {
            protection.permissions.select_locked_cells
        } else {
            protection.permissions.select_unlocked_cells
        }
    }

    /// Whether a sheet is protected, with a password or not, and what it allows.
    pub fn get_sheet_protection(&self, sheet: u32) -> SheetProtectionInfo {
        match self.model.sheet_protection(sheet) {
            Some(protection) => SheetProtectionInfo {
                is_protected: protection.is_protected,
                has_password: protection.is_protected && protection.password.is_some(),
                permissions: protection.permissions.clone(),
            },
            None => SheetProtectionInfo {
                is_protected: false,
                has_password: false,
                permissions: ProtectionPermissions::default(),
            },
        }
    }

    /// Whether a cell is locked and whether its formula is hidden.
    pub fn get_cell_protection(&self, sheet: u32, row: i32, column: i32) -> CellProtection {
        self.model.get_cell_protection(sheet, row, column)
    }

    /// Whether the protection of the sheet lets the content of a cell change.
    pub fn is_cell_editable(&self, sheet: u32, row: i32, column: i32) -> bool {
        self.model
            .is_area_editable(sheet, (row, column, row, column))
    }

    /// Tells whoever follows the workbook's changes, without making it
    /// something that can be undone.
    fn record_protection_change(
        &mut self,
        sheet_id: u32,
        old_value: Option<SheetProtection>,
        sheet: u32,
    ) {
        let new_value = self.model.sheet_protection(sheet).cloned();
        self.send_queue.push(QueueDiffs {
            r#type: DiffType::Redo,
            list: vec![Diff::SetCellProtection {
                sheet_id,
                old_value: Box::new(old_value),
                new_value: Box::new(new_value),
            }],
        });
    }

    /// Protects a sheet. As in Excel this can't be undone, and what could be
    /// undone before can't be any more: undo must not get around it.
    pub fn protect_sheet(
        &mut self,
        sheet: u32,
        password: Option<&str>,
        permissions: ProtectionPermissions,
    ) -> Result<(), String> {
        let sheet_id = self.model.workbook.worksheet(sheet)?.sheet_id;
        if self.model.is_sheet_protected(sheet) {
            return Err("This sheet is already protected".to_string());
        }
        let password = password
            .filter(|password| !password.is_empty())
            .map(ProtectionPassword::new)
            .transpose()?;
        let mut protection = self
            .model
            .sheet_protection(sheet)
            .cloned()
            .unwrap_or_default();
        protection.sheet_id = sheet_id;
        protection.is_protected = true;
        protection.password = password;
        protection.permissions = permissions;
        let old_value = self.model.sheet_protection(sheet).cloned();
        self.model.set_sheet_protection(sheet_id, Some(protection));
        self.record_protection_change(sheet_id, old_value, sheet);
        self.history = History::default();
        Ok(())
    }

    /// Takes the protection off a sheet, if `password` is its password.
    pub fn unprotect_sheet(&mut self, sheet: u32, password: Option<&str>) -> Result<(), String> {
        let sheet_id = self.model.workbook.worksheet(sheet)?.sheet_id;
        let Some(protection) = self.model.active_protection(sheet).cloned() else {
            return Ok(());
        };
        if let Some(expected) = &protection.password {
            if !expected.matches(password.unwrap_or(""))? {
                return Err("The password is not correct".to_string());
            }
        }
        let old_value = Some(protection.clone());
        self.model.set_sheet_protection(
            sheet_id,
            Some(SheetProtection {
                is_protected: false,
                password: None,
                ..protection
            }),
        );
        self.record_protection_change(sheet_id, old_value, sheet);
        Ok(())
    }

    /// Sets whether cells are locked and whether their formula is hidden.
    /// `None` leaves that as it is.
    pub fn set_cell_protection(
        &mut self,
        sheet: u32,
        area: PivotArea,
        locked: Option<bool>,
        hidden: Option<bool>,
    ) -> Result<(), String> {
        self.guard(sheet, Permission::Never)?;
        let sheet_id = self.model.workbook.worksheet(sheet)?.sheet_id;
        let old_value = self.model.sheet_protection(sheet).cloned();
        let mut protection = old_value.clone().unwrap_or(SheetProtection {
            sheet_id,
            ..Default::default()
        });
        let range = (
            area.first_row,
            area.first_column,
            area.last_row,
            area.last_column,
        );
        match locked {
            Some(true) => remove_range(&mut protection.unlocked, range),
            Some(false) => add_range(&mut protection.unlocked, range),
            None => {}
        }
        match hidden {
            Some(true) => add_range(&mut protection.hidden, range),
            Some(false) => remove_range(&mut protection.hidden, range),
            None => {}
        }
        self.model.set_sheet_protection(sheet_id, Some(protection));
        let new_value = self.model.sheet_protection(sheet).cloned();
        if new_value != old_value {
            self.push_diff_list(vec![Diff::SetCellProtection {
                sheet_id,
                old_value: Box::new(old_value),
                new_value: Box::new(new_value),
            }]);
        }
        Ok(())
    }

    /// Makes pasted cells locked or unlocked like the cells they came from.
    /// A paste into a protected sheet leaves its cells as they are.
    pub(crate) fn paste_cell_protection(
        &mut self,
        source_sheet: u32,
        source: CellRange,
        sheet: u32,
        target: (i32, i32),
        is_cut: bool,
    ) -> Result<Vec<Diff>, String> {
        let source_protection = self.model.sheet_protection(source_sheet).cloned();
        let target_protection = self.model.sheet_protection(sheet).cloned();
        if (source_protection.is_none() && target_protection.is_none())
            || self.model.is_sheet_protected(sheet)
        {
            return Ok(Vec::new());
        }
        let (r1, c1, r2, c2) = source;
        let delta_row = target.0 - r1;
        let delta_column = target.1 - c1;
        let translated = |ranges: &[CellRange]| -> Vec<CellRange> {
            ranges
                .iter()
                .filter_map(|&(a, b, c, d)| {
                    let (first_row, first_column) = (a.max(r1), b.max(c1));
                    let (last_row, last_column) = (c.min(r2), d.min(c2));
                    (first_row <= last_row && first_column <= last_column).then_some((
                        first_row + delta_row,
                        first_column + delta_column,
                        last_row + delta_row,
                        last_column + delta_column,
                    ))
                })
                .collect()
        };
        let (unlocked, hidden) = match &source_protection {
            Some(protection) => (
                translated(&protection.unlocked),
                translated(&protection.hidden),
            ),
            None => (Vec::new(), Vec::new()),
        };
        let mut diffs = Vec::new();
        if is_cut && !self.model.is_sheet_protected(source_sheet) {
            if let Some(mut protection) = source_protection.clone() {
                remove_range(&mut protection.unlocked, source);
                remove_range(&mut protection.hidden, source);
                let sheet_id = protection.sheet_id;
                self.model.set_sheet_protection(sheet_id, Some(protection));
                diffs.push(Diff::SetCellProtection {
                    sheet_id,
                    old_value: Box::new(source_protection),
                    new_value: Box::new(self.model.sheet_protection(source_sheet).cloned()),
                });
            }
        }
        let sheet_id = self.model.workbook.worksheet(sheet)?.sheet_id;
        let old_value = self.model.sheet_protection(sheet).cloned();
        let mut protection = old_value.clone().unwrap_or(SheetProtection {
            sheet_id,
            ..Default::default()
        });
        let area = (target.0, target.1, r2 + delta_row, c2 + delta_column);
        remove_range(&mut protection.unlocked, area);
        remove_range(&mut protection.hidden, area);
        for range in unlocked {
            add_range(&mut protection.unlocked, range);
        }
        for range in hidden {
            add_range(&mut protection.hidden, range);
        }
        self.model.set_sheet_protection(sheet_id, Some(protection));
        let new_value = self.model.sheet_protection(sheet).cloned();
        if new_value != old_value {
            diffs.push(Diff::SetCellProtection {
                sheet_id,
                old_value: Box::new(old_value),
                new_value: Box::new(new_value),
            });
        }
        Ok(diffs)
    }

    /// A duplicated sheet is protected like the original.
    pub(crate) fn copy_sheet_protection(
        &mut self,
        source: u32,
        duplicate: u32,
    ) -> Result<(), String> {
        let sheet_id = self.model.workbook.worksheet(duplicate)?.sheet_id;
        let copy = self
            .model
            .sheet_protection(source)
            .cloned()
            .map(|value| SheetProtection { sheet_id, ..value });
        self.model.set_sheet_protection(sheet_id, copy);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::{expressions::types::Area, protection::legacy_password_hash, UserModel};

    fn new_empty_user_model() -> UserModel<'static> {
        UserModel::new_empty("model", "en", "UTC", "en").unwrap()
    }

    fn area(first_row: i32, first_column: i32, last_row: i32, last_column: i32) -> PivotArea {
        PivotArea {
            first_row,
            first_column,
            last_row,
            last_column,
        }
    }

    fn cells(row: i32, column: i32, height: i32, width: i32) -> Area {
        Area {
            sheet: 0,
            row,
            column,
            width,
            height,
        }
    }

    fn protected() -> UserModel<'static> {
        let mut model = new_empty_user_model();
        model.set_user_input(0, 1, 1, "locked").unwrap();
        model
            .set_cell_protection(0, area(2, 1, 3, 1), Some(false), None)
            .unwrap();
        model
            .protect_sheet(0, None, ProtectionPermissions::default())
            .unwrap();
        model
    }

    fn refused_with<T: std::fmt::Debug>(result: Result<T, String>) {
        assert_eq!(result.unwrap_err(), PROTECTED_SHEET_ERROR);
    }

    fn allowing(change: impl Fn(&mut ProtectionPermissions)) -> UserModel<'static> {
        let mut model = new_empty_user_model();
        let mut permissions = ProtectionPermissions::default();
        change(&mut permissions);
        model.protect_sheet(0, None, permissions).unwrap();
        model
    }

    #[test]
    fn only_unlocked_cells_can_be_edited() {
        let mut model = protected();
        refused_with(model.set_user_input(0, 1, 1, "changed"));
        assert_eq!(model.get_cell_content(0, 1, 1).unwrap(), "locked");
        model.set_user_input(0, 2, 1, "fine").unwrap();
        assert_eq!(model.get_cell_content(0, 2, 1).unwrap(), "fine");
        assert!(!model.is_cell_editable(0, 1, 1));
        assert!(model.is_cell_editable(0, 3, 1));
    }

    #[test]
    fn locks_only_matter_on_a_protected_sheet() {
        let mut model = new_empty_user_model();
        model.set_user_input(0, 1, 1, "free").unwrap();
        assert!(model.get_cell_protection(0, 1, 1).locked);
        assert!(model.is_cell_editable(0, 1, 1));
    }

    #[test]
    fn clearing_needs_every_cell_unlocked() {
        let mut model = protected();
        refused_with(model.range_clear_contents(&cells(1, 1, 2, 1)));
        assert_eq!(model.get_cell_content(0, 1, 1).unwrap(), "locked");
        model.range_clear_contents(&cells(2, 1, 2, 1)).unwrap();
    }

    #[test]
    fn formatting_needs_its_permission() {
        let mut model = protected();
        refused_with(model.update_range_style(&cells(2, 1, 1, 1), "font.b", "true"));
        let mut allowed = allowing(|p| p.format_cells = true);
        allowed
            .update_range_style(&cells(1, 1, 1, 1), "font.b", "true")
            .unwrap();
    }

    #[test]
    fn rows_and_columns_need_their_permissions() {
        let mut model = protected();
        refused_with(model.insert_rows(0, 1, 1));
        refused_with(model.insert_columns(0, 1, 1));
        refused_with(model.delete_rows(0, 1, 1));
        refused_with(model.delete_columns(0, 1, 1));
        refused_with(model.set_columns_width(0, 1, 1, 200.0));
        refused_with(model.set_rows_height(0, 1, 1, 50.0));
        refused_with(model.set_rows_hidden(0, 1, 1, true));

        let mut allowed = allowing(|p| {
            p.insert_rows = true;
            p.format_rows = true;
        });
        allowed.insert_rows(0, 1, 1).unwrap();
        allowed.set_rows_height(0, 1, 1, 50.0).unwrap();
        refused_with(allowed.insert_columns(0, 1, 1));
    }

    #[test]
    fn deleting_rows_needs_them_fully_unlocked() {
        let mut model = new_empty_user_model();
        model
            .set_cell_protection(0, area(3, 1, 3, LAST_COLUMN), Some(false), None)
            .unwrap();
        let permissions = ProtectionPermissions {
            delete_rows: true,
            ..Default::default()
        };
        model.protect_sheet(0, None, permissions).unwrap();
        refused_with(model.delete_rows(0, 2, 1));
        model.delete_rows(0, 3, 1).unwrap();
    }

    #[test]
    fn structure_changes_are_refused() {
        let mut model = protected();
        refused_with(model.merge_cells(&cells(2, 1, 2, 1)));
        refused_with(model.insert_cells(0, area(2, 1, 2, 1), true));
        refused_with(model.create_table(0, area(1, 1, 3, 2), true, None));
        refused_with(model.toggle_sheet_filter(0, area(1, 1, 3, 1)));
        refused_with(model.clear_conditional_formatting(0, None));
        refused_with(model.set_comment(0, 2, 1, "note"));
    }

    #[test]
    fn protecting_counts_as_a_change_to_save() {
        let mut model = new_empty_user_model();
        let empty = model.flush_send_queue();
        model
            .protect_sheet(0, None, ProtectionPermissions::default())
            .unwrap();
        assert_ne!(model.flush_send_queue(), empty);
        model.unprotect_sheet(0, None).unwrap();
        assert_ne!(model.flush_send_queue(), empty);
    }

    #[test]
    fn protecting_clears_undo() {
        let mut model = new_empty_user_model();
        model.set_user_input(0, 1, 1, "before").unwrap();
        assert!(model.can_undo());
        model
            .protect_sheet(0, None, ProtectionPermissions::default())
            .unwrap();
        assert!(!model.can_undo());
    }

    #[test]
    fn the_password_is_needed_to_unprotect() {
        let mut model = new_empty_user_model();
        model
            .protect_sheet(0, Some("sécret"), ProtectionPermissions::default())
            .unwrap();
        assert!(model.get_sheet_protection(0).has_password);
        assert!(model.unprotect_sheet(0, Some("wrong")).is_err());
        assert!(model.unprotect_sheet(0, None).is_err());
        assert!(model.get_sheet_protection(0).is_protected);
        model.unprotect_sheet(0, Some("sécret")).unwrap();
        assert!(!model.get_sheet_protection(0).is_protected);
        model.set_user_input(0, 1, 1, "free again").unwrap();
    }

    #[test]
    fn the_older_password_hash_is_understood() {
        assert_eq!(legacy_password_hash("test"), 0xCBEB);
        let password = ProtectionPassword {
            legacy_hash: Some("CBEB".to_string()),
            ..Default::default()
        };
        assert!(password.matches("test").unwrap());
        assert!(!password.matches("nope").unwrap());
    }

    #[test]
    fn a_hidden_formula_stops_showing_when_protected() {
        let mut model = new_empty_user_model();
        model.set_user_input(0, 1, 1, "=1+1").unwrap();
        model
            .set_cell_protection(0, area(1, 1, 1, 1), None, Some(true))
            .unwrap();
        assert_eq!(model.get_cell_content(0, 1, 1).unwrap(), "=1+1");
        model
            .protect_sheet(0, None, ProtectionPermissions::default())
            .unwrap();
        assert_eq!(model.get_cell_content(0, 1, 1).unwrap(), "");
        assert_eq!(model.get_formatted_cell_value(0, 1, 1).unwrap(), "2");
    }

    #[test]
    fn locked_cells_can_be_kept_from_being_selected() {
        let mut model = new_empty_user_model();
        model
            .set_cell_protection(0, area(1, 3, 1, 3), Some(false), None)
            .unwrap();
        let permissions = ProtectionPermissions {
            select_locked_cells: false,
            ..Default::default()
        };
        model.protect_sheet(0, None, permissions).unwrap();
        model.set_selected_cell(5, 5).unwrap();
        assert_eq!(model.get_selected_cell().1, 1);
        model.set_selected_cell(1, 3).unwrap();
        assert_eq!(model.get_selected_cell().2, 3);
    }

    #[test]
    fn unlocked_cells_stay_with_their_rows() {
        let mut model = new_empty_user_model();
        model
            .set_cell_protection(0, area(5, 1, 5, 1), Some(false), None)
            .unwrap();
        model.insert_rows(0, 2, 2).unwrap();
        assert!(model.get_cell_protection(0, 5, 1).locked);
        assert!(!model.get_cell_protection(0, 7, 1).locked);
        model.undo().unwrap();
        assert!(!model.get_cell_protection(0, 5, 1).locked);
    }

    #[test]
    fn changing_a_lock_can_be_undone() {
        let mut model = new_empty_user_model();
        model
            .set_cell_protection(0, area(1, 1, 2, 2), Some(false), None)
            .unwrap();
        assert!(!model.get_cell_protection(0, 2, 2).locked);
        model.undo().unwrap();
        assert!(model.get_cell_protection(0, 2, 2).locked);
        model.redo().unwrap();
        assert!(!model.get_cell_protection(0, 2, 2).locked);
    }

    #[test]
    fn protection_survives_saving() {
        let mut model = protected();
        model.set_user_input(0, 2, 1, "kept").unwrap();
        let bytes = model.to_bytes();
        let mut reopened = UserModel::from_bytes(&bytes, "en").unwrap();
        assert!(reopened.get_sheet_protection(0).is_protected);
        refused_with(reopened.set_user_input(0, 1, 1, "changed"));
        reopened.set_user_input(0, 3, 1, "fine").unwrap();
    }

    #[test]
    fn a_duplicated_sheet_is_protected_too() {
        let mut model = protected();
        model.duplicate_sheet(0).unwrap();
        assert!(model.get_sheet_protection(1).is_protected);
        refused_with(model.set_user_input(1, 1, 1, "changed"));
        model.set_user_input(1, 2, 1, "fine").unwrap();
    }
}
