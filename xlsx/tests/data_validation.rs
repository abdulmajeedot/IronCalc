#![allow(clippy::unwrap_used)]

use std::io::{Cursor, Read};

use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::data_validation::{
    ValidationAlertStyle, ValidationKind, ValidationOperator, ValidationSettings,
};
use ironcalc_base::pivot::PivotArea;
use ironcalc_base::{Model, UserModel};

fn cell(row: i32, column: i32) -> PivotArea {
    PivotArea {
        first_row: row,
        first_column: column,
        last_row: row,
        last_column: column,
    }
}

fn export(model: &UserModel) -> Vec<u8> {
    save_xlsx_to_writer(model.get_model(), Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn import(bytes: &[u8]) -> UserModel<'static> {
    let workbook = load_from_xlsx_bytes(bytes, "book", "en", "UTC").unwrap();
    UserModel::from_model(Model::from_workbook(workbook, "en").unwrap())
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut file = archive.by_name(name).unwrap();
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    text
}

fn with_sheet(bytes: &[u8], sheet: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        if file.is_dir() {
            continue;
        }
        let name = file.name().unwrap().to_string();
        writer.start_file(name.as_str(), options).unwrap();
        if name == "xl/worksheets/sheet1.xml" {
            std::io::Write::write_all(&mut writer, sheet.as_bytes()).unwrap();
        } else {
            std::io::copy(&mut file, &mut writer).unwrap();
        }
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn rules_survive_a_round_trip() {
    let mut model = UserModel::new_empty("book", "en", "UTC", "en").unwrap();
    model.new_sheet().unwrap();
    model.set_selected_sheet(0).unwrap();
    let list = ValidationSettings {
        kind: ValidationKind::List,
        formula1: "cat,dog & bird".to_string(),
        in_cell_dropdown: false,
        input_title: "Pick".to_string(),
        input_message: "One of <these>".to_string(),
        error_style: ValidationAlertStyle::Warning,
        error_title: "Nope".to_string(),
        error_message: "Not on the list".to_string(),
        ..Default::default()
    };
    model
        .set_data_validation(0, cell(1, 1), list.clone())
        .unwrap();
    let number = ValidationSettings {
        kind: ValidationKind::Whole,
        operator: ValidationOperator::NotBetween,
        formula1: "1".to_string(),
        formula2: "=$C$1".to_string(),
        ignore_blank: false,
        show_input: false,
        show_error: false,
        ..Default::default()
    };
    model
        .set_data_validation(0, cell(2, 1), number.clone())
        .unwrap();
    let cross_sheet = ValidationSettings {
        kind: ValidationKind::List,
        formula1: "=Sheet2!$A$1:$A$3".to_string(),
        ..Default::default()
    };
    model
        .set_data_validation(0, cell(3, 1), cross_sheet.clone())
        .unwrap();

    let bytes = export(&model);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<dataValidations count=\"3\">"));
    assert!(sheet.contains("showDropDown=\"1\""));
    assert!(sheet.contains("&quot;cat,dog &amp; bird&quot;"));
    assert!(sheet.find("<sheetData>").unwrap() < sheet.find("<dataValidations").unwrap());

    let mut reopened = import(&bytes);
    let mut expected = list;
    expected.formula1 = "cat,dog & bird".to_string();
    assert_eq!(
        reopened.get_data_validation(0, 1, 1).unwrap().unwrap(),
        expected
    );
    assert_eq!(
        reopened.get_data_validation(0, 2, 1).unwrap().unwrap(),
        number
    );
    assert_eq!(
        reopened.get_data_validation(0, 3, 1).unwrap().unwrap(),
        cross_sheet
    );
}

#[test]
fn excel_ext_rules_are_read() {
    let model = UserModel::new_empty("book", "en", "UTC", "en").unwrap();
    let bytes = export(&model);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml").replace(
        "</worksheet>",
        "<extLst><ext uri=\"{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}\" xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"><x14:dataValidations count=\"1\" xmlns:xm=\"http://schemas.microsoft.com/office/excel/2006/main\"><x14:dataValidation type=\"list\" allowBlank=\"1\" showInputMessage=\"1\" showErrorMessage=\"1\"><x14:formula1><xm:f>Sheet1!$C$1:$C$2</xm:f></x14:formula1><xm:sqref>B2:B4</xm:sqref></x14:dataValidation></x14:dataValidations></ext></extLst></worksheet>",
    );
    let mut reopened = import(&with_sheet(&bytes, &sheet));
    let rule = reopened.get_data_validation(0, 3, 2).unwrap().unwrap();
    assert_eq!(rule.kind, ValidationKind::List);
    assert_eq!(rule.formula1, "=Sheet1!$C$1:$C$2");
    assert!(rule.in_cell_dropdown);
}

#[test]
fn long_texts_are_cut_to_what_excel_allows() {
    let mut model = UserModel::new_empty("book", "en", "UTC", "en").unwrap();
    let list = ValidationSettings {
        kind: ValidationKind::List,
        formula1: "a".to_string(),
        input_title: "t".repeat(50),
        input_message: "m".repeat(400),
        ..Default::default()
    };
    model.set_data_validation(0, cell(1, 1), list).unwrap();
    let mut reopened = import(&export(&model));
    let rule = reopened.get_data_validation(0, 1, 1).unwrap().unwrap();
    assert_eq!(rule.input_title.len(), 32);
    assert_eq!(rule.input_message.len(), 255);
}
