//! Writes cell comments. Excel needs two parts for each sheet: the text in
//! `xl/commentsN.xml` and, for the box drawn on the sheet, a legacy VML drawing.

use ironcalc_base::expressions::utils::parse_reference_a1;
use ironcalc_base::types::{Comment, Workbook};

use super::escape::escape_xml;
use super::tables::ExportedExtras;
use super::xml_constants::XML_DECLARATION;

const MAIN_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
/// A VML drawing numbers its shapes in blocks of this size.
const SHAPES_PER_BLOCK: usize = 1024;

/// A comment and its zero-based row and column.
type Placed<'a> = (&'a Comment, i32, i32);

fn comments_xml(comments: &[Placed]) -> String {
    let mut authors: Vec<&str> = Vec::new();
    let mut list = String::new();
    for (comment, ..) in comments {
        let author = comment.author_name.as_str();
        let id = authors
            .iter()
            .position(|known| *known == author)
            .unwrap_or_else(|| {
                authors.push(author);
                authors.len() - 1
            });
        list.push_str(&format!(
            "<comment ref=\"{}\" authorId=\"{id}\" shapeId=\"0\"><text><t xml:space=\"preserve\">{}</t></text></comment>",
            escape_xml(&comment.cell_ref),
            escape_xml(&comment.text)
        ));
    }
    let authors: String = authors
        .iter()
        .map(|author| format!("<author>{}</author>", escape_xml(author)))
        .collect();
    format!(
        "{XML_DECLARATION}\n<comments xmlns=\"{MAIN_NS}\"><authors>{authors}</authors><commentList>{list}</commentList></comments>"
    )
}

fn vml_xml(comments: &[Placed], first_block: usize, blocks: usize) -> String {
    let id_map: Vec<String> = (first_block..first_block + blocks)
        .map(|block| block.to_string())
        .collect();
    let mut shapes = String::new();
    for (index, (_, row, column)) in comments.iter().enumerate() {
        let shape_id = first_block * SHAPES_PER_BLOCK + index + 1;
        let top = (row - 1).max(0);
        let (left, right, bottom) = (column + 1, column + 3, top + 4);
        shapes.push_str(&format!(
            "<v:shape id=\"_x0000_s{shape_id}\" type=\"#_x0000_t202\" style=\"position:absolute;margin-left:60pt;margin-top:2pt;width:108pt;height:60pt;z-index:{z};visibility:hidden\" fillcolor=\"#ffffe1\" o:insetmode=\"auto\">\
<v:fill color2=\"#ffffe1\"/><v:shadow on=\"t\" color=\"black\" obscured=\"t\"/><v:path o:connecttype=\"none\"/>\
<v:textbox style=\"mso-direction-alt:auto\"><div style=\"text-align:left\"></div></v:textbox>\
<x:ClientData ObjectType=\"Note\"><x:MoveWithCells/><x:SizeWithCells/>\
<x:Anchor>{left}, 15, {top}, 2, {right}, 15, {bottom}, 2</x:Anchor>\
<x:AutoFill>False</x:AutoFill><x:Row>{row}</x:Row><x:Column>{column}</x:Column></x:ClientData></v:shape>",
            z = index + 1,
        ));
    }
    format!(
        "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\">\
<o:shapelayout v:ext=\"edit\"><o:idmap v:ext=\"edit\" data=\"{ids}\"/></o:shapelayout>\
<v:shapetype id=\"_x0000_t202\" coordsize=\"21600,21600\" o:spt=\"202\" path=\"m,l,21600r21600,l21600,xe\">\
<v:stroke joinstyle=\"miter\"/><v:path gradientshapeok=\"t\" o:connecttype=\"rect\"/></v:shapetype>{shapes}</xml>",
        ids = id_map.join(","),
    )
}

/// Adds the parts of every sheet that has comments.
pub(super) fn export_comments(workbook: &Workbook, extras: &mut ExportedExtras) {
    let mut number = 0;
    let mut block = 1;
    for (sheet, worksheet) in workbook.worksheets.iter().enumerate() {
        let mut placed: Vec<Placed> = worksheet
            .comments
            .iter()
            .filter_map(|comment| {
                let cell = parse_reference_a1(&comment.cell_ref)?;
                Some((comment, cell.row - 1, cell.column - 1))
            })
            .collect();
        placed.sort_by_key(|(_, row, column)| (*row, *column));
        // Excel treats a second comment on a cell as a corrupt file.
        placed.dedup_by_key(|(_, row, column)| (*row, *column));
        if placed.is_empty() {
            continue;
        }
        number += 1;
        let blocks = placed.len().div_ceil(SHAPES_PER_BLOCK);
        extras
            .parts
            .push((format!("xl/comments{number}.xml"), comments_xml(&placed)));
        extras.parts.push((
            format!("xl/drawings/vmlDrawing{number}.vml"),
            vml_xml(&placed, block, blocks),
        ));
        extras.content_types.push(format!(
            r#"<Override PartName="/xl/comments{number}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/>"#
        ));
        extras.sheets[sheet].add_comments(number);
        block += blocks;
    }
    if number > 0 {
        extras.content_types.push(
            r#"<Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/>"#
                .to_string(),
        );
    }
}
