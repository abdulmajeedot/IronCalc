#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::{Model, UserModel};

const LINK_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<externalLink xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><externalBook r:id="rId1"><sheetNames><sheetName val="Prices"/><sheetName val="Q1 Sales"/></sheetNames><sheetDataSet><sheetData sheetId="0"><row r="1"><cell r="A1"><v>10</v></cell><cell r="B1" t="str"><v>widget</v></cell></row><row r="2"><cell r="A2"><v>32</v></cell><cell r="B2" t="b"><v>1</v></cell></row></sheetData><sheetData sheetId="1"><row r="3"><cell r="C3"><v>7.5</v></cell></row></sheetData></sheetDataSet></externalBook></externalLink>"#;

const LINK_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLinkPath" Target="file:///C:\Finance\Price%20List.xlsx" TargetMode="External"/></Relationships>"#;

const VBA_BYTES: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0x00, 0xFF, 0x80, 0x7F, 1, 2, 3];
const SIGNATURE_BYTES: &[u8] = &[9, 8, 7, 0, 255];
const VBA_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProjectSignature" Target="vbaProjectSignature.bin"/></Relationships>"#;

fn read_all(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut parts = BTreeMap::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        if file.is_dir() {
            continue;
        }
        let name = file.name().unwrap().to_string();
        let mut data = Vec::new();
        file.read_to_end(&mut data).unwrap();
        parts.insert(name, data);
    }
    parts
}

fn write_all(parts: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in parts {
        out.start_file(name.as_str(), zip::write::SimpleFileOptions::default())
            .unwrap();
        out.write_all(data).unwrap();
    }
    out.finish().unwrap().into_inner()
}

fn text(parts: &BTreeMap<String, Vec<u8>>, name: &str) -> String {
    String::from_utf8(parts[name].clone()).unwrap()
}

fn edit(parts: &mut BTreeMap<String, Vec<u8>>, name: &str, change: impl Fn(String) -> String) {
    let changed = change(text(parts, name));
    parts.insert(name.to_string(), changed.into_bytes());
}

fn base_file() -> BTreeMap<String, Vec<u8>> {
    let mut model = UserModel::new_empty("book", "en", "UTC", "en").unwrap();
    model.set_user_input(0, 1, 1, "PLACEHOLDER_A").unwrap();
    model.set_user_input(0, 2, 1, "PLACEHOLDER_B").unwrap();
    model.set_user_input(0, 3, 1, "PLACEHOLDER_C").unwrap();
    let bytes = save_xlsx_to_writer(model.get_model(), Cursor::new(Vec::new()))
        .unwrap()
        .into_inner();
    read_all(&bytes)
}

fn formula_cell(reference: &str, formula: &str, value: &str) -> String {
    format!(r#"<c r="{reference}" t="str"><f>{formula}</f><v>{value}</v></c>"#)
}

/// A file with formulas into another workbook, the way Excel writes one.
fn linked_file() -> Vec<u8> {
    let mut parts = base_file();
    edit(&mut parts, "xl/worksheets/sheet1.xml", |xml| {
        let start = xml.find("<sheetData>").unwrap() + "<sheetData>".len();
        let end = xml.find("</sheetData>").unwrap();
        let rows = format!(
            r#"<row r="1">{}</row><row r="2">{}</row><row r="3">{}</row><row r="4">{}</row>"#,
            formula_cell("A1", "[1]Prices!$A$1*2", "stale"),
            formula_cell("A2", "SUM([1]Prices!A1:A2)", "stale"),
            formula_cell("A3", "'[1]Q1 Sales'!C3", "stale"),
            formula_cell("A4", "[1]Prices!B1&amp;\"[1]text\"", "stale"),
        );
        format!("{}{}{}", &xml[..start], rows, &xml[end..])
    });
    edit(&mut parts, "xl/workbook.xml", |xml| {
        xml.replace(
            "<definedNames>",
            r#"<externalReferences><externalReference r:id="rId900"/></externalReferences><definedNames><definedName name="price">[1]Prices!$A$2</definedName>"#,
        )
    });
    edit(&mut parts, "xl/_rels/workbook.xml.rels", |xml| {
        xml.replace(
            "</Relationships>",
            r#"<Relationship Id="rId900" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLink" Target="externalLinks/externalLink1.xml"/></Relationships>"#,
        )
    });
    edit(&mut parts, "[Content_Types].xml", |xml| {
        xml.replace(
            "</Types>",
            r#"<Override PartName="/xl/externalLinks/externalLink1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.externalLink+xml"/></Types>"#,
        )
    });
    parts.insert(
        "xl/externalLinks/externalLink1.xml".to_string(),
        LINK_XML.as_bytes().to_vec(),
    );
    parts.insert(
        "xl/externalLinks/_rels/externalLink1.xml.rels".to_string(),
        LINK_RELS.as_bytes().to_vec(),
    );
    write_all(&parts)
}

fn macro_file() -> Vec<u8> {
    let mut parts = base_file();
    edit(&mut parts, "xl/workbook.xml", |xml| {
        xml.replace(
            "<bookViews>",
            r#"<workbookPr codeName="ThisWorkbook"/><bookViews>"#,
        )
    });
    edit(&mut parts, "xl/worksheets/sheet1.xml", |xml| {
        xml.replace("<dimension", r#"<sheetPr codeName="Hoja1"/><dimension"#)
    });
    edit(&mut parts, "xl/_rels/workbook.xml.rels", |xml| {
        xml.replace(
            "</Relationships>",
            r#"<Relationship Id="rId901" Type="http://schemas.microsoft.com/office/2006/relationships/vbaProject" Target="vbaProject.bin"/></Relationships>"#,
        )
    });
    edit(&mut parts, "[Content_Types].xml", |xml| {
        xml.replace(
            "sheet.main+xml",
            "NOT_USED",
        )
        .replace(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.NOT_USED",
            "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
        )
        .replace(
            "</Types>",
            r#"<Default Extension="bin" ContentType="application/vnd.ms-office.vbaProject"/><Override PartName="/xl/vbaProjectSignature.bin" ContentType="application/vnd.ms-office.vbaProjectSignature"/></Types>"#,
        )
    });
    parts.insert("xl/vbaProject.bin".to_string(), VBA_BYTES.to_vec());
    parts.insert(
        "xl/vbaProjectSignature.bin".to_string(),
        SIGNATURE_BYTES.to_vec(),
    );
    parts.insert(
        "xl/_rels/vbaProject.bin.rels".to_string(),
        VBA_RELS.as_bytes().to_vec(),
    );
    write_all(&parts)
}

fn import(bytes: &[u8]) -> UserModel<'static> {
    let workbook = load_from_xlsx_bytes(bytes, "book", "en", "UTC").unwrap();
    UserModel::from_model(Model::from_workbook(workbook, "en").unwrap())
}

fn export(model: &UserModel) -> BTreeMap<String, Vec<u8>> {
    let bytes = save_xlsx_to_writer(model.get_model(), Cursor::new(Vec::new()))
        .unwrap()
        .into_inner();
    read_all(&bytes)
}

#[test]
fn links_calculate_from_what_the_file_remembers() {
    let mut model = import(&linked_file());
    // Worked out again, not just the results the file had for the formulas
    model.set_user_input(0, 9, 9, "1").unwrap();
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 1, 1).unwrap(), "20");
    assert_eq!(model.get_formatted_cell_value(0, 2, 1).unwrap(), "42");
    assert_eq!(model.get_formatted_cell_value(0, 3, 1).unwrap(), "7.5");
    assert_eq!(
        model.get_formatted_cell_value(0, 4, 1).unwrap(),
        "widget[1]text"
    );
}

#[test]
fn a_formula_shows_the_name_of_the_file() {
    let model = import(&linked_file());
    assert_eq!(
        model.get_cell_content(0, 1, 1).unwrap(),
        "='[Price List.xlsx]Prices'!$A$1*2"
    );
    assert_eq!(
        model.get_cell_content(0, 3, 1).unwrap(),
        "='[Price List.xlsx]Q1 Sales'!C3"
    );
}

#[test]
fn links_stay_out_of_sight() {
    let model = import(&linked_file());
    let sheets = model.get_worksheets_properties();
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0].name, "Sheet1");
}

#[test]
fn links_are_written_back_the_way_they_came() {
    let parts = export(&import(&linked_file()));
    assert_eq!(text(&parts, "xl/externalLinks/externalLink1.xml"), LINK_XML);
    assert_eq!(
        text(&parts, "xl/externalLinks/_rels/externalLink1.xml.rels"),
        LINK_RELS
    );
    let workbook = text(&parts, "xl/workbook.xml");
    let sheets = workbook.find("</sheets>").unwrap();
    let references = workbook.find("<externalReferences>").unwrap();
    let names = workbook.find("<definedNames>").unwrap();
    assert!(sheets < references && references < names, "{workbook}");
    assert!(workbook.contains("[1]Prices"), "{workbook}");
    assert!(!workbook.contains("Price List"), "{workbook}");
    assert!(!workbook.contains("veryHidden"), "{workbook}");
    assert!(text(&parts, "xl/_rels/workbook.xml.rels").contains("externalLinks/externalLink1.xml"));
    assert!(text(&parts, "[Content_Types].xml").contains("externalLink+xml"));
    assert!(!parts.contains_key("xl/worksheets/sheet2.xml"));

    let sheet = text(&parts, "xl/worksheets/sheet1.xml");
    assert!(!sheet.contains("Price List"), "{sheet}");
    assert!(sheet.contains("[1]Prices"), "{sheet}");
    assert!(sheet.contains("<f>[1]Prices!$A$1*2</f>"), "{sheet}");
    assert!(sheet.contains("&apos;[1]Q1 Sales&apos;!C3"), "{sheet}");
    assert!(sheet.contains("[1]text"), "{sheet}");
}

#[test]
fn links_survive_saving_inside_the_app() {
    let model = import(&linked_file());
    let bytes = model.to_bytes();
    let mut reopened = UserModel::from_bytes(&bytes, "en").unwrap();
    reopened.set_user_input(0, 9, 9, "1").unwrap();
    reopened.evaluate();
    assert_eq!(reopened.get_formatted_cell_value(0, 2, 1).unwrap(), "42");
    let parts = export(&reopened);
    assert_eq!(text(&parts, "xl/externalLinks/externalLink1.xml"), LINK_XML);
}

#[test]
fn a_second_trip_changes_nothing() {
    let once = export(&import(&linked_file()));
    let twice = export(&import(&write_all(&once)));
    assert!(text(&twice, "xl/worksheets/sheet1.xml").contains("[1]Prices"));
    assert_eq!(
        twice["xl/externalLinks/externalLink1.xml"],
        once["xl/externalLinks/externalLink1.xml"]
    );
}

#[test]
fn a_new_sheet_goes_before_the_linked_ones() {
    let mut model = import(&linked_file());
    model.new_sheet().unwrap();
    let sheets = model.get_worksheets_properties();
    assert_eq!(sheets.len(), 2);
    assert_eq!(sheets[1].name, "Sheet2");
    let parts = export(&model);
    assert!(parts.contains_key("xl/worksheets/sheet2.xml"));
    assert!(!parts.contains_key("xl/worksheets/sheet3.xml"));
    assert!(model.move_sheet(1, 2).is_err());
}

#[test]
fn macros_come_back_byte_for_byte() {
    let parts = export(&import(&macro_file()));
    assert_eq!(parts["xl/vbaProject.bin"], VBA_BYTES);
    assert_eq!(parts["xl/vbaProjectSignature.bin"], SIGNATURE_BYTES);
    assert_eq!(text(&parts, "xl/_rels/vbaProject.bin.rels"), VBA_RELS);
}

#[test]
fn a_file_with_macros_is_marked_as_one() {
    let parts = export(&import(&macro_file()));
    let types = text(&parts, "[Content_Types].xml");
    assert!(types.contains("macroEnabled.main+xml"), "{types}");
    assert!(!types.contains("spreadsheetml.sheet.main+xml"), "{types}");
    assert!(types.contains(r#"Extension="bin""#), "{types}");
    assert!(types.contains("vbaProjectSignature"), "{types}");
    let rels = text(&parts, "xl/_rels/workbook.xml.rels");
    assert!(rels.contains("relationships/vbaProject"), "{rels}");
    assert!(rels.contains(r#"Target="vbaProject.bin""#), "{rels}");
}

#[test]
fn macros_still_find_the_workbook_and_its_sheets() {
    let parts = export(&import(&macro_file()));
    let workbook = text(&parts, "xl/workbook.xml");
    assert!(
        workbook.contains(r#"codeName="ThisWorkbook""#),
        "{workbook}"
    );
    assert!(workbook.find("<workbookPr").unwrap() < workbook.find("<bookViews").unwrap());
    let sheet = text(&parts, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains(r#"<sheetPr codeName="Hoja1"/>"#), "{sheet}");
}

#[test]
fn macros_survive_saving_inside_the_app() {
    let model = import(&macro_file());
    let reopened = UserModel::from_bytes(&model.to_bytes(), "en").unwrap();
    assert_eq!(export(&reopened)["xl/vbaProject.bin"], VBA_BYTES);
}

#[test]
fn a_plain_file_stays_a_plain_file() {
    let parts = base_file();
    let types = text(&parts, "[Content_Types].xml");
    assert!(types.contains("spreadsheetml.sheet.main+xml"));
    assert!(!types.contains("macroEnabled"));
    assert!(!text(&parts, "xl/workbook.xml").contains("externalReferences"));
    assert!(!text(&parts, "xl/workbook.xml").contains("workbookPr"));
}
