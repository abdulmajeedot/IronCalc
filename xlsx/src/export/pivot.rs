//! Writes the PivotTables of a workbook.
//!
//! A pivot imported from a file and not edited since goes back out with its
//! original XML, patched only for what may have moved. Every other pivot is
//! generated from its definition. All of them are marked to refresh when the
//! file is opened, since the values on the sheet may have changed since the
//! pivot's cache was built.

#![allow(clippy::unwrap_used)]

use ironcalc_base::expressions::utils::number_to_column;
use ironcalc_base::pivot::{
    FieldData, PivotAggregation, PivotArea, PivotData, PivotField, PivotGrid, PivotItem,
    PivotLabelCondition, PivotLabelFilter, PivotLayout, PivotLine, PivotLineKind, PivotShowAs,
    PivotSortOrder, PivotSubtotals, PivotTable, PivotTopUnit, PivotValueCondition,
    PivotValueFilter, PivotValueRule,
};
use ironcalc_base::types::Workbook;
use ironcalc_base::Model;

use super::escape::escape_xml;
use super::xml_constants::XML_DECLARATION;

const MAIN_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const CACHE_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheDefinition";
const RECORDS_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/pivotCacheRecords";
const DEFAULT_STYLE: &str = "PivotStyleLight16";

/// The parts of one pivot, ready to go in the package.
pub(crate) struct ExportedPivot {
    /// 1-based, used in the part names and as the cache id.
    pub(crate) number: usize,
    pub(crate) sheet_index: usize,
    table_xml: String,
    cache_xml: String,
    records_xml: Option<String>,
    records_rel_id: String,
}

impl ExportedPivot {
    pub(crate) fn content_types(&self) -> Vec<String> {
        let n = self.number;
        let mut types = vec![
            format!(
                r#"<Override PartName="/xl/pivotTables/pivotTable{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotTable+xml"/>"#
            ),
            format!(
                r#"<Override PartName="/xl/pivotCache/pivotCacheDefinition{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheDefinition+xml"/>"#
            ),
        ];
        if self.records_xml.is_some() {
            types.push(format!(r#"<Override PartName="/xl/pivotCache/pivotCacheRecords{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.pivotCacheRecords+xml"/>"#));
        }
        types
    }

    /// `(path, contents)` of every part.
    pub(crate) fn parts(&self) -> Vec<(String, &str)> {
        let n = self.number;
        let mut parts = vec![
            (
                format!("xl/pivotTables/pivotTable{n}.xml"),
                self.table_xml.as_str(),
            ),
            (
                format!("xl/pivotCache/pivotCacheDefinition{n}.xml"),
                self.cache_xml.as_str(),
            ),
        ];
        if let Some(records) = &self.records_xml {
            parts.push((
                format!("xl/pivotCache/pivotCacheRecords{n}.xml"),
                records.as_str(),
            ));
        }
        parts
    }

    fn table_rels(&self) -> String {
        let n = self.number;
        format!(
            "{XML_DECLARATION}\n<Relationships xmlns=\"{RELS_NS}\"><Relationship Id=\"rId1\" Type=\"{CACHE_REL}\" Target=\"../pivotCache/pivotCacheDefinition{n}.xml\"/></Relationships>"
        )
    }

    fn cache_rels(&self) -> Option<String> {
        self.records_xml.as_ref()?;
        let n = self.number;
        let id = &self.records_rel_id;
        Some(format!(
            "{XML_DECLARATION}\n<Relationships xmlns=\"{RELS_NS}\"><Relationship Id=\"{id}\" Type=\"{RECORDS_REL}\" Target=\"pivotCacheRecords{n}.xml\"/></Relationships>"
        ))
    }

    /// `(path, contents)` of the relationship parts.
    pub(crate) fn rels(&self) -> Vec<(String, String)> {
        let n = self.number;
        let mut parts = vec![(
            format!("xl/pivotTables/_rels/pivotTable{n}.xml.rels"),
            self.table_rels(),
        )];
        if let Some(rels) = self.cache_rels() {
            parts.push((
                format!("xl/pivotCache/_rels/pivotCacheDefinition{n}.xml.rels"),
                rels,
            ));
        }
        parts
    }
}

fn workbook_rel_id(workbook: &Workbook, pivot: &ExportedPivot) -> usize {
    // Worksheets, then styles, shared strings, metadata and theme
    workbook.worksheets.len() + 4 + pivot.number
}

/// The `<pivotCaches>` element of the workbook part.
pub(crate) fn pivot_caches_xml(workbook: &Workbook, pivots: &[ExportedPivot]) -> String {
    if pivots.is_empty() {
        return String::new();
    }
    let caches: String = pivots
        .iter()
        .map(|p| {
            format!(
                "<pivotCache cacheId=\"{}\" r:id=\"rId{}\"/>",
                p.number,
                workbook_rel_id(workbook, p)
            )
        })
        .collect();
    format!("<pivotCaches>{caches}</pivotCaches>")
}

pub(crate) fn pivot_workbook_rels(workbook: &Workbook, pivots: &[ExportedPivot]) -> Vec<String> {
    pivots
        .iter()
        .map(|p| {
            format!(
                "<Relationship Id=\"rId{}\" Type=\"{CACHE_REL}\" Target=\"pivotCache/pivotCacheDefinition{}.xml\"/>",
                workbook_rel_id(workbook, p),
                p.number
            )
        })
        .collect()
}

fn area_ref(area: &PivotArea) -> String {
    let first = number_to_column(area.first_column).unwrap();
    let last = number_to_column(area.last_column).unwrap();
    format!("{first}{}:{last}{}", area.first_row, area.last_row)
}

/// The span of the opening tag of the first `element`.
fn opening_tag(xml: &str, element: &str) -> Option<(usize, usize)> {
    let needle = format!("<{element}");
    let mut from = 0;
    loop {
        let start = from + xml[from..].find(&needle)?;
        let after = xml[start + needle.len()..].chars().next()?;
        if after.is_whitespace() || after == '>' || after == '/' {
            let end = start + xml[start..].find('>')?;
            return Some((start, end));
        }
        from = start + needle.len();
    }
}

/// Sets an attribute on the first `element`, adding it if it is missing.
fn set_attribute(xml: &str, element: &str, name: &str, value: &str) -> String {
    let Some((start, end)) = opening_tag(xml, element) else {
        return xml.to_string();
    };
    let tag = &xml[start..end];
    let quoted = format!(" {name}=\"");
    let new_tag = match tag.find(&quoted) {
        Some(position) => {
            let value_start = position + quoted.len();
            let value_end = value_start + tag[value_start..].find('"').unwrap_or(0);
            format!("{}{value}{}", &tag[..value_start], &tag[value_end..])
        }
        None => {
            let insert_at = tag.strip_suffix('/').map_or(tag.len(), str::len);
            format!(
                "{} {name}=\"{value}\"{}",
                &tag[..insert_at],
                &tag[insert_at..]
            )
        }
    };
    format!("{}{new_tag}{}", &xml[..start], &xml[end..])
}

/// The value of an attribute on the first `element`.
fn get_attribute<'a>(xml: &'a str, element: &str, name: &str) -> Option<&'a str> {
    let (start, end) = opening_tag(xml, element)?;
    let tag = &xml[start..end];
    let quoted = format!(" {name}=\"");
    let value_start = tag.find(&quoted)? + quoted.len();
    let value_end = value_start + tag[value_start..].find('"')?;
    Some(&tag[value_start..value_end])
}

/// Replaces the whole `<worksheetSource>` element: a table source would point
/// at a table the exported file does not have.
fn set_worksheet_source(xml: &str, reference: &str, sheet: &str) -> String {
    let Some((start, end)) = opening_tag(xml, "worksheetSource") else {
        return xml.to_string();
    };
    let source = format!("<worksheetSource ref=\"{reference}\" sheet=\"{sheet}\"/>");
    let closing = "</worksheetSource>";
    let tail = if xml[..end].ends_with('/') {
        end + 1
    } else {
        match xml[end..].find(closing) {
            Some(position) => end + position + closing.len(),
            None => return xml.to_string(),
        }
    };
    format!("{}{source}{}", &xml[..start], &xml[tail..])
}

fn export_raw(
    pivot: &PivotTable,
    number: usize,
    sheet_index: usize,
    source_name: &str,
) -> Option<ExportedPivot> {
    let raw = pivot.raw.as_ref()?;
    let output = pivot.output?;
    let body = PivotArea {
        first_row: output.first_row + raw.page_rows,
        ..output
    };
    let cache_id = number.to_string();
    let table_xml = set_attribute(&raw.table_xml, "pivotTableDefinition", "cacheId", &cache_id);
    let table_xml = set_attribute(&table_xml, "location", "ref", &area_ref(&body));
    let cache_xml = set_worksheet_source(
        &raw.cache_xml,
        &area_ref(&pivot.source),
        &escape_xml(source_name),
    );
    let cache_xml = set_attribute(&cache_xml, "pivotCacheDefinition", "refreshOnLoad", "1");
    let records_rel_id = get_attribute(&cache_xml, "pivotCacheDefinition", "r:id")
        .unwrap_or_default()
        .to_string();
    let records_xml = if records_rel_id.is_empty() {
        None
    } else {
        raw.records_xml.clone()
    };
    Some(ExportedPivot {
        number,
        sheet_index,
        table_xml,
        cache_xml,
        records_xml,
        records_rel_id,
    })
}

fn format_number(value: f64) -> String {
    format!("{value}")
}

fn shared_item_xml(item: &PivotItem) -> String {
    match item {
        PivotItem::Number(n) => format!("<n v=\"{}\"/>", format_number(*n)),
        PivotItem::Text(t) => format!("<s v=\"{}\"/>", escape_xml(t)),
        PivotItem::Boolean(b) => format!("<b v=\"{}\"/>", u8::from(*b)),
        PivotItem::Error(e) => format!("<e v=\"{}\"/>", escape_xml(e)),
        PivotItem::Blank => "<m/>".to_string(),
    }
}

/// The attributes describing what a field holds.
fn shared_items_flags(field: &FieldData) -> String {
    let mut numbers = field.items.iter().filter_map(|i| match i.value {
        PivotItem::Number(n) => Some(n),
        _ => None,
    });
    let first = numbers.next();
    let has_text = field
        .items
        .iter()
        .any(|i| matches!(i.value, PivotItem::Text(_)));
    let has_other = field
        .items
        .iter()
        .any(|i| matches!(i.value, PivotItem::Boolean(_) | PivotItem::Error(_)));
    let has_blank = field.items.iter().any(|i| i.value == PivotItem::Blank);
    let has_number = first.is_some();

    let mut flags = String::new();
    if !has_text && !has_blank {
        flags.push_str(" containsSemiMixedTypes=\"0\"");
    }
    if !has_text {
        flags.push_str(" containsString=\"0\"");
    }
    if has_blank {
        flags.push_str(" containsBlank=\"1\"");
    }
    if (has_text as u8 + has_number as u8 + has_other as u8) > 1 {
        flags.push_str(" containsMixedTypes=\"1\"");
    }
    if let Some(first) = first {
        let (min, max, integer) =
            field
                .items
                .iter()
                .fold(
                    (first, first, true),
                    |(min, max, integer), item| match item.value {
                        PivotItem::Number(n) => {
                            (min.min(n), max.max(n), integer && n.fract() == 0.0)
                        }
                        _ => (min, max, integer),
                    },
                );
        flags.push_str(" containsNumber=\"1\"");
        if integer {
            flags.push_str(" containsInteger=\"1\"");
        }
        flags.push_str(&format!(
            " minValue=\"{}\" maxValue=\"{}\"",
            format_number(min),
            format_number(max)
        ));
    }
    flags
}

/// Which fields sit on an axis and so have their items listed.
fn axis_fields(pivot: &PivotTable, data: &PivotData) -> Vec<bool> {
    data.fields
        .iter()
        .map(|field| {
            pivot
                .rows
                .iter()
                .chain(&pivot.columns)
                .chain(&pivot.filters)
                .any(|f| f.field == field.name)
        })
        .collect()
}

fn cache_xml(pivot: &PivotTable, data: &PivotData, source_name: &str) -> (String, String) {
    let on_axis = axis_fields(pivot, data);
    let mut fields_xml = String::new();
    for (field, is_axis) in data.fields.iter().zip(&on_axis) {
        let flags = shared_items_flags(field);
        let name = escape_xml(&field.name);
        if *is_axis {
            let items: String = field
                .items
                .iter()
                .map(|i| shared_item_xml(&i.value))
                .collect();
            fields_xml.push_str(&format!(
                "<cacheField name=\"{name}\" numFmtId=\"0\"><sharedItems{flags} count=\"{}\">{items}</sharedItems></cacheField>",
                field.items.len()
            ));
        } else {
            fields_xml.push_str(&format!(
                "<cacheField name=\"{name}\" numFmtId=\"0\"><sharedItems{flags}/></cacheField>"
            ));
        }
    }
    let reference = area_ref(&pivot.source);
    let definition = format!(
        "{XML_DECLARATION}\n<pivotCacheDefinition xmlns=\"{MAIN_NS}\" xmlns:r=\"{R_NS}\" r:id=\"rId1\" refreshOnLoad=\"1\" refreshedBy=\"IronCalc\" createdVersion=\"6\" refreshedVersion=\"6\" minRefreshableVersion=\"3\" recordCount=\"{count}\"><cacheSource type=\"worksheet\"><worksheetSource ref=\"{reference}\" sheet=\"{sheet}\"/></cacheSource><cacheFields count=\"{fields}\">{fields_xml}</cacheFields></pivotCacheDefinition>",
        count = data.records.len(),
        sheet = escape_xml(source_name),
        fields = data.fields.len(),
    );

    let mut records_xml = String::new();
    for record in &data.records {
        records_xml.push_str("<r>");
        for ((field, item), is_axis) in data.fields.iter().zip(record).zip(&on_axis) {
            if *is_axis {
                records_xml.push_str(&format!("<x v=\"{item}\"/>"));
            } else {
                records_xml.push_str(&shared_item_xml(&field.items[*item as usize].value));
            }
        }
        records_xml.push_str("</r>");
    }
    let records = format!(
        "{XML_DECLARATION}\n<pivotCacheRecords xmlns=\"{MAIN_NS}\" xmlns:r=\"{R_NS}\" count=\"{}\">{records_xml}</pivotCacheRecords>",
        data.records.len()
    );
    (definition, records)
}

/// For each item, in sorted order, its place in the field's list.
fn item_positions(layout: &PivotField, field: &FieldData) -> Vec<u32> {
    let count = field.items.len();
    let mut ranks: Vec<usize> = (0..count).collect();
    if layout.sort.order == PivotSortOrder::Manual && layout.sort.by_value.is_none() {
        ranks.sort_by_key(|rank| {
            layout
                .item_order
                .iter()
                .position(|label| label == &field.items[*rank].label)
                .unwrap_or(layout.item_order.len())
        });
    }
    let mut positions = vec![0; count];
    for (position, rank) in ranks.into_iter().enumerate() {
        positions[rank] = position as u32;
    }
    positions
}

fn items_xml(field: &FieldData, layout: &PivotField, with_default: bool) -> String {
    let positions = item_positions(layout, field);
    let mut ordered: Vec<usize> = (0..field.items.len()).collect();
    ordered.sort_by_key(|rank| positions[*rank]);
    let mut items = String::new();
    for rank in ordered {
        let label = &field.items[rank].label;
        let hidden = if layout.hidden_items.contains(label) {
            " h=\"1\""
        } else {
            ""
        };
        let collapsed = if layout.collapsed_items.contains(label) {
            " sd=\"0\""
        } else {
            ""
        };
        items.push_str(&format!("<item{hidden}{collapsed} x=\"{rank}\"/>"));
    }
    let mut count = field.items.len();
    if with_default {
        items.push_str("<item t=\"default\"/>");
        count += 1;
    }
    format!("<items count=\"{count}\">{items}</items>")
}

fn value_position(pivot: &PivotTable, caption: &str) -> Option<usize> {
    pivot.values.iter().position(|v| v.caption() == caption)
}

fn pivot_field_xml(pivot: &PivotTable, field: &FieldData) -> String {
    let find = |list: &'_ [PivotField]| list.iter().position(|f| f.field == field.name);
    let is_data = pivot.values.iter().any(|v| v.field == field.name);
    let tabular = pivot.options.layout == PivotLayout::Tabular;
    let mut attributes = String::new();
    let (axis, layout) = if let Some(i) = find(&pivot.rows) {
        ("axisRow", Some(&pivot.rows[i]))
    } else if let Some(i) = find(&pivot.columns) {
        ("axisCol", Some(&pivot.columns[i]))
    } else if let Some(i) = find(&pivot.filters) {
        ("axisPage", Some(&pivot.filters[i]))
    } else {
        ("", None)
    };
    let subtotal = layout
        .and_then(|l| l.subtotal)
        .unwrap_or(pivot.options.subtotals);
    if !axis.is_empty() {
        attributes.push_str(&format!(" axis=\"{axis}\""));
    }
    if is_data {
        attributes.push_str(" dataField=\"1\"");
    }
    if tabular {
        attributes.push_str(" compact=\"0\"");
        if subtotal != PivotSubtotals::Top {
            attributes.push_str(" outline=\"0\"");
        }
    }
    if axis == "axisRow" || axis == "axisCol" {
        match subtotal {
            PivotSubtotals::None => attributes.push_str(" defaultSubtotal=\"0\""),
            PivotSubtotals::Bottom => attributes.push_str(" subtotalTop=\"0\""),
            PivotSubtotals::Top => {}
        }
    }
    attributes.push_str(" showAll=\"0\"");
    let Some(layout) = layout else {
        return format!("<pivotField{attributes}/>");
    };
    if layout.blank_line {
        attributes.push_str(" insertBlankRow=\"1\"");
    }
    match layout.sort.order {
        PivotSortOrder::Ascending => attributes.push_str(" sortType=\"ascending\""),
        PivotSortOrder::Descending => attributes.push_str(" sortType=\"descending\""),
        PivotSortOrder::Manual => {}
    }
    if axis == "axisPage" && layout.hidden_items.len() + 1 < field.items.len() {
        attributes.push_str(" multipleItemSelectionAllowed=\"1\"");
    }
    let with_default = axis == "axisPage" || subtotal != PivotSubtotals::None;
    let scope = layout
        .sort
        .by_value
        .as_ref()
        .and_then(|name| value_position(pivot, name))
        .map(|value| {
            format!("<autoSortScope><pivotArea dataOnly=\"0\" outline=\"0\" fieldPosition=\"0\"><references count=\"1\"><reference field=\"4294967294\" count=\"1\" selected=\"0\"><x v=\"{value}\"/></reference></references></pivotArea></autoSortScope>")
        })
        .unwrap_or_default();
    format!(
        "<pivotField{attributes}>{}{scope}</pivotField>",
        items_xml(field, layout, with_default)
    )
}

/// For each field of an axis, the place of each item in the field's list.
fn axis_positions(fields: &[PivotField], data: &PivotData) -> Vec<Vec<u32>> {
    fields
        .iter()
        .filter_map(|f| {
            let index = data.field_index(&f.field)?;
            Some(item_positions(f, &data.fields[index]))
        })
        .collect()
}

fn x_xml(position: u32) -> String {
    if position == 0 {
        "<x/>".to_string()
    } else {
        format!("<x v=\"{position}\"/>")
    }
}

fn line_items_xml(path: &[u32], from: usize, positions: &[Vec<u32>]) -> String {
    path.iter()
        .enumerate()
        .skip(from)
        .map(|(level, item)| x_xml(positions[level][*item as usize]))
        .collect()
}

fn shared_levels(line: &PivotLine, previous: Option<&PivotLine>) -> usize {
    previous
        .map(|p| {
            p.path
                .iter()
                .zip(&line.path)
                .take_while(|(a, b)| a == b)
                .count()
        })
        .unwrap_or(0)
}

fn axis_line_xml(line: &PivotLine, previous: Option<&PivotLine>, positions: &[Vec<u32>]) -> String {
    let mut attributes = String::new();
    let mut body = String::new();
    match line.kind {
        PivotLineKind::Grand => {
            attributes.push_str(" t=\"grand\"");
            body.push_str("<x/>");
        }
        PivotLineKind::Blank => {
            attributes.push_str(" t=\"blank\"");
            body.push_str(&line_items_xml(&line.path, 0, positions));
        }
        PivotLineKind::Total | PivotLineKind::Item => {
            let shared = shared_levels(line, previous).min(line.path.len().saturating_sub(1));
            if line.kind == PivotLineKind::Total {
                attributes.push_str(" t=\"default\"");
            }
            if shared > 0 {
                attributes.push_str(&format!(" r=\"{shared}\""));
            }
            body.push_str(&line_items_xml(&line.path, shared, positions));
        }
    }
    item_xml(&attributes, &body)
}

fn item_xml(attributes: &str, body: &str) -> String {
    if body.is_empty() {
        format!("<i{attributes}/>")
    } else {
        format!("<i{attributes}>{body}</i>")
    }
}

fn row_items_xml(grid: &PivotGrid, positions: &[Vec<u32>]) -> String {
    let items: String = grid
        .row_lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let previous = index.checked_sub(1).map(|i| &grid.row_lines[i]);
            axis_line_xml(line, previous, positions)
        })
        .collect();
    format!(
        "<rowItems count=\"{}\">{items}</rowItems>",
        grid.row_lines.len()
    )
}

fn column_items_xml(grid: &PivotGrid, positions: &[Vec<u32>], value_count: usize) -> String {
    let column_fields = positions.len();
    if column_fields == 0 && value_count <= 1 {
        return "<colItems count=\"1\"><i/></colItems>".to_string();
    }
    let mut items = String::new();
    let mut previous: Option<&PivotLine> = None;
    for line in &grid.column_lines {
        let value = line.value.unwrap_or(0);
        let mut attributes = String::new();
        let mut body = String::new();
        if line.kind == PivotLineKind::Grand && column_fields > 0 {
            attributes.push_str(" t=\"grand\"");
            if value > 0 {
                attributes.push_str(&format!(" i=\"{value}\""));
            }
            body.push_str("<x/>");
        } else {
            let same_line = previous.is_some_and(|p| p.path == line.path);
            if line.kind == PivotLineKind::Total {
                attributes.push_str(" t=\"default\"");
            }
            if value > 0 {
                attributes.push_str(&format!(" i=\"{value}\""));
            }
            let shared = if same_line && value_count > 1 {
                line.path.len()
            } else {
                shared_levels(line, previous).min(line.path.len().saturating_sub(1))
            };
            if shared > 0 {
                attributes.push_str(&format!(" r=\"{shared}\""));
            }
            body.push_str(&line_items_xml(&line.path, shared, positions));
            if value_count > 1 && line.kind != PivotLineKind::Total {
                body.push_str(&x_xml(value as u32));
            }
        }
        items.push_str(&item_xml(&attributes, &body));
        previous = Some(line);
    }
    format!(
        "<colItems count=\"{}\">{items}</colItems>",
        grid.column_lines.len()
    )
}

fn custom_filters(and: bool, filters: &[(Option<&str>, String)]) -> String {
    let list: String = filters
        .iter()
        .map(|(operator, value)| {
            let operator = operator
                .map(|o| format!(" operator=\"{o}\""))
                .unwrap_or_default();
            format!("<customFilter{operator} val=\"{}\"/>", escape_xml(value))
        })
        .collect();
    let and = if and { " and=\"1\"" } else { "" };
    format!("<customFilters{and}>{list}</customFilters>")
}

fn wrap_filter(attributes: String, column: String) -> String {
    format!("<filter {attributes}><autoFilter ref=\"A1\"><filterColumn colId=\"0\">{column}</filterColumn></autoFilter></filter>")
}

fn label_filter_xml(field: usize, id: usize, filter: &PivotLabelFilter) -> String {
    use PivotLabelCondition as C;
    let a = filter.value.clone();
    let b = filter.value2.clone();
    let (kind, column) = match filter.condition {
        C::Equals => ("captionEqual", custom_filters(false, &[(None, a.clone())])),
        C::NotEquals => (
            "captionNotEqual",
            custom_filters(false, &[(Some("notEqual"), a.clone())]),
        ),
        C::BeginsWith => (
            "captionBeginsWith",
            custom_filters(false, &[(None, format!("{a}*"))]),
        ),
        C::NotBeginsWith => (
            "captionNotBeginsWith",
            custom_filters(false, &[(Some("notEqual"), format!("{a}*"))]),
        ),
        C::EndsWith => (
            "captionEndsWith",
            custom_filters(false, &[(None, format!("*{a}"))]),
        ),
        C::NotEndsWith => (
            "captionNotEndsWith",
            custom_filters(false, &[(Some("notEqual"), format!("*{a}"))]),
        ),
        C::Contains => (
            "captionContains",
            custom_filters(false, &[(None, format!("*{a}*"))]),
        ),
        C::NotContains => (
            "captionNotContains",
            custom_filters(false, &[(Some("notEqual"), format!("*{a}*"))]),
        ),
        C::GreaterThan => (
            "captionGreaterThan",
            custom_filters(false, &[(Some("greaterThan"), a.clone())]),
        ),
        C::GreaterThanOrEqual => (
            "captionGreaterThanOrEqual",
            custom_filters(false, &[(Some("greaterThanOrEqual"), a.clone())]),
        ),
        C::LessThan => (
            "captionLessThan",
            custom_filters(false, &[(Some("lessThan"), a.clone())]),
        ),
        C::LessThanOrEqual => (
            "captionLessThanOrEqual",
            custom_filters(false, &[(Some("lessThanOrEqual"), a.clone())]),
        ),
        C::Between => (
            "captionBetween",
            custom_filters(
                true,
                &[
                    (Some("greaterThanOrEqual"), a.clone()),
                    (Some("lessThanOrEqual"), b.clone()),
                ],
            ),
        ),
        C::NotBetween => (
            "captionNotBetween",
            custom_filters(
                false,
                &[
                    (Some("lessThan"), a.clone()),
                    (Some("greaterThan"), b.clone()),
                ],
            ),
        ),
    };
    let mut attributes = format!(
        "fld=\"{field}\" type=\"{kind}\" evalOrder=\"-1\" id=\"{id}\" stringValue1=\"{}\"",
        escape_xml(&a)
    );
    if matches!(filter.condition, C::Between | C::NotBetween) {
        attributes.push_str(&format!(" stringValue2=\"{}\"", escape_xml(&b)));
    }
    wrap_filter(attributes, column)
}

fn value_filter_xml(field: usize, id: usize, measure: usize, filter: &PivotValueFilter) -> String {
    use PivotValueCondition as C;
    let (kind, column) = match &filter.rule {
        PivotValueRule::Compare {
            condition,
            value,
            value2,
        } => {
            let a = format_number(*value);
            let b = format_number(*value2);
            match condition {
                C::Equals => ("valueEqual", custom_filters(false, &[(None, a)])),
                C::NotEquals => (
                    "valueNotEqual",
                    custom_filters(false, &[(Some("notEqual"), a)]),
                ),
                C::GreaterThan => (
                    "valueGreaterThan",
                    custom_filters(false, &[(Some("greaterThan"), a)]),
                ),
                C::GreaterThanOrEqual => (
                    "valueGreaterThanOrEqual",
                    custom_filters(false, &[(Some("greaterThanOrEqual"), a)]),
                ),
                C::LessThan => (
                    "valueLessThan",
                    custom_filters(false, &[(Some("lessThan"), a)]),
                ),
                C::LessThanOrEqual => (
                    "valueLessThanOrEqual",
                    custom_filters(false, &[(Some("lessThanOrEqual"), a)]),
                ),
                C::Between => (
                    "valueBetween",
                    custom_filters(
                        true,
                        &[
                            (Some("greaterThanOrEqual"), a),
                            (Some("lessThanOrEqual"), b),
                        ],
                    ),
                ),
                C::NotBetween => (
                    "valueNotBetween",
                    custom_filters(false, &[(Some("lessThan"), a), (Some("greaterThan"), b)]),
                ),
            }
        }
        PivotValueRule::Top {
            count,
            bottom,
            unit,
        } => {
            let kind = match unit {
                PivotTopUnit::Items => "count",
                PivotTopUnit::Percent => "percent",
                PivotTopUnit::Sum => "sum",
            };
            let top = if *bottom { " top=\"0\"" } else { "" };
            let percent = if *unit == PivotTopUnit::Percent {
                " percent=\"1\""
            } else {
                ""
            };
            let count = format_number(*count);
            (
                kind,
                format!("<top10{top}{percent} val=\"{count}\" filterVal=\"{count}\"/>"),
            )
        }
    };
    wrap_filter(
        format!("fld=\"{field}\" type=\"{kind}\" evalOrder=\"-1\" id=\"{id}\" iMeasureFld=\"{measure}\""),
        column,
    )
}

fn filters_xml(pivot: &PivotTable, data: &PivotData) -> String {
    let mut filters = Vec::new();
    for layout in pivot.rows.iter().chain(&pivot.columns) {
        let Some(field) = data.field_index(&layout.field) else {
            continue;
        };
        if let Some(filter) = &layout.label_filter {
            filters.push(label_filter_xml(field, filters.len() + 1, filter));
        }
        if let Some(filter) = &layout.value_filter {
            if let Some(measure) = value_position(pivot, &filter.value_name) {
                filters.push(value_filter_xml(field, filters.len() + 1, measure, filter));
            }
        }
    }
    if filters.is_empty() {
        return String::new();
    }
    format!(
        "<filters count=\"{}\">{}</filters>",
        filters.len(),
        filters.join("")
    )
}

fn show_as_attribute(show_as: PivotShowAs) -> &'static str {
    match show_as {
        PivotShowAs::Normal => "",
        PivotShowAs::PercentOfTotal => " showDataAs=\"percentOfTotal\"",
        PivotShowAs::PercentOfColumn => " showDataAs=\"percentOfCol\"",
        PivotShowAs::PercentOfRow => " showDataAs=\"percentOfRow\"",
        PivotShowAs::PercentOfParentRow => " showDataAs=\"percentOfParentRow\"",
        PivotShowAs::PercentOfParentColumn => " showDataAs=\"percentOfParentCol\"",
        PivotShowAs::PercentOfParent => " showDataAs=\"percentOfParent\"",
    }
}

fn subtotal_attribute(aggregation: PivotAggregation) -> &'static str {
    match aggregation {
        PivotAggregation::Sum => "",
        PivotAggregation::Count => " subtotal=\"count\"",
        PivotAggregation::Average => " subtotal=\"average\"",
        PivotAggregation::Max => " subtotal=\"max\"",
        PivotAggregation::Min => " subtotal=\"min\"",
        PivotAggregation::Product => " subtotal=\"product\"",
        PivotAggregation::CountNumbers => " subtotal=\"countNums\"",
        PivotAggregation::StdDev => " subtotal=\"stdDev\"",
        PivotAggregation::StdDevP => " subtotal=\"stdDevp\"",
        PivotAggregation::Var => " subtotal=\"var\"",
        PivotAggregation::VarP => " subtotal=\"varp\"",
    }
}

fn table_xml(pivot: &PivotTable, grid: &PivotGrid, number: usize) -> String {
    let data = &grid.data;
    let index_of = |name: &str| data.field_index(name);
    let rows: Vec<usize> = pivot
        .rows
        .iter()
        .filter_map(|f| index_of(&f.field))
        .collect();
    let columns: Vec<usize> = pivot
        .columns
        .iter()
        .filter_map(|f| index_of(&f.field))
        .collect();
    let value_count = pivot.values.len();
    let output = pivot.output.unwrap_or(PivotArea {
        first_row: pivot.row,
        first_column: pivot.column,
        last_row: pivot.row,
        last_column: pivot.column,
    });
    let body = PivotArea {
        first_row: output.first_row + grid.filter_rows,
        ..output
    };

    let mut root = format!(
        "name=\"{}\" cacheId=\"{number}\" applyNumberFormats=\"0\" applyBorderFormats=\"0\" applyFontFormats=\"0\" applyPatternFormats=\"0\" applyAlignmentFormats=\"0\" applyWidthHeightFormats=\"1\" dataCaption=\"Values\" updatedVersion=\"6\" minRefreshableVersion=\"3\" useAutoFormatting=\"1\" itemPrintTitles=\"1\" createdVersion=\"6\" indent=\"0\"",
        escape_xml(&pivot.name)
    );
    if pivot.options.layout == PivotLayout::Tabular {
        root.push_str(" compact=\"0\" compactData=\"0\"");
    } else {
        root.push_str(" outline=\"1\" outlineData=\"1\"");
    }
    if !pivot.options.grand_totals_rows {
        root.push_str(" rowGrandTotals=\"0\"");
    }
    if !pivot.options.grand_totals_columns {
        root.push_str(" colGrandTotals=\"0\"");
    }
    let both_filters = pivot
        .rows
        .iter()
        .chain(&pivot.columns)
        .any(|f| f.label_filter.is_some() && f.value_filter.is_some());
    root.push_str(if both_filters {
        " multipleFieldFilters=\"1\""
    } else {
        " multipleFieldFilters=\"0\""
    });

    let values_are_columns = value_count > 1;
    let first_header_row = i32::from(!(columns.is_empty() && values_are_columns));
    let page = if pivot.filters.is_empty() {
        String::new()
    } else {
        format!(
            " rowPageCount=\"{}\" colPageCount=\"1\"",
            pivot.filters.len()
        )
    };
    let location = format!(
        "<location ref=\"{}\" firstHeaderRow=\"{first_header_row}\" firstDataRow=\"{}\" firstDataCol=\"{}\"{page}/>",
        area_ref(&body),
        grid.header_rows.max(1),
        grid.label_columns,
    );

    let fields: String = data
        .fields
        .iter()
        .map(|f| pivot_field_xml(pivot, f))
        .collect();
    let pivot_fields = format!(
        "<pivotFields count=\"{}\">{fields}</pivotFields>",
        data.fields.len()
    );

    let row_fields = if rows.is_empty() {
        String::new()
    } else {
        let list: String = rows.iter().map(|x| format!("<field x=\"{x}\"/>")).collect();
        format!("<rowFields count=\"{}\">{list}</rowFields>", rows.len())
    };
    let row_items = row_items_xml(grid, &axis_positions(&pivot.rows, data));

    let mut column_list: Vec<String> = columns
        .iter()
        .map(|x| format!("<field x=\"{x}\"/>"))
        .collect();
    if values_are_columns {
        column_list.push("<field x=\"-2\"/>".to_string());
    }
    let col_fields = if column_list.is_empty() {
        String::new()
    } else {
        format!(
            "<colFields count=\"{}\">{}</colFields>",
            column_list.len(),
            column_list.join("")
        )
    };
    let col_items = column_items_xml(grid, &axis_positions(&pivot.columns, data), value_count);

    let page_fields = if pivot.filters.is_empty() {
        String::new()
    } else {
        let list: String = pivot
            .filters
            .iter()
            .filter_map(|f| {
                let index = index_of(&f.field)?;
                let items = &data.fields[index].items;
                let visible: Vec<usize> = items
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| !f.hidden_items.contains(&i.label))
                    .map(|(i, _)| i)
                    .collect();
                let item = if visible.len() == 1 && !f.hidden_items.is_empty() {
                    format!(" item=\"{}\"", visible[0])
                } else {
                    String::new()
                };
                Some(format!("<pageField fld=\"{index}\"{item} hier=\"-1\"/>"))
            })
            .collect();
        format!(
            "<pageFields count=\"{}\">{list}</pageFields>",
            pivot.filters.len()
        )
    };

    let data_fields = if pivot.values.is_empty() {
        String::new()
    } else {
        let list: String = pivot
            .values
            .iter()
            .filter_map(|value| {
                let index = index_of(&value.field)?;
                let base = index_of(&value.base_field).unwrap_or(0);
                let number_format = if value.show_as == PivotShowAs::Normal {
                    ""
                } else {
                    " numFmtId=\"10\""
                };
                Some(format!(
                    "<dataField name=\"{}\" fld=\"{index}\"{}{} baseField=\"{base}\" baseItem=\"0\"{number_format}/>",
                    escape_xml(&value.caption()),
                    subtotal_attribute(value.aggregation),
                    show_as_attribute(value.show_as),
                ))
            })
            .collect();
        format!("<dataFields count=\"{value_count}\">{list}</dataFields>")
    };

    let style = if pivot.style_name.is_empty() {
        DEFAULT_STYLE
    } else {
        pivot.style_name.as_str()
    };
    format!(
        "{XML_DECLARATION}\n<pivotTableDefinition xmlns=\"{MAIN_NS}\" {root}>{location}{pivot_fields}{row_fields}{row_items}{col_fields}{col_items}{page_fields}{data_fields}<pivotTableStyleInfo name=\"{}\" showRowHeaders=\"1\" showColHeaders=\"1\" showRowStripes=\"0\" showColStripes=\"0\" showLastColumn=\"1\"/>{filters}</pivotTableDefinition>",
        escape_xml(style),
        filters = filters_xml(pivot, data)
    )
}

fn export_generated(
    model: &Model,
    pivot: &PivotTable,
    number: usize,
    sheet_index: usize,
    source_name: &str,
) -> Option<ExportedPivot> {
    if pivot.rows.is_empty() && pivot.columns.is_empty() && pivot.values.is_empty() {
        return None;
    }
    let grid = model.compute_pivot(pivot).ok()?;
    let (cache, records) = cache_xml(pivot, &grid.data, source_name);
    Some(ExportedPivot {
        number,
        sheet_index,
        table_xml: table_xml(pivot, &grid, number),
        cache_xml: cache,
        records_xml: Some(records),
        records_rel_id: "rId1".to_string(),
    })
}

/// Builds every pivot whose sheets still exist.
pub(crate) fn export_pivots(model: &Model) -> Vec<ExportedPivot> {
    let mut exported = Vec::new();
    for pivot in &model.workbook.pivot_tables {
        let Some(sheet_index) = model.pivot_sheet_index(pivot.sheet_id) else {
            continue;
        };
        let Some(source_index) = model.pivot_sheet_index(pivot.source_sheet_id) else {
            continue;
        };
        let source_name = &model.workbook.worksheets[source_index as usize].name;
        let number = exported.len() + 1;
        let item = if pivot.raw.is_some() {
            export_raw(pivot, number, sheet_index as usize, source_name)
        } else {
            export_generated(model, pivot, number, sheet_index as usize, source_name)
        };
        exported.extend(item);
    }
    exported
}

#[cfg(test)]
mod tests {
    use super::{get_attribute, set_attribute, set_worksheet_source};

    #[test]
    fn set_attribute_replaces_or_adds() {
        let xml = r#"<root><a x="1" y="2"/></root>"#;
        assert_eq!(
            set_attribute(xml, "a", "x", "9"),
            r#"<root><a x="9" y="2"/></root>"#
        );
        assert_eq!(
            set_attribute(xml, "a", "z", "3"),
            r#"<root><a x="1" y="2" z="3"/></root>"#
        );
    }

    #[test]
    fn opening_tag_does_not_match_a_longer_name() {
        let xml = r#"<locationX ref="A"/><location ref="B"/>"#;
        assert_eq!(get_attribute(xml, "location", "ref"), Some("B"));
    }

    #[test]
    fn worksheet_source_is_replaced_whole() {
        let xml = r#"<a><worksheetSource name="Table1"/></a>"#;
        assert_eq!(
            set_worksheet_source(xml, "A1:B2", "Data"),
            r#"<a><worksheetSource ref="A1:B2" sheet="Data"/></a>"#
        );
    }
}
