use std::io::BufReader;

use ironcalc_base::protection::{
    add_range, remove_range, CellRange, ProtectionPassword, ProtectionPermissions, SheetProtection,
};
use ironcalc_base::types::Worksheet;

use crate::error::XlsxError;

use super::util::{get_bool, get_bool_false};
use super::xml::XmlNode;

const LAST_ROW: i32 = 1_048_576;
const LAST_COLUMN: i32 = 16_384;

/// For each cell format, whether it is unlocked and whether its formula is hidden.
#[derive(Default)]
pub(super) struct XfProtection {
    formats: Vec<(bool, bool)>,
}

impl XfProtection {
    fn unlocked(&self, style: i32) -> bool {
        usize::try_from(style)
            .ok()
            .and_then(|index| self.formats.get(index))
            .is_some_and(|format| format.0)
    }

    fn hidden(&self, style: i32) -> bool {
        usize::try_from(style)
            .ok()
            .and_then(|index| self.formats.get(index))
            .is_some_and(|format| format.1)
    }

    fn is_unused(&self) -> bool {
        self.formats.iter().all(|format| *format == (false, false))
    }
}

pub(super) fn load_xf_protection<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::read::ZipArchive<R>,
) -> Result<XfProtection, XlsxError> {
    let file = archive.by_name("xl/styles.xml")?;
    let style_sheet = XmlNode::parse(BufReader::new(file))?;
    let mut formats = Vec::new();
    let cell_xfs = style_sheet
        .children()
        .find(|node| node.has_tag_name("cellXfs"));
    for xf in cell_xfs.iter().flat_map(|node| node.children()) {
        let protection = xf.children().find(|node| node.has_tag_name("protection"));
        formats.push(match protection {
            Some(node) => (!get_bool(node, "locked"), get_bool_false(node, "hidden")),
            None => (false, false),
        });
    }
    Ok(XfProtection { formats })
}

/// The cells of a sheet with a property, as ranges that don't overlap.
fn ranges_with(worksheet: &Worksheet, has: impl Fn(i32) -> bool) -> Vec<CellRange> {
    let mut ranges = Vec::new();
    for column in &worksheet.cols {
        if column.style.is_some_and(&has) {
            add_range(&mut ranges, (1, column.min, LAST_ROW, column.max));
        }
    }
    for row in worksheet.rows.iter().filter(|row| row.custom_format) {
        let line = (row.r, 1, row.r, LAST_COLUMN);
        if has(row.s) {
            add_range(&mut ranges, line);
        } else {
            remove_range(&mut ranges, line);
        }
    }
    let mut cells = Vec::new();
    for (row, column, cell) in worksheet.sheet_data.cells() {
        if has(cell.get_style()) {
            cells.push((row, column));
        } else if !ranges.is_empty() {
            remove_range(&mut ranges, (row, column, row, column));
        }
    }
    cells.sort_unstable();
    let mut blocks: Vec<CellRange> = Vec::new();
    let mut open: std::collections::HashMap<(i32, i32), usize> = std::collections::HashMap::new();
    let mut index = 0;
    while index < cells.len() {
        let (row, first_column) = cells[index];
        let mut last_column = first_column;
        index += 1;
        while index < cells.len() && cells[index] == (row, last_column + 1) {
            last_column += 1;
            index += 1;
        }
        let key = (first_column, last_column);
        match open.get(&key) {
            Some(&block) if blocks[block].2 == row - 1 => blocks[block].2 = row,
            _ => {
                open.insert(key, blocks.len());
                blocks.push((row, first_column, row, last_column));
            }
        }
    }
    for block in blocks {
        add_range(&mut ranges, block);
    }
    ranges
}

fn load_password(node: &XmlNode) -> Option<ProtectionPassword> {
    let text = |name: &str| node.attribute(name).map(str::to_string);
    let password = ProtectionPassword {
        legacy_hash: text("password"),
        algorithm: text("algorithmName"),
        hash: text("hashValue"),
        salt: text("saltValue"),
        spin_count: node.attribute("spinCount").and_then(|s| s.parse().ok()),
    };
    (password.legacy_hash.is_some() || password.hash.is_some()).then_some(password)
}

/// The protection of a sheet, if it is protected or has unlocked or hidden cells.
pub(super) fn load_sheet_protection(
    ws: &XmlNode,
    worksheet: &Worksheet,
    formats: &XfProtection,
) -> Option<SheetProtection> {
    let node = ws
        .children()
        .find(|node| node.has_tag_name("sheetProtection"));
    let (unlocked, hidden) = if formats.is_unused() {
        (Vec::new(), Vec::new())
    } else {
        (
            ranges_with(worksheet, |style| formats.unlocked(style)),
            ranges_with(worksheet, |style| formats.hidden(style)),
        )
    };
    let is_protected = node.is_some_and(|node| get_bool_false(node, "sheet"));
    if !is_protected && unlocked.is_empty() && hidden.is_empty() {
        return None;
    }
    let permissions = match node {
        Some(node) => ProtectionPermissions {
            select_locked_cells: !get_bool_false(node, "selectLockedCells"),
            select_unlocked_cells: !get_bool_false(node, "selectUnlockedCells"),
            format_cells: !get_bool(node, "formatCells"),
            format_columns: !get_bool(node, "formatColumns"),
            format_rows: !get_bool(node, "formatRows"),
            insert_columns: !get_bool(node, "insertColumns"),
            insert_rows: !get_bool(node, "insertRows"),
            insert_hyperlinks: !get_bool(node, "insertHyperlinks"),
            delete_columns: !get_bool(node, "deleteColumns"),
            delete_rows: !get_bool(node, "deleteRows"),
            sort: !get_bool(node, "sort"),
            auto_filter: !get_bool(node, "autoFilter"),
            pivot_tables: !get_bool(node, "pivotTables"),
            objects: !get_bool_false(node, "objects"),
            scenarios: !get_bool_false(node, "scenarios"),
        },
        None => ProtectionPermissions::default(),
    };
    Some(SheetProtection {
        sheet_id: worksheet.sheet_id,
        is_protected,
        password: node.filter(|_| is_protected).and_then(load_password),
        permissions,
        unlocked,
        hidden,
    })
}
