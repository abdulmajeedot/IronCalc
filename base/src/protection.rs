//! Sheet protection: which sheets are protected, what they still allow, and
//! which cells are unlocked or have their formula hidden.
//!
//! As in Excel, whether a cell is locked is independent of the sheet being
//! protected, and it only has an effect once it is.

use base64::{engine::general_purpose::STANDARD, Engine};
use bitcode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha384, Sha512};

use crate::{
    data_validation::{displaced_range, ranges_after_block, subtract_range},
    expressions::parser::stringify::DisplaceData,
    model::Model,
};

/// First row, first column, last row, last column.
pub type CellRange = (i32, i32, i32, i32);

/// What a change refused by the protection of a sheet fails with.
pub const PROTECTED_SHEET_ERROR: &str =
    "This sheet is protected. To change it, unprotect the sheet first.";

const NEW_PASSWORD_ALGORITHM: &str = "SHA-512";
const NEW_PASSWORD_SPIN_COUNT: u32 = 100_000;
const NEW_PASSWORD_SALT_BYTES: usize = 16;

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn random_byte() -> u8 {
    rand::random()
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn random_byte() -> u8 {
    (js_sys::Math::random() * 256.0) as u8
}

/// What people can still do on a protected sheet.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone)]
#[serde(default)]
pub struct ProtectionPermissions {
    pub select_locked_cells: bool,
    pub select_unlocked_cells: bool,
    pub format_cells: bool,
    pub format_columns: bool,
    pub format_rows: bool,
    pub insert_columns: bool,
    pub insert_rows: bool,
    pub insert_hyperlinks: bool,
    pub delete_columns: bool,
    pub delete_rows: bool,
    pub sort: bool,
    pub auto_filter: bool,
    pub pivot_tables: bool,
    pub objects: bool,
    pub scenarios: bool,
}

impl Default for ProtectionPermissions {
    fn default() -> Self {
        ProtectionPermissions {
            select_locked_cells: true,
            select_unlocked_cells: true,
            format_cells: false,
            format_columns: false,
            format_rows: false,
            insert_columns: false,
            insert_rows: false,
            insert_hyperlinks: false,
            delete_columns: false,
            delete_rows: false,
            sort: false,
            auto_filter: false,
            pivot_tables: false,
            objects: false,
            scenarios: false,
        }
    }
}

/// A password in the forms an xlsx file keeps it. The password itself is never
/// stored.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Default)]
#[serde(default)]
pub struct ProtectionPassword {
    /// The 16 bit hash older files use, in hexadecimal.
    pub legacy_hash: Option<String>,
    pub algorithm: Option<String>,
    pub hash: Option<String>,
    pub salt: Option<String>,
    pub spin_count: Option<u32>,
}

fn iterated_hash<D: Digest>(password: &[u8], salt: &[u8], spin_count: u32) -> Vec<u8> {
    let mut hash = D::new()
        .chain_update(salt)
        .chain_update(password)
        .finalize()
        .to_vec();
    for iteration in 0..spin_count {
        hash = D::new()
            .chain_update(&hash)
            .chain_update(iteration.to_le_bytes())
            .finalize()
            .to_vec();
    }
    hash
}

fn password_hash(
    algorithm: &str,
    password: &str,
    salt: &[u8],
    spin_count: u32,
) -> Result<Vec<u8>, String> {
    let bytes: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
    match algorithm {
        "SHA-512" => Ok(iterated_hash::<Sha512>(&bytes, salt, spin_count)),
        "SHA-384" => Ok(iterated_hash::<Sha384>(&bytes, salt, spin_count)),
        "SHA-256" => Ok(iterated_hash::<Sha256>(&bytes, salt, spin_count)),
        _ => Err(format!(
            "Passwords hashed with {algorithm} are not supported"
        )),
    }
}

/// The 16 bit hash of the `password` attribute.
pub fn legacy_password_hash(password: &str) -> u16 {
    let mut hash: u32 = 0;
    let mut length: u32 = 0;
    for (index, unit) in password.encode_utf16().enumerate() {
        let shifted = u32::from(unit) << (index + 1);
        hash ^= (shifted & 0x7fff) | (shifted >> 15);
        length += 1;
    }
    ((hash ^ length ^ 0xCE4B) & 0xffff) as u16
}

impl ProtectionPassword {
    pub fn new(password: &str) -> Result<ProtectionPassword, String> {
        let salt: Vec<u8> = (0..NEW_PASSWORD_SALT_BYTES)
            .map(|_| random_byte())
            .collect();
        let hash = password_hash(
            NEW_PASSWORD_ALGORITHM,
            password,
            &salt,
            NEW_PASSWORD_SPIN_COUNT,
        )?;
        Ok(ProtectionPassword {
            legacy_hash: None,
            algorithm: Some(NEW_PASSWORD_ALGORITHM.to_string()),
            hash: Some(STANDARD.encode(hash)),
            salt: Some(STANDARD.encode(salt)),
            spin_count: Some(NEW_PASSWORD_SPIN_COUNT),
        })
    }

    /// Whether `password` is this password. An error means it can't be told.
    pub fn matches(&self, password: &str) -> Result<bool, String> {
        if let (Some(algorithm), Some(hash)) = (&self.algorithm, &self.hash) {
            let unreadable = |_| "The password of this sheet can't be read".to_string();
            let salt = match &self.salt {
                Some(salt) => STANDARD.decode(salt).map_err(unreadable)?,
                None => Vec::new(),
            };
            let expected = STANDARD.decode(hash).map_err(unreadable)?;
            let spin_count = self.spin_count.unwrap_or(0);
            return Ok(password_hash(algorithm, password, &salt, spin_count)? == expected);
        }
        if let Some(legacy) = &self.legacy_hash {
            return match u16::from_str_radix(legacy, 16) {
                Ok(expected) => Ok(legacy_password_hash(password) == expected),
                Err(_) => Err("The password of this sheet can't be read".to_string()),
            };
        }
        Ok(true)
    }
}

/// Everything about the protection of one sheet.
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone, Default)]
#[serde(default)]
pub struct SheetProtection {
    pub sheet_id: u32,
    pub is_protected: bool,
    pub password: Option<ProtectionPassword>,
    pub permissions: ProtectionPermissions,
    /// Cells are locked unless they are in one of these. They don't overlap.
    pub unlocked: Vec<CellRange>,
    /// Cells whose formula doesn't show on a protected sheet. They don't overlap.
    pub hidden: Vec<CellRange>,
}

impl SheetProtection {
    fn is_unused(&self) -> bool {
        !self.is_protected && self.unlocked.is_empty() && self.hidden.is_empty()
    }
}

/// What the app needs to know about a sheet's protection.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct SheetProtectionInfo {
    pub is_protected: bool,
    pub has_password: bool,
    pub permissions: ProtectionPermissions,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct CellProtection {
    pub locked: bool,
    pub hidden: bool,
}

/// Adds a range to a list of ranges that don't overlap.
pub fn add_range(ranges: &mut Vec<CellRange>, range: CellRange) {
    remove_range(ranges, range);
    ranges.push(range);
}

/// Takes a range out of a list of ranges that don't overlap.
pub fn remove_range(ranges: &mut Vec<CellRange>, range: CellRange) {
    *ranges = ranges
        .iter()
        .flat_map(|existing| subtract_range(*existing, range))
        .collect();
}

fn contains(ranges: &[CellRange], row: i32, column: i32) -> bool {
    ranges
        .iter()
        .any(|&(r1, c1, r2, c2)| (r1..=r2).contains(&row) && (c1..=c2).contains(&column))
}

/// Whether every cell of `area` is in `ranges`.
pub(crate) fn covers(ranges: &[CellRange], area: CellRange) -> bool {
    let mut uncovered = vec![area];
    for range in ranges {
        uncovered = uncovered
            .iter()
            .flat_map(|piece| subtract_range(*piece, *range))
            .collect();
        if uncovered.is_empty() {
            return true;
        }
    }
    uncovered.is_empty()
}

impl Model<'_> {
    pub(crate) fn sheet_protection(&self, sheet: u32) -> Option<&SheetProtection> {
        let sheet_id = self.workbook.worksheets.get(sheet as usize)?.sheet_id;
        self.workbook
            .sheet_protections
            .iter()
            .find(|protection| protection.sheet_id == sheet_id)
    }

    pub(crate) fn active_protection(&self, sheet: u32) -> Option<&SheetProtection> {
        self.sheet_protection(sheet)
            .filter(|protection| protection.is_protected)
    }

    /// Installs what is known about a sheet's protection, by its id.
    pub(crate) fn set_sheet_protection(&mut self, sheet_id: u32, value: Option<SheetProtection>) {
        let list = &mut self.workbook.sheet_protections;
        list.retain(|protection| protection.sheet_id != sheet_id);
        if let Some(value) = value.filter(|value| !value.is_unused()) {
            list.push(value);
        }
    }

    pub fn is_sheet_protected(&self, sheet: u32) -> bool {
        self.active_protection(sheet).is_some()
    }

    pub fn get_cell_protection(&self, sheet: u32, row: i32, column: i32) -> CellProtection {
        match self.sheet_protection(sheet) {
            Some(protection) => CellProtection {
                locked: !contains(&protection.unlocked, row, column),
                hidden: contains(&protection.hidden, row, column),
            },
            None => CellProtection {
                locked: true,
                hidden: false,
            },
        }
    }

    /// Whether the protection of the sheet lets every cell of `area` change.
    pub fn is_area_editable(&self, sheet: u32, area: CellRange) -> bool {
        match self.active_protection(sheet) {
            Some(protection) => covers(&protection.unlocked, area),
            None => true,
        }
    }

    /// Keeps locked and hidden cells on their cells when rows, columns or
    /// blocks of cells move.
    pub(crate) fn displace_sheet_protection(&mut self, sheet: u32, data: &DisplaceData) {
        let Some(sheet_id) = self
            .workbook
            .worksheets
            .get(sheet as usize)
            .map(|worksheet| worksheet.sheet_id)
        else {
            return;
        };
        let moved = |ranges: &[CellRange]| -> Vec<CellRange> {
            ranges
                .iter()
                .flat_map(|range| ranges_after_block(*range, data))
                .filter_map(|range| displaced_range(range, data, sheet))
                .collect()
        };
        for protection in &mut self.workbook.sheet_protections {
            if protection.sheet_id == sheet_id {
                protection.unlocked = moved(&protection.unlocked);
                protection.hidden = moved(&protection.hidden);
            }
        }
    }
}
