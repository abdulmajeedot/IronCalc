use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek};

use ironcalc_base::expressions::token::get_error_by_english_name;
use ironcalc_base::expressions::token::Error;
use ironcalc_base::preserved::{ExternalLink, MacroPart, MacroProject};
use ironcalc_base::types::{Cell, Color, SheetData, SheetState, Worksheet};

use base64::{engine::general_purpose::STANDARD, Engine};

use crate::error::XlsxError;

use super::shared_strings::SharedStringTable;
use super::worksheets::{parse_cell_reference, Relationship};
use super::xml::XmlNode;

const RELATIONSHIPS_NAMESPACE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The workbooks a file links to, and how formulas name them.
#[derive(Default)]
pub(super) struct ExternalBooks {
    pub(super) links: Vec<ExternalLink>,
    pub(super) sheets: Vec<Worksheet>,
}

fn read_text<R: Read + Seek>(
    archive: &mut zip::read::ZipArchive<R>,
    path: &str,
) -> Result<String, XlsxError> {
    let mut file = archive.by_name(path)?;
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(text)
}

fn part_path(target: &str) -> String {
    match target.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None => format!("xl/{target}"),
    }
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|digits| u8::from_str_radix(digits, 16).ok());
        match hex {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// The file name in a link's target, the way a formula shows it.
fn book_name(target: &str) -> String {
    let path = target.split(['?', '#']).next().unwrap_or(target);
    let last = path.rsplit(['/', '\\']).next().unwrap_or(path);
    percent_decoded(last)
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | '\''))
        .collect()
}

fn cached_cell(node: &XmlNode, strings: &mut SharedStringTable) -> Option<Cell> {
    let value = node.child("v").and_then(XmlNode::text).unwrap_or("");
    Some(match node.attribute("t").unwrap_or("n") {
        "n" => Cell::NumberCell {
            v: value.parse().ok()?,
            s: 0,
        },
        "b" => Cell::BooleanCell {
            v: value == "1" || value.eq_ignore_ascii_case("true"),
            s: 0,
        },
        "e" => Cell::ErrorCell {
            ei: get_error_by_english_name(value).unwrap_or(Error::ERROR),
            s: 0,
        },
        _ => Cell::SharedString {
            si: strings.index_of(value),
            s: 0,
        },
    })
}

fn cache_sheet(name: String, sheet_id: u32, sheet_data: SheetData) -> Worksheet {
    Worksheet {
        dimension: "A1".to_string(),
        cols: Vec::new(),
        rows: Vec::new(),
        name,
        sheet_data,
        shared_formulas: Vec::new(),
        sheet_id,
        state: SheetState::VeryHidden,
        color: Color::None,
        merged_cells: Vec::new(),
        comments: Vec::new(),
        frozen_rows: 0,
        frozen_columns: 0,
        show_grid_lines: true,
        conditional_formatting: Vec::new(),
        links: HashMap::new(),
    }
}

pub(super) fn load_external_links<R: Read + Seek>(
    archive: &mut zip::read::ZipArchive<R>,
    rels: &HashMap<String, Relationship>,
    references: &[String],
    first_sheet_id: u32,
    strings: &mut SharedStringTable,
) -> Result<ExternalBooks, XlsxError> {
    let mut books = ExternalBooks::default();
    let mut names_used = HashSet::new();
    let mut sheet_id = first_sheet_id;
    for reference in references {
        let Some(relationship) = rels.get(reference) else {
            continue;
        };
        let path = part_path(&relationship.target);
        let part_xml = read_text(archive, &path)?;
        let (folder, file) = path.rsplit_once('/').unwrap_or(("", &path));
        let rels_xml = read_text(archive, &format!("{folder}/_rels/{file}.rels")).ok();
        let root = XmlNode::parse_str(&part_xml)?;
        let mut display_name = String::new();
        if let Some(book) = root.child("externalBook") {
            let target = rels_xml.as_deref().and_then(|text| {
                let wanted = book.attribute_ns(RELATIONSHIPS_NAMESPACE, "id")?;
                let doc = XmlNode::parse_str(text).ok()?;
                let found = doc
                    .children()
                    .find(|node| node.attribute("Id") == Some(wanted))
                    .and_then(|node| node.attribute("Target"))
                    .map(str::to_string);
                found
            });
            let base = book_name(target.as_deref().unwrap_or("External.xlsx"));
            display_name = base.clone();
            let mut copy = 2;
            while !names_used.insert(display_name.to_uppercase()) {
                display_name = format!("{copy} {base}");
                copy += 1;
            }

            let sheet_names: Vec<String> = book
                .child("sheetNames")
                .iter()
                .flat_map(|node| node.children())
                .filter_map(|node| node.attribute("val").map(str::to_string))
                .collect();
            let mut data: Vec<SheetData> = sheet_names.iter().map(|_| SheetData::new()).collect();
            let sets = book.child("sheetDataSet");
            for set in sets.iter().flat_map(|node| node.children()) {
                let index = set
                    .attribute("sheetId")
                    .and_then(|id| id.parse::<usize>().ok());
                let Some(sheet) = index.and_then(|index| data.get_mut(index)) else {
                    continue;
                };
                for cell_node in set.descendants().filter(|node| node.has_tag_name("cell")) {
                    let reference = cell_node.attribute("r").unwrap_or("");
                    let Ok((row, column)) = parse_cell_reference(reference) else {
                        continue;
                    };
                    if let Some(cell) = cached_cell(cell_node, strings) {
                        sheet.set_cell(row, column, cell);
                    }
                }
            }
            for (sheet_name, sheet_data) in sheet_names.iter().zip(data) {
                let name = format!("[{display_name}]{sheet_name}");
                books.sheets.push(cache_sheet(name, sheet_id, sheet_data));
                sheet_id += 1;
            }
        }
        books.links.push(ExternalLink {
            display_name,
            part_xml,
            rels_xml,
            ..Default::default()
        });
    }
    Ok(books)
}

impl ExternalBooks {
    pub(super) fn sheet_names(&self) -> Vec<String> {
        self.sheets.iter().map(|sheet| sheet.name.clone()).collect()
    }

    fn name_of(&self, digits: &str) -> Option<&str> {
        let number: usize = digits.parse().ok()?;
        let link = self.links.get(number.checked_sub(1)?)?;
        (!link.display_name.is_empty()).then_some(link.display_name.as_str())
    }

    /// `[1]Sheet1!A1` in a file becomes `'[Book.xlsx]Sheet1'!A1` here.
    pub(super) fn to_internal(&self, formula: &str) -> String {
        if self.links.is_empty() || !formula.contains('[') {
            return formula.to_string();
        }
        let chars: Vec<char> = formula.chars().collect();
        let mut out = String::with_capacity(formula.len() + 16);
        let mut index = 0;
        while index < chars.len() {
            let c = chars[index];
            if c == '"' || c == '\'' {
                let start = index;
                index += 1;
                while index < chars.len() {
                    if chars[index] == c {
                        if chars.get(index + 1) == Some(&c) {
                            index += 1;
                        } else {
                            break;
                        }
                    }
                    index += 1;
                }
                let end = (index + 1).min(chars.len());
                let quoted: String = chars[start..end].iter().collect();
                index = end;
                if let Some(rewritten) = (c == '\'')
                    .then(|| self.quoted_reference(&quoted))
                    .flatten()
                {
                    out.push_str(&rewritten);
                } else {
                    out.push_str(&quoted);
                }
                continue;
            }
            let at_token_start = index == 0
                || !(chars[index - 1].is_alphanumeric()
                    || matches!(chars[index - 1], '_' | '.' | ']' | '@'));
            if c == '[' && at_token_start {
                if let Some((text, next)) = self.bare_reference(&chars, index) {
                    out.push_str(&text);
                    index = next;
                    continue;
                }
            }
            out.push(c);
            index += 1;
        }
        out
    }

    fn quoted_reference(&self, quoted: &str) -> Option<String> {
        let inner = quoted.strip_prefix("'[")?;
        let (digits, rest) = inner.split_once(']')?;
        let name = self.name_of(digits)?;
        Some(format!("'[{name}]{rest}"))
    }

    fn bare_reference(&self, chars: &[char], start: usize) -> Option<(String, usize)> {
        let close = start + chars[start..].iter().position(|c| *c == ']')?;
        let digits: String = chars[start + 1..close].iter().collect();
        let name = self.name_of(&digits)?;
        let mut end = close + 1;
        while chars
            .get(end)
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.'))
        {
            end += 1;
        }
        if end == close + 1 || chars.get(end) != Some(&'!') {
            return None;
        }
        let sheet: String = chars[close + 1..end].iter().collect();
        Some((format!("'[{name}]{sheet}'"), end))
    }
}

fn read_bytes<R: Read + Seek>(
    archive: &mut zip::read::ZipArchive<R>,
    path: &str,
) -> Result<Vec<u8>, XlsxError> {
    let mut file = archive.by_name(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The macros of a file, untouched.
pub(super) fn load_macros<R: Read + Seek>(
    archive: &mut zip::read::ZipArchive<R>,
    rels: &HashMap<String, Relationship>,
    workbook_code_name: Option<String>,
) -> Result<Option<MacroProject>, XlsxError> {
    let Some(relationship) = rels.values().find(|r| r.rel_type.ends_with("/vbaProject")) else {
        return Ok(None);
    };
    let path = part_path(&relationship.target);
    let mut paths = vec![path.clone()];
    let (folder, file) = path.rsplit_once('/').unwrap_or(("", &path));
    let rels_path = format!("{folder}/_rels/{file}.rels");
    if let Ok(text) = read_text(archive, &rels_path) {
        for node in XmlNode::parse_str(&text)?.children() {
            if let Some(target) = node.attribute("Target") {
                paths.push(match target.strip_prefix('/') {
                    Some(absolute) => absolute.to_string(),
                    None => format!("{folder}/{target}"),
                });
            }
        }
        paths.push(rels_path);
    }
    let content_types = XmlNode::parse_str(&read_text(archive, "[Content_Types].xml")?)?;
    let mut parts = Vec::new();
    for part in paths {
        let wanted = format!("/{part}");
        let content_type = content_types
            .children()
            .find(|node| node.attribute("PartName") == Some(wanted.as_str()))
            .and_then(|node| node.attribute("ContentType"))
            .map(str::to_string);
        let data = STANDARD.encode(read_bytes(archive, &part)?);
        parts.push(MacroPart {
            path: part,
            content_type,
            data,
        });
    }
    Ok(Some(MacroProject {
        parts,
        workbook_code_name,
        sheet_code_names: Vec::new(),
    }))
}
