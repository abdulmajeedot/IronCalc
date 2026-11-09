//! Reads what surrounds the tables of an xlsx file: their filters and sort,
//! and the slicers of tables and pivots. A slicer that cannot be understood is
//! left out, so it never blocks opening the file.

use std::collections::HashMap;
use std::io::{Read, Seek};

use ironcalc_base::pivot::{PivotField, PivotTable, BLANK_ITEM_LABEL};
use ironcalc_base::table::{
    sheet_filter_name, SheetFilter, Slicer, SlicerPivot, SlicerPlacement, SlicerSource,
    TableColumnFilter, TableFilterOperator, TableFilterRule, TableFilterState, TableSort,
    BLANK_FILTER_LABEL, DEFAULT_SLICER_STYLE,
};
use ironcalc_base::types::{Table, Worksheet};

use super::pivot::{parse_xml, read_part, read_rels, resolve, sheet_parts};
use super::worksheets::Relationship;
use super::xml::XmlNode;

type Archive<R> = zip::ZipArchive<R>;

const EMU_PER_PIXEL: f64 = 9525.0;

fn operator_of(name: Option<&str>) -> Option<TableFilterOperator> {
    use TableFilterOperator as O;
    Some(match name.unwrap_or("equal") {
        "equal" => O::Equals,
        "notEqual" => O::NotEquals,
        "greaterThan" => O::GreaterThan,
        "greaterThanOrEqual" => O::GreaterThanOrEqual,
        "lessThan" => O::LessThan,
        "lessThanOrEqual" => O::LessThanOrEqual,
        _ => return None,
    })
}

/// `abc*` is stored as "equals" with a wildcard. It reads better unfolded.
fn unfold_wildcards(operator: TableFilterOperator, value: &str) -> (TableFilterOperator, String) {
    use TableFilterOperator as O;
    let negated = operator == O::NotEquals;
    if operator != O::Equals && !negated {
        return (operator, value.to_string());
    }
    let inner = value.strip_prefix('*').and_then(|v| v.strip_suffix('*'));
    if let Some(inner) = inner.filter(|v| !v.contains(['*', '?'])) {
        let op = if negated { O::NotContains } else { O::Contains };
        return (op, inner.to_string());
    }
    if let Some(inner) = value.strip_suffix('*').filter(|v| !v.contains(['*', '?'])) {
        let op = if negated {
            O::NotBeginsWith
        } else {
            O::BeginsWith
        };
        return (op, inner.to_string());
    }
    if let Some(inner) = value.strip_prefix('*').filter(|v| !v.contains(['*', '?'])) {
        let op = if negated { O::NotEndsWith } else { O::EndsWith };
        return (op, inner.to_string());
    }
    (operator, value.to_string())
}

fn is_on(node: &XmlNode, name: &str, default: bool) -> bool {
    match node.attribute(name) {
        Some(value) => value == "1" || value.eq_ignore_ascii_case("true"),
        None => default,
    }
}

fn read_filter_rule(column: &XmlNode) -> Option<TableFilterRule> {
    use TableFilterOperator as O;
    if let Some(filters) = column.child("filters") {
        if filters.child("dateGroupItem").is_some() {
            return None;
        }
        let mut visible: Vec<String> = filters
            .children()
            .filter(|n| n.has_tag_name("filter"))
            .filter_map(|n| n.attribute("val").map(str::to_string))
            .collect();
        if is_on(filters, "blank", false) {
            visible.push(BLANK_FILTER_LABEL.to_string());
        }
        return Some(TableFilterRule::Values { visible });
    }
    if let Some(custom) = column.child("customFilters") {
        let rules: Vec<&XmlNode> = custom
            .children()
            .filter(|n| n.has_tag_name("customFilter"))
            .collect();
        let value = |node: &XmlNode| node.attribute("val").unwrap_or("").to_string();
        return match rules.as_slice() {
            [only] => {
                let operator = operator_of(only.attribute("operator"))?;
                let (operator, value) = unfold_wildcards(operator, &value(only));
                Some(TableFilterRule::Condition {
                    operator,
                    value,
                    value2: String::new(),
                })
            }
            [first, second] => {
                let a = operator_of(first.attribute("operator"))?;
                let b = operator_of(second.attribute("operator"))?;
                let joined = is_on(custom, "and", false);
                let operator = match (a, b, joined) {
                    (O::GreaterThanOrEqual, O::LessThanOrEqual, true) => O::Between,
                    (O::LessThan, O::GreaterThan, false) => O::NotBetween,
                    _ => return None,
                };
                Some(TableFilterRule::Condition {
                    operator,
                    value: value(first),
                    value2: value(second),
                })
            }
            _ => None,
        };
    }
    if let Some(top) = column.child("top10") {
        return Some(TableFilterRule::Top {
            count: top.attribute("val")?.parse().ok()?,
            bottom: !is_on(top, "top", true),
            percent: is_on(top, "percent", false),
        });
    }
    if let Some(dynamic) = column.child("dynamicFilter") {
        let operator = match dynamic.attribute("type")? {
            "aboveAverage" => O::AboveAverage,
            "belowAverage" => O::BelowAverage,
            _ => return None,
        };
        return Some(TableFilterRule::Condition {
            operator,
            value: String::new(),
            value2: String::new(),
        });
    }
    None
}

fn read_filter_state(document: &XmlNode, table: &Table) -> Option<TableFilterState> {
    let mut columns = Vec::new();
    if let Some(filter) = document.child("autoFilter") {
        for column in filter.children().filter(|n| n.has_tag_name("filterColumn")) {
            let Some(rule) = read_filter_rule(column) else {
                continue;
            };
            let index: usize = column.attribute("colId")?.parse().ok()?;
            let Some(definition) = table.columns.get(index) else {
                continue;
            };
            columns.push(TableColumnFilter {
                column: definition.name.clone(),
                rule,
            });
        }
    }
    let sort = document
        .descendants()
        .find(|n| n.has_tag_name("sortCondition"))
        .and_then(|condition| {
            let reference = condition.attribute("ref")?;
            let first = reference.split(':').next()?;
            let letters: String = first.chars().take_while(|c| c.is_alphabetic()).collect();
            let column = ironcalc_base::expressions::utils::column_to_number(&letters).ok()?;
            let area = ironcalc_base::table::table_area(table)?;
            let definition = table.columns.get((column - area.first_column) as usize)?;
            Some(TableSort {
                column: definition.name.clone(),
                descending: is_on(condition, "descending", false),
            })
        });
    (!columns.is_empty() || sort.is_some()).then(|| TableFilterState {
        table: table.name.clone(),
        columns,
        sort,
    })
}

/// A sheet's filter and what it filters by.
pub(super) type LoadedSheetFilter = (SheetFilter, Option<TableFilterState>);

/// The `<autoFilter>` of a worksheet, as opposed to a table's.
pub(super) fn load_sheet_filter(ws: &XmlNode, sheet_id: u32) -> Option<LoadedSheetFilter> {
    use ironcalc_base::expressions::utils::{number_to_column, parse_reference_a1};
    use ironcalc_base::pivot::PivotArea;

    let filter = ws.child("autoFilter")?;
    let reference = filter.attribute("ref")?;
    let (first, last) = reference.split_once(':').unwrap_or((reference, reference));
    let (first, last) = (parse_reference_a1(first)?, parse_reference_a1(last)?);
    let area = PivotArea {
        first_row: first.row,
        first_column: first.column,
        last_row: last.row,
        last_column: last.column,
    };
    let name = sheet_filter_name(sheet_id);
    let mut columns = Vec::new();
    for column in filter.children().filter(|n| n.has_tag_name("filterColumn")) {
        let Some(rule) = read_filter_rule(column) else {
            continue;
        };
        let Some(offset) = column
            .attribute("colId")
            .and_then(|id| id.parse::<i32>().ok())
        else {
            continue;
        };
        if let Some(column) = number_to_column(area.first_column + offset) {
            columns.push(TableColumnFilter { column, rule });
        }
    }
    let sort = ws
        .child("sortState")
        .or_else(|| filter.child("sortState"))
        .and_then(|state| state.child("sortCondition"))
        .and_then(|condition| {
            let reference = condition.attribute("ref")?;
            let cell = parse_reference_a1(reference.split(':').next()?)?;
            Some(TableSort {
                column: number_to_column(cell.column)?,
                descending: is_on(condition, "descending", false),
            })
        });
    let state = (!columns.is_empty() || sort.is_some()).then_some(TableFilterState {
        table: name,
        columns,
        sort,
    });
    Some((SheetFilter { sheet_id, area }, state))
}

fn number_in(node: &XmlNode, name: &str) -> Option<f64> {
    node.child(name)?.text()?.trim().parse().ok()
}

/// Where a shape sits, as a drawing part describes it.
struct Anchor {
    row: i32,
    column: i32,
    offset_x: f64,
    offset_y: f64,
    width: f64,
    height: f64,
    placement: SlicerPlacement,
    print: bool,
    alt_text: String,
}

fn read_anchor(anchor: &XmlNode, worksheet: &Worksheet) -> Option<Anchor> {
    let from = anchor.child("from");
    let (mut row, mut column, mut offset_x, mut offset_y) = (1, 1, 0.0, 0.0);
    if let Some(from) = from {
        column = number_in(from, "col")? as i32 + 1;
        offset_x = number_in(from, "colOff").unwrap_or(0.0) / EMU_PER_PIXEL;
        row = number_in(from, "row")? as i32 + 1;
        offset_y = number_in(from, "rowOff").unwrap_or(0.0) / EMU_PER_PIXEL;
    }
    let (width, height) = if let Some(to) = anchor.child("to") {
        let to_column = number_in(to, "col")? as i32 + 1;
        let to_row = number_in(to, "row")? as i32 + 1;
        let mut width = number_in(to, "colOff").unwrap_or(0.0) / EMU_PER_PIXEL - offset_x;
        for c in column..to_column {
            width += worksheet.get_column_width(c).ok()?;
        }
        let mut height = number_in(to, "rowOff").unwrap_or(0.0) / EMU_PER_PIXEL - offset_y;
        for r in row..to_row {
            height += worksheet.row_height(r).ok()?;
        }
        (width, height)
    } else {
        let extent = anchor
            .descendants()
            .find(|n| n.has_tag_name("ext") && n.has_attribute("cx"))?;
        (
            extent.attribute("cx")?.parse::<f64>().ok()? / EMU_PER_PIXEL,
            extent.attribute("cy")?.parse::<f64>().ok()? / EMU_PER_PIXEL,
        )
    };
    let placement = if anchor.has_tag_name("absoluteAnchor") {
        SlicerPlacement::Free
    } else {
        match anchor.attribute("editAs") {
            Some("oneCell") => SlicerPlacement::Move,
            Some("absolute") => SlicerPlacement::Free,
            _ => SlicerPlacement::MoveAndSize,
        }
    };
    let print = anchor
        .child("clientData")
        .is_none_or(|c| is_on(c, "fPrintsWithSheet", true));
    let alt_text = anchor
        .descendants()
        .find(|n| n.has_tag_name("cNvPr"))
        .and_then(|n| n.attribute("descr"))
        .unwrap_or("")
        .to_string();
    Some(Anchor {
        row,
        column,
        offset_x,
        offset_y,
        width,
        height,
        placement,
        print,
        alt_text,
    })
}

/// A slicer cache: the column or field a slicer controls.
struct Cache {
    source: SlicerSource,
    /// The labels of the items switched off, for fields outside the layout.
    field_hidden: Vec<String>,
}

fn read_cache(
    document: &XmlNode,
    table_ids: &HashMap<u32, String>,
    tables: &HashMap<String, Table>,
    pivots: &[PivotTable],
) -> Option<Cache> {
    let field = document.attribute("sourceName")?.to_string();
    if let Some(link) = document
        .descendants()
        .find(|n| n.has_tag_name("tableSlicerCache"))
    {
        let id: u32 = link.attribute("tableId")?.parse().ok()?;
        let column_id: u32 = link.attribute("column")?.parse().ok()?;
        let name = table_ids.get(&id)?;
        let table = tables.get(name)?;
        let column = table.columns.iter().find(|c| c.id == column_id)?;
        return Some(Cache {
            source: SlicerSource::Table {
                table: name.clone(),
                column: column.name.clone(),
            },
            field_hidden: vec![],
        });
    }
    let list = document.child("pivotTables")?;
    let links: Vec<SlicerPivot> = list
        .children()
        .filter(|n| n.has_tag_name("pivotTable"))
        .filter_map(|n| {
            Some(SlicerPivot {
                name: n.attribute("name")?.to_string(),
                sheet_id: n.attribute("tabId")?.parse().ok()?,
            })
        })
        .collect();
    let first = links.iter().find_map(|l| {
        pivots
            .iter()
            .find(|p| p.name == l.name && p.sheet_id == l.sheet_id)
    })?;

    let mut field_hidden = Vec::new();
    let in_layout = first
        .rows
        .iter()
        .chain(&first.columns)
        .chain(&first.filters)
        .any(|f| f.field == field);
    if !in_layout {
        let labels = first
            .raw
            .as_deref()
            .and_then(|raw| parse_xml(&raw.cache_xml))
            .map(|cache| shared_labels(&cache, &field))
            .unwrap_or_default();
        for item in document
            .descendants()
            .filter(|n| n.has_tag_name("i") && n.attribute("s") != Some("1"))
        {
            let position: usize = item.attribute("x")?.parse().ok()?;
            if let Some(label) = labels.get(position) {
                field_hidden.push(label.clone());
            }
        }
    }
    Some(Cache {
        source: SlicerSource::Pivot {
            pivots: links,
            field,
        },
        field_hidden,
    })
}

fn shared_labels(cache: &XmlNode, field: &str) -> Vec<String> {
    let Some(definition) = cache
        .descendants()
        .find(|n| n.has_tag_name("cacheField") && n.attribute("name") == Some(field))
    else {
        return vec![];
    };
    let Some(shared) = definition.child("sharedItems") else {
        return vec![];
    };
    shared
        .children()
        .map(|item| match item.attribute("v") {
            Some(value) => value.to_string(),
            None => BLANK_ITEM_LABEL.to_string(),
        })
        .collect()
}

/// Filters of the tables, and every slicer of the file.
pub(super) fn load_filters_and_slicers<R: Read + Seek>(
    archive: &mut Archive<R>,
    rels: &HashMap<String, Relationship>,
    worksheets: &[Worksheet],
    tables: &HashMap<String, Table>,
    pivots: &mut [PivotTable],
) -> (Vec<TableFilterState>, Vec<Slicer>) {
    let mut filters = Vec::new();
    let mut table_ids = HashMap::new();
    struct SheetSlicers {
        worksheet: usize,
        anchors: HashMap<String, XmlNode>,
        parts: Vec<String>,
    }
    let mut sheets = Vec::new();
    for (name, part) in sheet_parts(archive, rels) {
        let Some(worksheet) = worksheets.iter().position(|w| w.name == name) else {
            continue;
        };
        let mut sheet = SheetSlicers {
            worksheet,
            anchors: HashMap::new(),
            parts: vec![],
        };
        for (_, kind, target) in read_rels(archive, &part) {
            let path = resolve(&part, &target);
            if kind.ends_with("/table") {
                let Some(document) = read_part(archive, &path).and_then(|t| parse_xml(&t)) else {
                    continue;
                };
                let Some(table) = document.attribute("name").and_then(|n| tables.get(n)) else {
                    continue;
                };
                if let Some(id) = document.attribute("id").and_then(|i| i.parse::<u32>().ok()) {
                    table_ids.insert(id, table.name.clone());
                }
                filters.extend(read_filter_state(&document, table));
            } else if kind.ends_with("/slicer") {
                sheet.parts.push(path);
            } else if kind.ends_with("/drawing") {
                let Some(document) = read_part(archive, &path).and_then(|t| parse_xml(&t)) else {
                    continue;
                };
                for anchor in document.children() {
                    if let Some(slicer) = anchor
                        .descendants()
                        .find(|n| n.has_tag_name("slicer") && n.has_attribute("name"))
                    {
                        if let Some(name) = slicer.attribute("name") {
                            sheet.anchors.insert(name.to_string(), anchor.clone());
                        }
                    }
                }
            }
        }
        if !sheet.parts.is_empty() {
            sheets.push(sheet);
        }
    }

    let mut caches = HashMap::new();
    for relationship in rels.values() {
        if !relationship.rel_type.ends_with("/slicerCache") {
            continue;
        }
        let path = resolve("xl/workbook.xml", &relationship.target);
        let Some(document) = read_part(archive, &path).and_then(|t| parse_xml(&t)) else {
            continue;
        };
        let Some(name) = document.attribute("name").map(str::to_string) else {
            continue;
        };
        if let Some(cache) = read_cache(&document, &table_ids, tables, pivots) {
            caches.insert(name, cache);
        }
    }

    let mut slicers = Vec::new();
    for sheet in &sheets {
        let worksheet = &worksheets[sheet.worksheet];
        for path in &sheet.parts {
            let Some(document) = read_part(archive, path).and_then(|t| parse_xml(&t)) else {
                continue;
            };
            for node in document.children().filter(|n| n.has_tag_name("slicer")) {
                let (Some(name), Some(cache_name)) =
                    (node.attribute("name"), node.attribute("cache"))
                else {
                    continue;
                };
                let (Some(cache), Some(anchor)) = (
                    caches.get(cache_name),
                    sheet
                        .anchors
                        .get(name)
                        .and_then(|a| read_anchor(a, worksheet)),
                ) else {
                    continue;
                };
                if let SlicerSource::Pivot {
                    pivots: links,
                    field,
                } = &cache.source
                {
                    if !cache.field_hidden.is_empty() {
                        for pivot in pivots.iter_mut().filter(|p| {
                            links
                                .iter()
                                .any(|l| l.name == p.name && l.sheet_id == p.sheet_id)
                        }) {
                            if !pivot.slicer_filters.iter().any(|f| f.field == *field) {
                                pivot.slicer_filters.push(PivotField {
                                    field: field.clone(),
                                    hidden_items: cache.field_hidden.clone(),
                                    ..Default::default()
                                });
                            }
                        }
                    }
                }
                let row_height = node
                    .attribute("rowHeight")
                    .and_then(|v| v.parse::<f64>().ok())
                    .map_or(25.0, |emu| emu / EMU_PER_PIXEL);
                slicers.push(Slicer {
                    name: name.to_string(),
                    caption: node.attribute("caption").unwrap_or(name).to_string(),
                    show_header: is_on(node, "showCaption", true),
                    sheet_id: worksheet.sheet_id,
                    source: cache.source.clone(),
                    row: anchor.row,
                    column: anchor.column,
                    offset_x: anchor.offset_x,
                    offset_y: anchor.offset_y,
                    width: anchor.width.max(40.0),
                    height: anchor.height.max(30.0),
                    button_columns: node
                        .attribute("columnCount")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1),
                    button_height: row_height,
                    placement: anchor.placement,
                    locked: is_on(node, "lockedPosition", false),
                    print: anchor.print,
                    style: node
                        .attribute("style")
                        .unwrap_or(DEFAULT_SLICER_STYLE)
                        .to_string(),
                    alt_text: anchor.alt_text,
                    multi_select: false,
                });
            }
        }
    }
    (filters, slicers)
}
