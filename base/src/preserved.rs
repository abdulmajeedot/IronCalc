//! Parts of an xlsx file IronCalc doesn't act on but keeps for when the file
//! is saved again: links to other workbooks and macros.
//!
//! What a linked workbook last contained is kept as very hidden sheets named
//! `[Book.xlsx]Sheet1`, after the sheets of the workbook. A formula refers to
//! them like to any other sheet, so ranges and functions just work, and they
//! show in a formula as Excel shows them.

use serde::{Deserialize, Serialize};

use std::collections::BTreeSet;

use crate::{
    expressions::token::{get_error_by_english_name, Error},
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
    /// For a link to a document on the backend, which one.
    pub document_id: Option<String>,
    /// And the version of it that was last read.
    pub version: Option<i64>,
}

/// A value read from a linked document.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(untagged)]
pub enum LinkedValue {
    Number(f64),
    Boolean(bool),
    Text(String),
    Error { error: String },
}

/// The filled cells of one sheet of a linked document.
#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct LinkedSheet {
    pub name: String,
    pub cells: Vec<(i32, i32, LinkedValue)>,
}

/// A workbook that formulas name, and what is known of it. Without a document
/// id, nothing has been read for it yet.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct ExternalBook {
    pub name: String,
    pub document_id: Option<String>,
    pub version: Option<i64>,
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

    /// The workbook names in `'[Name]Sheet'!` references of the formulas.
    fn book_names_in_formulas(&self) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        let formulas = self
            .workbook
            .worksheets
            .iter()
            .flat_map(|worksheet| worksheet.shared_formulas.iter())
            .chain(self.workbook.defined_names.iter().map(|name| &name.formula));
        for formula in formulas {
            let mut rest = formula.as_str();
            while let Some(start) = rest.find("'[") {
                let after = &rest[start + 2..];
                match after.find(']') {
                    Some(end) => {
                        if !after[..end].contains('\'') {
                            names.insert(after[..end].to_string());
                        }
                        rest = &after[end..];
                    }
                    None => break,
                }
            }
        }
        names
    }

    /// The workbooks formulas name that can be read from the backend. Links
    /// that came in an Excel file are left out.
    pub fn external_books(&self) -> Vec<ExternalBook> {
        let links = &self.workbook.external_links;
        let mut books: Vec<ExternalBook> = links
            .iter()
            .filter(|link| link.document_id.is_some())
            .map(|link| ExternalBook {
                name: link.display_name.clone(),
                document_id: link.document_id.clone(),
                version: link.version,
            })
            .collect();
        for name in self.book_names_in_formulas() {
            if !links.iter().any(|link| link.display_name == name) {
                books.push(ExternalBook {
                    name,
                    document_id: None,
                    version: None,
                });
            }
        }
        books
    }

    /// Replaces what is known of a linked document and recalculates.
    pub fn set_linked_workbook(
        &mut self,
        name: &str,
        document_id: &str,
        version: i64,
        sheets: Vec<LinkedSheet>,
    ) -> Result<(), String> {
        if name.is_empty() || name.contains(['[', ']', '\'']) {
            return Err(format!("Invalid workbook name: '{name}'"));
        }
        let prefix = format!("[{name}]");
        self.workbook.worksheets.retain(|worksheet| {
            !(is_external_cache_sheet(worksheet) && worksheet.name.starts_with(&prefix))
        });
        for sheet in sheets {
            let sheet_id = self.get_new_sheet_id();
            let mut worksheet =
                Model::new_empty_worksheet(&format!("{prefix}{}", sheet.name), sheet_id);
            worksheet.state = SheetState::VeryHidden;
            self.workbook.worksheets.push(worksheet);
            let index = self.workbook.worksheets.len() as u32 - 1;
            for (row, column, value) in sheet.cells {
                match value {
                    LinkedValue::Number(number) => self
                        .workbook
                        .worksheet_mut(index)?
                        .set_cell_with_number(row, column, number, 0)?,
                    LinkedValue::Boolean(boolean) => self
                        .workbook
                        .worksheet_mut(index)?
                        .set_cell_with_boolean(row, column, boolean, 0)?,
                    LinkedValue::Text(text) => {
                        self.set_cell_with_string(index, row, column, &text, 0)?
                    }
                    LinkedValue::Error { error } => {
                        let error = get_error_by_english_name(&error).unwrap_or(Error::ERROR);
                        self.workbook
                            .worksheet_mut(index)?
                            .set_cell_with_error(row, column, error, 0)?
                    }
                }
            }
        }
        let links = &mut self.workbook.external_links;
        let position = links.iter().position(|link| link.display_name == name);
        let link = ExternalLink {
            display_name: name.to_string(),
            document_id: Some(document_id.to_string()),
            version: Some(version),
            ..Default::default()
        };
        match position {
            Some(position) => links[position] = link,
            None => links.push(link),
        }
        self.reset_parsed_structures();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn sales() -> Vec<LinkedSheet> {
        vec![
            LinkedSheet {
                name: "Sheet1".to_string(),
                cells: vec![
                    (1, 1, LinkedValue::Number(10.0)),
                    (2, 1, LinkedValue::Number(32.0)),
                    (1, 2, LinkedValue::Text("widget".to_string())),
                    (2, 2, LinkedValue::Boolean(true)),
                    (
                        3,
                        1,
                        LinkedValue::Error {
                            error: "#DIV/0!".to_string(),
                        },
                    ),
                ],
            },
            LinkedSheet {
                name: "Q1 Totals".to_string(),
                cells: vec![(1, 1, LinkedValue::Number(7.0))],
            },
        ]
    }

    fn model() -> Model<'static> {
        Model::new_empty("book", "en", "UTC", "en").unwrap()
    }

    fn value(model: &Model, row: i32, column: i32) -> String {
        model.get_formatted_cell_value(0, row, column).unwrap()
    }

    fn enter(model: &mut Model, row: i32, column: i32, text: &str) {
        model
            .set_user_input(0, row, column, text.to_string())
            .unwrap();
    }

    #[test]
    fn a_formula_names_a_workbook_before_it_is_linked() {
        let mut model = model();
        enter(&mut model, 1, 1, "='[Sales]Sheet1'!A1*2");
        model.evaluate();
        assert_eq!(value(&model, 1, 1), "#REF!");
        assert_eq!(
            model.external_books(),
            vec![ExternalBook {
                name: "Sales".to_string(),
                document_id: None,
                version: None,
            }]
        );
    }

    #[test]
    fn linking_makes_formulas_calculate() {
        let mut model = model();
        enter(&mut model, 1, 1, "='[Sales]Sheet1'!A1*2");
        enter(&mut model, 2, 1, "=SUM('[Sales]Sheet1'!A1:A2)");
        enter(&mut model, 3, 1, "='[Sales]Sheet1'!B1");
        enter(&mut model, 4, 1, "='[Sales]Q1 Totals'!A1");
        enter(&mut model, 5, 1, "='[Sales]Sheet1'!B2");
        enter(&mut model, 6, 1, "='[Sales]Sheet1'!A3");
        model.evaluate();
        model
            .set_linked_workbook("Sales", "doc-1", 4, sales())
            .unwrap();
        assert_eq!(value(&model, 1, 1), "20");
        assert_eq!(value(&model, 2, 1), "42");
        assert_eq!(value(&model, 3, 1), "widget");
        assert_eq!(value(&model, 4, 1), "7");
        assert_eq!(value(&model, 5, 1), "TRUE");
        assert_eq!(value(&model, 6, 1), "#DIV/0!");
        assert_eq!(
            model.external_books(),
            vec![ExternalBook {
                name: "Sales".to_string(),
                document_id: Some("doc-1".to_string()),
                version: Some(4),
            }]
        );
    }

    #[test]
    fn a_newer_version_replaces_the_old_values() {
        let mut model = model();
        enter(&mut model, 1, 1, "='[Sales]Sheet1'!A1");
        enter(&mut model, 2, 1, "='[Sales]Sheet1'!A2");
        model
            .set_linked_workbook("Sales", "doc-1", 1, sales())
            .unwrap();
        assert_eq!(value(&model, 2, 1), "32");
        let newer = vec![LinkedSheet {
            name: "Sheet1".to_string(),
            cells: vec![(1, 1, LinkedValue::Number(99.0))],
        }];
        model
            .set_linked_workbook("Sales", "doc-1", 2, newer)
            .unwrap();
        assert_eq!(value(&model, 1, 1), "99");
        assert_eq!(value(&model, 2, 1), "0");
        assert_eq!(model.workbook.external_links.len(), 1);
        assert_eq!(
            model
                .workbook
                .worksheets
                .iter()
                .filter(|w| is_external_cache_sheet(w))
                .count(),
            1
        );
    }

    #[test]
    fn linked_sheets_stay_out_of_sight_and_last() {
        let mut model = model();
        model
            .set_linked_workbook("Sales", "doc-1", 1, sales())
            .unwrap();
        assert_eq!(model.get_worksheets_properties().len(), 1);
        let (_, index) = model.new_sheet();
        assert_eq!(index, 1);
        assert_eq!(model.get_worksheets_properties().len(), 2);
    }

    #[test]
    fn a_link_survives_saving() {
        let mut model = model();
        enter(&mut model, 1, 1, "='[Sales]Sheet1'!A1");
        model
            .set_linked_workbook("Sales", "doc-1", 3, sales())
            .unwrap();
        let reopened = Model::from_bytes(&model.to_bytes(), "en").unwrap();
        assert_eq!(value(&reopened, 1, 1), "10");
        assert_eq!(reopened.external_books()[0].version, Some(3));
    }

    #[test]
    fn a_bad_name_is_refused() {
        let mut model = model();
        assert!(model
            .set_linked_workbook("a]b", "d", 1, Vec::new())
            .is_err());
        assert!(model.set_linked_workbook("", "d", 1, Vec::new()).is_err());
    }
}
