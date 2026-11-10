//! Parts of an xlsx file IronCalc doesn't act on but keeps for when the file
//! is saved again: links to other workbooks and macros.
//!
//! What a linked workbook last contained is kept as very hidden sheets named
//! `[Book.xlsx]Sheet1`, after the sheets of the workbook. A formula refers to
//! them like to any other sheet, so ranges and functions just work, and they
//! show in a formula as Excel shows them.

use serde::{Deserialize, Serialize};

use crate::{
    model::Model,
    types::{SheetState, Worksheet},
};

/// A linked workbook, with its parts of the file as they were read.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Default)]
#[serde(default)]
pub struct ExternalLink {
    /// The file name the formulas show. Empty for a link that isn't to a workbook.
    pub display_name: String,
    pub part_xml: String,
    pub rels_xml: Option<String>,
}

/// One part of a file's macros, with its bytes in base64.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Default)]
#[serde(default)]
pub struct MacroPart {
    pub path: String,
    /// When the file named a type for this part in particular.
    pub content_type: Option<String>,
    pub data: String,
}

/// The macros of a workbook. They are kept, and never run.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Default)]
#[serde(default)]
pub struct MacroProject {
    pub parts: Vec<MacroPart>,
    pub workbook_code_name: Option<String>,
    /// The name macros know each sheet by, by sheet id.
    pub sheet_code_names: Vec<(u32, String)>,
}

/// Whether a sheet only holds what a linked workbook last contained.
pub fn is_external_cache_sheet(worksheet: &Worksheet) -> bool {
    worksheet.state == SheetState::VeryHidden && worksheet.name.starts_with('[')
}

impl Model<'_> {
    /// Where the sheets of linked workbooks start: they stay after the others.
    pub(crate) fn external_cache_start(&self) -> usize {
        self.workbook
            .worksheets
            .iter()
            .position(is_external_cache_sheet)
            .unwrap_or(self.workbook.worksheets.len())
    }
}
