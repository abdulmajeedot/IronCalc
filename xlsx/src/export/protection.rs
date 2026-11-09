use std::collections::HashMap;

use ironcalc_base::protection::{remove_range, CellRange, SheetProtection};
use ironcalc_base::types::{Cell, Col, Row, Workbook, Worksheet};

use super::escape::escape_xml;

const LAST_ROW: i32 = 1_048_576;
const LAST_COLUMN: i32 = 16_384;
const DEFAULT_COLUMN_WIDTH: f64 = 10.0;
const DEFAULT_ROW_HEIGHT: f64 = 16.0;

/// A copy of a workbook where locked and hidden are in the cell formats, which
/// is where a file keeps them, and what each new format says.
pub(crate) struct ProtectedWorkbook {
    pub(crate) workbook: Workbook,
    pub(crate) formats: HashMap<i32, (bool, bool)>,
}

struct Formats<'a> {
    workbook_xfs: &'a mut Vec<ironcalc_base::types::CellXfs>,
    variants: HashMap<(i32, bool, bool), i32>,
    formats: HashMap<i32, (bool, bool)>,
}

impl Formats<'_> {
    /// The format that is `style` with this protection.
    fn variant(&mut self, style: i32, locked: bool, hidden: bool) -> i32 {
        if locked && !hidden {
            return style;
        }
        if let Some(index) = self.variants.get(&(style, locked, hidden)) {
            return *index;
        }
        let Some(mut xf) = self
            .workbook_xfs
            .get(style as usize)
            .or(self.workbook_xfs.first())
            .cloned()
        else {
            return style;
        };
        xf.apply_protection = true;
        let index = self.workbook_xfs.len() as i32;
        self.workbook_xfs.push(xf);
        self.variants.insert((style, locked, hidden), index);
        self.formats.insert(index, (locked, hidden));
        index
    }
}

fn contains(ranges: &[CellRange], row: i32, column: i32) -> bool {
    ranges
        .iter()
        .any(|&(r1, c1, r2, c2)| (r1..=r2).contains(&row) && (c1..=c2).contains(&column))
}

/// The whole columns and rows of a list that are mostly in it. They are
/// written as column and row formats, with the odd cell as an exception.
struct Lines {
    columns: Vec<CellRange>,
    rows: Vec<CellRange>,
}

fn spans(boundaries: impl Iterator<Item = (i32, i32)>) -> Vec<(i32, i32)> {
    let mut cuts: Vec<i32> = boundaries.flat_map(|(a, b)| [a, b + 1]).collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2).map(|pair| (pair[0], pair[1] - 1)).collect()
}

fn lines_of(list: &[CellRange]) -> Lines {
    let mut columns = Vec::new();
    for (first, last) in spans(list.iter().map(|range| (range.1, range.3))) {
        let covered: i64 = list
            .iter()
            .filter(|range| range.1 <= first && last <= range.3)
            .map(|range| i64::from(range.2 - range.0 + 1))
            .sum();
        if covered > i64::from(LAST_ROW / 2) {
            columns.push((1, first, LAST_ROW, last));
        }
    }
    let mut rest = list.to_vec();
    for column in &columns {
        remove_range(&mut rest, *column);
    }
    let mut rows = Vec::new();
    for (first, last) in spans(rest.iter().map(|range| (range.0, range.2))) {
        let covered: i64 = rest
            .iter()
            .filter(|range| range.0 <= first && last <= range.2)
            .map(|range| i64::from(range.3 - range.1 + 1))
            .sum();
        if covered > i64::from(LAST_COLUMN / 2) {
            rows.push((first, 1, last, LAST_COLUMN));
        }
    }
    Lines { columns, rows }
}

fn apply_to_columns(
    worksheet: &mut Worksheet,
    unlocked: &Lines,
    hidden: &Lines,
    formats: &mut Formats,
) {
    if unlocked.columns.is_empty() && hidden.columns.is_empty() {
        return;
    }
    let mut cuts = vec![1, LAST_COLUMN + 1];
    for col in &worksheet.cols {
        cuts.extend([col.min, col.max + 1]);
    }
    for range in unlocked.columns.iter().chain(&hidden.columns) {
        cuts.extend([range.1, range.3 + 1]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let mut cols: Vec<Col> = Vec::new();
    for pair in cuts.windows(2) {
        let (min, max) = (pair[0], pair[1] - 1);
        let existing = worksheet
            .cols
            .iter()
            .find(|col| col.min <= min && max <= col.max);
        let locked = !contains(&unlocked.columns, 1, min);
        let is_hidden = contains(&hidden.columns, 1, min);
        if locked && !is_hidden {
            if let Some(col) = existing {
                cols.push(Col {
                    min,
                    max,
                    ..col.clone()
                });
            }
            continue;
        }
        let style = existing.and_then(|col| col.style).unwrap_or(0);
        let style = formats.variant(style, locked, is_hidden);
        cols.push(match existing {
            Some(col) => Col {
                min,
                max,
                style: Some(style),
                ..col.clone()
            },
            None => Col {
                min,
                max,
                width: DEFAULT_COLUMN_WIDTH,
                custom_width: false,
                hidden: false,
                style: Some(style),
            },
        });
    }
    worksheet.cols = cols;
}

fn apply_to_rows(
    worksheet: &mut Worksheet,
    unlocked: &Lines,
    hidden: &Lines,
    formats: &mut Formats,
) {
    let mut lines: Vec<i32> = unlocked
        .rows
        .iter()
        .chain(&hidden.rows)
        .flat_map(|range| range.0..=range.2)
        .collect();
    lines.sort_unstable();
    lines.dedup();
    for line in lines {
        let locked = !contains(&unlocked.rows, line, 1);
        let is_hidden = contains(&hidden.rows, line, 1);
        match worksheet.rows.iter_mut().find(|row| row.r == line) {
            Some(row) => {
                row.s = formats.variant(row.s, locked, is_hidden);
                row.custom_format = true;
            }
            None => {
                let s = formats.variant(0, locked, is_hidden);
                worksheet.rows.push(Row {
                    r: line,
                    height: DEFAULT_ROW_HEIGHT,
                    custom_format: true,
                    custom_height: false,
                    s,
                    hidden: false,
                });
            }
        }
    }
}

/// Where a blank cell can differ from what its row or column format says.
fn cells_to_check(
    protection: &SheetProtection,
    unlocked: &Lines,
    hidden: &Lines,
    styled_rows: &[i32],
) -> Vec<CellRange> {
    let mut regions = Vec::new();
    for (list, lines) in [
        (&protection.unlocked, unlocked),
        (&protection.hidden, hidden),
    ] {
        let mut inside = list.clone();
        for line in lines.columns.iter().chain(&lines.rows) {
            remove_range(&mut inside, *line);
        }
        regions.extend(inside);
        for line in lines.columns.iter().chain(&lines.rows) {
            let mut gaps = vec![*line];
            for range in list {
                remove_range(&mut gaps, *range);
            }
            regions.extend(gaps);
        }
    }
    for &row in styled_rows {
        for column in unlocked.columns.iter().chain(&hidden.columns) {
            regions.push((row, column.1, row, column.3));
        }
    }
    regions
}

fn apply_to_cells(
    worksheet: &mut Worksheet,
    source: &Worksheet,
    protection: &SheetProtection,
    unlocked: &Lines,
    hidden: &Lines,
    formats: &mut Formats,
) {
    let filled: Vec<(i32, i32, i32)> = worksheet
        .sheet_data
        .cells()
        .map(|(row, column, cell)| (row, column, cell.get_style()))
        .collect();
    for (row, column, style) in filled {
        let locked = !contains(&protection.unlocked, row, column);
        let is_hidden = contains(&protection.hidden, row, column);
        let new_style = formats.variant(style, locked, is_hidden);
        if new_style != style {
            if let Some(cell) = worksheet.sheet_data.cell_mut(row, column) {
                cell.set_style(new_style);
            }
        }
    }
    let formatted_row = |row: i32| source.rows.iter().find(|r| r.r == row && r.custom_format);
    let styled_rows: Vec<i32> = source
        .rows
        .iter()
        .filter(|row| row.custom_format)
        .map(|row| row.r)
        .chain(
            unlocked
                .rows
                .iter()
                .chain(&hidden.rows)
                .flat_map(|range| range.0..=range.2),
        )
        .collect();
    let styled: std::collections::HashSet<i32> = styled_rows.iter().copied().collect();
    // A row format beats a column format, and a cell format beats both.
    let implied = |row: i32, column: i32| -> (bool, bool) {
        if styled.contains(&row) {
            (
                contains(&unlocked.rows, row, 1),
                contains(&hidden.rows, row, 1),
            )
        } else {
            (
                contains(&unlocked.columns, 1, column),
                contains(&hidden.columns, 1, column),
            )
        }
    };
    let base_style = |row: i32, column: i32| -> i32 {
        match formatted_row(row) {
            Some(formatted) => formatted.s,
            None => source
                .cols
                .iter()
                .find(|col| col.min <= column && column <= col.max)
                .and_then(|col| col.style)
                .unwrap_or(0),
        }
    };
    for region in cells_to_check(protection, unlocked, hidden, &styled_rows) {
        for row in region.0..=region.2 {
            for column in region.1..=region.3 {
                if worksheet.cell(row, column).is_some() {
                    continue;
                }
                let is_unlocked = contains(&protection.unlocked, row, column);
                let is_hidden = contains(&protection.hidden, row, column);
                if (is_unlocked, is_hidden) == implied(row, column) {
                    continue;
                }
                let s = formats.variant(base_style(row, column), !is_unlocked, is_hidden);
                worksheet
                    .sheet_data
                    .set_cell(row, column, Cell::EmptyCell { s });
            }
        }
    }
}

/// `None` when no sheet has locked or hidden cells to write.
pub(crate) fn with_cell_protection(source: &Workbook) -> Option<ProtectedWorkbook> {
    if source
        .sheet_protections
        .iter()
        .all(|protection| protection.unlocked.is_empty() && protection.hidden.is_empty())
    {
        return None;
    }
    let mut workbook = source.clone();
    let mut xfs = std::mem::take(&mut workbook.styles.cell_xfs);
    let mut formats = Formats {
        workbook_xfs: &mut xfs,
        variants: HashMap::new(),
        formats: HashMap::new(),
    };
    for protection in &source.sheet_protections {
        let Some(worksheet) = workbook
            .worksheets
            .iter_mut()
            .find(|worksheet| worksheet.sheet_id == protection.sheet_id)
        else {
            continue;
        };
        let source_sheet = worksheet.clone();
        let unlocked = lines_of(&protection.unlocked);
        let hidden = lines_of(&protection.hidden);
        apply_to_columns(worksheet, &unlocked, &hidden, &mut formats);
        apply_to_rows(worksheet, &unlocked, &hidden, &mut formats);
        apply_to_cells(
            worksheet,
            &source_sheet,
            protection,
            &unlocked,
            &hidden,
            &mut formats,
        );
    }
    let formats = formats.formats;
    workbook.styles.cell_xfs = xfs;
    Some(ProtectedWorkbook { workbook, formats })
}

/// The `<protection>` element of a cell format, empty for the default.
pub(crate) fn get_format_protection_xml(locked: bool, hidden: bool) -> String {
    let locked = if locked { "" } else { r#" locked="0""# };
    let hidden = if hidden { r#" hidden="1""# } else { "" };
    format!("<protection{locked}{hidden}/>")
}

/// The `<sheetProtection>` element of a sheet, empty if it isn't protected.
pub(crate) fn get_sheet_protection_xml(workbook: &Workbook, worksheet: &Worksheet) -> String {
    let Some(protection) = workbook
        .sheet_protections
        .iter()
        .find(|protection| protection.sheet_id == worksheet.sheet_id && protection.is_protected)
    else {
        return String::new();
    };
    let mut attributes = String::from(r#" sheet="1""#);
    if let Some(password) = &protection.password {
        let mut add = |name: &str, value: &str| {
            attributes.push_str(&format!(r#" {name}="{}""#, escape_xml(value)));
        };
        if let Some(value) = &password.legacy_hash {
            add("password", value);
        }
        if let Some(value) = &password.algorithm {
            add("algorithmName", value);
        }
        if let Some(value) = &password.hash {
            add("hashValue", value);
        }
        if let Some(value) = &password.salt {
            add("saltValue", value);
        }
        if let Some(value) = password.spin_count {
            add("spinCount", &value.to_string());
        }
    }
    let allowed = &protection.permissions;
    // These are protected unless a file says otherwise.
    for (name, allow) in [
        ("formatCells", allowed.format_cells),
        ("formatColumns", allowed.format_columns),
        ("formatRows", allowed.format_rows),
        ("insertColumns", allowed.insert_columns),
        ("insertRows", allowed.insert_rows),
        ("insertHyperlinks", allowed.insert_hyperlinks),
        ("deleteColumns", allowed.delete_columns),
        ("deleteRows", allowed.delete_rows),
        ("sort", allowed.sort),
        ("autoFilter", allowed.auto_filter),
        ("pivotTables", allowed.pivot_tables),
    ] {
        if allow {
            attributes.push_str(&format!(r#" {name}="0""#));
        }
    }
    // And these are open unless a file says otherwise.
    for (name, allow) in [
        ("objects", allowed.objects),
        ("scenarios", allowed.scenarios),
        ("selectLockedCells", allowed.select_locked_cells),
        ("selectUnlockedCells", allowed.select_unlocked_cells),
    ] {
        if !allow {
            attributes.push_str(&format!(r#" {name}="1""#));
        }
    }
    format!("<sheetProtection{attributes}/>")
}
