#![allow(clippy::unwrap_used)]

use std::io::{Cursor, Read};

use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::pivot::PivotArea;
use ironcalc_base::table::{TableFilterOperator, TableFilterRule};
use ironcalc_base::{Model, UserModel};

fn sales() -> (UserModel<'static>, String) {
    let mut model = UserModel::new_empty("book", "en", "UTC", "en").unwrap();
    for (row, values) in [
        ["Region", "Qty"],
        ["North", "4"],
        ["South", "12"],
        ["North", "8"],
    ]
    .iter()
    .enumerate()
    {
        for (column, value) in values.iter().enumerate() {
            model
                .set_user_input(0, row as i32 + 2, column as i32 + 2, value)
                .unwrap();
        }
    }
    let cell = PivotArea {
        first_row: 2,
        first_column: 2,
        last_row: 2,
        last_column: 2,
    };
    model.toggle_sheet_filter(0, cell).unwrap();
    let name = model.get_sheet_filter(0).unwrap().name;
    (model, name)
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

#[test]
fn a_filter_survives_a_round_trip() {
    let (mut model, name) = sales();
    model
        .set_table_filter(
            &name,
            "B",
            Some(TableFilterRule::Values {
                visible: vec!["North".to_string()],
            }),
        )
        .unwrap();
    model.sort_table(&name, "C", true).unwrap();

    let bytes = export(&model);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<autoFilter ref=\"B2:C5\"><filterColumn colId=\"0\">"));
    assert!(sheet.contains("<sortState ref=\"B3:C5\">"));
    assert!(sheet.find("</sheetData>").unwrap() < sheet.find("<autoFilter").unwrap());
    let workbook = part(&bytes, "xl/workbook.xml");
    assert!(workbook.contains(
        "name=\"_xlnm._FilterDatabase\" localSheetId=\"0\" hidden=\"1\">&apos;Sheet1&apos;!$B$2:$C$5<"
    ));

    let reopened = import(&bytes);
    let info = reopened.get_sheet_filter(0).unwrap();
    assert_eq!(info.area.last_row, 5);
    assert!(info.filtered);
    let markers = reopened.get_table_markers(0);
    assert!(markers[0].filtered);
    assert_eq!(markers[1].sort_descending, Some(true));
    assert_eq!(
        reopened.get_table_filter(&info.name, "B"),
        Some(TableFilterRule::Values {
            visible: vec!["North".to_string()]
        })
    );
    // The hidden name isn't kept as an ordinary one, so there is never a second
    let again = export(&reopened);
    assert_eq!(
        part(&again, "xl/workbook.xml")
            .matches("_xlnm._FilterDatabase")
            .count(),
        1
    );
}

#[test]
fn a_condition_survives_a_round_trip() {
    let (mut model, name) = sales();
    let rule = TableFilterRule::Condition {
        operator: TableFilterOperator::GreaterThan,
        value: "5".to_string(),
        value2: String::new(),
    };
    model
        .set_table_filter(&name, "C", Some(rule.clone()))
        .unwrap();
    let reopened = import(&export(&model));
    let info = reopened.get_sheet_filter(0).unwrap();
    assert_eq!(reopened.get_table_filter(&info.name, "C"), Some(rule));
}

#[test]
fn no_filter_writes_nothing() {
    let (mut model, _) = sales();
    let cell = PivotArea {
        first_row: 2,
        first_column: 2,
        last_row: 2,
        last_column: 2,
    };
    model.toggle_sheet_filter(0, cell).unwrap();
    let bytes = export(&model);
    assert!(!part(&bytes, "xl/worksheets/sheet1.xml").contains("autoFilter"));
    assert!(!part(&bytes, "xl/workbook.xml").contains("_FilterDatabase"));
}
