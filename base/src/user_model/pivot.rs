//! Creating and editing PivotTables with undo. See [`crate::pivot`].

use crate::user_model::protection::Permission;
use std::collections::{BTreeMap, BTreeSet};

use bitcode::{Decode, Encode};

use crate::constants::{LAST_COLUMN, LAST_ROW};
use crate::expressions::token::get_error_by_english_name;
use crate::pivot::{
    apply_pivot_look, PivotArea, PivotCellInfo, PivotCellKind, PivotFieldInfo, PivotGrid,
    PivotItem, PivotMarker, PivotTable, PivotTableEntry, PIVOT_OVERWRITE_ERROR,
};
use crate::types::{Cell, HorizontalAlignment, Style};
use crate::user_model::common::UserModel;
use crate::user_model::history::Diff;

const CHARACTER_WIDTH: f64 = 7.5;
const CELL_PADDING: f64 = 16.0;

/// A cell changed by a pivot: both states are kept whole, styles included.
#[derive(Clone, Encode, Decode)]
pub(crate) struct PivotCellDiff {
    pub(crate) row: i32,
    pub(crate) column: i32,
    pub(crate) old_value: Option<Cell>,
    pub(crate) new_value: Option<Cell>,
}

impl UserModel<'_> {
    /// Every pivot whose output sheet still exists.
    pub fn get_pivot_tables(&self) -> Vec<PivotTableEntry> {
        self.model
            .workbook
            .pivot_tables
            .iter()
            .enumerate()
            .filter_map(|(index, pivot)| {
                Some(PivotTableEntry {
                    index: index as u32,
                    sheet: self.model.pivot_sheet_index(pivot.sheet_id)?,
                    source_sheet: self.model.pivot_sheet_index(pivot.source_sheet_id),
                    pivot: pivot.clone(),
                })
            })
            .collect()
    }

    /// The index of the pivot covering a cell. A pivot without fields covers its anchor.
    pub fn get_pivot_table_at(&self, sheet: u32, row: i32, column: i32) -> Option<u32> {
        let sheet_id = self.model.workbook.worksheets.get(sheet as usize)?.sheet_id;
        self.model
            .workbook
            .pivot_tables
            .iter()
            .position(|pivot| {
                pivot.sheet_id == sheet_id
                    && match &pivot.output {
                        Some(output) => output.contains(row, column),
                        None => pivot.row == row && pivot.column == column,
                    }
            })
            .map(|index| index as u32)
    }

    /// What a cell of a pivot is, for its right-click menu.
    pub fn get_pivot_cell_info(&self, sheet: u32, row: i32, column: i32) -> Option<PivotCellInfo> {
        self.model.pivot_cell_info(sheet, row, column)
    }

    /// The filter arrows and +/- buttons of a sheet.
    pub fn get_pivot_markers(&self, sheet: u32) -> Vec<PivotMarker> {
        self.model.pivot_markers(sheet)
    }

    /// Lists on a new sheet the source records behind a value of a pivot.
    /// Returns the index of the new sheet.
    pub fn show_pivot_details(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        new_sheet_name: &str,
    ) -> Result<u32, String> {
        let info = self
            .model
            .pivot_cell_info(sheet, row, column)
            .ok_or_else(|| "This cell is not part of a PivotTable".to_string())?;
        let pivot = self.pivot_at(info.index)?;
        let source_sheet = self
            .model
            .pivot_sheet_index(pivot.source_sheet_id)
            .ok_or_else(|| "The source sheet of this PivotTable no longer exists".to_string())?;
        let data = self.model.read_pivot_data(source_sheet, &pivot.source)?;
        let wanted: Vec<(usize, &str)> = pivot
            .rows
            .iter()
            .zip(&info.row_path)
            .chain(pivot.columns.iter().zip(&info.column_path))
            .filter_map(|(field, label)| Some((data.field_index(&field.field)?, label.as_str())))
            .collect();
        let hidden: Vec<(usize, &Vec<String>)> = pivot
            .rows
            .iter()
            .chain(&pivot.columns)
            .chain(&pivot.filters)
            .chain(&pivot.slicer_filters)
            .filter_map(|f| Some((data.field_index(&f.field)?, &f.hidden_items)))
            .collect();
        let records: Vec<&Vec<u32>> = data
            .records
            .iter()
            .filter(|record| {
                let label = |field: usize| &data.fields[field].items[record[field] as usize].label;
                wanted.iter().all(|(field, item)| label(*field) == item)
                    && hidden
                        .iter()
                        .all(|(field, items)| !items.contains(label(*field)))
            })
            .collect();

        // Like Excel, the new sheet goes just before the current one
        let index = sheet;
        self.model.insert_sheet(new_sheet_name, index, None)?;
        self.set_selected_sheet(index)?;
        let header_style = self.pivot_style(PivotCellKind::Header, "")?;
        let plain_style = self.pivot_style(PivotCellKind::Item, "")?;
        let mut cells = Vec::new();
        for (offset, field) in data.fields.iter().enumerate() {
            let text = PivotItem::Text(field.name.clone());
            self.write_pivot_cell(index, 1, offset as i32 + 1, &text, header_style)?;
        }
        for (row_offset, record) in records.iter().enumerate() {
            for (offset, position) in record.iter().enumerate() {
                let item = &data.fields[offset].items[*position as usize];
                let style = if item.number_format.is_empty() {
                    plain_style
                } else {
                    self.pivot_style(PivotCellKind::Item, &item.number_format)?
                };
                self.write_pivot_cell(
                    index,
                    row_offset as i32 + 2,
                    offset as i32 + 1,
                    &item.value,
                    style,
                )?;
            }
        }
        let last_row = records.len() as i32 + 1;
        let worksheet = self.model.workbook.worksheet(index)?;
        for row in 1..=last_row {
            for column in 1..=data.fields.len() as i32 {
                if let Some(cell) = worksheet.cell(row, column) {
                    cells.push(PivotCellDiff {
                        row,
                        column,
                        old_value: None,
                        new_value: Some(cell.into_owned()),
                    });
                }
            }
        }
        let pivots = self.model.workbook.pivot_tables.clone();
        self.push_diff_list(vec![
            Diff::NewSheet {
                index,
                name: new_sheet_name.to_string(),
            },
            Diff::SetPivotTables {
                sheet: index,
                old_pivots: pivots.clone(),
                new_pivots: pivots,
                cells,
            },
        ]);
        self.evaluate_if_not_paused();
        Ok(index)
    }

    /// The fields a source range offers.
    pub fn get_pivot_fields(
        &self,
        sheet: u32,
        area: &PivotArea,
    ) -> Result<Vec<PivotFieldInfo>, String> {
        let data = self.model.read_pivot_data(sheet, area)?;
        Ok(data
            .fields
            .into_iter()
            .map(|field| {
                let mut has_number = false;
                let mut has_other = false;
                for item in &field.items {
                    match item.value {
                        PivotItem::Number(_) => has_number = true,
                        PivotItem::Blank => {}
                        _ => has_other = true,
                    }
                }
                PivotFieldInfo {
                    name: field.name,
                    numeric: has_number && !has_other,
                    items: field.items.into_iter().map(|i| i.label).collect(),
                }
            })
            .collect())
    }

    /// Adds a pivot, on a new sheet if `new_sheet_name` is given, as one undo step.
    /// Returns its index.
    pub fn create_pivot_table(
        &mut self,
        mut pivot: PivotTable,
        new_sheet_name: Option<&str>,
        overwrite: bool,
    ) -> Result<u32, String> {
        if new_sheet_name.is_none() {
            if let Some(sheet) = self.model.pivot_sheet_index(pivot.sheet_id) {
                self.guard(sheet, Permission::Never)?;
            }
        }
        let mut diffs = Vec::new();
        // Like Excel, a new sheet goes just before the current one
        let index = self.view.sheet;
        if let Some(name) = new_sheet_name {
            self.model.insert_sheet(name, index, None)?;
            pivot.sheet_id = self.model.workbook.worksheet(index)?.sheet_id;
            diffs.push(Diff::NewSheet {
                index,
                name: name.to_string(),
            });
            self.set_selected_sheet(index)?;
        }
        if pivot.name.is_empty() {
            pivot.name = self.next_pivot_name();
        }
        let result = self.write_pivot(pivot, None, overwrite, diffs);
        if result.is_err() && new_sheet_name.is_some() {
            // The sheet that was current moved up to this position
            self.model.delete_sheet(index)?;
            self.set_selected_sheet(index)?;
        }
        Ok(result? as u32)
    }

    /// Replaces the definition of a pivot and rewrites its cells.
    pub fn update_pivot_table(
        &mut self,
        index: u32,
        pivot: PivotTable,
        overwrite: bool,
    ) -> Result<(), String> {
        self.guard_pivot(index, Permission::PivotTables)?;
        let current = self.pivot_at(index)?;
        if current.read_only {
            return Err("This PivotTable uses features that cannot be edited here".to_string());
        }
        let mut pivot = pivot;
        pivot.output = current.output;
        pivot.sheet_id = current.sheet_id;
        pivot.row = current.row;
        pivot.column = current.column;
        self.write_pivot(pivot, Some(index as usize), overwrite, vec![])?;
        Ok(())
    }

    /// Recomputes a pivot from the current values of its source.
    pub fn refresh_pivot_table(&mut self, index: u32) -> Result<(), String> {
        self.guard_pivot(index, Permission::PivotTables)?;
        let pivot = self.pivot_at(index)?;
        if pivot.read_only {
            return Err("This PivotTable uses features that cannot be edited here".to_string());
        }
        self.write_pivot(pivot, Some(index as usize), true, vec![])?;
        Ok(())
    }

    /// Removes a pivot and clears its cells.
    pub fn delete_pivot_table(&mut self, index: u32) -> Result<(), String> {
        self.guard_pivot(index, Permission::Never)?;
        let pivot = self.pivot_at(index)?;
        let old_pivots = self.model.workbook.pivot_tables.clone();
        let mut new_pivots = old_pivots.clone();
        new_pivots.remove(index as usize);
        let sheet = self
            .model
            .pivot_sheet_index(pivot.sheet_id)
            .ok_or_else(|| "The sheet of this PivotTable no longer exists".to_string())?;
        let mut cells = Vec::new();
        if let Some(output) = pivot.output {
            for row in output.first_row..=output.last_row {
                for column in output.first_column..=output.last_column {
                    let old_value = self.take_cell(sheet, row, column)?;
                    if old_value.is_some() {
                        cells.push(PivotCellDiff {
                            row,
                            column,
                            old_value,
                            new_value: None,
                        });
                    }
                }
            }
        }
        self.model.workbook.pivot_tables = new_pivots.clone();
        self.push_diff_list(vec![Diff::SetPivotTables {
            sheet,
            old_pivots,
            new_pivots,
            cells,
        }]);
        self.evaluate_if_not_paused();
        Ok(())
    }

    fn pivot_at(&self, index: u32) -> Result<PivotTable, String> {
        self.model
            .workbook
            .pivot_tables
            .get(index as usize)
            .cloned()
            .ok_or_else(|| format!("There is no PivotTable with index {index}"))
    }

    fn next_pivot_name(&self) -> String {
        let pivots = &self.model.workbook.pivot_tables;
        (1..)
            .map(|n| format!("PivotTable{n}"))
            .find(|name| pivots.iter().all(|p| &p.name != name))
            .unwrap_or_default()
    }

    fn take_cell(&mut self, sheet: u32, row: i32, column: i32) -> Result<Option<Cell>, String> {
        Ok(self
            .model
            .workbook
            .worksheet_mut(sheet)?
            .sheet_data
            .remove_cell(row, column))
    }

    fn pivot_style(&mut self, kind: PivotCellKind, number_format: &str) -> Result<i32, String> {
        let mut style: Style = self.model.workbook.styles.get_style(0)?;
        style.font.b = false;
        apply_pivot_look(&mut style, kind);
        if !number_format.is_empty() {
            style.num_fmt = number_format.to_string();
            if style.alignment.is_none() && kind == PivotCellKind::Header {
                style.alignment = Some(crate::types::Alignment {
                    horizontal: HorizontalAlignment::Left,
                    ..Default::default()
                });
            }
        }
        Ok(self.model.workbook.styles.get_style_index_or_create(&style))
    }

    fn write_pivot_cell(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        value: &PivotItem,
        style: i32,
    ) -> Result<(), String> {
        let worksheet = self.model.workbook.worksheet_mut(sheet)?;
        match value {
            PivotItem::Blank => worksheet.update_cell(row, column, Cell::EmptyCell { s: style }),
            PivotItem::Number(number) => {
                worksheet.set_cell_with_number(row, column, *number, style)
            }
            PivotItem::Boolean(boolean) => {
                worksheet.set_cell_with_boolean(row, column, *boolean, style)
            }
            PivotItem::Error(error) => match get_error_by_english_name(error) {
                Some(error) => worksheet.set_cell_with_error(row, column, error, style),
                None => worksheet.update_cell(row, column, Cell::EmptyCell { s: style }),
            },
            PivotItem::Text(text) => self
                .model
                .set_cell_with_string(sheet, row, column, text, style),
        }
    }

    fn check_overwrite(
        &self,
        sheet: u32,
        new_area: &PivotArea,
        old_area: Option<&PivotArea>,
    ) -> Result<(), String> {
        let worksheet = self.model.workbook.worksheet(sheet)?;
        for row in new_area.first_row..=new_area.last_row {
            for column in new_area.first_column..=new_area.last_column {
                if old_area.is_some_and(|a| a.contains(row, column)) {
                    continue;
                }
                let occupied = worksheet
                    .cell(row, column)
                    .is_some_and(|cell| !matches!(&*cell, Cell::EmptyCell { .. }));
                if occupied {
                    return Err(PIVOT_OVERWRITE_ERROR.to_string());
                }
            }
        }
        Ok(())
    }

    fn write_pivot(
        &mut self,
        mut pivot: PivotTable,
        slot: Option<usize>,
        overwrite: bool,
        mut diffs: Vec<Diff>,
    ) -> Result<usize, String> {
        let grid = self.model.compute_pivot(&pivot)?;
        let sheet = self
            .model
            .pivot_sheet_index(pivot.sheet_id)
            .ok_or_else(|| "The sheet of this PivotTable no longer exists".to_string())?;
        let new_area = grid.area(pivot.row, pivot.column);
        let old_area = pivot.output;
        if let Some(area) = &new_area {
            if area.last_row > LAST_ROW || area.last_column > LAST_COLUMN {
                return Err("The PivotTable does not fit in the sheet".to_string());
            }
            if let Some(conflict) = self.model.pivot_area_conflict(&pivot, slot, area) {
                return Err(conflict);
            }
            if !overwrite {
                self.check_overwrite(sheet, area, old_area.as_ref())?;
            }
        }

        // Clear what is there, keeping the previous cells for undo
        let mut coordinates = BTreeSet::new();
        for area in [old_area, new_area].into_iter().flatten() {
            for row in area.first_row..=area.last_row {
                for column in area.first_column..=area.last_column {
                    coordinates.insert((row, column));
                }
            }
        }
        let mut old_cells = Vec::with_capacity(coordinates.len());
        for (row, column) in coordinates {
            let old_value = self.take_cell(sheet, row, column)?;
            old_cells.push((row, column, old_value));
        }

        self.write_grid(sheet, &pivot, &grid)?;

        let worksheet = self.model.workbook.worksheet(sheet)?;
        let cells = old_cells
            .into_iter()
            .filter_map(|(row, column, old_value)| {
                let new_value = worksheet.cell(row, column).map(|c| c.into_owned());
                if old_value.is_none() && new_value.is_none() {
                    return None;
                }
                Some(PivotCellDiff {
                    row,
                    column,
                    old_value,
                    new_value,
                })
            })
            .collect();

        pivot.output = new_area;
        pivot.output_map = Some(grid.map.clone());
        pivot.raw = None;
        let old_pivots = self.model.workbook.pivot_tables.clone();
        let mut new_pivots = old_pivots.clone();
        let index = match slot {
            Some(index) => {
                new_pivots[index] = pivot;
                index
            }
            None => {
                new_pivots.push(pivot);
                new_pivots.len() - 1
            }
        };
        self.model.workbook.pivot_tables = new_pivots.clone();
        diffs.extend(self.fit_pivot_columns(sheet, &new_pivots[index], &grid)?);
        diffs.push(Diff::SetPivotTables {
            sheet,
            old_pivots,
            new_pivots,
            cells,
        });
        self.push_diff_list(diffs);
        self.evaluate_if_not_paused();
        Ok(index)
    }

    fn write_grid(
        &mut self,
        sheet: u32,
        pivot: &PivotTable,
        grid: &PivotGrid,
    ) -> Result<(), String> {
        for cell in &grid.cells {
            let style = self.pivot_style(cell.kind, &cell.number_format)?;
            self.write_pivot_cell(
                sheet,
                pivot.row + cell.row,
                pivot.column + cell.column,
                &cell.value,
                style,
            )?;
        }
        Ok(())
    }

    /// Widens the columns of a pivot so its text fits. Columns are never narrowed.
    fn fit_pivot_columns(
        &mut self,
        sheet: u32,
        pivot: &PivotTable,
        grid: &PivotGrid,
    ) -> Result<Vec<Diff>, String> {
        let mut widths: BTreeMap<i32, f64> = BTreeMap::new();
        for cell in &grid.cells {
            let characters = match &cell.value {
                PivotItem::Text(text) => text.chars().count(),
                _ => continue,
            };
            let width = characters as f64 * CHARACTER_WIDTH + CELL_PADDING;
            let entry = widths.entry(pivot.column + cell.column).or_default();
            *entry = entry.max(width);
        }
        let mut diffs = Vec::new();
        for (column, width) in widths {
            let old_value = self.model.get_column_width(sheet, column)?;
            if width > old_value {
                self.model.set_column_width(sheet, column, width)?;
                diffs.push(Diff::SetColumnWidth {
                    sheet,
                    column,
                    new_value: width,
                    old_value,
                });
            }
        }
        Ok(diffs)
    }

    /// Installs one state of a pivot edit, for undo and redo.
    pub(super) fn apply_pivot_cells(
        &mut self,
        sheet: u32,
        cells: &[PivotCellDiff],
        pivots: &[PivotTable],
        use_old: bool,
    ) -> Result<(), String> {
        let worksheet = self.model.workbook.worksheet_mut(sheet)?;
        for cell in cells {
            let value = if use_old {
                &cell.old_value
            } else {
                &cell.new_value
            };
            match value {
                Some(value) => worksheet.update_cell(cell.row, cell.column, value.clone())?,
                None => {
                    worksheet.sheet_data.remove_cell(cell.row, cell.column);
                }
            }
        }
        self.model.workbook.pivot_tables = pivots.to_vec();
        Ok(())
    }

    /// A no-op step that lets undo restore pivot positions exactly after rows
    /// or columns are deleted.
    pub(super) fn pivot_snapshot_diff(&self, sheet: u32) -> Option<Diff> {
        let pivots = &self.model.workbook.pivot_tables;
        if pivots.is_empty() {
            return None;
        }
        Some(Diff::SetPivotTables {
            sheet,
            old_pivots: pivots.clone(),
            new_pivots: pivots.clone(),
            cells: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::constants::{LAST_COLUMN, LAST_ROW};
    use crate::pivot::{
        PivotAggregation, PivotArea, PivotCellZone, PivotField, PivotMarkerKind, PivotOptions,
        PivotTable, PivotValue, PIVOT_OVERWRITE_ERROR,
    };
    use crate::UserModel;

    fn model() -> UserModel<'static> {
        let mut model = UserModel::new_empty("model", "en", "UTC", "en").unwrap();
        let rows = [
            ["Region", "Units"],
            ["East", "1"],
            ["West", "3"],
            ["East", "2"],
        ];
        for (r, row) in rows.iter().enumerate() {
            for (c, value) in row.iter().enumerate() {
                model
                    .set_user_input(0, r as i32 + 1, c as i32 + 1, value)
                    .unwrap();
            }
        }
        model
    }

    fn definition() -> PivotTable {
        PivotTable {
            name: String::new(),
            sheet_id: 1,
            row: 3,
            column: 5,
            source_sheet_id: 1,
            source: PivotArea {
                first_row: 1,
                first_column: 1,
                last_row: 4,
                last_column: 2,
            },
            rows: vec![PivotField {
                field: "Region".to_string(),
                hidden_items: vec![],
                ..Default::default()
            }],
            columns: vec![],
            values: vec![PivotValue {
                field: "Units".to_string(),
                aggregation: PivotAggregation::Sum,
                name: String::new(),
                show_as: Default::default(),
                base_field: String::new(),
            }],
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

    fn text(model: &UserModel, sheet: u32, row: i32, column: i32) -> String {
        model.get_formatted_cell_value(sheet, row, column).unwrap()
    }

    #[test]
    fn create_writes_cells_and_undo_removes_them() {
        let mut model = model();
        let index = model.create_pivot_table(definition(), None, false).unwrap();
        assert_eq!(index, 0);
        assert_eq!(model.get_pivot_tables()[0].pivot.name, "PivotTable1");
        assert_eq!(text(&model, 0, 3, 5), "Row Labels");
        assert_eq!(text(&model, 0, 4, 5), "East");
        assert_eq!(text(&model, 0, 4, 6), "3");
        assert_eq!(text(&model, 0, 6, 6), "6");
        assert_eq!(model.get_pivot_table_at(0, 6, 6), Some(0));
        assert_eq!(model.get_pivot_table_at(0, 7, 6), None);

        model.undo().unwrap();
        assert_eq!(text(&model, 0, 3, 5), "");
        assert!(model.get_pivot_tables().is_empty());

        model.redo().unwrap();
        assert_eq!(text(&model, 0, 6, 6), "6");
        assert_eq!(model.get_pivot_tables().len(), 1);
    }

    #[test]
    fn new_sheet_is_one_undo_step() {
        let mut model = model();
        let mut pivot = definition();
        pivot.row = 3;
        pivot.column = 1;
        model
            .create_pivot_table(pivot, Some("Pivot"), false)
            .unwrap();
        assert_eq!(model.get_model().workbook.worksheets.len(), 2);
        assert_eq!(text(&model, 0, 4, 2), "3");
        model.undo().unwrap();
        assert_eq!(model.get_model().workbook.worksheets.len(), 1);
        assert!(model.get_pivot_tables().is_empty());
    }

    #[test]
    fn a_new_sheet_goes_before_the_current_one() {
        let mut model = model();
        model.new_sheet().unwrap();
        model.set_selected_sheet(1).unwrap();
        let mut pivot = definition();
        pivot.row = 3;
        pivot.column = 1;
        model
            .create_pivot_table(pivot, Some("Pivot"), false)
            .unwrap();
        let names = |model: &UserModel| model.get_model().workbook.get_worksheet_names();
        assert_eq!(names(&model), ["Sheet1", "Pivot", "Sheet2"]);
        assert_eq!(model.get_selected_view().sheet, 1);
        assert_eq!(text(&model, 1, 4, 2), "3");

        model.undo().unwrap();
        assert_eq!(names(&model), ["Sheet1", "Sheet2"]);
        assert_eq!(model.get_selected_view().sheet, 1);

        model.redo().unwrap();
        assert_eq!(names(&model), ["Sheet1", "Pivot", "Sheet2"]);
        assert_eq!(text(&model, 1, 4, 2), "3");
        assert_eq!(model.get_pivot_tables().len(), 1);
    }

    #[test]
    fn a_failed_pivot_removes_its_new_sheet_and_keeps_the_current_one() {
        let mut model = model();
        model.new_sheet().unwrap();
        model.set_selected_sheet(1).unwrap();
        let mut pivot = definition();
        pivot.row = LAST_ROW;
        pivot.column = LAST_COLUMN;
        assert!(model
            .create_pivot_table(pivot, Some("Pivot"), false)
            .is_err());
        assert_eq!(
            model.get_model().workbook.get_worksheet_names(),
            ["Sheet1", "Sheet2"]
        );
        assert_eq!(model.get_selected_view().sheet, 1);
    }

    #[test]
    fn occupied_cells_are_not_replaced_without_permission() {
        let mut model = model();
        model.set_user_input(0, 5, 6, "keep").unwrap();
        let error = model
            .create_pivot_table(definition(), None, false)
            .unwrap_err();
        assert_eq!(error, PIVOT_OVERWRITE_ERROR);
        assert_eq!(text(&model, 0, 5, 6), "keep");
        assert!(model.get_pivot_tables().is_empty());

        model.create_pivot_table(definition(), None, true).unwrap();
        assert_eq!(text(&model, 0, 5, 6), "3");
        model.undo().unwrap();
        assert_eq!(text(&model, 0, 5, 6), "keep");
    }

    #[test]
    fn a_pivot_cannot_cover_its_source() {
        let mut model = model();
        let mut pivot = definition();
        pivot.column = 2;
        assert!(model.create_pivot_table(pivot, None, true).is_err());
        assert_eq!(text(&model, 0, 1, 2), "Units");
        assert!(model.get_pivot_tables().is_empty());
    }

    #[test]
    fn editing_clears_cells_the_new_layout_does_not_use() {
        let mut model = model();
        model.create_pivot_table(definition(), None, false).unwrap();
        let mut pivot = model.get_pivot_tables()[0].pivot.clone();
        pivot.rows.clear();
        model.update_pivot_table(0, pivot, false).unwrap();
        assert_eq!(text(&model, 0, 3, 5), "Sum of Units");
        assert_eq!(text(&model, 0, 4, 5), "6");
        assert_eq!(text(&model, 0, 5, 5), "");
        assert_eq!(text(&model, 0, 6, 6), "");
        model.undo().unwrap();
        assert_eq!(text(&model, 0, 6, 6), "6");
    }

    #[test]
    fn refresh_reads_the_source_again() {
        let mut model = model();
        model.create_pivot_table(definition(), None, false).unwrap();
        model.set_user_input(0, 2, 2, "10").unwrap();
        assert_eq!(text(&model, 0, 6, 6), "6");
        model.refresh_pivot_table(0).unwrap();
        assert_eq!(text(&model, 0, 6, 6), "15");
    }

    #[test]
    fn delete_clears_the_output() {
        let mut model = model();
        model.create_pivot_table(definition(), None, false).unwrap();
        model.delete_pivot_table(0).unwrap();
        assert!(model.get_pivot_tables().is_empty());
        assert_eq!(text(&model, 0, 3, 5), "");
        model.undo().unwrap();
        assert_eq!(text(&model, 0, 3, 5), "Row Labels");
        assert_eq!(model.get_pivot_tables().len(), 1);
    }

    #[test]
    fn text_that_looks_like_a_formula_stays_text() {
        let mut model = model();
        model.set_user_input(0, 2, 1, "'=1+1").unwrap();
        model.create_pivot_table(definition(), None, false).unwrap();
        // "=1+1" sorts before "East"
        assert_eq!(text(&model, 0, 4, 5), "=1+1");
        assert_eq!(model.get_cell_content(0, 4, 5).unwrap(), "=1+1".to_string());
    }

    #[test]
    fn pivots_follow_inserted_rows_and_undo_restores_deletions() {
        let mut model = model();
        model.create_pivot_table(definition(), None, false).unwrap();
        model.insert_rows(0, 1, 2).unwrap();
        let moved = &model.get_pivot_tables()[0].pivot;
        assert_eq!(moved.row, 5);
        assert_eq!(moved.source.first_row, 3);
        assert_eq!(moved.source.last_row, 6);

        model.delete_rows(0, 1, 1).unwrap();
        assert_eq!(model.get_pivot_tables()[0].pivot.row, 4);
        model.undo().unwrap();
        let restored = &model.get_pivot_tables()[0].pivot;
        assert_eq!(restored.row, 5);
        assert_eq!(restored.source.last_row, 6);
    }

    #[test]
    fn saved_bytes_keep_pivots_and_old_files_still_load() {
        let mut model = model();
        let plain = model.to_bytes();
        assert_eq!(plain, bitcode::encode(&model.get_model().workbook));
        assert!(UserModel::from_bytes(&plain, "en").is_ok());

        model.create_pivot_table(definition(), None, false).unwrap();
        let reopened = UserModel::from_bytes(&model.to_bytes(), "en").unwrap();
        assert_eq!(reopened.get_pivot_tables().len(), 1);
        assert_eq!(text(&reopened, 0, 6, 6), "6");
        assert!(UserModel::from_bytes(b"ICPIVOT1trunc", "en").is_err());
    }

    #[test]
    fn cell_info_and_markers_come_from_the_written_output() {
        let mut model = model();
        let mut pivot = definition();
        pivot.rows.push(PivotField {
            field: "Units".to_string(),
            hidden_items: vec![],
            ..Default::default()
        });
        pivot.values.clear();
        model.create_pivot_table(pivot, None, false).unwrap();
        let info = model.get_pivot_cell_info(0, 4, 5).unwrap();
        assert_eq!(info.zone, PivotCellZone::RowLabel);
        assert_eq!(info.field.as_deref(), Some("Region"));
        assert_eq!(info.item.as_deref(), Some("East"));
        assert!(info.has_children);
        assert_eq!(model.get_pivot_cell_info(0, 1, 1), None);

        let markers = model.get_pivot_markers(0);
        assert_eq!(markers[0].kind, PivotMarkerKind::Filter);
        assert!(markers
            .iter()
            .any(|m| m.kind == PivotMarkerKind::Collapse && m.item.as_deref() == Some("East")));
    }

    #[test]
    fn show_details_lists_the_records_behind_a_value() {
        let mut model = model();
        model.create_pivot_table(definition(), None, false).unwrap();
        let sheet = model.show_pivot_details(0, 4, 6, "Details1").unwrap();
        assert_eq!(sheet, 0);
        assert_eq!(model.get_model().workbook.worksheets[1].name, "Sheet1");
        assert_eq!(text(&model, 0, 1, 1), "Region");
        assert_eq!(text(&model, 0, 2, 1), "East");
        assert_eq!(text(&model, 0, 3, 2), "2");
        assert_eq!(text(&model, 0, 4, 1), "");
        model.undo().unwrap();
        assert_eq!(model.get_model().workbook.worksheets.len(), 1);
        assert_eq!(model.get_selected_view().sheet, 0);
    }

    #[test]
    fn the_first_pivot_format_loads_with_plain_cells() {
        let mut model = model();
        model.create_pivot_table(definition(), None, false).unwrap();
        let body = bitcode::encode(&model.get_model().workbook);
        let mut bytes = b"ICPIVOT1".to_vec();
        bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&body);
        bytes.extend_from_slice(b"anything");
        let reopened = UserModel::from_bytes(&bytes, "en").unwrap();
        assert!(reopened.get_pivot_tables().is_empty());
        assert_eq!(text(&reopened, 0, 6, 6), "6");
    }
}
