#![allow(clippy::unwrap_used)]

use std::io::{Cursor, Read};

use ironcalc::export::{save_xlsx_to_writer, save_xlsx_view_to_writer};
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::types::{Color, Comment, SheetState};
use ironcalc_base::{Model, UserModel};

fn export(model: &UserModel) -> Vec<u8> {
    let view = model.saved_view();
    save_xlsx_view_to_writer(model.get_model(), &view, Cursor::new(Vec::new()))
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

fn two_sheets() -> UserModel<'static> {
    let mut model = UserModel::new_empty("book", "en", "UTC", "en").unwrap();
    model.new_sheet().unwrap();
    model.set_selected_sheet(0).unwrap();
    model
}

fn comment(cell_ref: &str, text: &str, author: &str) -> Comment {
    Comment {
        text: text.to_string(),
        author_name: author.to_string(),
        author_id: None,
        cell_ref: cell_ref.to_string(),
    }
}

/// Replaces one part of an xlsx file.
fn with_part(bytes: &[u8], name: &str, contents: &str) -> Vec<u8> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).unwrap();
        let file_name = file.name().unwrap().to_string();
        if file.is_dir() {
            continue;
        }
        writer.start_file(file_name.as_str(), options).unwrap();
        if file_name == name {
            std::io::Write::write_all(&mut writer, contents.as_bytes()).unwrap();
        } else {
            std::io::copy(&mut file, &mut writer).unwrap();
        }
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn reopens_at_the_saved_sheet_cell_and_scroll_position() {
    let mut model = two_sheets();
    model.set_selected_sheet(1).unwrap();
    model.set_selected_cell(40, 6).unwrap();
    model.set_top_left_visible_cell(30, 3).unwrap();

    let bytes = export(&model);
    assert!(part(&bytes, "xl/workbook.xml").contains("activeTab=\"1\""));
    let sheet = part(&bytes, "xl/worksheets/sheet2.xml");
    assert!(sheet.contains("tabSelected=\"1\""));
    assert!(sheet.contains("topLeftCell=\"C30\""));
    assert!(sheet.contains("activeCell=\"F40\""));
    assert!(!part(&bytes, "xl/worksheets/sheet1.xml").contains("tabSelected"));

    let view = import(&bytes).get_selected_view();
    assert_eq!(view.sheet, 1);
    assert_eq!((view.row, view.column), (40, 6));
    assert_eq!((view.top_row, view.left_column), (30, 3));
}

#[test]
fn every_sheet_keeps_its_own_selection() {
    let mut model = two_sheets();
    model.set_selected_cell(5, 2).unwrap();
    model.set_selected_sheet(1).unwrap();
    model.set_selected_cell(9, 4).unwrap();

    let mut reopened = import(&export(&model));
    assert_eq!(reopened.get_selected_cell(), (1, 9, 4));
    reopened.set_selected_sheet(0).unwrap();
    assert_eq!(reopened.get_selected_cell(), (0, 5, 2));
}

#[test]
fn a_selected_range_survives() {
    let mut model = two_sheets();
    model.set_selected_cell(2, 2).unwrap();
    model.set_selected_range(2, 2, 6, 4).unwrap();

    let bytes = export(&model);
    assert!(part(&bytes, "xl/worksheets/sheet1.xml").contains("sqref=\"B2:D6\""));
    assert_eq!(import(&bytes).get_selected_view().range, [2, 2, 6, 4]);
}

#[test]
fn frozen_panes_write_the_scroll_position_on_the_pane() {
    let mut model = two_sheets();
    model.set_frozen_rows_count(0, 2).unwrap();
    model.set_frozen_columns_count(0, 1).unwrap();
    model.set_selected_cell(50, 8).unwrap();
    model.set_top_left_visible_cell(40, 5).unwrap();

    let bytes = export(&model);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("topLeftCell=\"E40\" activePane=\"bottomRight\""));
    assert!(sheet.contains("pane=\"bottomRight\" activeCell=\"H50\""));

    let view = import(&bytes).get_selected_view();
    assert_eq!((view.row, view.column), (50, 8));
    assert_eq!((view.top_row, view.left_column), (40, 5));
}

#[test]
fn a_file_at_a1_is_unchanged() {
    let model = two_sheets();
    let bytes = export(&model);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("activeCell=\"A1\" sqref=\"A1\""));
    assert!(!sheet.contains("topLeftCell"));
    assert!(!sheet.contains("sheetPr"));
}

#[test]
fn a_hidden_active_sheet_is_not_opened() {
    let mut model = two_sheets();
    model.set_selected_sheet(1).unwrap();
    let bytes = export(&model);
    let workbook_xml = part(&bytes, "xl/workbook.xml");
    let hidden = workbook_xml.replace("sheetId=\"2\"", "sheetId=\"2\" state=\"hidden\"");
    let reopened = import(&with_part(&bytes, "xl/workbook.xml", &hidden));
    assert_eq!(
        reopened.get_model().workbook.worksheets[1].state,
        SheetState::Hidden
    );
    assert_eq!(reopened.get_selected_view().sheet, 0);
}

#[test]
fn an_active_cell_inside_several_ranges_picks_its_own() {
    let mut model = two_sheets();
    model.set_selected_cell(3, 3).unwrap();
    let bytes = export(&model);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml").replace(
        "activeCell=\"C3\" sqref=\"C3\"",
        "activeCell=\"E7\" sqref=\"A1:B2 D6:F8\"",
    );
    let view = import(&with_part(&bytes, "xl/worksheets/sheet1.xml", &sheet)).get_selected_view();
    assert_eq!((view.row, view.column), (7, 5));
    assert_eq!(view.range, [6, 4, 8, 6]);
}

#[test]
fn tab_colors_survive() {
    let mut model = two_sheets();
    model
        .set_sheet_color(0, &Color::Rgb("#dbbe29".to_string()))
        .unwrap();
    model.set_sheet_color(1, &Color::Theme(5, -0.25)).unwrap();

    let bytes = export(&model);
    let first = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(first.contains("<sheetPr><tabColor rgb=\"FFDBBE29\"/></sheetPr>"));
    assert!(first.find("<sheetPr>").unwrap() < first.find("<dimension").unwrap());

    let reopened = import(&bytes);
    let worksheets = &reopened.get_model().workbook.worksheets;
    assert_eq!(worksheets[0].color, Color::Rgb("#DBBE29".to_string()));
    assert_eq!(worksheets[1].color, Color::Theme(5, -0.25));
}

fn with_comments() -> Model<'static> {
    let mut model = Model::new_empty("book", "en", "UTC", "en").unwrap();
    model.workbook.worksheets[0].comments = vec![
        comment("C5", "second & last\nline two", "Ann"),
        comment("B2", "first <note>", "Ben"),
        comment("D9", "third", "Ann"),
    ];
    model
}

#[test]
fn comments_survive_with_their_authors() {
    let model = with_comments();
    let bytes = save_xlsx_to_writer(&model, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner();

    let content_types = part(&bytes, "[Content_Types].xml");
    assert!(content_types.contains("Extension=\"vml\""));
    assert!(content_types.contains("/xl/comments1.xml"));
    let rels = part(&bytes, "xl/worksheets/_rels/sheet1.xml.rels");
    assert!(rels.contains("../comments1.xml"));
    assert!(rels.contains("../drawings/vmlDrawing1.vml"));
    assert!(part(&bytes, "xl/worksheets/sheet1.xml").contains("<legacyDrawing r:id=\"rId2\"/>"));
    let vml = part(&bytes, "xl/drawings/vmlDrawing1.vml");
    assert_eq!(vml.matches("<v:shape ").count(), 3);
    assert!(vml.contains("<x:Row>1</x:Row><x:Column>1</x:Column>"));

    let workbook = load_from_xlsx_bytes(&bytes, "book", "en", "UTC").unwrap();
    let comments = &workbook.worksheets[0].comments;
    assert_eq!(comments.len(), 3);
    assert_eq!(comments[0].cell_ref, "B2");
    assert_eq!(comments[0].text, "first <note>");
    assert_eq!(comments[0].author_name, "Ben");
    assert_eq!(comments[1].cell_ref, "C5");
    assert_eq!(comments[1].text, "second & last\nline two");
    assert_eq!(comments[1].author_name, "Ann");
}

#[test]
fn legacy_drawing_sits_between_drawing_and_table_parts() {
    let mut model = UserModel::new_empty("book", "en", "UTC", "en").unwrap();
    for (row, value) in ["Item", "Apple", "Pear"].iter().enumerate() {
        model.set_user_input(0, row as i32 + 1, 1, value).unwrap();
    }
    model
        .create_table(
            0,
            ironcalc_base::pivot::PivotArea {
                first_row: 1,
                first_column: 1,
                last_row: 3,
                last_column: 1,
            },
            true,
            None,
        )
        .unwrap();
    let bytes = save_xlsx_to_writer(model.get_model(), Cursor::new(Vec::new()))
        .unwrap()
        .into_inner();
    let workbook = load_from_xlsx_bytes(&bytes, "book", "en", "UTC").unwrap();
    let mut with_note = Model::from_workbook(workbook, "en").unwrap();
    with_note.workbook.worksheets[0].comments = vec![comment("A1", "note", "Ann")];
    let bytes = save_xlsx_to_writer(&with_note, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner();
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.find("<legacyDrawing").unwrap() < sheet.find("<tableParts").unwrap());
    let rels = part(&bytes, "xl/worksheets/_rels/sheet1.xml.rels");
    assert!(rels.contains("Id=\"rId3\""));
}

#[test]
fn a_threaded_comment_becomes_a_plain_note() {
    let model = with_comments();
    let bytes = save_xlsx_to_writer(&model, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner();
    let threaded = "<comments xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><authors><author>tc={ABC}</author></authors><commentList><comment ref=\"B2\" authorId=\"0\"><text><t xml:space=\"preserve\">[Threaded comment]\n\nYour version of Excel allows you to read this threaded comment.\n\nComment:\n    Please check\nReply:\n    Done</t></text></comment></commentList></comments>";
    let file = with_part(&bytes, "xl/comments1.xml", threaded);
    let workbook = load_from_xlsx_bytes(&file, "book", "en", "UTC").unwrap();
    let comments = &workbook.worksheets[0].comments;
    assert_eq!(comments[0].text, "Please check\nReply:\nDone");
    assert_eq!(comments[0].author_name, "");
}

#[test]
fn comments_follow_inserted_and_deleted_rows_and_undo() {
    let mut model = Model::new_empty("book", "en", "UTC", "en").unwrap();
    model.workbook.worksheets[0].comments = vec![
        comment("B2", "a", ""),
        comment("C5", "b", ""),
        comment("D9", "c", ""),
    ];
    let mut model = UserModel::from_model(model);
    let refs = |model: &UserModel| -> Vec<String> {
        model.get_model().workbook.worksheets[0]
            .comments
            .iter()
            .map(|c| c.cell_ref.clone())
            .collect()
    };
    model.insert_rows(0, 3, 2).unwrap();
    assert_eq!(refs(&model), ["B2", "C7", "D11"]);
    model.undo().unwrap();
    assert_eq!(refs(&model), ["B2", "C5", "D9"]);

    model.delete_rows(0, 5, 1).unwrap();
    assert_eq!(refs(&model), ["B2", "D8"]);
    model.undo().unwrap();
    assert_eq!(refs(&model), ["B2", "C5", "D9"]);
    model.redo().unwrap();
    assert_eq!(refs(&model), ["B2", "D8"]);

    model.insert_columns(0, 1, 1).unwrap();
    assert_eq!(refs(&model), ["C2", "E8"]);
}

#[test]
fn the_view_survives_saved_bytes() {
    let mut model = two_sheets();
    model.set_selected_sheet(1).unwrap();
    model.set_selected_cell(12, 3).unwrap();
    let bytes = model.to_bytes();
    let reopened = UserModel::from_bytes(&bytes, "en").unwrap();
    assert_eq!(reopened.get_selected_cell(), (1, 12, 3));

    // And what the server does: bytes to a plain model to xlsx.
    let plain = Model::from_bytes(&bytes, "en").unwrap();
    let file = save_xlsx_to_writer(&plain, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner();
    assert_eq!(import(&file).get_selected_cell(), (1, 12, 3));
}

#[test]
fn a_workbook_at_a1_stays_plain_bytes() {
    let model = two_sheets();
    assert!(!model.to_bytes().starts_with(b"ICSHEET3"));
}

#[test]
fn nonsense_in_the_file_falls_back_to_a1() {
    let mut model = two_sheets();
    model.set_selected_cell(3, 3).unwrap();
    let bytes = export(&model);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml").replace(
        "activeCell=\"C3\" sqref=\"C3\"",
        "activeCell=\"XFE9999999\" sqref=\"nope\"",
    );
    let view = import(&with_part(&bytes, "xl/worksheets/sheet1.xml", &sheet)).get_selected_view();
    assert_eq!((view.row, view.column), (1, 1));
    assert_eq!(view.range, [1, 1, 1, 1]);
}
