#![allow(clippy::unwrap_used)]

use std::io::{Cursor, Read};

use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::pivot::PivotArea;
use ironcalc_base::protection::{ProtectionPermissions, PROTECTED_SHEET_ERROR};
use ironcalc_base::{Model, UserModel};

fn model() -> UserModel<'static> {
    UserModel::new_empty("book", "en", "UTC", "en").unwrap()
}

fn area(first_row: i32, first_column: i32, last_row: i32, last_column: i32) -> PivotArea {
    PivotArea {
        first_row,
        first_column,
        last_row,
        last_column,
    }
}

fn export(model: &UserModel) -> Vec<u8> {
    save_xlsx_to_writer(model.get_model(), Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut file = archive.by_name(name).unwrap();
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    text
}

fn sheet_xml(bytes: &[u8]) -> String {
    part(bytes, "xl/worksheets/sheet1.xml")
}

fn import(bytes: &[u8]) -> UserModel<'static> {
    let workbook = load_from_xlsx_bytes(bytes, "book", "en", "UTC").unwrap();
    UserModel::from_model(Model::from_workbook(workbook, "en").unwrap())
}

#[test]
fn an_unprotected_workbook_writes_no_protection() {
    let bytes = export(&model());
    assert!(!sheet_xml(&bytes).contains("sheetProtection"));
    assert!(!part(&bytes, "xl/styles.xml").contains("<protection"));
}

#[test]
fn protection_comes_right_after_the_cells() {
    let mut model = model();
    model
        .merge_cells(&ironcalc_base::expressions::types::Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 2,
            height: 1,
        })
        .unwrap();
    model
        .protect_sheet(0, None, ProtectionPermissions::default())
        .unwrap();
    let xml = sheet_xml(&export(&model));
    let protection = xml.find("<sheetProtection").unwrap();
    assert!(xml.find("</sheetData>").unwrap() < protection);
    assert!(protection < xml.find("<mergeCells").unwrap());
}

#[test]
fn permissions_are_written_the_way_excel_reads_them() {
    let mut model = model();
    let permissions = ProtectionPermissions {
        format_cells: true,
        sort: true,
        select_locked_cells: false,
        ..Default::default()
    };
    model.protect_sheet(0, None, permissions.clone()).unwrap();
    let bytes = export(&model);
    let xml = sheet_xml(&bytes);
    assert!(xml.contains(r#"sheet="1""#), "{xml}");
    assert!(xml.contains(r#"formatCells="0""#), "{xml}");
    assert!(xml.contains(r#"sort="0""#), "{xml}");
    assert!(xml.contains(r#"objects="1""#), "{xml}");
    assert!(xml.contains(r#"scenarios="1""#), "{xml}");
    assert!(xml.contains(r#"selectLockedCells="1""#), "{xml}");
    assert!(!xml.contains("insertRows"), "{xml}");
    assert_eq!(
        import(&bytes).get_sheet_protection(0).permissions,
        permissions
    );
}

#[test]
fn a_password_survives_and_still_works() {
    let mut model = model();
    model
        .protect_sheet(0, Some("hunter2"), ProtectionPermissions::default())
        .unwrap();
    let bytes = export(&model);
    let xml = sheet_xml(&bytes);
    assert!(xml.contains(r#"algorithmName="SHA-512""#), "{xml}");
    assert!(xml.contains(r#"spinCount="100000""#), "{xml}");
    assert!(!xml.contains("hunter2"));

    let mut reopened = import(&bytes);
    assert!(reopened.get_sheet_protection(0).has_password);
    assert!(reopened.unprotect_sheet(0, Some("nope")).is_err());
    reopened.unprotect_sheet(0, Some("hunter2")).unwrap();
}

#[test]
fn a_password_from_an_older_file_is_kept_and_checked() {
    let mut model = model();
    model
        .protect_sheet(0, None, ProtectionPermissions::default())
        .unwrap();
    let bytes = export(&model);
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.as_slice())).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        let name = file.name().unwrap().to_string();
        let mut text = Vec::new();
        file.read_to_end(&mut text).unwrap();
        if name == "xl/worksheets/sheet1.xml" {
            text = String::from_utf8(text)
                .unwrap()
                .replace(
                    r#"<sheetProtection sheet="1""#,
                    r#"<sheetProtection sheet="1" password="CBEB""#,
                )
                .into_bytes();
        }
        out.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut out, &text).unwrap();
    }
    let edited = out.finish().unwrap().into_inner();

    let mut reopened = import(&edited);
    assert!(reopened.get_sheet_protection(0).has_password);
    assert!(sheet_xml(&export(&reopened)).contains(r#"password="CBEB""#));
    assert!(reopened.unprotect_sheet(0, Some("wrong")).is_err());
    reopened.unprotect_sheet(0, Some("test")).unwrap();
}

#[test]
fn unlocked_and_hidden_cells_round_trip() {
    let mut model = model();
    model.set_user_input(0, 1, 1, "kept").unwrap();
    model.set_user_input(0, 2, 2, "=1+1").unwrap();
    model
        .set_cell_protection(0, area(2, 2, 3, 3), Some(false), None)
        .unwrap();
    model
        .set_cell_protection(0, area(1, 1, 1, 1), None, Some(true))
        .unwrap();
    model
        .protect_sheet(0, None, ProtectionPermissions::default())
        .unwrap();
    let bytes = export(&model);
    let styles = part(&bytes, "xl/styles.xml");
    assert!(styles.contains(r#"<protection locked="0"/>"#), "{styles}");
    assert!(styles.contains(r#"<protection hidden="1"/>"#), "{styles}");
    assert!(styles.contains(r#"applyProtection="1""#), "{styles}");

    let mut reopened = import(&bytes);
    assert!(reopened.is_cell_editable(0, 2, 2));
    assert!(reopened.is_cell_editable(0, 3, 3));
    assert!(!reopened.is_cell_editable(0, 1, 2));
    assert!(reopened.get_cell_protection(0, 1, 1).hidden);
    assert_eq!(reopened.get_cell_content(0, 1, 1).unwrap(), "");
    assert_eq!(reopened.get_cell_content(0, 2, 2).unwrap(), "=1+1");
    assert_eq!(
        reopened.set_user_input(0, 1, 2, "x").unwrap_err(),
        PROTECTED_SHEET_ERROR
    );
    reopened.set_user_input(0, 3, 2, "fine").unwrap();
}

#[test]
fn a_cell_keeps_its_look_when_it_is_unlocked() {
    let mut model = model();
    let cell = ironcalc_base::expressions::types::Area {
        sheet: 0,
        row: 1,
        column: 1,
        width: 1,
        height: 1,
    };
    model.set_user_input(0, 1, 1, "bold").unwrap();
    model.update_range_style(&cell, "font.b", "true").unwrap();
    model
        .set_cell_protection(0, area(1, 1, 1, 1), Some(false), None)
        .unwrap();
    let reopened = import(&export(&model));
    assert!(reopened.get_cell_style(0, 1, 1).unwrap().font.b);
    assert!(!reopened.get_cell_protection(0, 1, 1).locked);
}

#[test]
fn whole_columns_and_rows_stay_small_in_the_file() {
    let mut model = model();
    model
        .set_cell_protection(0, area(1, 2, 1_048_576, 3), Some(false), None)
        .unwrap();
    model
        .set_cell_protection(0, area(5, 1, 5, 16_384), Some(false), None)
        .unwrap();
    let bytes = export(&model);
    let xml = sheet_xml(&bytes);
    assert!(xml.len() < 4_000, "{}", xml.len());
    assert!(xml.contains(r#"<col min="2" max="3""#), "{xml}");
    assert!(xml.contains(r#"customFormat="1""#), "{xml}");

    let reopened = import(&bytes);
    assert!(!reopened.get_cell_protection(0, 900_000, 2).locked);
    assert!(!reopened.get_cell_protection(0, 5, 200).locked);
    assert!(reopened.get_cell_protection(0, 6, 1).locked);
}
