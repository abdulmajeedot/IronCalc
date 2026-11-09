//! Creating and editing tables and slicers with undo. See [`crate::table`].

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use bitcode::{Decode, Encode};

use crate::constants::{LAST_COLUMN, LAST_ROW};
use crate::pivot::{PivotArea, PivotField, PivotTable};
use crate::table::{
    area_reference, compare_filter_cells, table_area, table_column_index, table_data_rows, Slicer,
    SlicerConnection, SlicerEntry, SlicerPivot, SlicerPlacement, SlicerSource, TableColumnEntry,
    TableColumnFilter, TableEntry, TableFilterItem, TableFilterRule, TableFilterState, TableMarker,
    TableOptions, TableSort, DEFAULT_SLICER_STYLE, DEFAULT_TABLE_STYLE,
};
use crate::types::{Cell, Comment, Table, TableColumn, TableStyleInfo};
use crate::user_model::common::UserModel;
use crate::user_model::history::Diff;
use crate::user_model::pivot::PivotCellDiff;

const SLICER_WIDTH: f64 = 192.0;
const SLICER_HEIGHT: f64 = 250.0;
const SLICER_BUTTON_HEIGHT: f64 = 25.0;

/// Everything a table edit may replace wholesale.
#[derive(Clone, Encode, Decode)]
pub(crate) struct TableSnapshot {
    tables: HashMap<String, Table>,
    filters: Vec<TableFilterState>,
    slicers: Vec<Slicer>,
    /// A sheet's comments, kept when rows or columns are deleted.
    comments: Option<(u32, Vec<Comment>)>,
    validations: Vec<crate::data_validation::DataValidation>,
}

#[derive(Clone, Encode, Decode)]
pub(crate) struct TableRowDiff {
    row: i32,
    old_hidden: bool,
    new_hidden: bool,
}

/// The `SUBTOTAL` code of a total-row function, as named in xlsx files.
fn subtotal_code(function: &str) -> Option<i32> {
    match function {
        "average" => Some(101),
        "countNums" => Some(102),
        "count" => Some(103),
        "max" => Some(104),
        "min" => Some(105),
        "stdDev" => Some(107),
        "sum" => Some(109),
        "var" => Some(110),
        _ => None,
    }
}

fn valid_table_name(name: &str) -> bool {
    let mut chars = name.chars();
    let starts_well = chars
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '\\');
    if !starts_well || !chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.') {
        return false;
    }
    if name.len() == 1 && matches!(name, "c" | "C" | "r" | "R") {
        return false;
    }
    // A name must not read as a cell reference
    let letters = name.chars().take_while(|c| c.is_ascii_alphabetic()).count();
    let digits = &name[letters..];
    !(letters > 0
        && letters <= 3
        && !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit()))
}

impl UserModel<'_> {
    fn table_snapshot(&self) -> TableSnapshot {
        TableSnapshot {
            tables: self.model.workbook.tables.clone(),
            filters: self.model.workbook.table_filters.clone(),
            slicers: self.model.workbook.slicers.clone(),
            comments: None,
            validations: self.model.workbook.data_validations.clone(),
        }
    }

    fn table_snapshot_with_comments(&self, sheet: u32) -> TableSnapshot {
        let mut snapshot = self.table_snapshot();
        snapshot.comments = self
            .model
            .workbook
            .worksheets
            .get(sheet as usize)
            .map(|worksheet| (sheet, worksheet.comments.clone()));
        snapshot
    }

    fn table_state_diff(&self, sheet: u32, old: TableSnapshot, new: TableSnapshot) -> Diff {
        Diff::SetTableState {
            sheet,
            old: Box::new(old),
            new: Box::new(new),
            cells: vec![],
            rows: vec![],
        }
    }

    /// A no-op step that lets undo restore tables and comments exactly after
    /// rows or columns are deleted.
    pub(super) fn table_snapshot_diff(&self, sheet: u32) -> Option<Diff> {
        let workbook = &self.model.workbook;
        let has_comments = !workbook.worksheets.get(sheet as usize)?.comments.is_empty();
        if workbook.tables.is_empty()
            && workbook.slicers.is_empty()
            && workbook.data_validations.is_empty()
            && !has_comments
        {
            return None;
        }
        let snapshot = self.table_snapshot_with_comments(sheet);
        Some(self.table_state_diff(sheet, snapshot.clone(), snapshot))
    }

    fn install_table_snapshot(&mut self, snapshot: &TableSnapshot) {
        self.model.workbook.tables = snapshot.tables.clone();
        self.model.workbook.table_filters = snapshot.filters.clone();
        self.model.workbook.slicers = snapshot.slicers.clone();
        self.model.workbook.data_validations = snapshot.validations.clone();
        if let Some((sheet, comments)) = &snapshot.comments {
            if let Some(worksheet) = self.model.workbook.worksheets.get_mut(*sheet as usize) {
                worksheet.comments = comments.clone();
            }
        }
        self.model.sync_table_parser();
    }

    /// Installs one state of a table edit, for undo and redo.
    pub(super) fn apply_table_state(
        &mut self,
        sheet: u32,
        cells: &[PivotCellDiff],
        rows: &[TableRowDiff],
        snapshot: &TableSnapshot,
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
        for row in rows {
            let hidden = if use_old {
                row.old_hidden
            } else {
                row.new_hidden
            };
            worksheet.set_row_hidden(row.row, hidden)?;
        }
        self.install_table_snapshot(snapshot);
        Ok(())
    }

    /// Runs an edit as one undo step. `regions` are the cells and rows it may
    /// change. A failed edit leaves everything as it was.
    fn table_edit<T>(
        &mut self,
        sheet: u32,
        regions: &[PivotArea],
        edit: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        let old = self.table_snapshot_with_comments(sheet);
        let mut coordinates = BTreeSet::new();
        let mut row_numbers = BTreeSet::new();
        for area in regions {
            for row in area.first_row..=area.last_row {
                row_numbers.insert(row);
                for column in area.first_column..=area.last_column {
                    coordinates.insert((row, column));
                }
            }
        }
        let worksheet = self.model.workbook.worksheet(sheet)?;
        let mut before: BTreeMap<(i32, i32), Option<Cell>> = BTreeMap::new();
        for (row, column) in &coordinates {
            before.insert(
                (*row, *column),
                worksheet.cell(*row, *column).map(|c| c.into_owned()),
            );
        }
        let mut hidden_before = BTreeMap::new();
        for row in &row_numbers {
            hidden_before.insert(*row, worksheet.is_row_hidden(*row)?);
        }

        let result = edit(self);
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                let worksheet = self.model.workbook.worksheet_mut(sheet)?;
                for ((row, column), cell) in before {
                    match cell {
                        Some(cell) => worksheet.update_cell(row, column, cell)?,
                        None => {
                            worksheet.sheet_data.remove_cell(row, column);
                        }
                    }
                }
                for (row, hidden) in hidden_before {
                    worksheet.set_row_hidden(row, hidden)?;
                }
                self.install_table_snapshot(&old);
                self.model.sync_tables();
                return Err(error);
            }
        };

        // Headers follow the table straight away, inside this same undo step
        self.model.sync_table_parser();
        self.model.sync_tables();
        let worksheet = self.model.workbook.worksheet(sheet)?;
        let mut cells = Vec::new();
        for ((row, column), old_value) in before {
            let new_value = worksheet.cell(row, column).map(|c| c.into_owned());
            if new_value != old_value {
                cells.push(PivotCellDiff {
                    row,
                    column,
                    old_value,
                    new_value,
                });
            }
        }
        let mut rows = Vec::new();
        for (row, old_hidden) in hidden_before {
            let new_hidden = worksheet.is_row_hidden(row)?;
            if new_hidden != old_hidden {
                rows.push(TableRowDiff {
                    row,
                    old_hidden,
                    new_hidden,
                });
            }
        }
        let new = self.table_snapshot_with_comments(sheet);
        self.push_diff_list(vec![Diff::SetTableState {
            sheet,
            old: Box::new(old),
            new: Box::new(new),
            cells,
            rows,
        }]);
        self.evaluate_if_not_paused();
        Ok(value)
    }

    fn table_named(&self, name: &str) -> Result<&Table, String> {
        self.model
            .workbook
            .tables
            .get(name)
            .ok_or_else(|| format!("Table not found: {name}"))
    }

    fn table_place(&self, name: &str) -> Result<(u32, PivotArea), String> {
        let table = self.table_named(name)?;
        let area = table_area(table).ok_or("Invalid table range")?;
        let sheet = self.model.table_sheet(table).ok_or("Sheet not found")?;
        Ok((sheet, area))
    }

    fn table_entry(&self, table: &Table) -> Option<TableEntry> {
        let area = table_area(table)?;
        let sheet = self.model.table_sheet(table)?;
        let state = self.model.table_filter_state(&table.name);
        let columns = table
            .columns
            .iter()
            .map(|c| TableColumnEntry {
                name: c.name.clone(),
                totals_function: c.totals_row_function.clone(),
                totals_label: c.totals_row_label.clone(),
                filtered: state.is_some_and(|s| s.columns.iter().any(|f| f.column == c.name)),
            })
            .collect();
        Some(TableEntry {
            name: table.name.clone(),
            sheet,
            area,
            reference: table.reference.clone(),
            columns,
            options: TableOptions {
                name: table.name.clone(),
                style_name: table.style_info.name.clone(),
                header_row: table.header_row_count > 0,
                total_row: table.totals_row_count > 0,
                banded_rows: table.style_info.show_row_stripes,
                banded_columns: table.style_info.show_column_stripes,
                first_column: table.style_info.show_first_column,
                last_column: table.style_info.show_last_column,
                filter_button: table.has_filters,
            },
            filtered: state.is_some_and(|s| !s.columns.is_empty()),
            sort: state.and_then(|s| s.sort.clone()),
        })
    }

    /// Every table on a sheet, by name.
    pub fn get_tables(&self, sheet: u32) -> Vec<TableEntry> {
        let mut entries: Vec<TableEntry> = self
            .model
            .workbook
            .tables
            .values()
            .filter_map(|t| self.table_entry(t))
            .filter(|e| e.sheet == sheet)
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    /// The table covering a cell.
    pub fn get_table_at(&self, sheet: u32, row: i32, column: i32) -> Option<TableEntry> {
        self.get_tables(sheet)
            .into_iter()
            .find(|e| e.area.contains(row, column))
    }

    /// The colours of a table style, for the style gallery.
    pub fn get_table_style_swatch(&self, name: &str) -> Option<crate::table::TableStyleSwatch> {
        crate::table::table_style_swatch(name, &self.model.workbook.theme)
    }

    /// The filter buttons of a sheet's tables.
    pub fn get_table_markers(&self, sheet: u32) -> Vec<TableMarker> {
        let mut markers = Vec::new();
        for table in self.model.workbook.tables.values() {
            let Some(area) = table_area(table) else {
                continue;
            };
            if self.model.table_sheet(table) != Some(sheet)
                || !table.has_filters
                || table.header_row_count == 0
            {
                continue;
            }
            let state = self.model.table_filter_state(&table.name);
            for (offset, column) in table.columns.iter().enumerate() {
                markers.push(TableMarker {
                    table: table.name.clone(),
                    column: column.name.clone(),
                    row: area.first_row,
                    col: area.first_column + offset as i32,
                    filtered: state
                        .is_some_and(|s| s.columns.iter().any(|f| f.column == column.name)),
                    sort_descending: state
                        .and_then(|s| s.sort.as_ref())
                        .filter(|s| s.column == column.name)
                        .map(|s| s.descending),
                });
            }
        }
        markers
    }

    /// The values a filter list offers for a table column.
    pub fn get_table_filter_items(
        &self,
        table: &str,
        column: &str,
    ) -> Result<Vec<TableFilterItem>, String> {
        self.model.table_filter_items(table, column)
    }

    /// The filter set on a table column, if any.
    pub fn get_table_filter(&self, table: &str, column: &str) -> Option<TableFilterRule> {
        self.model
            .table_filter_state(table)?
            .columns
            .iter()
            .find(|f| f.column == column)
            .map(|f| f.rule.clone())
    }

    fn check_table_area(
        &self,
        sheet: u32,
        area: &PivotArea,
        ignore: Option<&str>,
    ) -> Result<(), String> {
        if area.first_row > area.last_row
            || area.first_column > area.last_column
            || area.last_row > LAST_ROW
            || area.last_column > LAST_COLUMN
        {
            return Err("That range cannot be a table".to_string());
        }
        for table in self.model.workbook.tables.values() {
            if Some(table.name.as_str()) == ignore || self.model.table_sheet(table) != Some(sheet) {
                continue;
            }
            if table_area(table).is_some_and(|other| {
                other.first_row <= area.last_row
                    && area.first_row <= other.last_row
                    && other.first_column <= area.last_column
                    && area.first_column <= other.last_column
            }) {
                return Err("A table cannot overlap another table".to_string());
            }
        }
        let sheet_id = self.model.workbook.worksheet(sheet)?.sheet_id;
        for pivot in &self.model.workbook.pivot_tables {
            if pivot.sheet_id == sheet_id
                && pivot.output.is_some_and(|other| {
                    other.first_row <= area.last_row
                        && area.first_row <= other.last_row
                        && other.first_column <= area.last_column
                        && area.first_column <= other.last_column
                })
            {
                return Err("A table cannot overlap a PivotTable".to_string());
            }
        }
        for merged in self.model.get_merged_cells(sheet)? {
            let (first_row, first_column) = (merged.row, merged.column);
            let (last_row, last_column) = (
                merged.row + merged.height - 1,
                merged.column + merged.width - 1,
            );
            if first_row <= area.last_row
                && area.first_row <= last_row
                && first_column <= area.last_column
                && area.first_column <= last_column
            {
                return Err("A table cannot contain merged cells".to_string());
            }
        }
        Ok(())
    }

    fn next_table_name(&self) -> String {
        let workbook = &self.model.workbook;
        (1..)
            .map(|n| format!("Table{n}"))
            .find(|name| !self.table_name_taken(name, None))
            .unwrap_or_else(|| format!("Table{}", workbook.tables.len() + 1))
    }

    fn table_name_taken(&self, name: &str, ignore: Option<&str>) -> bool {
        let lower = name.to_lowercase();
        let workbook = &self.model.workbook;
        workbook
            .tables
            .keys()
            .any(|t| t.to_lowercase() == lower && Some(t.as_str()) != ignore)
            || workbook
                .defined_names
                .iter()
                .any(|d| d.name.to_lowercase() == lower)
    }

    fn region_is_empty(&self, sheet: u32, area: &PivotArea) -> Result<bool, String> {
        let worksheet = self.model.workbook.worksheet(sheet)?;
        for row in area.first_row..=area.last_row {
            for column in area.first_column..=area.last_column {
                if worksheet
                    .cell(row, column)
                    .is_some_and(|c| !matches!(&*c, Cell::EmptyCell { .. }))
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn take_cell_at(&mut self, sheet: u32, row: i32, column: i32) -> Result<Option<Cell>, String> {
        Ok(self
            .model
            .workbook
            .worksheet_mut(sheet)?
            .sheet_data
            .remove_cell(row, column))
    }

    fn put_cell_at(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        cell: Option<Cell>,
    ) -> Result<(), String> {
        let worksheet = self.model.workbook.worksheet_mut(sheet)?;
        match cell {
            Some(cell) => worksheet.update_cell(row, column, cell)?,
            None => {
                worksheet.sheet_data.remove_cell(row, column);
            }
        }
        Ok(())
    }

    /// Makes a table from a range. Without headers, its data moves down a row
    /// to make room for generated ones. Returns the table's name.
    pub fn create_table(
        &mut self,
        sheet: u32,
        area: PivotArea,
        has_headers: bool,
        style_name: Option<String>,
    ) -> Result<String, String> {
        let mut full = area;
        if !has_headers {
            full.last_row += 1;
        }
        self.check_table_area(sheet, &full, None)?;
        if full.last_row == full.first_row {
            full.last_row += 1;
            self.check_table_area(sheet, &full, None)?;
        }
        if !has_headers {
            let below = PivotArea {
                first_row: full.last_row,
                last_row: full.last_row,
                ..full
            };
            if !self.region_is_empty(sheet, &below)? {
                return Err("The row below the range must be empty".to_string());
            }
        }
        let name = self.next_table_name();
        let sheet_name = self.model.workbook.worksheet(sheet)?.get_name();
        let columns: Vec<TableColumn> = (0..=full.last_column - full.first_column)
            .map(|offset| TableColumn {
                id: offset as u32 + 1,
                name: String::new(),
                ..Default::default()
            })
            .collect();
        let table = Table {
            name: name.clone(),
            display_name: name.clone(),
            sheet_name,
            reference: area_reference(&full),
            totals_row_count: 0,
            header_row_count: 1,
            header_row_dxf_id: None,
            data_dxf_id: None,
            totals_row_dxf_id: None,
            columns,
            style_info: TableStyleInfo {
                name: Some(style_name.unwrap_or_else(|| DEFAULT_TABLE_STYLE.to_string())),
                show_first_column: false,
                show_last_column: false,
                show_row_stripes: true,
                show_column_stripes: false,
            },
            has_filters: true,
        };
        let result_name = name.clone();
        self.table_edit(sheet, &[full], move |user| {
            if !has_headers {
                for row in (area.first_row..=area.last_row).rev() {
                    for column in area.first_column..=area.last_column {
                        let cell = user.take_cell_at(sheet, row, column)?;
                        user.put_cell_at(sheet, row + 1, column, cell)?;
                    }
                }
            }
            user.model.workbook.tables.insert(name, table);
            Ok(())
        })?;
        Ok(result_name)
    }

    fn write_total_cell(
        &mut self,
        sheet: u32,
        area: &PivotArea,
        table: &Table,
        offset: usize,
    ) -> Result<(), String> {
        let column = area.first_column + offset as i32;
        let row = area.last_row;
        let definition = &table.columns[offset];
        let text = if let Some(label) = &definition.totals_row_label {
            label.clone()
        } else if let Some(code) = definition
            .totals_row_function
            .as_deref()
            .and_then(subtotal_code)
        {
            let (first, last) = table_data_rows(table, area);
            let letter = crate::expressions::utils::number_to_column(column).unwrap_or_default();
            format!("=SUBTOTAL({code},{letter}{first}:{letter}{last})")
        } else {
            String::new()
        };
        let style = self.model.get_cell_style_index(sheet, row, column)?;
        self.take_cell_at(sheet, row, column)?;
        if text.is_empty() {
            self.model.workbook.worksheet_mut(sheet)?.update_cell(
                row,
                column,
                Cell::EmptyCell { s: style },
            )?;
        } else {
            self.model.set_user_input(sheet, row, column, text)?;
        }
        Ok(())
    }

    fn rewrite_totals(&mut self, name: &str) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        let table = self.table_named(name)?.clone();
        if table.totals_row_count == 0 {
            return Ok(());
        }
        for offset in 0..table.columns.len() {
            self.write_total_cell(sheet, &area, &table, offset)?;
        }
        Ok(())
    }

    /// Adds or removes the total row of a table, inside an edit.
    fn set_total_row(&mut self, name: &str, show: bool) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        let table = self.table_named(name)?.clone();
        if show == (table.totals_row_count > 0) {
            return Ok(());
        }
        let mut moved = area;
        if show {
            moved.last_row += 1;
            let below = PivotArea {
                first_row: moved.last_row,
                last_row: moved.last_row,
                ..area
            };
            if !self.region_is_empty(sheet, &below)? {
                return Err("The row below the table must be empty".to_string());
            }
            self.check_table_area(sheet, &moved, Some(name))?;
        } else {
            for column in area.first_column..=area.last_column {
                self.take_cell_at(sheet, area.last_row, column)?;
            }
            moved.last_row -= 1;
        }
        let table = self
            .model
            .workbook
            .tables
            .get_mut(name)
            .ok_or("Table not found")?;
        table.reference = area_reference(&moved);
        table.totals_row_count = u32::from(show);
        if show {
            let last = table.columns.len() - 1;
            for (offset, column) in table.columns.iter_mut().enumerate() {
                column.totals_row_label = (offset == 0 && last != 0).then(|| "Total".to_string());
                column.totals_row_function = (offset == last).then(|| "sum".to_string());
            }
        } else {
            for column in &mut table.columns {
                column.totals_row_label = None;
                column.totals_row_function = None;
            }
        }
        self.model.sync_table_parser();
        self.rewrite_totals(name)
    }

    fn set_header_row(&mut self, name: &str, show: bool) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        let table = self.table_named(name)?.clone();
        if show == (table.header_row_count > 0) {
            return Ok(());
        }
        let mut moved = area;
        if show {
            if area.first_row == 1 {
                return Err("There is no row above the table".to_string());
            }
            moved.first_row -= 1;
            let above = PivotArea {
                last_row: moved.first_row,
                ..moved
            };
            if !self.region_is_empty(sheet, &above)? {
                return Err("The row above the table must be empty".to_string());
            }
            self.check_table_area(sheet, &moved, Some(name))?;
            for (offset, column) in table.columns.iter().enumerate() {
                let at = area.first_column + offset as i32;
                let style = self
                    .model
                    .get_cell_style_index(sheet, moved.first_row, at)?;
                self.model
                    .set_cell_with_string(sheet, moved.first_row, at, &column.name, style)?;
            }
        } else {
            if table_data_rows(&table, &area).0 > area.last_row - table.totals_row_count as i32 {
                return Err("A table needs at least one row of data".to_string());
            }
            for column in area.first_column..=area.last_column {
                self.take_cell_at(sheet, area.first_row, column)?;
            }
            moved.first_row += 1;
        }
        let table = self
            .model
            .workbook
            .tables
            .get_mut(name)
            .ok_or("Table not found")?;
        table.reference = area_reference(&moved);
        table.header_row_count = u32::from(show);
        if !show {
            table.has_filters = false;
        }
        Ok(())
    }

    fn rename_table(&mut self, name: &str, new_name: &str) -> Result<(), String> {
        if !valid_table_name(new_name) {
            return Err(format!("'{new_name}' is not a valid table name"));
        }
        if self.table_name_taken(new_name, Some(name)) {
            return Err(format!("The name '{new_name}' is already used"));
        }
        let workbook = &mut self.model.workbook;
        let mut table = workbook.tables.remove(name).ok_or("Table not found")?;
        table.name = new_name.to_string();
        table.display_name = new_name.to_string();
        workbook.tables.insert(new_name.to_string(), table);
        for state in workbook
            .table_filters
            .iter_mut()
            .filter(|s| s.table == name)
        {
            state.table = new_name.to_string();
        }
        for slicer in &mut workbook.slicers {
            if let SlicerSource::Table { table, .. } = &mut slicer.source {
                if table == name {
                    *table = new_name.to_string();
                }
            }
        }
        Ok(())
    }

    /// Hides and shows rows to match the table's filters, inside an edit.
    fn reapply_table_filters(&mut self, name: &str) -> Result<(), String> {
        let (sheet, _) = self.table_place(name)?;
        let visibility = self.model.table_row_visibility(name)?;
        let worksheet = self.model.workbook.worksheet_mut(sheet)?;
        for (row, hidden) in visibility {
            if worksheet.is_row_hidden(row)? != hidden {
                worksheet.set_row_hidden(row, hidden)?;
            }
        }
        Ok(())
    }

    /// Applies the Table Design panel.
    pub fn set_table_options(&mut self, name: &str, options: TableOptions) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        let name = name.to_string();
        let grown = PivotArea {
            first_row: (area.first_row - 1).max(1),
            last_row: (area.last_row + 1).min(LAST_ROW),
            ..area
        };
        self.table_edit(sheet, &[grown], move |user| {
            let new_name = options.name.trim().to_string();
            let mut current = name.clone();
            if new_name != name {
                user.rename_table(&name, &new_name)?;
                current = new_name;
            }
            user.set_header_row(&current, options.header_row)?;
            user.set_total_row(&current, options.total_row)?;
            let table = user
                .model
                .workbook
                .tables
                .get_mut(&current)
                .ok_or("Table not found")?;
            table.style_info = TableStyleInfo {
                name: options.style_name.clone(),
                show_first_column: options.first_column,
                show_last_column: options.last_column,
                show_row_stripes: options.banded_rows,
                show_column_stripes: options.banded_columns,
            };
            let filter_button = options.filter_button && options.header_row;
            let dropped_filters = table.has_filters && !filter_button;
            table.has_filters = filter_button;
            if dropped_filters {
                user.model
                    .workbook
                    .table_filters
                    .retain(|s| s.table != current);
                user.reapply_table_filters(&current)?;
            }
            Ok(())
        })
    }

    /// Moves a table's edges. Its header row must stay on the same row.
    pub fn resize_table(&mut self, name: &str, new_area: PivotArea) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        if new_area.first_row != area.first_row || new_area.first_column != area.first_column {
            return Err("The headers of a table must stay in the same place".to_string());
        }
        let table = self.table_named(name)?;
        let header = table.header_row_count as i32;
        let totals = table.totals_row_count as i32;
        if new_area.last_row - new_area.first_row + 1 < header + totals + 1 {
            return Err("A table needs at least one row of data".to_string());
        }
        self.check_table_area(sheet, &new_area, Some(name))?;
        let had_totals = totals > 0;
        let name = name.to_string();
        let union = PivotArea {
            first_row: area.first_row,
            first_column: area.first_column,
            last_row: area.last_row.max(new_area.last_row) + 1,
            last_column: area.last_column.max(new_area.last_column),
        };
        self.table_edit(sheet, &[union], move |user| {
            if had_totals {
                user.set_total_row(&name, false)?;
            }
            let table = user
                .model
                .workbook
                .tables
                .get_mut(&name)
                .ok_or("Table not found")?;
            let last_row = new_area.last_row - i32::from(had_totals);
            table.reference = area_reference(&PivotArea {
                last_row,
                ..new_area
            });
            let width = (new_area.last_column - new_area.first_column + 1) as usize;
            table.columns.truncate(width);
            while table.columns.len() < width {
                table.columns.push(TableColumn {
                    id: 0,
                    name: String::new(),
                    ..Default::default()
                });
            }
            let names: HashSet<String> = table.columns.iter().map(|c| c.name.clone()).collect();
            let state = user
                .model
                .workbook
                .table_filters
                .iter_mut()
                .find(|s| s.table == name);
            if let Some(state) = state {
                state.columns.retain(|f| names.contains(&f.column));
            }
            user.model.sync_tables();
            if had_totals {
                user.set_total_row(&name, true)?;
            }
            user.reapply_table_filters(&name)
        })
    }

    /// Sets what a column's total-row cell shows. `None` leaves it empty.
    pub fn set_table_total_function(
        &mut self,
        name: &str,
        column: &str,
        function: Option<String>,
    ) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        let table = self.table_named(name)?;
        if table.totals_row_count == 0 {
            return Err("This table has no total row".to_string());
        }
        let offset = table_column_index(table, column).ok_or("Column not found")?;
        if function
            .as_deref()
            .is_some_and(|f| subtotal_code(f).is_none())
        {
            return Err("Unknown total function".to_string());
        }
        let name = name.to_string();
        let region = PivotArea {
            first_row: area.last_row,
            ..area
        };
        self.table_edit(sheet, &[region], move |user| {
            let table = user
                .model
                .workbook
                .tables
                .get_mut(&name)
                .ok_or("Table not found")?;
            table.columns[offset].totals_row_function = function;
            table.columns[offset].totals_row_label = None;
            let snapshot = table.clone();
            user.write_total_cell(sheet, &area, &snapshot, offset)
        })
    }

    /// Replaces one column's filter, or clears it with `None`.
    pub fn set_table_filter(
        &mut self,
        name: &str,
        column: &str,
        rule: Option<TableFilterRule>,
    ) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        let table = self.table_named(name)?;
        table_column_index(table, column).ok_or("Column not found")?;
        let (first, last) = table_data_rows(table, &area);
        let name = name.to_string();
        let column = column.to_string();
        let region = PivotArea {
            first_row: first,
            last_row: last,
            first_column: area.first_column,
            last_column: area.first_column,
        };
        self.table_edit(sheet, &[region], move |user| {
            let filters = &mut user.model.workbook.table_filters;
            let position = filters.iter().position(|s| s.table == name);
            let index = match position {
                Some(index) => index,
                None => {
                    filters.push(TableFilterState {
                        table: name.clone(),
                        columns: vec![],
                        sort: None,
                    });
                    filters.len() - 1
                }
            };
            let state = &mut filters[index];
            state.columns.retain(|f| f.column != column);
            if let Some(rule) = rule {
                state.columns.push(TableColumnFilter { column, rule });
            }
            if state.columns.is_empty() && state.sort.is_none() {
                filters.remove(index);
            }
            user.reapply_table_filters(&name)
        })
    }

    /// Removes every filter of a table and shows its rows.
    pub fn clear_table_filters(&mut self, name: &str) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        let name = name.to_string();
        self.table_edit(sheet, &[area], move |user| {
            let filters = &mut user.model.workbook.table_filters;
            if let Some(state) = filters.iter_mut().find(|s| s.table == name) {
                state.columns.clear();
            }
            filters.retain(|s| !(s.columns.is_empty() && s.sort.is_none()));
            user.reapply_table_filters(&name)
        })
    }

    /// Rows move whole, and only the rows showing take part, as in Excel.
    pub fn sort_table(&mut self, name: &str, column: &str, descending: bool) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        let table = self.table_named(name)?;
        let offset = table_column_index(table, column).ok_or("Column not found")?;
        let column_name = table.columns[offset].name.clone();
        let (first, last) = table_data_rows(table, &area);
        let name = name.to_string();
        self.table_edit(sheet, &[area], move |user| {
            let worksheet = user.model.workbook.worksheet(sheet)?;
            let mut rows = Vec::new();
            for row in first..=last {
                if !worksheet.is_row_hidden(row)? {
                    rows.push(row);
                }
            }
            let at = area.first_column + offset as i32;
            let mut keyed = Vec::with_capacity(rows.len());
            for row in &rows {
                let cell = user
                    .model
                    .table_filter_cells(sheet, at, *row, *row)?
                    .remove(0);
                keyed.push((*row, cell));
            }
            keyed.sort_by(|a, b| compare_filter_cells(&a.1, &b.1, descending));
            let destinations: BTreeMap<i32, i32> = keyed
                .iter()
                .map(|(row, _)| *row)
                .zip(rows.iter().copied())
                .collect();
            user.remap_comments(sheet, &area, |row, column| {
                Some((destinations.get(&row).copied().unwrap_or(row), column))
            })?;
            let mut moved = Vec::with_capacity(rows.len());
            for (row, _) in &keyed {
                let mut cells = Vec::new();
                for c in area.first_column..=area.last_column {
                    cells.push(user.take_cell_at(sheet, *row, c)?);
                }
                moved.push(cells);
            }
            for (row, cells) in rows.iter().zip(moved) {
                for (c, cell) in (area.first_column..).zip(cells) {
                    user.put_cell_at(sheet, *row, c, cell)?;
                }
            }
            let filters = &mut user.model.workbook.table_filters;
            let sort = Some(TableSort {
                column: column_name,
                descending,
            });
            match filters.iter_mut().find(|s| s.table == name) {
                Some(state) => state.sort = sort,
                None => filters.push(TableFilterState {
                    table: name,
                    columns: vec![],
                    sort,
                }),
            }
            Ok(())
        })
    }

    /// Turns a table back into cells that keep its look.
    pub fn convert_table_to_range(&mut self, name: &str) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        let name = name.to_string();
        self.table_edit(sheet, &[area], move |user| {
            let mut styles = Vec::new();
            for row in area.first_row..=area.last_row {
                for column in area.first_column..=area.last_column {
                    let mut style = user.model.get_style_for_cell(sheet, row, column)?;
                    user.model.apply_table_style(sheet, row, column, &mut style);
                    styles.push((row, column, style));
                }
            }
            for (row, column, style) in styles {
                let index = user.model.workbook.styles.get_style_index_or_create(&style);
                user.model
                    .workbook
                    .worksheet_mut(sheet)?
                    .set_cell_style(row, column, index)?;
            }
            let worksheet = user.model.workbook.worksheet_mut(sheet)?;
            for row in area.first_row..=area.last_row {
                if worksheet.is_row_hidden(row)? {
                    worksheet.set_row_hidden(row, false)?;
                }
            }
            let workbook = &mut user.model.workbook;
            workbook.tables.remove(&name);
            workbook.table_filters.retain(|s| s.table != name);
            workbook.slicers.retain(
                |s| !matches!(&s.source, SlicerSource::Table { table, .. } if *table == name),
            );
            Ok(())
        })
    }

    /// Closes up rows of a table's data, moving what is below them up.
    fn remove_table_rows(&mut self, name: &str, doomed: &BTreeSet<i32>) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        let table = self.table_named(name)?.clone();
        let (first, last) = table_data_rows(&table, &area);
        let mut target = first;
        let mut destinations = BTreeMap::new();
        for row in first..=area.last_row {
            if doomed.contains(&row) {
                continue;
            }
            destinations.insert(row, target);
            if target != row {
                for column in area.first_column..=area.last_column {
                    let cell = self.take_cell_at(sheet, row, column)?;
                    self.put_cell_at(sheet, target, column, cell)?;
                }
            }
            target += 1;
        }
        for row in target..=area.last_row {
            for column in area.first_column..=area.last_column {
                self.take_cell_at(sheet, row, column)?;
            }
        }
        let rows = PivotArea {
            first_row: first,
            ..area
        };
        self.remap_comments(sheet, &rows, |row, column| {
            destinations.get(&row).map(|target| (*target, column))
        })?;
        let remaining = (last - first + 1) - doomed.len() as i32;
        let mut moved = area;
        if remaining < 1 {
            // A table always keeps one empty row of data
            moved.last_row = first + table.totals_row_count as i32;
            if table.totals_row_count > 0 {
                let from = target - 1;
                for column in area.first_column..=area.last_column {
                    let cell = self.take_cell_at(sheet, from, column)?;
                    self.put_cell_at(sheet, moved.last_row, column, cell)?;
                }
            }
        } else {
            moved.last_row -= doomed.len() as i32;
        }
        let table = self
            .model
            .workbook
            .tables
            .get_mut(name)
            .ok_or("Table not found")?;
        table.reference = area_reference(&moved);
        self.model.sync_table_parser();
        self.rewrite_totals(name)?;
        self.reapply_table_filters(name)
    }

    /// Deletes one row of a table. What is beside the table stays.
    pub fn delete_table_row(&mut self, name: &str, row: i32) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        let table = self.table_named(name)?;
        let (first, last) = table_data_rows(table, &area);
        if row < first || row > last {
            return Err("That row is not part of the table's data".to_string());
        }
        let name = name.to_string();
        self.table_edit(sheet, &[area], move |user| {
            user.remove_table_rows(&name, &BTreeSet::from([row]))
        })
    }

    /// Deletes one column of a table. What is above and below it stays.
    pub fn delete_table_column(&mut self, name: &str, column: i32) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        if !(area.first_column..=area.last_column).contains(&column) {
            return Err("That column is not part of the table".to_string());
        }
        if area.first_column == area.last_column {
            return Err("A table needs at least one column".to_string());
        }
        let name = name.to_string();
        self.table_edit(sheet, &[area], move |user| {
            user.remap_comments(sheet, &area, |r, c| match c.cmp(&column) {
                std::cmp::Ordering::Less => Some((r, c)),
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some((r, c - 1)),
            })?;
            for row in area.first_row..=area.last_row {
                user.take_cell_at(sheet, row, column)?;
                for c in column + 1..=area.last_column {
                    let cell = user.take_cell_at(sheet, row, c)?;
                    user.put_cell_at(sheet, row, c - 1, cell)?;
                }
            }
            let table = user
                .model
                .workbook
                .tables
                .get_mut(&name)
                .ok_or("Table not found")?;
            let removed = table.columns.remove((column - area.first_column) as usize);
            table.reference = area_reference(&PivotArea {
                last_column: area.last_column - 1,
                ..area
            });
            let workbook = &mut user.model.workbook;
            for state in workbook
                .table_filters
                .iter_mut()
                .filter(|s| s.table == name)
            {
                state.columns.retain(|f| f.column != removed.name);
                if state
                    .sort
                    .as_ref()
                    .is_some_and(|s| s.column == removed.name)
                {
                    state.sort = None;
                }
            }
            workbook
                .table_filters
                .retain(|s| !(s.columns.is_empty() && s.sort.is_none()));
            workbook.slicers.retain(|s| {
                !matches!(&s.source, SlicerSource::Table { table, column }
                    if *table == name && *column == removed.name)
            });
            user.model.sync_table_parser();
            user.reapply_table_filters(&name)
        })
    }

    /// Inserts a row into a table so that the new row is `row`. Rows beside
    /// the table move too, which keeps every formula pointing at the right cell.
    pub fn insert_table_row(&mut self, name: &str, row: i32) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        let table = self.table_named(name)?;
        let (first, last) = table_data_rows(table, &area);
        if row < first || row > last + 1 {
            return Err("A new row must be inside the table".to_string());
        }
        let extend = row == last + 1 && table.totals_row_count == 0;
        let name = name.to_string();
        let before = self.table_snapshot();
        let mut diffs = vec![self.table_state_diff(sheet, before.clone(), before)];
        self.model.insert_rows(sheet, row, 1)?;
        diffs.push(Diff::InsertRows {
            sheet,
            row,
            count: 1,
        });
        let after_insert = self.table_snapshot();
        if extend {
            let table = self
                .model
                .workbook
                .tables
                .get_mut(&name)
                .ok_or("Table not found")?;
            table.reference = area_reference(&PivotArea {
                last_row: area.last_row + 1,
                ..area
            });
        }
        self.model.sync_table_parser();
        let finished = self.table_snapshot();
        diffs.push(self.table_state_diff(sheet, after_insert, finished));
        self.push_diff_list(diffs);
        self.evaluate_if_not_paused();
        Ok(())
    }

    /// Inserts a column into a table so that the new column is `column`.
    pub fn insert_table_column(&mut self, name: &str, column: i32) -> Result<(), String> {
        let (sheet, area) = self.table_place(name)?;
        if !(area.first_column..=area.last_column).contains(&column) {
            return Err("A new column must be inside the table".to_string());
        }
        let name = name.to_string();
        let before = self.table_snapshot();
        let mut diffs = vec![self.table_state_diff(sheet, before.clone(), before)];
        self.model.insert_columns(sheet, column, 1)?;
        diffs.push(Diff::InsertColumns {
            sheet,
            column,
            count: 1,
        });
        let after_insert = self.table_snapshot();
        if column == area.first_column {
            // The insert pushed the whole table along: take the new column in
            let table = self
                .model
                .workbook
                .tables
                .get_mut(&name)
                .ok_or("Table not found")?;
            table.reference = area_reference(&PivotArea {
                last_column: area.last_column + 1,
                ..area
            });
            table.columns.insert(
                0,
                TableColumn {
                    id: 0,
                    name: String::new(),
                    ..Default::default()
                },
            );
        }
        self.model.sync_table_parser();
        let finished = self.table_snapshot();
        diffs.push(self.table_state_diff(sheet, after_insert, finished));
        self.push_diff_list(diffs);
        self.evaluate_if_not_paused();
        Ok(())
    }

    /// Removes rows repeating an earlier one in `columns`. Returns how many
    /// rows went and how many stayed.
    pub fn remove_table_duplicates(
        &mut self,
        name: &str,
        columns: &[String],
    ) -> Result<(u32, u32), String> {
        let (sheet, area) = self.table_place(name)?;
        let table = self.table_named(name)?;
        let (first, last) = table_data_rows(table, &area);
        let offsets: Vec<usize> = columns
            .iter()
            .filter_map(|c| table_column_index(table, c))
            .collect();
        if offsets.is_empty() {
            return Err("Choose at least one column".to_string());
        }
        let mut seen = HashSet::new();
        let mut doomed = BTreeSet::new();
        for row in first..=last {
            let mut key = Vec::with_capacity(offsets.len());
            for offset in &offsets {
                let at = area.first_column + *offset as i32;
                key.push(
                    self.model
                        .get_formatted_cell_value(sheet, row, at)?
                        .to_lowercase(),
                );
            }
            if !seen.insert(key) {
                doomed.insert(row);
            }
        }
        let removed = doomed.len() as u32;
        let remaining = (last - first + 1) as u32 - removed;
        if removed > 0 {
            let name = name.to_string();
            self.table_edit(sheet, &[area], move |user| {
                user.remove_table_rows(&name, &doomed)
            })?;
        }
        Ok((removed, remaining))
    }

    /// Removes the rows of `area` that repeat an earlier row in `columns`.
    /// Only the cells of `area` move. Returns how many rows went and how many
    /// are left.
    pub fn remove_duplicates(
        &mut self,
        sheet: u32,
        area: PivotArea,
        columns: &[i32],
        has_headers: bool,
    ) -> Result<(u32, u32), String> {
        if columns.is_empty() {
            return Err("Choose at least one column".to_string());
        }
        let worksheet = self.model.workbook.worksheet(sheet)?;
        let sheet_name = worksheet.get_name();
        let dimension = worksheet.dimension();
        let area = PivotArea {
            last_row: area.last_row.min(dimension.max_row),
            last_column: area.last_column.min(dimension.max_column),
            ..area
        };
        let width = area.last_column - area.first_column + 1;
        let height = area.last_row - area.first_row + 1;
        if worksheet
            .merged_cells
            .iter()
            .any(|m| m.intersects(area.first_row, area.first_column, width, height))
        {
            return Err("Can't remove duplicates where there are merged cells.".to_string());
        }
        let in_table = self.model.workbook.tables.values().any(|table| {
            table.sheet_name == sheet_name
                && table_area(table).is_some_and(|place| place.intersects(&area))
        });
        if in_table {
            return Err("Select the data inside the table, or only cells outside it.".to_string());
        }
        for row in area.first_row..=area.last_row {
            for column in area.first_column..=area.last_column {
                if self.get_pivot_table_at(sheet, row, column).is_some() {
                    return Err("Can't remove duplicates in a PivotTable.".to_string());
                }
            }
        }

        let first = area.first_row + i32::from(has_headers);
        let mut seen = HashSet::new();
        let mut doomed = BTreeSet::new();
        for row in first..=area.last_row {
            let mut key = Vec::with_capacity(columns.len());
            for column in columns {
                key.push(
                    self.model
                        .get_formatted_cell_value(sheet, row, *column)?
                        .to_lowercase(),
                );
            }
            if !seen.insert(key) {
                doomed.insert(row);
            }
        }
        let removed = doomed.len() as u32;
        let remaining = (area.last_row - first + 1).max(0) as u32 - removed;
        if removed > 0 {
            self.table_edit(sheet, &[area], move |user| {
                let mut target = first;
                let mut destinations = BTreeMap::new();
                for row in first..=area.last_row {
                    if doomed.contains(&row) {
                        continue;
                    }
                    destinations.insert(row, target);
                    if target != row {
                        for column in area.first_column..=area.last_column {
                            let cell = user.take_cell_at(sheet, row, column)?;
                            user.put_cell_at(sheet, target, column, cell)?;
                        }
                    }
                    target += 1;
                }
                for row in target..=area.last_row {
                    for column in area.first_column..=area.last_column {
                        user.take_cell_at(sheet, row, column)?;
                    }
                }
                let rows = PivotArea {
                    first_row: first,
                    ..area
                };
                user.remap_comments(sheet, &rows, |row, column| {
                    destinations.get(&row).map(|target| (*target, column))
                })
            })?;
        }
        Ok((removed, remaining))
    }

    /// Typing right under a table or right beside it makes the table bigger.
    /// It is its own undo step, so one undo leaves the typed value.
    pub(super) fn auto_expand_table(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        value: &str,
    ) -> Result<(), String> {
        if value.is_empty() {
            return Ok(());
        }
        let mut found = None;
        for table in self.model.workbook.tables.values() {
            let Some(area) = table_area(table) else {
                continue;
            };
            if self.model.table_sheet(table) != Some(sheet) || table.totals_row_count > 0 {
                continue;
            }
            let span_rows = area.first_row..=area.last_row;
            let span_columns = area.first_column..=area.last_column;
            let mut grown = area;
            if row == area.last_row + 1 && span_columns.contains(&column) {
                grown.last_row += 1;
            } else if column == area.last_column + 1 && span_rows.contains(&row) {
                grown.last_column += 1;
            } else {
                continue;
            }
            found = Some((table.name.clone(), area, grown));
            break;
        }
        let Some((name, area, grown)) = found else {
            return Ok(());
        };
        if self.check_table_area(sheet, &grown, Some(&name)).is_err() {
            return Ok(());
        }
        // Only a row or column with nothing else in it joins the table
        let new_part = if grown.last_row > area.last_row {
            PivotArea {
                first_row: grown.last_row,
                ..grown
            }
        } else {
            PivotArea {
                first_column: grown.last_column,
                ..grown
            }
        };
        let worksheet = self.model.workbook.worksheet(sheet)?;
        for r in new_part.first_row..=new_part.last_row {
            for c in new_part.first_column..=new_part.last_column {
                if (r, c) != (row, column)
                    && worksheet
                        .cell(r, c)
                        .is_some_and(|cell| !matches!(&*cell, Cell::EmptyCell { .. }))
                {
                    return Ok(());
                }
            }
        }
        self.table_edit(sheet, &[grown], move |user| {
            let table = user
                .model
                .workbook
                .tables
                .get_mut(&name)
                .ok_or("Table not found")?;
            table.reference = area_reference(&grown);
            if grown.last_column > area.last_column {
                table.columns.push(TableColumn {
                    id: 0,
                    name: String::new(),
                    ..Default::default()
                });
            }
            Ok(())
        })
    }

    fn slicer_position(&self, name: &str) -> Result<usize, String> {
        self.model
            .workbook
            .slicers
            .iter()
            .position(|s| s.name == name)
            .ok_or_else(|| format!("Slicer not found: {name}"))
    }

    fn slicer_edit(&mut self, name: &str, edit: impl FnOnce(&mut Slicer)) -> Result<(), String> {
        let position = self.slicer_position(name)?;
        let sheet_id = self.model.workbook.slicers[position].sheet_id;
        let sheet = self
            .model
            .pivot_sheet_index(sheet_id)
            .ok_or("Sheet not found")?;
        self.table_edit(sheet, &[], move |user| {
            edit(&mut user.model.workbook.slicers[position]);
            Ok(())
        })
    }

    fn pivot_position(&self, link: &SlicerPivot) -> Option<usize> {
        self.model
            .workbook
            .pivot_tables
            .iter()
            .position(|p| p.name == link.name && p.sheet_id == link.sheet_id)
    }

    fn pivot_field_filter<'a>(pivot: &'a PivotTable, field: &str) -> Option<&'a PivotField> {
        pivot
            .rows
            .iter()
            .chain(&pivot.columns)
            .chain(&pivot.filters)
            .chain(&pivot.slicer_filters)
            .find(|f| f.field == field)
    }

    fn pivot_slicer_items(
        &self,
        pivot: &PivotTable,
        field: &str,
    ) -> Result<Vec<TableFilterItem>, String> {
        let source_sheet = self
            .model
            .pivot_sheet_index(pivot.source_sheet_id)
            .ok_or("The source sheet of this PivotTable no longer exists")?;
        let data = self.model.read_pivot_data(source_sheet, &pivot.source)?;
        let index = data.field_index(field).ok_or("Field not found")?;
        let hidden = Self::pivot_field_filter(pivot, field)
            .map(|f| f.hidden_items.clone())
            .unwrap_or_default();
        let others: Vec<(usize, &Vec<String>)> = pivot
            .rows
            .iter()
            .chain(&pivot.columns)
            .chain(&pivot.filters)
            .chain(&pivot.slicer_filters)
            .filter(|f| f.field != field && !f.hidden_items.is_empty())
            .filter_map(|f| Some((data.field_index(&f.field)?, &f.hidden_items)))
            .collect();
        let mut has_data = vec![false; data.fields[index].items.len()];
        for record in &data.records {
            let passes = others.iter().all(|(other, items)| {
                !items.contains(&data.fields[*other].items[record[*other] as usize].label)
            });
            if passes {
                has_data[record[index] as usize] = true;
            }
        }
        let mut items: Vec<(usize, &str)> = data.fields[index]
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| (i, item.label.as_str()))
            .collect();
        items.sort_by_key(|(_, label)| label.to_lowercase());
        Ok(items
            .into_iter()
            .map(|(i, label)| TableFilterItem {
                label: label.to_string(),
                visible: !hidden.iter().any(|h| h == label),
                has_data: has_data[i],
            })
            .collect())
    }

    fn slicer_entry(&self, slicer: &Slicer) -> Option<SlicerEntry> {
        let sheet = self.model.pivot_sheet_index(slicer.sheet_id)?;
        let workbook = &self.model.workbook;
        let (items, source_description, connections) = match &slicer.source {
            SlicerSource::Table { table, column } => {
                let items = self.model.table_filter_items(table, column).ok()?;
                let definition = workbook.tables.get(table)?;
                (
                    items,
                    format!(
                        "{}!{} [{}]",
                        definition.sheet_name, definition.reference, column
                    ),
                    vec![],
                )
            }
            SlicerSource::Pivot { pivots, field } => {
                let first = pivots.iter().find_map(|p| self.pivot_position(p))?;
                let pivot = &workbook.pivot_tables[first];
                let items = self.pivot_slicer_items(pivot, field).ok()?;
                let source_sheet = workbook
                    .worksheet(self.model.pivot_sheet_index(pivot.source_sheet_id)?)
                    .ok()?
                    .get_name();
                let connections = workbook
                    .pivot_tables
                    .iter()
                    .filter(|p| {
                        p.source_sheet_id == pivot.source_sheet_id && p.source == pivot.source
                    })
                    .filter_map(|p| {
                        let index = self.model.pivot_sheet_index(p.sheet_id)?;
                        Some(SlicerConnection {
                            name: p.name.clone(),
                            sheet_id: p.sheet_id,
                            sheet_name: workbook.worksheet(index).ok()?.get_name(),
                            connected: pivots
                                .iter()
                                .any(|l| l.name == p.name && l.sheet_id == p.sheet_id),
                        })
                    })
                    .collect();
                (
                    items,
                    format!("{source_sheet}!{} [{field}]", area_reference(&pivot.source)),
                    connections,
                )
            }
        };
        let filtered = items.iter().any(|i| !i.visible);
        Some(SlicerEntry {
            slicer: slicer.clone(),
            sheet,
            items,
            filtered,
            source_description,
            connections,
        })
    }

    /// The slicers floating over a sheet whose table or pivot still exists.
    pub fn get_slicers(&self, sheet: u32) -> Vec<SlicerEntry> {
        self.model
            .workbook
            .slicers
            .iter()
            .filter_map(|s| self.slicer_entry(s))
            .filter(|e| e.sheet == sheet)
            .collect()
    }

    fn next_slicer_name(&self, base: &str, taken: &[String]) -> String {
        let used = |candidate: &str| {
            self.model
                .workbook
                .slicers
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case(candidate))
                || taken.iter().any(|t| t.eq_ignore_ascii_case(candidate))
        };
        if !used(base) {
            return base.to_string();
        }
        (1..)
            .map(|n| format!("{base} {n}"))
            .find(|candidate| !used(candidate))
            .unwrap_or_else(|| base.to_string())
    }

    fn insert_slicers(
        &mut self,
        sheet: u32,
        anchor: (i32, i32),
        sources: Vec<(String, SlicerSource)>,
    ) -> Result<Vec<String>, String> {
        let sheet_id = self.model.workbook.worksheet(sheet)?.sheet_id;
        let mut names = Vec::new();
        let mut slicers = Vec::new();
        for (index, (field, source)) in sources.into_iter().enumerate() {
            let name = self.next_slicer_name(&field, &names);
            names.push(name.clone());
            slicers.push(Slicer {
                caption: field,
                name,
                show_header: true,
                sheet_id,
                source,
                row: anchor.0,
                column: anchor.1,
                offset_x: 24.0 * index as f64,
                offset_y: 24.0 * index as f64,
                width: SLICER_WIDTH,
                height: SLICER_HEIGHT,
                button_columns: 1,
                button_height: SLICER_BUTTON_HEIGHT,
                placement: SlicerPlacement::Move,
                locked: false,
                print: true,
                style: DEFAULT_SLICER_STYLE.to_string(),
                alt_text: String::new(),
                multi_select: false,
            });
        }
        let result = names.clone();
        self.table_edit(sheet, &[], move |user| {
            user.model.workbook.slicers.extend(slicers);
            Ok(())
        })?;
        Ok(result)
    }

    /// Adds a slicer for each of the given columns of a table.
    pub fn create_table_slicers(
        &mut self,
        name: &str,
        columns: &[String],
    ) -> Result<Vec<String>, String> {
        let (sheet, area) = self.table_place(name)?;
        let table = self.table_named(name)?;
        let mut sources = Vec::new();
        for column in columns {
            let offset = table_column_index(table, column).ok_or("Column not found")?;
            let real = table.columns[offset].name.clone();
            sources.push((
                real.clone(),
                SlicerSource::Table {
                    table: name.to_string(),
                    column: real,
                },
            ));
        }
        self.insert_slicers(sheet, (area.first_row, area.last_column + 2), sources)
    }

    /// Adds a slicer for each of the given fields of a pivot.
    pub fn create_pivot_slicers(
        &mut self,
        index: u32,
        fields: &[String],
    ) -> Result<Vec<String>, String> {
        let pivot = self
            .model
            .workbook
            .pivot_tables
            .get(index as usize)
            .ok_or("PivotTable not found")?;
        let sheet = self
            .model
            .pivot_sheet_index(pivot.sheet_id)
            .ok_or("Sheet not found")?;
        let link = SlicerPivot {
            name: pivot.name.clone(),
            sheet_id: pivot.sheet_id,
        };
        let anchor = match pivot.output {
            Some(area) => (area.first_row, area.last_column + 2),
            None => (pivot.row, pivot.column + 2),
        };
        // A slicer's field must list its items in the file even when it is not
        // part of the layout, so such a pivot is written from its definition.
        let pivot = &mut self.model.workbook.pivot_tables[index as usize];
        for field in fields {
            if Self::pivot_field_filter(pivot, field).is_none() {
                pivot.slicer_filters.push(PivotField {
                    field: field.clone(),
                    ..Default::default()
                });
                pivot.raw = None;
            }
        }
        let sources = fields
            .iter()
            .map(|field| {
                (
                    field.clone(),
                    SlicerSource::Pivot {
                        pivots: vec![link.clone()],
                        field: field.clone(),
                    },
                )
            })
            .collect();
        self.insert_slicers(sheet, anchor, sources)
    }

    /// Replaces what a slicer shows and where it sits. Its source stays.
    pub fn update_slicer(&mut self, name: &str, mut slicer: Slicer) -> Result<(), String> {
        let position = self.slicer_position(name)?;
        let current = self.model.workbook.slicers[position].clone();
        slicer.source = current.source;
        slicer.sheet_id = current.sheet_id;
        slicer.name = current.name;
        slicer.button_columns = slicer.button_columns.max(1);
        slicer.width = slicer.width.max(40.0);
        slicer.height = slicer.height.max(30.0);
        slicer.button_height = slicer.button_height.max(10.0);
        self.slicer_edit(name, move |s| *s = slicer)
    }

    /// Removes a slicer. The filter it set stays, as in Excel.
    pub fn delete_slicer(&mut self, name: &str) -> Result<(), String> {
        let position = self.slicer_position(name)?;
        let sheet_id = self.model.workbook.slicers[position].sheet_id;
        let sheet = self
            .model
            .pivot_sheet_index(sheet_id)
            .ok_or("Sheet not found")?;
        self.table_edit(sheet, &[], move |user| {
            user.model.workbook.slicers.remove(position);
            Ok(())
        })
    }

    /// Links a pivot slicer to the given pivots.
    pub fn set_slicer_connections(
        &mut self,
        name: &str,
        links: Vec<SlicerPivot>,
    ) -> Result<(), String> {
        if links.is_empty() {
            return Err("A slicer must stay connected to a PivotTable".to_string());
        }
        self.slicer_edit(name, move |s| {
            if let SlicerSource::Pivot { pivots, .. } = &mut s.source {
                *pivots = links;
            }
        })
    }

    /// Shows only `selected`. Selecting nothing, or everything, clears the filter.
    pub fn set_slicer_selection(&mut self, name: &str, selected: &[String]) -> Result<(), String> {
        let position = self.slicer_position(name)?;
        let slicer = self.model.workbook.slicers[position].clone();
        let keep: HashSet<&str> = selected.iter().map(String::as_str).collect();
        match &slicer.source {
            SlicerSource::Table { table, column } => {
                let items = self.model.table_filter_items(table, column)?;
                let all = items.iter().all(|i| keep.contains(i.label.as_str()));
                let rule = (!selected.is_empty() && !all).then(|| TableFilterRule::Values {
                    visible: selected.to_vec(),
                });
                self.set_table_filter(table, column, rule)
            }
            SlicerSource::Pivot { pivots, field } => {
                for link in pivots {
                    let Some(index) = self.pivot_position(link) else {
                        continue;
                    };
                    let mut pivot = self.model.workbook.pivot_tables[index].clone();
                    let items = self.pivot_slicer_items(&pivot, field)?;
                    let hidden: Vec<String> = if selected.is_empty() {
                        vec![]
                    } else {
                        items
                            .iter()
                            .filter(|i| !keep.contains(i.label.as_str()))
                            .map(|i| i.label.clone())
                            .collect()
                    };
                    let zones = pivot
                        .rows
                        .iter_mut()
                        .chain(&mut pivot.columns)
                        .chain(&mut pivot.filters)
                        .chain(&mut pivot.slicer_filters)
                        .find(|f| f.field == *field);
                    match zones {
                        Some(zone) => zone.hidden_items = hidden,
                        None => pivot.slicer_filters.push(PivotField {
                            field: field.clone(),
                            hidden_items: hidden,
                            ..Default::default()
                        }),
                    }
                    self.update_pivot_table(index as u32, pivot, false)?;
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::table::{TableFilterOperator, BLANK_FILTER_LABEL};

    fn model() -> UserModel<'static> {
        let mut model = UserModel::new_empty("wb", "en", "UTC", "en").unwrap();
        let rows = [
            ["Item", "Qty", "Kind"],
            ["Apple", "10", "Fruit"],
            ["Banana", "20", "Fruit"],
            ["Carrot", "5", "Veg"],
            ["Banana", "20", "Fruit"],
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

    fn area() -> PivotArea {
        PivotArea {
            first_row: 1,
            first_column: 1,
            last_row: 5,
            last_column: 3,
        }
    }

    fn with_table() -> UserModel<'static> {
        let mut model = model();
        model.create_table(0, area(), true, None).unwrap();
        model
    }

    fn text(model: &UserModel, row: i32, column: i32) -> String {
        model.get_formatted_cell_value(0, row, column).unwrap()
    }

    fn hidden(model: &UserModel, row: i32) -> bool {
        model.get_model().is_row_hidden(0, row).unwrap()
    }

    fn entry(model: &UserModel) -> TableEntry {
        model.get_tables(0).remove(0)
    }

    #[test]
    fn create_names_columns_from_the_headers_and_undoes() {
        let mut model = with_table();
        let table = entry(&model);
        assert_eq!(table.name, "Table1");
        assert_eq!(table.columns[1].name, "Qty");
        assert_eq!(table.reference, "A1:C5");
        model.undo().unwrap();
        assert!(model.get_tables(0).is_empty());
        model.redo().unwrap();
        assert_eq!(entry(&model).columns.len(), 3);
    }

    #[test]
    fn a_range_without_headers_gets_generated_ones() {
        let mut model = model();
        let data = PivotArea {
            first_row: 2,
            ..area()
        };
        model.create_table(0, data, false, None).unwrap();
        assert_eq!(text(&model, 2, 1), "Column1");
        assert_eq!(text(&model, 3, 1), "Apple");
        assert_eq!(entry(&model).reference, "A2:C6");
        model.undo().unwrap();
        assert_eq!(text(&model, 2, 1), "Apple");
    }

    #[test]
    fn tables_cannot_overlap() {
        let mut model = with_table();
        assert!(model.create_table(0, area(), true, None).is_err());
    }

    #[test]
    fn a_values_filter_hides_rows_and_undo_shows_them() {
        let mut model = with_table();
        let rule = TableFilterRule::Values {
            visible: vec!["Apple".to_string()],
        };
        model
            .set_table_filter("Table1", "Item", Some(rule))
            .unwrap();
        assert!(!hidden(&model, 2));
        assert!(hidden(&model, 3));
        assert!(hidden(&model, 4));
        let items = model.get_table_filter_items("Table1", "Item").unwrap();
        assert_eq!(items.len(), 3);
        assert!(items[0].visible && !items[1].visible);
        model.undo().unwrap();
        assert!(!hidden(&model, 3));
        assert!(!entry(&model).filtered);
    }

    #[test]
    fn filters_combine_and_grey_out_items_with_no_rows() {
        let mut model = with_table();
        let rule = TableFilterRule::Condition {
            operator: TableFilterOperator::GreaterThan,
            value: "10".to_string(),
            value2: String::new(),
        };
        model.set_table_filter("Table1", "Qty", Some(rule)).unwrap();
        assert!(hidden(&model, 2));
        assert!(!hidden(&model, 3));
        assert!(hidden(&model, 4));
        let items = model.get_table_filter_items("Table1", "Item").unwrap();
        let carrot = items.iter().find(|i| i.label == "Carrot").unwrap();
        assert!(carrot.visible && !carrot.has_data);
    }

    #[test]
    fn top_and_text_filters() {
        let mut model = with_table();
        let top = TableFilterRule::Top {
            count: 1.0,
            bottom: true,
            percent: false,
        };
        model.set_table_filter("Table1", "Qty", Some(top)).unwrap();
        assert!(hidden(&model, 2) && !hidden(&model, 4));
        let text_rule = TableFilterRule::Condition {
            operator: TableFilterOperator::BeginsWith,
            value: "b".to_string(),
            value2: String::new(),
        };
        model.set_table_filter("Table1", "Qty", None).unwrap();
        model
            .set_table_filter("Table1", "Item", Some(text_rule))
            .unwrap();
        assert!(hidden(&model, 2) && !hidden(&model, 3));
    }

    #[test]
    fn sorting_moves_whole_rows_and_undoes() {
        let mut model = with_table();
        model.sort_table("Table1", "Qty", false).unwrap();
        assert_eq!(text(&model, 2, 1), "Carrot");
        assert_eq!(text(&model, 2, 3), "Veg");
        assert_eq!(entry(&model).sort.unwrap().column, "Qty");
        model.undo().unwrap();
        assert_eq!(text(&model, 2, 1), "Apple");
    }

    #[test]
    fn sorting_leaves_filtered_out_rows_where_they_are() {
        let mut model = with_table();
        let rule = TableFilterRule::Values {
            visible: vec!["Banana".to_string(), "Carrot".to_string()],
        };
        model
            .set_table_filter("Table1", "Item", Some(rule))
            .unwrap();
        model.sort_table("Table1", "Item", true).unwrap();
        assert_eq!(text(&model, 2, 1), "Apple");
        assert_eq!(text(&model, 3, 1), "Carrot");
    }

    #[test]
    fn total_row_adds_a_subtotal_and_undoes() {
        let mut model = with_table();
        let mut options = entry(&model).options;
        options.total_row = true;
        model.set_table_options("Table1", options).unwrap();
        assert_eq!(entry(&model).reference, "A1:C6");
        assert_eq!(text(&model, 6, 1), "Total");
        model
            .set_table_total_function("Table1", "Qty", Some("sum".to_string()))
            .unwrap();
        assert_eq!(text(&model, 6, 2), "55");
        model.undo().unwrap();
        model.undo().unwrap();
        assert_eq!(entry(&model).reference, "A1:C5");
        assert_eq!(text(&model, 6, 1), "");
    }

    #[test]
    fn a_full_row_below_blocks_the_total_row() {
        let mut model = with_table();
        model.set_user_input(0, 6, 2, "x").unwrap();
        // Typing beneath the table took that row in; make room beyond it
        let table = entry(&model);
        assert_eq!(table.reference, "A1:C6");
        model.set_user_input(0, 7, 5, "beside").unwrap();
        model.set_user_input(0, 7, 1, "y").unwrap();
        assert_eq!(entry(&model).reference, "A1:C7");
        model.set_user_input(0, 8, 2, "z").unwrap();
        let mut grown = entry(&model).area;
        grown.last_row = 7;
        model.resize_table("Table1", grown).unwrap();
        let mut options = entry(&model).options;
        options.total_row = true;
        assert!(model.set_table_options("Table1", options).is_err());
        assert_eq!(entry(&model).reference, "A1:C7");
    }

    #[test]
    fn renaming_carries_filters_and_slicers() {
        let mut model = with_table();
        model
            .create_table_slicers("Table1", &["Item".to_string()])
            .unwrap();
        model
            .set_table_filter(
                "Table1",
                "Item",
                Some(TableFilterRule::Values {
                    visible: vec!["Apple".to_string()],
                }),
            )
            .unwrap();
        let mut options = entry(&model).options;
        options.name = "Fruit".to_string();
        model.set_table_options("Table1", options).unwrap();
        assert_eq!(entry(&model).name, "Fruit");
        assert!(entry(&model).filtered);
        assert_eq!(model.get_slicers(0).len(), 1);
    }

    #[test]
    fn bad_table_names_are_refused() {
        let mut model = with_table();
        for bad in ["A1", "two words", "", "1a"] {
            let mut options = entry(&model).options;
            options.name = bad.to_string();
            assert!(model.set_table_options("Table1", options).is_err(), "{bad}");
        }
    }

    #[test]
    fn editing_a_header_renames_the_column() {
        let mut model = with_table();
        model
            .create_table_slicers("Table1", &["Qty".to_string()])
            .unwrap();
        model.set_user_input(0, 1, 2, "Amount").unwrap();
        assert_eq!(entry(&model).columns[1].name, "Amount");
        assert_eq!(model.get_slicers(0).len(), 1);
        model.set_user_input(0, 1, 2, "Item").unwrap();
        assert_eq!(entry(&model).columns[1].name, "Item2");
        assert_eq!(text(&model, 1, 2), "Item2");
    }

    #[test]
    fn typing_under_a_table_extends_it_and_undo_keeps_the_value() {
        let mut model = with_table();
        model.set_user_input(0, 6, 1, "Dill").unwrap();
        assert_eq!(entry(&model).reference, "A1:C6");
        model.undo().unwrap();
        assert_eq!(entry(&model).reference, "A1:C5");
        assert_eq!(text(&model, 6, 1), "Dill");
    }

    #[test]
    fn typing_beside_a_table_adds_a_named_column() {
        let mut model = with_table();
        model.set_user_input(0, 1, 4, "Price").unwrap();
        let table = entry(&model);
        assert_eq!(table.reference, "A1:D5");
        assert_eq!(table.columns[3].name, "Price");
        model.set_user_input(0, 3, 5, "1").unwrap();
        assert_eq!(entry(&model).columns[4].name, "Column1");
        assert_eq!(text(&model, 1, 5), "Column1");
    }

    #[test]
    fn a_table_follows_inserted_and_deleted_rows() {
        let mut model = with_table();
        model.insert_rows(0, 1, 2).unwrap();
        assert_eq!(entry(&model).reference, "A3:C7");
        model.insert_rows(0, 5, 1).unwrap();
        assert_eq!(entry(&model).reference, "A3:C8");
        model.delete_rows(0, 5, 1).unwrap();
        assert_eq!(entry(&model).reference, "A3:C7");
        model.undo().unwrap();
        assert_eq!(entry(&model).reference, "A3:C8");
    }

    #[test]
    fn deleting_all_its_rows_removes_a_table_and_undo_restores_it() {
        let mut model = with_table();
        model.delete_rows(0, 1, 5).unwrap();
        assert!(model.get_tables(0).is_empty());
        model.undo().unwrap();
        assert_eq!(entry(&model).reference, "A1:C5");
    }

    #[test]
    fn a_table_follows_a_renamed_sheet() {
        let mut model = with_table();
        model.rename_sheet(0, "Data").unwrap();
        assert_eq!(entry(&model).name, "Table1");
    }

    #[test]
    fn a_column_inserted_into_a_table_gets_a_name() {
        let mut model = with_table();
        model.insert_table_column("Table1", 2).unwrap();
        let table = entry(&model);
        assert_eq!(table.reference, "A1:D5");
        assert_eq!(table.columns[1].name, "Column1");
        assert_eq!(table.columns[2].name, "Qty");
        model.undo().unwrap();
        let table = entry(&model);
        assert_eq!(table.reference, "A1:C5");
        assert_eq!(table.columns[1].name, "Qty");
        model.redo().unwrap();
        assert_eq!(entry(&model).columns[1].name, "Column1");
    }

    #[test]
    fn a_column_inserted_before_the_first_stays_in_the_table() {
        let mut model = with_table();
        model.insert_table_column("Table1", 1).unwrap();
        let table = entry(&model);
        assert_eq!(table.reference, "A1:D5");
        assert_eq!(table.columns[0].name, "Column1");
        assert_eq!(table.columns[1].name, "Item");
    }

    #[test]
    fn insert_and_delete_rows_inside_a_table() {
        let mut model = with_table();
        model.insert_table_row("Table1", 3).unwrap();
        assert_eq!(entry(&model).reference, "A1:C6");
        assert_eq!(text(&model, 3, 1), "");
        model.delete_table_row("Table1", 3).unwrap();
        assert_eq!(entry(&model).reference, "A1:C5");
        assert_eq!(text(&model, 3, 1), "Banana");
        model.insert_table_row("Table1", 6).unwrap();
        assert_eq!(entry(&model).reference, "A1:C6");
        model.undo().unwrap();
        assert_eq!(entry(&model).reference, "A1:C5");
        model.redo().unwrap();
        assert_eq!(entry(&model).reference, "A1:C6");
    }

    #[test]
    fn deleting_a_table_row_leaves_what_is_beside_it() {
        let mut model = with_table();
        model.set_user_input(0, 3, 6, "beside").unwrap();
        model.delete_table_row("Table1", 3).unwrap();
        assert_eq!(model.get_formatted_cell_value(0, 3, 6).unwrap(), "beside");
    }

    #[test]
    fn deleting_a_table_column_closes_the_gap() {
        let mut model = with_table();
        model.delete_table_column("Table1", 2).unwrap();
        let table = entry(&model);
        assert_eq!(table.reference, "A1:B5");
        assert_eq!(table.columns[1].name, "Kind");
        assert_eq!(text(&model, 2, 2), "Fruit");
        model.undo().unwrap();
        assert_eq!(entry(&model).columns[1].name, "Qty");
        assert_eq!(text(&model, 2, 2), "10");
    }

    #[test]
    fn remove_duplicates_keeps_the_first() {
        let mut model = with_table();
        let columns = ["Item".to_string(), "Qty".to_string()];
        let result = model.remove_table_duplicates("Table1", &columns).unwrap();
        assert_eq!(result, (1, 3));
        assert_eq!(entry(&model).reference, "A1:C4");
        assert_eq!(text(&model, 5, 1), "");
        model.undo().unwrap();
        assert_eq!(text(&model, 5, 1), "Banana");
    }

    #[test]
    fn the_style_is_drawn_over_the_cells_and_bands_follow_filters() {
        let mut model = with_table();
        let fill = |model: &UserModel, row| {
            model
                .get_model()
                .get_extended_style_for_cell(0, row, 1)
                .unwrap()
                .style
                .fill
                .color
        };
        assert_ne!(fill(&model, 1), Color::None);
        assert_ne!(fill(&model, 2), Color::None);
        assert_eq!(fill(&model, 3), Color::None);
        // The stored cell is untouched
        assert_eq!(
            model
                .get_model()
                .get_style_for_cell(0, 2, 1)
                .unwrap()
                .fill
                .color,
            Color::None
        );
        let rule = TableFilterRule::Values {
            visible: vec!["Apple".to_string(), "Carrot".to_string()],
        };
        model
            .set_table_filter("Table1", "Item", Some(rule))
            .unwrap();
        assert_eq!(fill(&model, 4), Color::None);
    }

    #[test]
    fn a_cells_own_fill_wins_over_the_table_style() {
        let mut model = with_table();
        let area = crate::expressions::types::Area {
            sheet: 0,
            row: 2,
            column: 1,
            width: 1,
            height: 1,
        };
        model
            .update_range_style(&area, "fill.color", "#FF0000")
            .unwrap();
        let style = model
            .get_model()
            .get_extended_style_for_cell(0, 2, 1)
            .unwrap()
            .style;
        assert_eq!(style.fill.color, Color::Rgb("#FF0000".to_string()));
    }

    #[test]
    fn convert_to_range_keeps_the_look_and_drops_the_table() {
        let mut model = with_table();
        model.convert_table_to_range("Table1").unwrap();
        assert!(model.get_tables(0).is_empty());
        assert_ne!(
            model
                .get_model()
                .get_style_for_cell(0, 2, 1)
                .unwrap()
                .fill
                .color,
            Color::None
        );
        model.undo().unwrap();
        assert_eq!(model.get_tables(0).len(), 1);
    }

    #[test]
    fn a_slicer_filters_its_table() {
        let mut model = with_table();
        let names = model
            .create_table_slicers("Table1", &["Kind".to_string()])
            .unwrap();
        assert_eq!(names, ["Kind"]);
        let entry = &model.get_slicers(0)[0];
        assert_eq!(entry.items.len(), 2);
        assert!(!entry.filtered);
        assert_eq!(entry.source_description, "Sheet1!A1:C5 [Kind]");
        model
            .set_slicer_selection("Kind", &["Veg".to_string()])
            .unwrap();
        assert!(hidden(&model, 2) && !hidden(&model, 4));
        assert!(model.get_slicers(0)[0].filtered);
        model
            .set_slicer_selection("Kind", &["Fruit".to_string(), "Veg".to_string()])
            .unwrap();
        assert!(!hidden(&model, 2));
        assert!(!model.get_slicers(0)[0].filtered);
    }

    #[test]
    fn slicers_can_be_moved_and_deleted_with_undo() {
        let mut model = with_table();
        model
            .create_table_slicers("Table1", &["Item".to_string(), "Kind".to_string()])
            .unwrap();
        let mut slicer = model.get_slicers(0)[0].slicer.clone();
        slicer.button_columns = 3;
        slicer.row = 9;
        model.update_slicer("Item", slicer).unwrap();
        assert_eq!(model.get_slicers(0)[0].slicer.button_columns, 3);
        model.undo().unwrap();
        assert_eq!(model.get_slicers(0)[0].slicer.button_columns, 1);
        model.delete_slicer("Kind").unwrap();
        assert_eq!(model.get_slicers(0).len(), 1);
        model.undo().unwrap();
        assert_eq!(model.get_slicers(0).len(), 2);
    }

    #[test]
    fn slicers_follow_rows_inserted_above_them() {
        let mut model = with_table();
        model
            .create_table_slicers("Table1", &["Item".to_string()])
            .unwrap();
        model.insert_rows(0, 1, 3).unwrap();
        assert_eq!(model.get_slicers(0)[0].slicer.row, 4);
    }

    #[test]
    fn filters_and_slicers_survive_saving() {
        let mut model = with_table();
        model
            .create_table_slicers("Table1", &["Kind".to_string()])
            .unwrap();
        model
            .set_slicer_selection("Kind", &["Veg".to_string()])
            .unwrap();
        let bytes = model.to_bytes();
        let reopened = UserModel::from_bytes(&bytes, "en").unwrap();
        assert_eq!(reopened.get_slicers(0).len(), 1);
        assert!(reopened.get_slicers(0)[0].filtered);
        assert_eq!(entry(&reopened).name, "Table1");
    }

    #[test]
    fn a_table_alone_keeps_the_plain_saved_format() {
        let model = with_table();
        let bytes = model.to_bytes();
        assert!(!bytes.starts_with(b"ICSHEET3"));
        assert_eq!(
            entry(&UserModel::from_bytes(&bytes, "en").unwrap()).name,
            "Table1"
        );
    }

    #[test]
    fn blank_cells_have_their_own_filter_entry() {
        let mut model = with_table();
        model.set_user_input(0, 3, 3, "").unwrap();
        let items = model.get_table_filter_items("Table1", "Kind").unwrap();
        assert_eq!(items.last().unwrap().label, BLANK_FILTER_LABEL);
    }

    use crate::types::Color;
}
