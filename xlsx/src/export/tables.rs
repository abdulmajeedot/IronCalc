//! Writes tables and slicers. Unlike pivots, nothing is carried over from the
//! original file: every part is generated from the workbook.

use std::collections::HashMap;

use ironcalc_base::table::{
    area_reference, table_area, table_data_rows, Slicer, SlicerPlacement, SlicerSource,
    TableColumnFilter, TableFilterOperator, TableFilterRule, BLANK_FILTER_LABEL,
};
use ironcalc_base::types::{Table, Workbook, Worksheet};
use ironcalc_base::Model;

use super::escape::escape_xml;
use super::pivot::ExportedPivot;
use super::xml_constants::XML_DECLARATION;

const MAIN_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const X14_NS: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const X15_NS: &str = "http://schemas.microsoft.com/office/spreadsheetml/2010/11/main";
const MC_NS: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const TABLE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/table";
const DRAWING_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing";
const SLICER_REL: &str = "http://schemas.microsoft.com/office/2007/relationships/slicer";
const SLICER_CACHE_REL: &str = "http://schemas.microsoft.com/office/2007/relationships/slicerCache";

const EMU_PER_PIXEL: f64 = 9525.0;

fn emu(pixels: f64) -> i64 {
    (pixels * EMU_PER_PIXEL).round() as i64
}

/// What one worksheet part has to mention.
#[derive(Default)]
pub(crate) struct SheetExtras {
    rels: Vec<(&'static str, String)>,
    drawing: Option<usize>,
    tables: Vec<usize>,
    pivot_slicers: Option<usize>,
    table_slicers: Option<usize>,
}

impl SheetExtras {
    /// `Relationship` elements, numbered after the `base` the sheet already has.
    pub(crate) fn rels_xml(&self, base: usize) -> Vec<String> {
        self.rels
            .iter()
            .enumerate()
            .map(|(offset, (kind, target))| {
                format!(
                    "<Relationship Id=\"rId{}\" Type=\"{kind}\" Target=\"{target}\"/>",
                    base + offset + 1
                )
            })
            .collect()
    }

    /// The `<drawing>` and `<tableParts>` elements.
    pub(crate) fn parts_xml(&self, base: usize) -> String {
        let mut xml = String::new();
        if let Some(drawing) = self.drawing {
            xml.push_str(&format!("<drawing r:id=\"rId{}\"/>", base + drawing + 1));
        }
        if !self.tables.is_empty() {
            let parts: String = self
                .tables
                .iter()
                .map(|t| format!("<tablePart r:id=\"rId{}\"/>", base + t + 1))
                .collect();
            xml.push_str(&format!(
                "<tableParts count=\"{}\">{parts}</tableParts>",
                self.tables.len()
            ));
        }
        xml
    }

    /// The `<ext>` elements listing this sheet's slicer parts.
    pub(crate) fn ext_xml(&self, base: usize) -> String {
        let mut xml = String::new();
        if let Some(part) = self.pivot_slicers {
            xml.push_str(&format!(
                "<ext uri=\"{{A8765BA9-456A-4dab-B4F3-ACF838C121DE}}\" xmlns:x14=\"{X14_NS}\"><x14:slicerList><x14:slicer r:id=\"rId{}\"/></x14:slicerList></ext>",
                base + part + 1
            ));
        }
        if let Some(part) = self.table_slicers {
            xml.push_str(&format!(
                "<ext uri=\"{{3A4CF648-6AED-40f4-86FF-DC5316D8AED3}}\" xmlns:x14=\"{X14_NS}\"><x14:slicerList><x14:slicer r:id=\"rId{}\"/></x14:slicerList></ext>",
                base + part + 1
            ));
        }
        xml
    }
}

/// Every table and slicer part of a workbook, ready to write.
#[derive(Default)]
pub(crate) struct ExportedExtras {
    pub(crate) parts: Vec<(String, String)>,
    pub(crate) content_types: Vec<String>,
    pub(crate) sheets: Vec<SheetExtras>,
    /// The workbook part's relationships to slicer caches: `(target, is_table)`.
    caches: Vec<(String, bool)>,
    /// Each slicer cache needs a defined name.
    names: Vec<String>,
}

impl ExportedExtras {
    pub(crate) fn defined_names_xml(&self) -> String {
        self.names
            .iter()
            .map(|name| {
                format!(
                    "<definedName name=\"{}\">#N/A</definedName>",
                    escape_xml(name)
                )
            })
            .collect()
    }

    fn workbook_rel_id(
        &self,
        workbook: &Workbook,
        pivots: &[ExportedPivot],
        index: usize,
    ) -> usize {
        // Worksheets, then styles, shared strings, metadata and theme, then pivots
        workbook.worksheets.len() + 4 + pivots.len() + index + 1
    }

    pub(crate) fn workbook_rels(
        &self,
        workbook: &Workbook,
        pivots: &[ExportedPivot],
    ) -> Vec<String> {
        self.caches
            .iter()
            .enumerate()
            .map(|(index, (target, _))| {
                format!(
                    "<Relationship Id=\"rId{}\" Type=\"{SLICER_CACHE_REL}\" Target=\"{target}\"/>",
                    self.workbook_rel_id(workbook, pivots, index)
                )
            })
            .collect()
    }

    /// The `<extLst>` of the workbook part.
    pub(crate) fn workbook_ext_xml(&self, workbook: &Workbook, pivots: &[ExportedPivot]) -> String {
        let list = |is_table: bool| -> String {
            self.caches
                .iter()
                .enumerate()
                .filter(|(_, (_, table))| *table == is_table)
                .map(|(index, _)| {
                    format!(
                        "<x14:slicerCache r:id=\"rId{}\"/>",
                        self.workbook_rel_id(workbook, pivots, index)
                    )
                })
                .collect()
        };
        let mut xml = String::new();
        let pivot_caches = list(false);
        if !pivot_caches.is_empty() {
            xml.push_str(&format!(
                "<ext uri=\"{{BBE1A952-AA13-448e-AADC-164F8A28A991}}\" xmlns:x14=\"{X14_NS}\"><x14:slicerCaches>{pivot_caches}</x14:slicerCaches></ext>"
            ));
        }
        let table_caches = list(true);
        if !table_caches.is_empty() {
            xml.push_str(&format!(
                "<ext uri=\"{{46BE6895-7355-4a93-B00E-2C351335B9C9}}\" xmlns:x15=\"{X15_NS}\"><x15:slicerCaches xmlns:x14=\"{X14_NS}\">{table_caches}</x15:slicerCaches></ext>"
            ));
        }
        if xml.is_empty() {
            return xml;
        }
        format!("<extLst>{xml}</extLst>")
    }
}

fn custom_filter(operator: &str, value: &str) -> String {
    format!(
        "<customFilter operator=\"{operator}\" val=\"{}\"/>",
        escape_xml(value)
    )
}

fn filter_xml(index: usize, filter: &TableColumnFilter) -> String {
    use TableFilterOperator as O;
    let body = match &filter.rule {
        TableFilterRule::Values { visible } => {
            let blank = visible.iter().any(|v| v == BLANK_FILTER_LABEL);
            let items: String = visible
                .iter()
                .filter(|v| *v != BLANK_FILTER_LABEL)
                .map(|v| format!("<filter val=\"{}\"/>", escape_xml(v)))
                .collect();
            let blank = if blank { " blank=\"1\"" } else { "" };
            format!("<filters{blank}>{items}</filters>")
        }
        TableFilterRule::Top {
            count,
            bottom,
            percent,
        } => format!(
            "<top10{}{} val=\"{count}\"/>",
            if *bottom { " top=\"0\"" } else { "" },
            if *percent { " percent=\"1\"" } else { "" },
        ),
        TableFilterRule::Condition {
            operator,
            value,
            value2,
        } => match operator {
            O::AboveAverage => "<dynamicFilter type=\"aboveAverage\"/>".to_string(),
            O::BelowAverage => "<dynamicFilter type=\"belowAverage\"/>".to_string(),
            O::Between => format!(
                "<customFilters and=\"1\">{}{}</customFilters>",
                custom_filter("greaterThanOrEqual", value),
                custom_filter("lessThanOrEqual", value2)
            ),
            O::NotBetween => format!(
                "<customFilters>{}{}</customFilters>",
                custom_filter("lessThan", value),
                custom_filter("greaterThan", value2)
            ),
            other => {
                let (name, text) = match other {
                    O::Equals => ("equal", value.clone()),
                    O::NotEquals => ("notEqual", value.clone()),
                    O::BeginsWith => ("equal", format!("{value}*")),
                    O::NotBeginsWith => ("notEqual", format!("{value}*")),
                    O::EndsWith => ("equal", format!("*{value}")),
                    O::NotEndsWith => ("notEqual", format!("*{value}")),
                    O::Contains => ("equal", format!("*{value}*")),
                    O::NotContains => ("notEqual", format!("*{value}*")),
                    O::GreaterThan => ("greaterThan", value.clone()),
                    O::GreaterThanOrEqual => ("greaterThanOrEqual", value.clone()),
                    O::LessThan => ("lessThan", value.clone()),
                    _ => ("lessThanOrEqual", value.clone()),
                };
                format!(
                    "<customFilters>{}</customFilters>",
                    custom_filter(name, &text)
                )
            }
        },
    };
    format!("<filterColumn colId=\"{index}\">{body}</filterColumn>")
}

fn column_name_xml(name: &str) -> String {
    escape_xml(name).replace('\n', "_x000a_")
}

fn table_xml(model: &Model, table: &Table, id: usize) -> Option<String> {
    let area = table_area(table)?;
    let (first, last) = table_data_rows(table, &area);
    let state = model.table_filter_state(&table.name);
    let mut filter_section = String::new();
    if table.has_filters && table.header_row_count > 0 {
        let filter_area = ironcalc_base::pivot::PivotArea {
            last_row: last,
            ..area
        };
        let columns: String = state
            .map(|s| {
                let mut filters: Vec<(usize, &TableColumnFilter)> = s
                    .columns
                    .iter()
                    .filter_map(|f| {
                        Some((
                            ironcalc_base::table::table_column_index(table, &f.column)?,
                            f,
                        ))
                    })
                    .collect();
                filters.sort_by_key(|(index, _)| *index);
                filters.iter().map(|(i, f)| filter_xml(*i, f)).collect()
            })
            .unwrap_or_default();
        filter_section = format!(
            "<autoFilter ref=\"{}\">{columns}</autoFilter>",
            area_reference(&filter_area)
        );
    }
    let sort_section = state
        .and_then(|s| s.sort.as_ref())
        .and_then(|sort| {
            let offset = ironcalc_base::table::table_column_index(table, &sort.column)? as i32;
            let column = area.first_column + offset;
            let data = ironcalc_base::pivot::PivotArea {
                first_row: first,
                last_row: last,
                ..area
            };
            let key = ironcalc_base::pivot::PivotArea {
                first_column: column,
                last_column: column,
                ..data
            };
            let descending = if sort.descending {
                " descending=\"1\""
            } else {
                ""
            };
            Some(format!(
                "<sortState ref=\"{}\"><sortCondition{descending} ref=\"{}\"/></sortState>",
                area_reference(&data),
                area_reference(&key)
            ))
        })
        .unwrap_or_default();
    let columns: String = table
        .columns
        .iter()
        .map(|column| {
            let label = column
                .totals_row_label
                .as_ref()
                .map(|l| format!(" totalsRowLabel=\"{}\"", escape_xml(l)))
                .unwrap_or_default();
            let function = column
                .totals_row_function
                .as_ref()
                .map(|f| format!(" totalsRowFunction=\"{}\"", escape_xml(f)))
                .unwrap_or_default();
            format!(
                "<tableColumn id=\"{}\" name=\"{}\"{label}{function}/>",
                column.id,
                column_name_xml(&column.name)
            )
        })
        .collect();
    let header = if table.header_row_count == 0 {
        " headerRowCount=\"0\""
    } else {
        ""
    };
    let totals = if table.totals_row_count > 0 {
        format!(" totalsRowCount=\"{}\"", table.totals_row_count)
    } else {
        " totalsRowShown=\"0\"".to_string()
    };
    let info = &table.style_info;
    let style_name = info
        .name
        .as_ref()
        .map(|n| format!(" name=\"{}\"", escape_xml(n)))
        .unwrap_or_default();
    let name = escape_xml(&table.name);
    Some(format!(
        "{XML_DECLARATION}\n<table xmlns=\"{MAIN_NS}\" id=\"{id}\" name=\"{name}\" displayName=\"{name}\" ref=\"{}\"{header}{totals}>{filter_section}{sort_section}<tableColumns count=\"{}\">{columns}</tableColumns><tableStyleInfo{style_name} showFirstColumn=\"{}\" showLastColumn=\"{}\" showRowStripes=\"{}\" showColumnStripes=\"{}\"/></table>",
        table.reference,
        table.columns.len(),
        u8::from(info.show_first_column),
        u8::from(info.show_last_column),
        u8::from(info.show_row_stripes),
        u8::from(info.show_column_stripes),
    ))
}

fn cache_name(slicer: &Slicer, used: &mut Vec<String>) -> String {
    let clean: String = slicer
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let base = format!("Slicer_{clean}");
    let mut name = base.clone();
    let mut number = 1;
    while used.iter().any(|u| u.eq_ignore_ascii_case(&name)) {
        number += 1;
        name = format!("{base}{number}");
    }
    used.push(name.clone());
    name
}

fn table_cache_xml(name: &str, column: &str, table_id: usize, column_id: u32) -> String {
    format!(
        "{XML_DECLARATION}\n<slicerCacheDefinition xmlns=\"{X14_NS}\" xmlns:mc=\"{MC_NS}\" mc:Ignorable=\"x\" xmlns:x=\"{MAIN_NS}\" name=\"{}\" sourceName=\"{}\"><extLst><x:ext uri=\"{{2F2917AC-EB37-4324-AD4E-5DD8C200BD13}}\" xmlns:x15=\"{X15_NS}\"><x15:tableSlicerCache tableId=\"{table_id}\" column=\"{column_id}\"/></x:ext></extLst></slicerCacheDefinition>",
        escape_xml(name),
        escape_xml(column),
    )
}

/// One `(column, x, row, y)` corner of a drawing anchor, zero-based.
type Corner = (i32, i64, i32, i64);

/// The two corners of the box a slicer covers.
fn corner_after(worksheet: &Worksheet, slicer: &Slicer) -> (Corner, Corner) {
    let from = (
        slicer.column - 1,
        emu(slicer.offset_x),
        slicer.row - 1,
        emu(slicer.offset_y),
    );
    let mut column = slicer.column;
    let mut x = slicer.offset_x + slicer.width;
    while let Ok(width) = worksheet.get_column_width(column) {
        if x < width {
            break;
        }
        x -= width;
        column += 1;
    }
    let mut row = slicer.row;
    let mut y = slicer.offset_y + slicer.height;
    while let Ok(height) = worksheet.row_height(row) {
        if y < height {
            break;
        }
        y -= height;
        row += 1;
    }
    (from, (column - 1, emu(x), row - 1, emu(y)))
}

fn anchor_xml(worksheet: &Worksheet, slicer: &Slicer, shape_id: usize, is_table: bool) -> String {
    let (from, to) = corner_after(worksheet, slicer);
    let edit_as = match slicer.placement {
        SlicerPlacement::MoveAndSize => "twoCell",
        SlicerPlacement::Move => "oneCell",
        SlicerPlacement::Free => "absolute",
    };
    let corner = |tag: &str, c: Corner| {
        format!(
            "<xdr:{tag}><xdr:col>{}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{}</xdr:row><xdr:rowOff>{}</xdr:rowOff></xdr:{tag}>",
            c.0, c.1, c.2, c.3
        )
    };
    let name = escape_xml(&slicer.name);
    let descr = if slicer.alt_text.is_empty() {
        String::new()
    } else {
        format!(" descr=\"{}\"", escape_xml(&slicer.alt_text))
    };
    let requires = if is_table {
        "<mc:Choice xmlns:sle15=\"http://schemas.microsoft.com/office/drawing/2012/slicer\" Requires=\"sle15\">"
    } else {
        "<mc:Choice xmlns:a14=\"http://schemas.microsoft.com/office/drawing/2010/main\" Requires=\"a14\">"
    };
    let print = if slicer.print {
        ""
    } else {
        " fPrintsWithSheet=\"0\""
    };
    format!(
        "<xdr:twoCellAnchor editAs=\"{edit_as}\">{}{}<mc:AlternateContent xmlns:mc=\"{MC_NS}\">{requires}<xdr:graphicFrame macro=\"\"><xdr:nvGraphicFramePr><xdr:cNvPr id=\"{shape_id}\" name=\"{name}\"{descr}/><xdr:cNvGraphicFramePr/></xdr:nvGraphicFramePr><xdr:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></xdr:xfrm><a:graphic><a:graphicData uri=\"http://schemas.microsoft.com/office/drawing/2010/slicer\"><sle:slicer xmlns:sle=\"http://schemas.microsoft.com/office/drawing/2010/slicer\" name=\"{name}\"/></a:graphicData></a:graphic></xdr:graphicFrame></mc:Choice><mc:Fallback xmlns=\"\"><xdr:sp macro=\"\" textlink=\"\"><xdr:nvSpPr><xdr:cNvPr id=\"0\" name=\"\"/><xdr:cNvSpPr><a:spLocks noTextEdit=\"1\"/></xdr:cNvSpPr></xdr:nvSpPr><xdr:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:solidFill><a:prstClr val=\"white\"/></a:solidFill></xdr:spPr><xdr:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"en-US\"/><a:t>Slicer</a:t></a:r></a:p></xdr:txBody></xdr:sp></mc:Fallback></mc:AlternateContent><xdr:clientData{print}/></xdr:twoCellAnchor>",
        corner("from", from),
        corner("to", to),
        emu(slicer.width),
        emu(slicer.height),
    )
}

fn slicers_part_xml(entries: &[(&Slicer, String)]) -> String {
    let items: String = entries
        .iter()
        .map(|(slicer, cache)| {
            let caption = if slicer.show_header {
                ""
            } else {
                " showCaption=\"0\""
            };
            format!(
                "<slicer name=\"{}\" cache=\"{}\" caption=\"{}\"{caption} columnCount=\"{}\" style=\"{}\" rowHeight=\"{}\"/>",
                escape_xml(&slicer.name),
                escape_xml(cache),
                escape_xml(&slicer.caption),
                slicer.button_columns.max(1),
                escape_xml(&slicer.style),
                emu(slicer.button_height),
            )
        })
        .collect();
    format!(
        "{XML_DECLARATION}\n<slicers xmlns=\"{X14_NS}\" xmlns:mc=\"{MC_NS}\" mc:Ignorable=\"x\" xmlns:x=\"{MAIN_NS}\">{items}</slicers>"
    )
}

/// Builds every table and slicer part. Pivots gain the id a slicer refers to.
pub(crate) fn export_extras(model: &Model, pivots: &mut [ExportedPivot]) -> ExportedExtras {
    let workbook = &model.workbook;
    let mut extras = ExportedExtras::default();
    extras
        .sheets
        .resize_with(workbook.worksheets.len(), SheetExtras::default);

    let mut names: Vec<&String> = workbook.tables.keys().collect();
    names.sort();
    let mut table_numbers: HashMap<&str, usize> = HashMap::new();
    let mut number = 0;
    for name in names {
        let table = &workbook.tables[name];
        let Some(sheet) = model.table_sheet(table) else {
            continue;
        };
        number += 1;
        let Some(xml) = table_xml(model, table, number) else {
            number -= 1;
            continue;
        };
        table_numbers.insert(name.as_str(), number);
        extras
            .parts
            .push((format!("xl/tables/table{number}.xml"), xml));
        extras.content_types.push(format!(
            r#"<Override PartName="/xl/tables/table{number}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml"/>"#
        ));
        let extra = &mut extras.sheets[sheet as usize];
        extra
            .rels
            .push((TABLE_REL, format!("../tables/table{number}.xml")));
        extra.tables.push(extra.rels.len() - 1);
    }

    let mut used_names = Vec::new();
    let mut cache_number = 0;
    let mut part_number = 0;
    let mut drawing_number = 0;
    for (sheet, worksheet) in workbook.worksheets.iter().enumerate() {
        let on_sheet: Vec<&Slicer> = workbook
            .slicers
            .iter()
            .filter(|s| s.sheet_id == worksheet.sheet_id)
            .collect();
        let mut table_entries = Vec::new();
        let mut pivot_entries = Vec::new();
        let mut anchors = String::new();
        let mut shape_id = 1;
        for slicer in on_sheet {
            let cache_xml = match &slicer.source {
                SlicerSource::Table { table, column } => {
                    let (Some(number), Some(definition)) = (
                        table_numbers.get(table.as_str()),
                        workbook.tables.get(table),
                    ) else {
                        continue;
                    };
                    let Some(column) = definition.columns.iter().find(|c| c.name == *column) else {
                        continue;
                    };
                    let name = cache_name(slicer, &mut used_names);
                    let xml = table_cache_xml(&name, &column.name, *number, column.id);
                    Some((name, xml, true))
                }
                SlicerSource::Pivot {
                    pivots: links,
                    field,
                } => {
                    let Some(first) = pivots
                        .iter()
                        .position(|p| links.iter().any(|l| p.is_named(&l.name, l.sheet_id)))
                    else {
                        continue;
                    };
                    let labels = pivots[first].shared_labels(field);
                    if labels.is_empty() {
                        continue;
                    }
                    let id = pivots[first].slicer_cache_id();
                    let list: String = links
                        .iter()
                        .map(|l| {
                            format!(
                                "<pivotTable tabId=\"{}\" name=\"{}\"/>",
                                l.sheet_id,
                                escape_xml(&l.name)
                            )
                        })
                        .collect();
                    let hidden = workbook
                        .pivot_tables
                        .iter()
                        .find(|p| {
                            links
                                .iter()
                                .any(|l| l.name == p.name && l.sheet_id == p.sheet_id)
                        })
                        .and_then(|p| {
                            p.rows
                                .iter()
                                .chain(&p.columns)
                                .chain(&p.filters)
                                .chain(&p.slicer_filters)
                                .find(|f| f.field == *field)
                        })
                        .map(|f| f.hidden_items.clone())
                        .unwrap_or_default();
                    let items: String = labels
                        .iter()
                        .enumerate()
                        .map(|(x, label)| {
                            let on = !hidden.iter().any(|h| h == label);
                            format!("<i x=\"{x}\" s=\"{}\"/>", u8::from(on))
                        })
                        .collect();
                    let name = cache_name(slicer, &mut used_names);
                    let xml = format!(
                        "{XML_DECLARATION}\n<slicerCacheDefinition xmlns=\"{X14_NS}\" xmlns:mc=\"{MC_NS}\" mc:Ignorable=\"x\" xmlns:x=\"{MAIN_NS}\" name=\"{}\" sourceName=\"{}\"><pivotTables>{list}</pivotTables><data><tabular pivotCacheId=\"{id}\"><items count=\"{}\">{items}</items></tabular></data></slicerCacheDefinition>",
                        escape_xml(&name),
                        escape_xml(field),
                        labels.len(),
                    );
                    Some((name, xml, false))
                }
            };
            let Some((name, xml, is_table)) = cache_xml else {
                continue;
            };
            cache_number += 1;
            extras.parts.push((
                format!("xl/slicerCaches/slicerCache{cache_number}.xml"),
                xml,
            ));
            extras.content_types.push(format!(
                r#"<Override PartName="/xl/slicerCaches/slicerCache{cache_number}.xml" ContentType="application/vnd.ms-excel.slicerCache+xml"/>"#
            ));
            extras.caches.push((
                format!("slicerCaches/slicerCache{cache_number}.xml"),
                is_table,
            ));
            extras.names.push(name.clone());
            shape_id += 1;
            anchors.push_str(&anchor_xml(worksheet, slicer, shape_id, is_table));
            if is_table {
                table_entries.push((slicer, name));
            } else {
                pivot_entries.push((slicer, name));
            }
        }
        if anchors.is_empty() {
            continue;
        }
        let extra = &mut extras.sheets[sheet];
        for (entries, is_table) in [(&pivot_entries, false), (&table_entries, true)] {
            if entries.is_empty() {
                continue;
            }
            part_number += 1;
            extras.parts.push((
                format!("xl/slicers/slicer{part_number}.xml"),
                slicers_part_xml(entries),
            ));
            extras.content_types.push(format!(
                r#"<Override PartName="/xl/slicers/slicer{part_number}.xml" ContentType="application/vnd.ms-excel.slicer+xml"/>"#
            ));
            extra
                .rels
                .push((SLICER_REL, format!("../slicers/slicer{part_number}.xml")));
            let index = Some(extra.rels.len() - 1);
            if is_table {
                extra.table_slicers = index;
            } else {
                extra.pivot_slicers = index;
            }
        }
        drawing_number += 1;
        extras.parts.push((
            format!("xl/drawings/drawing{drawing_number}.xml"),
            format!(
                "{XML_DECLARATION}\n<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">{anchors}</xdr:wsDr>"
            ),
        ));
        extras.content_types.push(format!(
            r#"<Override PartName="/xl/drawings/drawing{drawing_number}.xml" ContentType="application/vnd.openxmlformats-officedocument.drawing+xml"/>"#
        ));
        extra.rels.push((
            DRAWING_REL,
            format!("../drawings/drawing{drawing_number}.xml"),
        ));
        extra.drawing = Some(extra.rels.len() - 1);
    }
    extras
}
