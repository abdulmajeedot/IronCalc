#![allow(clippy::unwrap_used)]

use std::io::{Cursor, Read};

use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::pivot::{
    PivotAggregation, PivotArea, PivotField, PivotOptions, PivotTable, PivotValue,
};
use ironcalc_base::table::{SlicerSource, TableFilterOperator, TableFilterRule, TableOptions};
use ironcalc_base::{Model, UserModel};

fn user_model() -> UserModel<'static> {
    let mut model = UserModel::new_empty("book", "en", "UTC", "en").unwrap();
    let rows = [
        ["Item", "Qty", "Unit Price"],
        ["Apple", "10", "0.5"],
        ["Banana", "20", "0.3"],
        ["Orange", "15", "0.75"],
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

fn table_area() -> PivotArea {
    PivotArea {
        first_row: 1,
        first_column: 1,
        last_row: 4,
        last_column: 3,
    }
}

fn with_table() -> UserModel<'static> {
    let mut model = user_model();
    model.create_table(0, table_area(), true, None).unwrap();
    model
}

fn options(model: &UserModel) -> TableOptions {
    model.get_tables(0)[0].options.clone()
}

fn export(model: &Model) -> Vec<u8> {
    save_xlsx_to_writer(model, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn import(bytes: &[u8]) -> Model<'static> {
    let workbook = load_from_xlsx_bytes(bytes, "book", "en", "UTC").unwrap();
    Model::from_workbook(workbook, "en").unwrap()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut file = archive.by_name(name).unwrap();
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    text
}

fn names(bytes: &[u8]) -> Vec<String> {
    let archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    archive
        .file_names()
        .map(|name| name.unwrap().to_string())
        .collect()
}

fn pivot_definition() -> PivotTable {
    PivotTable {
        name: "PivotTable1".to_string(),
        sheet_id: 0,
        row: 8,
        column: 1,
        source_sheet_id: 0,
        source: table_area(),
        rows: vec![PivotField {
            field: "Item".to_string(),
            ..Default::default()
        }],
        columns: vec![],
        values: vec![PivotValue {
            field: "Qty".to_string(),
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

fn with_pivot(model: &mut UserModel) {
    let mut pivot = pivot_definition();
    let sheet_id = model.get_model().workbook.worksheets[0].sheet_id;
    pivot.sheet_id = sheet_id;
    pivot.source_sheet_id = sheet_id;
    model.create_pivot_table(pivot, None, false).unwrap();
}

#[test]
fn a_table_is_written_and_read_back() {
    let mut model = with_table();
    let mut design = options(&model);
    design.name = "Fruit".to_string();
    design.style_name = Some("TableStyleLight8".to_string());
    design.total_row = true;
    design.banded_columns = true;
    model.set_table_options("Table1", design).unwrap();
    let bytes = export(model.get_model());

    let table = part(&bytes, "xl/tables/table1.xml");
    assert!(table.contains("name=\"Fruit\""));
    assert!(table.contains("ref=\"A1:C5\""));
    assert!(table.contains("totalsRowCount=\"1\""));
    assert!(table.contains("<autoFilter ref=\"A1:C4\""));
    assert!(table.contains("totalsRowLabel=\"Total\""));
    assert!(table.contains("totalsRowFunction=\"sum\""));
    assert!(table.contains("name=\"TableStyleLight8\""));
    assert!(part(&bytes, "[Content_Types].xml").contains("/xl/tables/table1.xml"));
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<tablePart r:id=\"rId1\"/>"));
    assert!(part(&bytes, "xl/worksheets/_rels/sheet1.xml.rels").contains("../tables/table1.xml"));

    let imported = UserModel::from_model(import(&bytes));
    let table = &imported.get_tables(0)[0];
    assert_eq!(table.name, "Fruit");
    assert_eq!(
        table.options.style_name.as_deref(),
        Some("TableStyleLight8")
    );
    assert!(table.options.total_row);
    assert!(table.options.banded_columns);
    assert!(table.options.filter_button);
    assert_eq!(table.columns[2].name, "Unit Price");
    assert_eq!(table.columns[2].totals_function.as_deref(), Some("sum"));
    assert_eq!(table.columns[0].totals_label.as_deref(), Some("Total"));
}

#[test]
fn every_kind_of_filter_survives() {
    let rules = [
        TableFilterRule::Values {
            visible: vec!["Apple".to_string(), "(Blanks)".to_string()],
        },
        TableFilterRule::Condition {
            operator: TableFilterOperator::Contains,
            value: "an".to_string(),
            value2: String::new(),
        },
        TableFilterRule::Condition {
            operator: TableFilterOperator::NotBeginsWith,
            value: "b".to_string(),
            value2: String::new(),
        },
        TableFilterRule::Condition {
            operator: TableFilterOperator::GreaterThan,
            value: "12".to_string(),
            value2: String::new(),
        },
        TableFilterRule::Condition {
            operator: TableFilterOperator::Between,
            value: "1".to_string(),
            value2: "12".to_string(),
        },
        TableFilterRule::Condition {
            operator: TableFilterOperator::NotBetween,
            value: "1".to_string(),
            value2: "12".to_string(),
        },
        TableFilterRule::Condition {
            operator: TableFilterOperator::AboveAverage,
            value: String::new(),
            value2: String::new(),
        },
        TableFilterRule::Top {
            count: 2.0,
            bottom: true,
            percent: false,
        },
    ];
    for rule in rules {
        let mut model = with_table();
        model
            .set_table_filter("Table1", "Item", Some(rule.clone()))
            .unwrap();
        let imported = UserModel::from_model(import(&export(model.get_model())));
        assert_eq!(
            imported.get_table_filter("Table1", "Item"),
            Some(rule.clone()),
            "{rule:?}"
        );
    }
}

#[test]
fn hidden_rows_and_the_sort_survive() {
    let mut model = with_table();
    model.sort_table("Table1", "Qty", true).unwrap();
    model
        .set_table_filter(
            "Table1",
            "Item",
            Some(TableFilterRule::Values {
                visible: vec!["Banana".to_string()],
            }),
        )
        .unwrap();
    let bytes = export(model.get_model());
    assert!(part(&bytes, "xl/tables/table1.xml")
        .contains("<sortCondition descending=\"1\" ref=\"B2:B4\"/>"));
    let imported = UserModel::from_model(import(&bytes));
    let table = &imported.get_tables(0)[0];
    assert_eq!(table.sort.as_ref().unwrap().column, "Qty");
    assert!(table.filtered);
    assert!(!imported.get_model().is_row_hidden(0, 2).unwrap());
    assert!(imported.get_model().is_row_hidden(0, 3).unwrap());
}

#[test]
fn a_table_slicer_is_written_and_read_back() {
    let mut model = with_table();
    model
        .create_table_slicers("Table1", &["Item".to_string()])
        .unwrap();
    let mut slicer = model.get_slicers(0)[0].slicer.clone();
    slicer.button_columns = 3;
    slicer.caption = "Fruit".to_string();
    slicer.style = "SlicerStyleDark2".to_string();
    slicer.row = 2;
    slicer.column = 6;
    slicer.offset_x = 10.0;
    slicer.width = 300.0;
    slicer.height = 120.0;
    model.update_slicer("Item", slicer).unwrap();
    model
        .set_slicer_selection("Item", &["Apple".to_string()])
        .unwrap();
    let bytes = export(model.get_model());

    let workbook = part(&bytes, "xl/workbook.xml");
    assert!(workbook.contains("<definedName name=\"Slicer_Item\">#N/A</definedName>"));
    assert!(workbook.contains("x15:slicerCaches"));
    let rels = part(&bytes, "xl/_rels/workbook.xml.rels");
    assert!(rels.contains("slicerCaches/slicerCache1.xml"));
    let cache = part(&bytes, "xl/slicerCaches/slicerCache1.xml");
    assert!(cache.contains("tableSlicerCache tableId=\"1\" column=\"1\""));
    let slicers = part(&bytes, "xl/slicers/slicer1.xml");
    assert!(slicers.contains("columnCount=\"3\""));
    assert!(slicers.contains("caption=\"Fruit\""));
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<drawing r:id="));
    assert!(sheet.contains("3A4CF648-6AED-40f4-86FF-DC5316D8AED3"));
    assert!(names(&bytes).contains(&"xl/drawings/drawing1.xml".to_string()));
    let table = part(&bytes, "xl/tables/table1.xml");
    assert!(table.contains("<filter val=\"Apple\"/>"));

    let imported = UserModel::from_model(import(&bytes));
    let entries = imported.get_slicers(0);
    let entry = &entries[0];
    assert_eq!(entry.slicer.name, "Item");
    assert_eq!(entry.slicer.caption, "Fruit");
    assert_eq!(entry.slicer.button_columns, 3);
    assert_eq!(entry.slicer.style, "SlicerStyleDark2");
    assert_eq!((entry.slicer.row, entry.slicer.column), (2, 6));
    assert!((entry.slicer.offset_x - 10.0).abs() < 0.5);
    assert!((entry.slicer.width - 300.0).abs() < 0.5);
    assert!((entry.slicer.height - 120.0).abs() < 0.5);
    assert!(entry.filtered);
    assert!(matches!(&entry.slicer.source, SlicerSource::Table { table, .. } if table == "Table1"));
}

#[test]
fn a_pivot_slicer_is_written_and_read_back() {
    let mut model = user_model();
    with_pivot(&mut model);
    model
        .create_pivot_slicers(0, &["Item".to_string()])
        .unwrap();
    model
        .set_slicer_selection("Item", &["Apple".to_string()])
        .unwrap();
    let bytes = export(model.get_model());

    let workbook = part(&bytes, "xl/workbook.xml");
    assert!(workbook.contains("x14:slicerCaches"));
    assert!(part(&bytes, "xl/pivotCache/pivotCacheDefinition1.xml").contains("pivotCacheId=\"1\""));
    let cache = part(&bytes, "xl/slicerCaches/slicerCache1.xml");
    assert!(cache.contains("<tabular pivotCacheId=\"1\">"));
    assert!(cache.contains("name=\"PivotTable1\""));
    assert!(cache.contains(" s=\"0\"/>"));
    assert!(cache.contains(" s=\"1\"/>"));
    assert!(
        part(&bytes, "xl/worksheets/sheet1.xml").contains("A8765BA9-456A-4dab-B4F3-ACF838C121DE")
    );

    let imported = UserModel::from_model(import(&bytes));
    let entries = imported.get_slicers(0);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].filtered);
    let apple = entries[0]
        .items
        .iter()
        .find(|i| i.label == "Apple")
        .unwrap();
    assert!(apple.visible);
    assert_eq!(entries[0].connections[0].name, "PivotTable1");
}

#[test]
fn a_slicer_on_a_field_outside_the_layout_filters_the_pivot() {
    let mut model = user_model();
    with_pivot(&mut model);
    model
        .create_pivot_slicers(0, &["Unit Price".to_string()])
        .unwrap();
    model
        .set_slicer_selection("Unit Price", &["0.5".to_string()])
        .unwrap();
    let output = model.get_pivot_tables()[0].pivot.output.unwrap();
    assert_eq!(output.last_row - output.first_row, 2);

    let bytes = export(model.get_model());
    let table = part(&bytes, "xl/pivotTables/pivotTable1.xml");
    assert!(table.contains("h=\"1\""));
    let imported = UserModel::from_model(import(&bytes));
    assert!(imported.get_slicers(0)[0].filtered);
}

#[test]
fn every_relationship_points_at_a_part_with_a_content_type() {
    let mut model = with_table();
    model
        .create_table_slicers("Table1", &["Item".to_string(), "Qty".to_string()])
        .unwrap();
    let bytes = export(model.get_model());
    let files = names(&bytes);
    let types = part(&bytes, "[Content_Types].xml");
    for file in files.iter().filter(|f| {
        f.starts_with("xl/tables/")
            || f.starts_with("xl/slicers/")
            || f.starts_with("xl/slicerCaches/")
            || f.starts_with("xl/drawings/")
    }) {
        assert!(types.contains(&format!("/{file}")), "{file}");
    }
    let rels = part(&bytes, "xl/worksheets/_rels/sheet1.xml.rels");
    for target in rels.split("Target=\"../").skip(1) {
        let target = target.split('"').next().unwrap();
        assert!(files.contains(&format!("xl/{target}")), "{target}");
    }
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    for id in sheet.split("r:id=\"").skip(1) {
        let id = id.split('"').next().unwrap();
        assert!(rels.contains(&format!("Id=\"{id}\"")), "{id}");
    }
}

#[test]
fn elements_of_a_sheet_are_in_the_order_excel_requires() {
    let mut model = with_table();
    model
        .create_table_slicers("Table1", &["Item".to_string()])
        .unwrap();
    let bytes = export(model.get_model());
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    let drawing = sheet.find("<drawing").unwrap();
    let parts = sheet.find("<tableParts").unwrap();
    let ext = sheet.find("<extLst").unwrap();
    assert!(sheet.find("</sheetData>").unwrap() < drawing);
    assert!(drawing < parts && parts < ext);
}

#[test]
fn a_workbook_with_a_filter_saves_and_reopens_in_the_app_format() {
    let mut model = with_table();
    model
        .set_table_filter(
            "Table1",
            "Item",
            Some(TableFilterRule::Values {
                visible: vec!["Apple".to_string()],
            }),
        )
        .unwrap();
    let reopened = UserModel::from_bytes(&model.to_bytes(), "en").unwrap();
    assert!(reopened.get_tables(0)[0].filtered);
}
