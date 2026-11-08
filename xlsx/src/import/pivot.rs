//! Reads the PivotTables of an xlsx file.
//!
//! A pivot that can be expressed in IronCalc's model becomes editable. One that
//! uses features IronCalc does not model (calculated fields, grouped items and
//! so on) is read-only here. Either way its original XML is kept and written
//! back on export until the pivot is edited. A pivot that cannot be read at all
//! is dropped and its cells stay as plain values, so a damaged or unusual
//! pivot never blocks opening the file.

use std::collections::HashMap;
use std::io::{BufReader, Read, Seek};

use ironcalc_base::expressions::utils::parse_reference_a1;
use ironcalc_base::pivot::{
    paint_pivot_cells, PivotAggregation, PivotArea, PivotCellKind, PivotField, PivotLabelCondition,
    PivotLabelFilter, PivotLayout, PivotOptions, PivotOutputLine, PivotOutputLineKind,
    PivotOutputMap, PivotPaintCell, PivotRaw, PivotShowAs, PivotSort, PivotSortOrder,
    PivotSubtotals, PivotTable, PivotTopUnit, PivotValue, PivotValueCondition, PivotValueFilter,
    PivotValueRule, BLANK_ITEM_LABEL,
};
use ironcalc_base::types::{Table, Workbook, Worksheet};

use super::worksheets::Relationship;
use super::xml::XmlNode;

const RELATIONSHIPS_NS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

type Archive<R> = zip::ZipArchive<R>;

fn read_part<R: Read + Seek>(archive: &mut Archive<R>, path: &str) -> Option<String> {
    let mut file = archive.by_name(path).ok()?;
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    Some(text)
}

fn parse_xml(text: &str) -> Option<XmlNode> {
    XmlNode::parse(BufReader::new(text.as_bytes())).ok()
}

/// Resolves a relationship target against the part that holds it.
fn resolve(base: &str, target: &str) -> String {
    if let Some(absolute) = target.strip_prefix('/') {
        return absolute.to_string();
    }
    let mut parts: Vec<&str> = base.split('/').collect();
    parts.pop();
    for segment in target.split('/') {
        match segment {
            ".." => {
                parts.pop();
            }
            "." => {}
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn rels_path(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((directory, name)) => format!("{directory}/_rels/{name}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// `(id, type, target)` for each relationship of a part.
fn read_rels<R: Read + Seek>(
    archive: &mut Archive<R>,
    part: &str,
) -> Vec<(String, String, String)> {
    let Some(text) = read_part(archive, &rels_path(part)) else {
        return vec![];
    };
    let Some(document) = parse_xml(&text) else {
        return vec![];
    };
    document
        .children()
        .filter(|n| n.has_tag_name("Relationship"))
        .filter_map(|n| {
            Some((
                n.attribute("Id")?.to_string(),
                n.attribute("Type")?.to_string(),
                n.attribute("Target")?.to_string(),
            ))
        })
        .collect()
}

fn parse_area(reference: &str) -> Option<PivotArea> {
    let reference = reference.replace('$', "");
    let (first, last) = reference
        .split_once(':')
        .unwrap_or((&reference, &reference));
    let first = parse_reference_a1(first)?;
    let last = parse_reference_a1(last)?;
    Some(PivotArea {
        first_row: first.row,
        first_column: first.column,
        last_row: last.row,
        last_column: last.column,
    })
}

fn flag(node: &XmlNode, name: &str, default: bool) -> bool {
    match node.attribute(name) {
        Some(value) => value == "1" || value.eq_ignore_ascii_case("true"),
        None => default,
    }
}

fn child<'a>(node: &'a XmlNode, name: &str) -> Option<&'a XmlNode> {
    node.children().find(|n| n.has_tag_name(name))
}

fn children<'a>(node: &'a XmlNode, name: &'a str) -> impl Iterator<Item = &'a XmlNode> {
    node.children().filter(move |n| n.has_tag_name(name))
}

fn aggregation_of(subtotal: Option<&str>) -> PivotAggregation {
    match subtotal {
        Some("count") => PivotAggregation::Count,
        Some("average") => PivotAggregation::Average,
        Some("max") => PivotAggregation::Max,
        Some("min") => PivotAggregation::Min,
        Some("product") => PivotAggregation::Product,
        Some("countNums") => PivotAggregation::CountNumbers,
        Some("stdDev") => PivotAggregation::StdDev,
        Some("stdDevp") => PivotAggregation::StdDevP,
        Some("var") => PivotAggregation::Var,
        Some("varp") => PivotAggregation::VarP,
        _ => PivotAggregation::Sum,
    }
}

struct CacheField {
    name: String,
    /// Display labels of the shared items. None marks an item that has no
    /// label IronCalc can match, such as a date.
    items: Vec<Option<String>>,
    computed: bool,
}

fn cache_fields(cache: &XmlNode) -> Option<Vec<CacheField>> {
    let fields = child(cache, "cacheFields")?;
    Some(
        children(fields, "cacheField")
            .filter_map(|field| {
                let items = child(field, "sharedItems")
                    .map(|shared| {
                        shared
                            .children()
                            .map(|item| match item.tag_name() {
                                "m" => Some(BLANK_ITEM_LABEL.to_string()),
                                "d" => None,
                                _ => item.attribute("v").map(str::to_string),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(CacheField {
                    name: field.attribute("name")?.to_string(),
                    items,
                    computed: child(field, "fieldGroup").is_some()
                        || field.has_attribute("formula")
                        || !flag(field, "databaseField", true),
                })
            })
            .collect(),
    )
}

/// The source sheet and range of a cache, or None if it is not a worksheet range.
fn cache_source(
    cache: &XmlNode,
    worksheets: &[Worksheet],
    tables: &HashMap<String, Table>,
    pivot_sheet_id: u32,
) -> Option<(u32, PivotArea)> {
    let source = child(cache, "cacheSource")?;
    if source.attribute("type") != Some("worksheet") {
        return None;
    }
    let range = child(source, "worksheetSource")?;
    if range.attribute_ns(RELATIONSHIPS_NS, "id").is_some() {
        return None;
    }
    let sheet_id_of = |name: &str| {
        worksheets
            .iter()
            .find(|w| w.name == name)
            .map(|w| w.sheet_id)
    };
    if let Some(reference) = range.attribute("ref") {
        let sheet_id = match range.attribute("sheet") {
            Some(name) => sheet_id_of(name)?,
            None => pivot_sheet_id,
        };
        return Some((sheet_id, parse_area(reference)?));
    }
    let table = tables.get(range.attribute("name")?)?;
    Some((
        sheet_id_of(&table.sheet_name)?,
        parse_area(&table.reference)?,
    ))
}

fn field_on_axis<'a>(
    name: &str,
    rows: &'a mut [PivotField],
    columns: &'a mut [PivotField],
) -> Option<&'a mut PivotField> {
    rows.iter_mut()
        .chain(columns.iter_mut())
        .find(|f| f.field == name)
}

/// Reads which value each field sorts by. Returns true if one cannot be understood.
fn apply_sort_scopes(
    _table: &XmlNode,
    pivot_fields: &[&XmlNode],
    fields: &[CacheField],
    values: &[PivotValue],
    rows: &mut [PivotField],
    columns: &mut [PivotField],
) -> bool {
    let mut unknown = false;
    for (node, field) in pivot_fields.iter().zip(fields) {
        let Some(scope) = child(node, "autoSortScope") else {
            continue;
        };
        let value = scope
            .descendants()
            .find(|n| n.has_tag_name("reference") && n.attribute("field") == Some("4294967294"))
            .and_then(|reference| child(reference, "x"))
            .map(|x| {
                x.attribute("v")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0)
            })
            .and_then(|v| values.get(v));
        let Some(layout) = field_on_axis(&field.name, rows, columns) else {
            continue;
        };
        match value {
            Some(value) => layout.sort.by_value = Some(value.caption()),
            None => unknown = true,
        }
    }
    unknown
}

fn custom_filter_values(filter: &XmlNode) -> Vec<String> {
    filter
        .descendants()
        .filter(|n| n.has_tag_name("customFilter"))
        .filter_map(|n| n.attribute("val").map(str::to_string))
        .collect()
}

fn label_condition(kind: &str) -> Option<PivotLabelCondition> {
    use PivotLabelCondition as C;
    Some(match kind {
        "captionEqual" => C::Equals,
        "captionNotEqual" => C::NotEquals,
        "captionBeginsWith" => C::BeginsWith,
        "captionNotBeginsWith" => C::NotBeginsWith,
        "captionEndsWith" => C::EndsWith,
        "captionNotEndsWith" => C::NotEndsWith,
        "captionContains" => C::Contains,
        "captionNotContains" => C::NotContains,
        "captionGreaterThan" => C::GreaterThan,
        "captionGreaterThanOrEqual" => C::GreaterThanOrEqual,
        "captionLessThan" => C::LessThan,
        "captionLessThanOrEqual" => C::LessThanOrEqual,
        "captionBetween" => C::Between,
        "captionNotBetween" => C::NotBetween,
        _ => return None,
    })
}

fn value_condition(kind: &str) -> Option<PivotValueCondition> {
    use PivotValueCondition as C;
    Some(match kind {
        "valueEqual" => C::Equals,
        "valueNotEqual" => C::NotEquals,
        "valueGreaterThan" => C::GreaterThan,
        "valueGreaterThanOrEqual" => C::GreaterThanOrEqual,
        "valueLessThan" => C::LessThan,
        "valueLessThanOrEqual" => C::LessThanOrEqual,
        "valueBetween" => C::Between,
        "valueNotBetween" => C::NotBetween,
        _ => return None,
    })
}

/// Reads label, value and Top 10 filters. Returns true if one cannot be understood.
fn apply_filters(
    table: &XmlNode,
    fields: &[CacheField],
    values: &[PivotValue],
    rows: &mut [PivotField],
    columns: &mut [PivotField],
) -> bool {
    let Some(list) = child(table, "filters") else {
        return false;
    };
    let mut unknown = false;
    for filter in children(list, "filter") {
        let kind = filter.attribute("type").unwrap_or_default();
        let field = filter
            .attribute("fld")
            .and_then(|f| f.parse::<usize>().ok())
            .and_then(|f| fields.get(f));
        let Some(layout) = field.and_then(|f| field_on_axis(&f.name, rows, columns)) else {
            unknown = true;
            continue;
        };
        if let Some(condition) = label_condition(kind) {
            layout.label_filter = Some(PivotLabelFilter {
                condition,
                value: filter
                    .attribute("stringValue1")
                    .unwrap_or_default()
                    .to_string(),
                value2: filter
                    .attribute("stringValue2")
                    .unwrap_or_default()
                    .to_string(),
            });
            continue;
        }
        let value_name = filter
            .attribute("iMeasureFld")
            .and_then(|m| m.parse::<usize>().ok())
            .and_then(|m| values.get(m))
            .map(|v| v.caption());
        let Some(value_name) = value_name else {
            unknown = true;
            continue;
        };
        let rule = if let Some(condition) = value_condition(kind) {
            let numbers: Vec<f64> = custom_filter_values(filter)
                .iter()
                .filter_map(|v| v.parse().ok())
                .collect();
            let Some(value) = numbers.first().copied() else {
                unknown = true;
                continue;
            };
            PivotValueRule::Compare {
                condition,
                value,
                value2: numbers.get(1).copied().unwrap_or(0.0),
            }
        } else if matches!(kind, "count" | "percent" | "sum") {
            let Some(top) = filter.descendants().find(|n| n.has_tag_name("top10")) else {
                unknown = true;
                continue;
            };
            PivotValueRule::Top {
                count: top
                    .attribute("val")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(10.0),
                bottom: !flag(top, "top", true),
                unit: match kind {
                    "percent" => PivotTopUnit::Percent,
                    "sum" => PivotTopUnit::Sum,
                    _ => PivotTopUnit::Items,
                },
            }
        } else {
            unknown = true;
            continue;
        };
        layout.value_filter = Some(PivotValueFilter { value_name, rule });
    }
    unknown
}

/// The labels of a field's items, by their place in its list.
fn listed_labels(node: Option<&&XmlNode>, field: Option<&CacheField>) -> Vec<String> {
    let (Some(node), Some(field)) = (node, field) else {
        return vec![];
    };
    let Some(items) = child(node, "items") else {
        return vec![];
    };
    children(items, "item")
        .map(|item| {
            item.attribute("x")
                .and_then(|x| x.parse::<usize>().ok())
                .and_then(|x| field.items.get(x).cloned().flatten())
                .unwrap_or_default()
        })
        .collect()
}

fn read_axis_lines(
    node: Option<&XmlNode>,
    indexes: &[i32],
    pivot_fields: &[&XmlNode],
    fields: &[CacheField],
    value_count: usize,
) -> Vec<PivotOutputLine> {
    let Some(node) = node else {
        return vec![];
    };
    let labels: Vec<Vec<String>> = indexes
        .iter()
        .map(|x| {
            usize::try_from(*x)
                .map(|i| listed_labels(pivot_fields.get(i), fields.get(i)))
                .unwrap_or_default()
        })
        .collect();
    let mut previous: Vec<usize> = Vec::new();
    let mut lines = Vec::new();
    for item in children(node, "i") {
        let shared = item
            .attribute("r")
            .and_then(|r| r.parse::<usize>().ok())
            .unwrap_or(0);
        let mut positions: Vec<usize> = previous.iter().take(shared).copied().collect();
        for x in children(item, "x") {
            positions.push(x.attribute("v").and_then(|v| v.parse().ok()).unwrap_or(0));
        }
        let mut path = Vec::new();
        let mut value = item.attribute("i").and_then(|i| i.parse::<u32>().ok());
        let grand = item.attribute("t") == Some("grand");
        if !grand {
            for (level, position) in positions.iter().enumerate() {
                match indexes.get(level) {
                    Some(-2) => value = Some(*position as u32),
                    Some(_) => path.push(labels[level].get(*position).cloned().unwrap_or_default()),
                    None => {}
                }
            }
        }
        if value.is_none() && value_count > 0 {
            value = Some(0);
        }
        let kind = match item.attribute("t") {
            None if path.is_empty() => PivotOutputLineKind::Grand,
            None => PivotOutputLineKind::Item,
            Some("grand") => PivotOutputLineKind::Grand,
            Some("blank") => PivotOutputLineKind::Blank,
            Some(_) => PivotOutputLineKind::Total,
        };
        lines.push(PivotOutputLine {
            kind,
            path,
            shared: shared as u32,
            value,
        });
        previous = positions;
    }
    lines
}

/// What each line on the sheet shows, as the file recorded it.
fn read_output_map(
    table: &XmlNode,
    pivot_fields: &[&XmlNode],
    fields: &[CacheField],
    row_indexes: &[i32],
    column_indexes: &[i32],
    value_count: usize,
) -> PivotOutputMap {
    let attribute = |name: &str, default: i32| {
        child(table, "location")
            .and_then(|l| l.attribute(name))
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let mut rows = read_axis_lines(
        child(table, "rowItems"),
        row_indexes,
        pivot_fields,
        fields,
        0,
    );
    for line in &mut rows {
        line.value = None;
    }
    PivotOutputMap {
        filter_rows: 0,
        header_rows: attribute("firstDataRow", 1),
        label_columns: attribute("firstDataCol", 1),
        rows,
        columns: read_axis_lines(
            child(table, "colItems"),
            column_indexes,
            pivot_fields,
            fields,
            value_count,
        ),
    }
}

fn convert_pivot(
    table: &XmlNode,
    cache: &XmlNode,
    sheet_id: u32,
    worksheets: &[Worksheet],
    tables: &HashMap<String, Table>,
) -> Option<(PivotTable, i32)> {
    let (source_sheet_id, source) = cache_source(cache, worksheets, tables, sheet_id)?;
    let fields = cache_fields(cache)?;
    let location = child(table, "location")?;
    let body = parse_area(location.attribute("ref")?)?;
    let pivot_fields: Vec<&XmlNode> =
        children(child(table, "pivotFields")?, "pivotField").collect();

    let mut read_only = fields.iter().any(|f| f.computed)
        || flag(table, "dataOnRows", false)
        || child(table, "calculatedItems").is_some();

    // The field indexes listed under `area`. -2 is the virtual Values field.
    let axis = |area: &str, tag: &str, attribute: &str| -> Vec<i32> {
        child(table, area)
            .map(|node| {
                children(node, tag)
                    .filter_map(|f| f.attribute(attribute)?.parse().ok())
                    .collect()
            })
            .unwrap_or_default()
    };

    // Everything about a field that lives on its pivotField element
    let mut read_field = |index: usize, selected_item: Option<usize>| -> Option<PivotField> {
        let field = fields.get(index)?;
        let node = pivot_fields.get(index)?;
        let mut layout = PivotField {
            field: field.name.clone(),
            sort: PivotSort {
                order: match node.attribute("sortType") {
                    Some("ascending") => PivotSortOrder::Ascending,
                    Some("descending") => PivotSortOrder::Descending,
                    _ => PivotSortOrder::Manual,
                },
                by_value: None,
            },
            blank_line: flag(node, "insertBlankRow", false),
            ..PivotField::default()
        };
        if !flag(node, "defaultSubtotal", true) {
            layout.subtotal = Some(PivotSubtotals::None);
        } else if !flag(node, "outline", true) || !flag(node, "subtotalTop", true) {
            layout.subtotal = Some(PivotSubtotals::Bottom);
        } else {
            layout.subtotal = Some(PivotSubtotals::Top);
        }
        let Some(items) = child(node, "items") else {
            return Some(layout);
        };
        let mut every_label_known = true;
        for (position, item) in children(items, "item")
            .filter(|i| !i.has_attribute("t"))
            .enumerate()
        {
            let label = item
                .attribute("x")
                .and_then(|x| x.parse::<usize>().ok())
                .and_then(|x| field.items.get(x).cloned().flatten());
            let is_hidden = match selected_item {
                Some(selected) => position != selected,
                None => flag(item, "h", false),
            };
            match label {
                Some(label) => {
                    if is_hidden {
                        layout.hidden_items.push(label.clone());
                    }
                    if !flag(item, "sd", true) {
                        layout.collapsed_items.push(label.clone());
                    }
                    layout.item_order.push(label);
                }
                None => {
                    every_label_known = false;
                    // A filter that cannot be expressed by label stays as it is in the file
                    if is_hidden {
                        read_only = true;
                    }
                }
            }
        }
        if layout.sort.order == PivotSortOrder::Manual && !every_label_known {
            layout.sort.order = PivotSortOrder::Ascending;
        }
        if layout.sort.order != PivotSortOrder::Manual {
            layout.item_order.clear();
        }
        Some(layout)
    };

    let row_indexes = axis("rowFields", "field", "x");
    let column_indexes = axis("colFields", "field", "x");
    let values_on_rows = row_indexes.contains(&-2);
    let mut rows: Vec<PivotField> = row_indexes
        .iter()
        .filter(|x| **x >= 0)
        .filter_map(|x| read_field(*x as usize, None))
        .collect();
    let mut columns: Vec<PivotField> = column_indexes
        .iter()
        .filter(|x| **x >= 0)
        .filter_map(|x| read_field(*x as usize, None))
        .collect();

    let mut filters = Vec::new();
    if let Some(page_fields) = child(table, "pageFields") {
        for page in children(page_fields, "pageField") {
            let Some(index) = page.attribute("fld").and_then(|f| f.parse::<usize>().ok()) else {
                continue;
            };
            let selected = page.attribute("item").and_then(|i| i.parse::<usize>().ok());
            filters.extend(read_field(index, selected));
        }
    }

    let mut values = Vec::new();
    if let Some(data_fields) = child(table, "dataFields") {
        for data in children(data_fields, "dataField") {
            let index: usize = data.attribute("fld")?.parse().ok()?;
            let field = fields.get(index)?.name.clone();
            let show_as = match data.attribute("showDataAs") {
                None | Some("normal") => PivotShowAs::Normal,
                Some("percentOfTotal") => PivotShowAs::PercentOfTotal,
                Some("percentOfCol") => PivotShowAs::PercentOfColumn,
                Some("percentOfRow") => PivotShowAs::PercentOfRow,
                Some("percentOfParentRow") => PivotShowAs::PercentOfParentRow,
                Some("percentOfParentCol") => PivotShowAs::PercentOfParentColumn,
                Some("percentOfParent") => PivotShowAs::PercentOfParent,
                Some(_) => {
                    read_only = true;
                    PivotShowAs::Normal
                }
            };
            let base_field = if show_as == PivotShowAs::PercentOfParent {
                data.attribute("baseField")
                    .and_then(|b| b.parse::<usize>().ok())
                    .and_then(|b| fields.get(b))
                    .map(|f| f.name.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let aggregation = aggregation_of(data.attribute("subtotal"));
            let name = data.attribute("name").unwrap_or_default().to_string();
            let is_default = name == format!("{} of {}", aggregation.caption(), field);
            values.push(PivotValue {
                field,
                aggregation,
                name: if is_default { String::new() } else { name },
                show_as,
                base_field,
            });
        }
    }

    let read_only = apply_sort_scopes(
        table,
        &pivot_fields,
        &fields,
        &values,
        &mut rows,
        &mut columns,
    ) | apply_filters(table, &fields, &values, &mut rows, &mut columns)
        | read_only
        | values_on_rows;
    let output_map = read_output_map(
        table,
        &pivot_fields,
        &fields,
        &row_indexes,
        &column_indexes,
        values.len(),
    );
    let first_row_field = row_indexes
        .iter()
        .find(|x| **x >= 0)
        .and_then(|x| pivot_fields.get(*x as usize))
        .copied();
    let compact = flag(table, "compact", true);
    let subtotals = match first_row_field {
        Some(field) if !flag(field, "defaultSubtotal", true) => PivotSubtotals::None,
        Some(field) if !flag(field, "outline", true) || !flag(field, "subtotalTop", true) => {
            PivotSubtotals::Bottom
        }
        _ => PivotSubtotals::Top,
    };
    // Only a field that differs from the rest keeps its own setting
    for field in rows.iter_mut().chain(columns.iter_mut()) {
        if field.subtotal == Some(subtotals) {
            field.subtotal = None;
        }
    }
    let page_rows = if filters.is_empty() {
        0
    } else {
        location
            .attribute("rowPageCount")
            .and_then(|c| c.parse::<i32>().ok())
            .unwrap_or(filters.len() as i32)
            + 1
    };
    let output = PivotArea {
        first_row: body.first_row - page_rows,
        ..body
    };
    let style_name = child(table, "pivotTableStyleInfo")
        .and_then(|s| s.attribute("name"))
        .unwrap_or_default()
        .to_string();

    Some((
        PivotTable {
            name: table.attribute("name")?.to_string(),
            sheet_id,
            row: output.first_row,
            column: output.first_column,
            source_sheet_id,
            source,
            rows,
            columns,
            values,
            filters,
            options: PivotOptions {
                grand_totals_rows: flag(table, "rowGrandTotals", true),
                grand_totals_columns: flag(table, "colGrandTotals", true),
                subtotals,
                layout: if compact {
                    PivotLayout::Compact
                } else {
                    PivotLayout::Tabular
                },
                repeat_labels: false,
            },
            output: Some(output),
            output_map: Some(PivotOutputMap {
                filter_rows: page_rows,
                ..output_map
            }),
            style_name,
            read_only,
            raw: None,
        },
        page_rows,
    ))
}

fn load_pivot<R: Read + Seek>(
    archive: &mut Archive<R>,
    table_path: &str,
    sheet_id: u32,
    worksheets: &[Worksheet],
    tables: &HashMap<String, Table>,
) -> Option<PivotTable> {
    let table_xml = read_part(archive, table_path)?;
    let (_, _, target) = read_rels(archive, table_path)
        .into_iter()
        .find(|(_, kind, _)| kind.ends_with("/pivotCacheDefinition"))?;
    let cache_path = resolve(table_path, &target);
    let cache_xml = read_part(archive, &cache_path)?;
    let records_xml = read_rels(archive, &cache_path)
        .into_iter()
        .find(|(_, kind, _)| kind.ends_with("/pivotCacheRecords"))
        .and_then(|(_, _, target)| read_part(archive, &resolve(&cache_path, &target)));

    let table = parse_xml(&table_xml)?;
    let cache = parse_xml(&cache_xml)?;
    let (mut pivot, page_rows) = convert_pivot(&table, &cache, sheet_id, worksheets, tables)?;
    pivot.raw = Some(Box::new(PivotRaw {
        table_xml,
        cache_xml,
        records_xml,
        page_rows,
    }));
    Some(pivot)
}

/// The sheets of the workbook part as `(name, part path)`.
fn sheet_parts<R: Read + Seek>(
    archive: &mut Archive<R>,
    rels: &HashMap<String, Relationship>,
) -> Vec<(String, String)> {
    let Some(document) = read_part(archive, "xl/workbook.xml").and_then(|t| parse_xml(&t)) else {
        return vec![];
    };
    let Some(sheets) = child(&document, "sheets") else {
        return vec![];
    };
    children(sheets, "sheet")
        .filter_map(|sheet| {
            let id = sheet.attribute_ns(RELATIONSHIPS_NS, "id")?;
            let target = &rels.get(id)?.target;
            Some((
                sheet.attribute("name")?.to_string(),
                resolve("xl/workbook.xml", target),
            ))
        })
        .collect()
}

pub(super) fn load_pivots<R: Read + Seek>(
    archive: &mut Archive<R>,
    rels: &HashMap<String, Relationship>,
    worksheets: &[Worksheet],
    tables: &HashMap<String, Table>,
) -> Vec<PivotTable> {
    let mut pivots = Vec::new();
    for (name, part) in sheet_parts(archive, rels) {
        let Some(sheet_id) = worksheets
            .iter()
            .find(|w| w.name == name)
            .map(|w| w.sheet_id)
        else {
            continue;
        };
        for (_, kind, target) in read_rels(archive, &part) {
            if !kind.ends_with("/pivotTable") {
                continue;
            }
            let path = resolve(&part, &target);
            if let Some(pivot) = load_pivot(archive, &path, sheet_id, worksheets, tables) {
                pivots.push(pivot);
            }
        }
    }
    pivots
}

/// Where each cell of a pivot sits in its structure, read from the row layout
/// the file records for what is on the sheet. Reading it rather than
/// recalculating keeps item order and collapsed rows exactly as they were saved.
fn paint_plan(table: &XmlNode) -> Option<Vec<PivotPaintCell>> {
    let location = child(table, "location")?;
    let body = parse_area(location.attribute("ref")?)?;
    let first_data_row = location
        .attribute("firstDataRow")
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(1);
    let compact = flag(table, "compact", true);
    let row_fields = child(table, "rowFields").map_or(0, |n| children(n, "field").count());

    let paint = |row: i32, column: i32, kind: PivotCellKind, indent: usize| PivotPaintCell {
        row,
        column,
        kind,
        indent,
    };
    let columns = body.first_column..=body.last_column;
    let label_column = body.first_column;
    let mut cells = Vec::new();
    for row in body.first_row..body.first_row + first_data_row {
        cells.extend(
            columns
                .clone()
                .map(|c| paint(row, c, PivotCellKind::Header, 0)),
        );
    }
    let first_body_row = body.first_row + first_data_row;
    for (offset, item) in children(child(table, "rowItems")?, "i").enumerate() {
        let row = first_body_row + offset as i32;
        if row > body.last_row {
            break;
        }
        let repeated = item
            .attribute("r")
            .and_then(|r| r.parse::<usize>().ok())
            .unwrap_or(0);
        let Some(depth) = (repeated + children(item, "x").count()).checked_sub(1) else {
            continue;
        };
        let indent = if compact { depth } else { 0 };
        let whole_line = |kind: PivotCellKind| -> Vec<PivotPaintCell> {
            columns
                .clone()
                .map(|c| paint(row, c, kind, if c == label_column { indent } else { 0 }))
                .collect()
        };
        match item.attribute("t") {
            Some("grand") => {
                let grand = columns.clone();
                cells.extend(grand.map(|c| paint(row, c, PivotCellKind::GrandTotal, 0)));
            }
            Some("blank") => {}
            Some(_) => cells.extend(whole_line(PivotCellKind::Total)),
            None if depth + 1 < row_fields => cells.extend(whole_line(PivotCellKind::Parent)),
            None if compact => cells.push(paint(row, label_column, PivotCellKind::Item, indent)),
            None => cells.extend(
                (0..depth as i32).map(|c| paint(row, label_column + c, PivotCellKind::Parent, 0)),
            ),
        }
    }
    Some(cells)
}

/// Styles the cells of every pivot just read. It only changes how they look,
/// so a pivot it cannot make sense of simply stays plain.
pub(super) fn paint_imported_pivots(workbook: &mut Workbook) {
    let plans: Vec<(u32, Vec<PivotPaintCell>)> = workbook
        .pivot_tables
        .iter()
        .filter_map(|pivot| {
            let table = parse_xml(&pivot.raw.as_ref()?.table_xml)?;
            Some((pivot.sheet_id, paint_plan(&table)?))
        })
        .collect();
    for (sheet_id, cells) in plans {
        let _ = paint_pivot_cells(workbook, sheet_id, &cells);
    }
}
