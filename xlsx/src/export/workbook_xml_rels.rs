use ironcalc_base::types::Workbook;

use super::xml_constants::{XML_DECLARATION, XML_WORKSHEET};

pub(crate) fn get_workbook_xml_rels(
    workbook: &Workbook,
    pivots: &[super::pivot::ExportedPivot],
    extras: &super::tables::ExportedExtras,
) -> String {
    let mut relationships_str: Vec<String> = vec![];
    let worksheet_count = workbook.worksheets.len() + 1;
    for id in 1..worksheet_count {
        relationships_str.push(format!(
            "<Relationship Id=\"rId{id}\" Type=\"{XML_WORKSHEET}\" Target=\"worksheets/sheet{id}.xml\"/>"
        ));
    }
    let mut id = worksheet_count;
    relationships_str.push(
        format!("<Relationship Id=\"rId{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>")
    );
    id += 1;
    relationships_str.push(
        format!("<Relationship Id=\"rId{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings\" Target=\"sharedStrings.xml\"/>")
    );
    id += 1;
    relationships_str.push(
        format!("<Relationship Id=\"rId{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/sheetMetadata\" Target=\"metadata.xml\"/>")
    );
    id += 1;
    relationships_str.push(
        format!("<Relationship Id=\"rId{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/>")
    );
    relationships_str.extend(super::pivot::pivot_workbook_rels(workbook, pivots));
    relationships_str.extend(extras.workbook_rels(workbook, pivots));
    format!(
        "{XML_DECLARATION}\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{}</Relationships>",
        relationships_str.join("")
    )
}
