use base64::{engine::general_purpose::STANDARD, Engine};
use ironcalc_base::expressions::utils::number_to_column;
use ironcalc_base::preserved::is_external_cache_sheet;
use ironcalc_base::types::{Cell, Workbook, Worksheet};

use super::escape::escape_xml;

const EXTERNAL_LINK_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLink";
const VBA_PROJECT_TYPE: &str = "http://schemas.microsoft.com/office/2006/relationships/vbaProject";
const WORKBOOK_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
const MACRO_WORKBOOK_TYPE: &str = "application/vnd.ms-excel.sheet.macroEnabled.main+xml";

/// The file parts for a link to a document on the backend: Excel shows the
/// last values read.
fn linked_document_parts(workbook: &Workbook, name: &str) -> (String, String) {
    let prefix = format!("[{name}]");
    let sheets: Vec<&Worksheet> = workbook
        .worksheets
        .iter()
        .filter(|worksheet| {
            is_external_cache_sheet(worksheet) && worksheet.name.starts_with(&prefix)
        })
        .collect();
    let sheet_names: String = sheets
        .iter()
        .map(|worksheet| {
            format!(
                r#"<sheetName val="{}"/>"#,
                escape_xml(&worksheet.name[prefix.len()..])
            )
        })
        .collect();
    let mut data = String::new();
    for (index, worksheet) in sheets.iter().enumerate() {
        data.push_str(&format!(r#"<sheetData sheetId="{index}">"#));
        let mut cells: Vec<(i32, i32, _)> = worksheet.sheet_data.cells().collect();
        cells.sort_by_key(|(row, column, _)| (*row, *column));
        let mut current_row = None;
        for (row, column, cell) in cells {
            let Some(column_name) = number_to_column(column) else {
                continue;
            };
            let (kind, value) = match cell.as_ref() {
                Cell::NumberCell { v, .. } => ("", v.to_string()),
                Cell::BooleanCell { v, .. } => (r#" t="b""#, u8::from(*v).to_string()),
                Cell::ErrorCell { ei, .. } => (r#" t="e""#, ei.to_string()),
                Cell::SharedString { si, .. } => (
                    r#" t="str""#,
                    workbook
                        .shared_strings
                        .get(*si as usize)
                        .cloned()
                        .unwrap_or_default(),
                ),
                _ => continue,
            };
            if current_row != Some(row) {
                if current_row.is_some() {
                    data.push_str("</row>");
                }
                data.push_str(&format!(r#"<row r="{row}">"#));
                current_row = Some(row);
            }
            data.push_str(&format!(
                r#"<cell r="{column_name}{row}"{kind}><v>{}</v></cell>"#,
                escape_xml(&value)
            ));
        }
        if current_row.is_some() {
            data.push_str("</row>");
        }
        data.push_str("</sheetData>");
    }
    let part = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<externalLink xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><externalBook r:id="rId1"><sheetNames>{sheet_names}</sheetNames><sheetDataSet>{data}</sheetDataSet></externalBook></externalLink>"#
    );
    let target = escape_xml(&format!("{name}.xlsx")).to_string();
    let rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLinkPath" Target="{target}" TargetMode="External"/></Relationships>"#
    );
    (part, rels)
}

/// A copy of a workbook without the sheets that only hold linked workbooks.
/// Those go in the file as links.
pub(crate) fn without_cache_sheets(workbook: &Workbook) -> Option<Workbook> {
    if !workbook.worksheets.iter().any(is_external_cache_sheet)
        && workbook.external_links.is_empty()
    {
        return None;
    }
    let mut copy = workbook.clone();
    for link in &mut copy.external_links {
        if link.document_id.is_some() {
            let (part_xml, rels_xml) = linked_document_parts(workbook, &link.display_name);
            link.part_xml = part_xml;
            link.rels_xml = Some(rels_xml);
        }
    }
    copy.worksheets
        .retain(|worksheet| !is_external_cache_sheet(worksheet));
    Some(copy)
}

/// Whether `1]Sheet1` needs no quotes around it in a formula.
fn is_plain_book_reference(reference: &str) -> bool {
    let Some((number, sheet)) = reference.split_once(']') else {
        return false;
    };
    let mut sheet_chars = sheet.chars();
    !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
        && sheet_chars
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && sheet_chars.all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

/// Excel only quotes `[1]Sheet1!A1` when the sheet's name needs it.
fn without_needless_quotes(xml: &str) -> String {
    const OPEN: &str = "&apos;[";
    const CLOSE: &str = "&apos;!";
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(position) = rest.find(OPEN) {
        out.push_str(&rest[..position]);
        let after = &rest[position + OPEN.len()..];
        match after.find(CLOSE) {
            Some(end) if is_plain_book_reference(&after[..end]) => {
                out.push('[');
                out.push_str(&after[..end]);
                out.push('!');
                rest = &after[end + CLOSE.len()..];
            }
            _ => {
                out.push_str(OPEN);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `'[Book.xlsx]Sheet1'!A1` here is `[1]Sheet1!A1` in a file.
pub(crate) fn with_book_numbers(xml: String, workbook: &Workbook) -> String {
    if workbook.external_links.is_empty() {
        return xml;
    }
    let mut xml = xml;
    for (index, link) in workbook.external_links.iter().enumerate() {
        if link.display_name.is_empty() {
            continue;
        }
        let quoted = format!("'[{}]", link.display_name.replace('\'', "''"));
        let number = format!("'[{}]", index + 1);
        xml = xml.replace(escape_xml(&quoted).as_ref(), escape_xml(&number).as_ref());
    }
    without_needless_quotes(&xml)
}

pub(crate) fn workbook_content_type(workbook: &Workbook) -> &'static str {
    if workbook.macros.is_some() {
        MACRO_WORKBOOK_TYPE
    } else {
        WORKBOOK_TYPE
    }
}

pub(crate) fn content_types(workbook: &Workbook) -> Vec<String> {
    let mut content = Vec::new();
    for index in 1..=workbook.external_links.len() {
        content.push(format!(
            r#"<Override PartName="/xl/externalLinks/externalLink{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.externalLink+xml"/>"#
        ));
    }
    if let Some(macros) = &workbook.macros {
        content.push(
            r#"<Default Extension="bin" ContentType="application/vnd.ms-office.vbaProject"/>"#
                .to_string(),
        );
        for part in &macros.parts {
            if let Some(content_type) = &part.content_type {
                content.push(format!(
                    r#"<Override PartName="/{}" ContentType="{}"/>"#,
                    escape_xml(&part.path),
                    escape_xml(content_type)
                ));
            }
        }
    }
    content
}

pub(crate) fn workbook_rels(workbook: &Workbook) -> Vec<String> {
    let mut rels = Vec::new();
    for index in 1..=workbook.external_links.len() {
        rels.push(format!(
            r#"<Relationship Id="rIdExternal{index}" Type="{EXTERNAL_LINK_TYPE}" Target="externalLinks/externalLink{index}.xml"/>"#
        ));
    }
    if let Some(first) = workbook.macros.as_ref().and_then(|m| m.parts.first()) {
        let target = first.path.strip_prefix("xl/").unwrap_or(&first.path);
        rels.push(format!(
            r#"<Relationship Id="rIdVbaProject" Type="{VBA_PROJECT_TYPE}" Target="{}"/>"#,
            escape_xml(target)
        ));
    }
    rels
}

/// `<workbookPr>`, for macros to find the workbook by its name.
pub(crate) fn workbook_properties_xml(workbook: &Workbook) -> String {
    match workbook
        .macros
        .as_ref()
        .and_then(|macros| macros.workbook_code_name.as_ref())
    {
        Some(name) => format!(r#"<workbookPr codeName="{}"/>"#, escape_xml(name)),
        None => String::new(),
    }
}

pub(crate) fn external_references_xml(workbook: &Workbook) -> String {
    if workbook.external_links.is_empty() {
        return String::new();
    }
    let references: String = (1..=workbook.external_links.len())
        .map(|index| format!(r#"<externalReference r:id="rIdExternal{index}"/>"#))
        .collect();
    format!("<externalReferences>{references}</externalReferences>")
}

/// The name macros know a sheet by.
pub(crate) fn sheet_code_name(workbook: &Workbook, sheet_id: u32) -> Option<&str> {
    workbook
        .macros
        .as_ref()?
        .sheet_code_names
        .iter()
        .find(|(id, _)| *id == sheet_id)
        .map(|(_, name)| name.as_str())
}

/// Every part to add to the file, with its path.
pub(crate) fn parts(workbook: &Workbook) -> Vec<(String, Vec<u8>)> {
    let mut parts = Vec::new();
    for (index, link) in workbook.external_links.iter().enumerate() {
        let number = index + 1;
        parts.push((
            format!("xl/externalLinks/externalLink{number}.xml"),
            link.part_xml.clone().into_bytes(),
        ));
        if let Some(rels) = &link.rels_xml {
            parts.push((
                format!("xl/externalLinks/_rels/externalLink{number}.xml.rels"),
                rels.clone().into_bytes(),
            ));
        }
    }
    if let Some(macros) = &workbook.macros {
        for part in &macros.parts {
            if let Ok(bytes) = STANDARD.decode(&part.data) {
                parts.push((part.path.clone(), bytes));
            }
        }
    }
    parts
}
