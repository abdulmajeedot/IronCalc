#![allow(clippy::unwrap_used)]

use std::io::{Cursor, Read};

use ironcalc::base::pivot::{
    PivotAggregation, PivotArea, PivotField, PivotLabelCondition, PivotLabelFilter, PivotOptions,
    PivotRaw, PivotShowAs, PivotSort, PivotSortOrder, PivotSubtotals, PivotTable, PivotTopUnit,
    PivotValue, PivotValueFilter, PivotValueRule,
};
use ironcalc::base::types::Color;
use ironcalc::base::{Model, UserModel};
use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;

fn field(name: &str) -> PivotField {
    PivotField {
        field: name.to_string(),
        hidden_items: vec![],
        ..Default::default()
    }
}

fn user_model() -> UserModel<'static> {
    let mut model = UserModel::new_empty("model", "en", "UTC", "en").unwrap();
    let rows = [
        ["Region", "Product", "Units"],
        ["East", "Chair", "1"],
        ["East", "Desk", "2"],
        ["West", "Chair", "3"],
        ["West", "Lamp & Shade", "4"],
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

fn definition() -> PivotTable {
    PivotTable {
        name: "Sales".to_string(),
        sheet_id: 1,
        row: 3,
        column: 1,
        source_sheet_id: 1,
        source: PivotArea {
            first_row: 1,
            first_column: 1,
            last_row: 5,
            last_column: 3,
        },
        rows: vec![field("Region")],
        columns: vec![field("Product")],
        values: vec![PivotValue {
            field: "Units".to_string(),
            aggregation: PivotAggregation::Average,
            name: String::new(),
            show_as: Default::default(),
            base_field: String::new(),
        }],
        filters: vec![],
        options: PivotOptions {
            subtotals: PivotSubtotals::Bottom,
            ..PivotOptions::default()
        },
        output: None,
        output_map: None,
        style_name: String::new(),
        read_only: false,
        raw: None,
    }
}

fn export(model: &Model) -> Vec<u8> {
    let cursor = save_xlsx_to_writer(model, Cursor::new(Vec::new())).unwrap();
    cursor.into_inner()
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

fn import(bytes: &[u8]) -> Model<'static> {
    let workbook = load_from_xlsx_bytes(bytes, "model", "en", "UTC").unwrap();
    Model::from_workbook(workbook, "en").unwrap()
}

#[test]
fn a_new_pivot_survives_export_and_import() {
    let mut model = user_model();
    model
        .create_pivot_table(definition(), Some("Report"), false)
        .unwrap();
    let bytes = export(model.get_model());

    let workbook_xml = part(&bytes, "xl/workbook.xml");
    assert!(workbook_xml.contains("<pivotCache cacheId=\"1\""));
    assert!(part(&bytes, "[Content_Types].xml").contains("/xl/pivotTables/pivotTable1.xml"));
    assert!(part(&bytes, "xl/worksheets/_rels/sheet2.xml.rels").contains("pivotTable1.xml"));
    let cache = part(&bytes, "xl/pivotCache/pivotCacheDefinition1.xml");
    assert!(cache.contains("refreshOnLoad=\"1\""));
    assert!(cache.contains("sheet=\"Sheet1\""));
    assert!(cache.contains("Lamp &amp; Shade"));

    let imported = import(&bytes);
    let pivot = &imported.workbook.pivot_tables[0];
    assert_eq!(pivot.name, "Sales");
    assert!(!pivot.read_only);
    assert_eq!(
        pivot.sheet_id,
        model.get_model().workbook.worksheets[1].sheet_id
    );
    assert_eq!(pivot.rows, vec![field("Region")]);
    assert_eq!(pivot.columns, vec![field("Product")]);
    assert_eq!(pivot.values[0].aggregation, PivotAggregation::Average);
    assert_eq!(pivot.values[0].name, "");
    assert_eq!(pivot.options.subtotals, PivotSubtotals::Bottom);
    assert_eq!(
        pivot.output,
        model.get_pivot_tables()[0].pivot.output,
        "the output area comes back from the file"
    );
}

#[test]
fn hidden_items_and_filters_survive_export_and_import() {
    let mut model = user_model();
    let mut pivot = definition();
    pivot.columns.clear();
    pivot.rows[0].hidden_items = vec!["West".to_string()];
    pivot.filters = vec![PivotField {
        field: "Product".to_string(),
        hidden_items: vec!["Desk".to_string()],
        ..Default::default()
    }];
    model
        .create_pivot_table(pivot, Some("Report"), false)
        .unwrap();

    let imported = import(&export(model.get_model()));
    let pivot = &imported.workbook.pivot_tables[0];
    assert_eq!(pivot.rows[0].hidden_items, vec!["West".to_string()]);
    assert_eq!(pivot.filters[0].hidden_items, vec!["Desk".to_string()]);
    assert_eq!(pivot.row, 3);
}

#[test]
fn an_imported_pivot_keeps_its_xml_until_edited() {
    let mut model = user_model();
    model
        .create_pivot_table(definition(), Some("Report"), false)
        .unwrap();
    let bytes = export(model.get_model());
    let original = part(&bytes, "xl/pivotTables/pivotTable1.xml");

    let imported = import(&bytes);
    assert!(matches!(
        imported.workbook.pivot_tables[0].raw.as_deref(),
        Some(PivotRaw { .. })
    ));
    let again = export(&imported);
    assert_eq!(part(&again, "xl/pivotTables/pivotTable1.xml"), original);
}

#[test]
fn an_imported_pivot_follows_a_moved_output_and_renamed_source() {
    let mut model = user_model();
    model
        .create_pivot_table(definition(), Some("Report"), false)
        .unwrap();
    let bytes = export(model.get_model());

    let mut imported = UserModel::from_model(import(&bytes));
    imported.insert_rows(1, 1, 2).unwrap();
    imported.rename_sheet(0, "Data").unwrap();
    let again = export(imported.get_model());
    assert!(part(&again, "xl/pivotTables/pivotTable1.xml").contains("<location ref=\"A5:"));
    let cache = part(&again, "xl/pivotCache/pivotCacheDefinition1.xml");
    assert!(cache.contains("sheet=\"Data\""));
    assert!(cache.contains("refreshOnLoad=\"1\""));
}

#[test]
fn a_pivot_without_fields_is_not_written() {
    let mut model = user_model();
    let mut pivot = definition();
    pivot.rows.clear();
    pivot.columns.clear();
    pivot.values.clear();
    model
        .create_pivot_table(pivot, Some("Report"), false)
        .unwrap();
    assert!(!part(&export(model.get_model()), "xl/workbook.xml").contains("pivotCache"));
}

#[test]
fn a_file_with_a_broken_pivot_still_opens() {
    let mut model = user_model();
    model
        .create_pivot_table(definition(), Some("Report"), false)
        .unwrap();
    let bytes = export(model.get_model());

    let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        if file.is_dir() {
            continue;
        }
        let name = file.name().unwrap().to_string();
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).unwrap();
        out.start_file(name.clone(), options).unwrap();
        if name.ends_with("pivotTable1.xml") {
            std::io::Write::write_all(&mut out, b"<not-closed").unwrap();
        } else {
            std::io::Write::write_all(&mut out, &contents).unwrap();
        }
    }
    let broken = out.finish().unwrap().into_inner();
    let imported = import(&broken);
    assert!(imported.workbook.pivot_tables.is_empty());
    assert_eq!(imported.workbook.worksheets.len(), 2);
}

/// What Excel or WPS saves: the same pivot, with plain cells and no indents.
fn without_cell_formatting(bytes: &[u8]) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        if file.is_dir() {
            continue;
        }
        let name = file.name().unwrap().to_string();
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).unwrap();
        if name == "xl/worksheets/sheet2.xml" {
            let mut text = String::from_utf8(contents).unwrap();
            while let Some(start) = text.find(" s=\"") {
                let end = start + 4 + text[start + 4..].find('"').unwrap();
                text.replace_range(start..=end, "");
            }
            contents = text.into_bytes();
        } else if name == "xl/sharedStrings.xml" {
            contents = String::from_utf8(contents)
                .unwrap()
                .replace("  ", "")
                .into_bytes();
        }
        out.start_file(name, options).unwrap();
        std::io::Write::write_all(&mut out, &contents).unwrap();
    }
    out.finish().unwrap().into_inner()
}

fn nested_pivot() -> Vec<u8> {
    let mut model = user_model();
    let mut pivot = definition();
    pivot.columns.clear();
    pivot.rows.push(field("Product"));
    pivot.options = PivotOptions::default();
    model
        .create_pivot_table(pivot, Some("Report"), false)
        .unwrap();
    without_cell_formatting(&export(model.get_model()))
}

#[test]
fn a_pivot_saved_by_another_app_looks_like_a_pivot_when_opened() {
    let imported = import(&nested_pivot());
    let style = |row, column| imported.get_style_for_cell(1, row, column).unwrap();
    let text = |row, column| imported.get_formatted_cell_value(1, row, column).unwrap();

    // Header, parent with its value, indented child, grand total
    assert!(style(3, 1).font.b);
    assert_ne!(style(3, 1).fill.color, Color::None);
    assert_ne!(style(3, 2).fill.color, Color::None);
    assert_eq!(text(4, 1), "East");
    assert!(style(4, 1).font.b);
    assert!(style(4, 2).font.b);
    assert_eq!(text(5, 1), "  Chair");
    assert!(!style(5, 1).font.b);
    assert!(!style(5, 2).font.b);
    assert_eq!(text(5, 2), "1");
    assert_eq!(text(10, 1), "Grand Total");
    assert!(style(10, 2).border.top.is_some());
    assert!(imported.workbook.pivot_tables[0].raw.is_some());
}

#[test]
fn opening_a_pivot_twice_does_not_indent_it_twice() {
    let imported = import(&nested_pivot());
    let again = import(&export(&imported));
    assert_eq!(again.get_formatted_cell_value(1, 5, 1).unwrap(), "  Chair");
}

#[test]
fn sorts_filters_and_collapsed_items_survive_export_and_import() {
    let mut model = user_model();
    let mut pivot = definition();
    pivot.columns.clear();
    pivot.rows.push(field("Product"));
    pivot.rows[0].sort = PivotSort {
        order: PivotSortOrder::Descending,
        by_value: Some("Average of Units".to_string()),
    };
    pivot.rows[0].collapsed_items = vec!["East".to_string()];
    pivot.rows[0].label_filter = Some(PivotLabelFilter {
        condition: PivotLabelCondition::Contains,
        value: "st".to_string(),
        value2: String::new(),
    });
    pivot.rows[1].value_filter = Some(PivotValueFilter {
        value_name: "Average of Units".to_string(),
        rule: PivotValueRule::Top {
            count: 2.0,
            bottom: true,
            unit: PivotTopUnit::Items,
        },
    });
    pivot.values[0].show_as = PivotShowAs::PercentOfRow;
    model
        .create_pivot_table(pivot, Some("Report"), false)
        .unwrap();

    let bytes = export(model.get_model());
    let table = part(&bytes, "xl/pivotTables/pivotTable1.xml");
    assert!(table.contains("sortType=\"descending\""));
    assert!(table.contains("<autoSortScope>"));
    assert!(table.contains("sd=\"0\""));
    assert!(table.contains("type=\"captionContains\""));
    assert!(table.contains("<top10 top=\"0\""));
    assert!(table.contains("showDataAs=\"percentOfRow\""));

    let imported = import(&bytes);
    let read = &imported.workbook.pivot_tables[0];
    assert!(!read.read_only);
    assert_eq!(read.rows[0].sort.order, PivotSortOrder::Descending);
    assert_eq!(
        read.rows[0].sort.by_value.as_deref(),
        Some("Average of Units")
    );
    assert_eq!(read.rows[0].collapsed_items, vec!["East".to_string()]);
    assert_eq!(
        read.rows[0].label_filter.as_ref().unwrap().condition,
        PivotLabelCondition::Contains
    );
    assert!(matches!(
        read.rows[1].value_filter.as_ref().unwrap().rule,
        PivotValueRule::Top { bottom: true, .. }
    ));
    assert_eq!(read.values[0].show_as, PivotShowAs::PercentOfRow);

    let map = read.output_map.as_ref().unwrap();
    assert_eq!(map.rows[0].path, vec!["West".to_string()]);
}
