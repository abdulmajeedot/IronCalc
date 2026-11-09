//! Reads data validation rules: the plain `<dataValidations>` of a worksheet
//! and the x14 ones in its `<extLst>`, which is where Excel puts lists that
//! come from another sheet.

use ironcalc_base::data_validation::{
    DataValidation, ValidationAlertStyle, ValidationKind, ValidationOperator, ValidationSettings,
};

use super::shared_strings::decode_xlsx_escapes;
use super::xml::XmlNode;

fn flag(node: &XmlNode, name: &str) -> bool {
    matches!(node.attribute(name), Some("1") | Some("true"))
}

fn text(node: &XmlNode, name: &str) -> String {
    decode_xlsx_escapes(node.attribute(name).unwrap_or(""))
}

fn kind(node: &XmlNode) -> ValidationKind {
    match node.attribute("type") {
        Some("whole") => ValidationKind::Whole,
        Some("decimal") => ValidationKind::Decimal,
        Some("list") => ValidationKind::List,
        Some("date") => ValidationKind::Date,
        Some("time") => ValidationKind::Time,
        Some("textLength") => ValidationKind::TextLength,
        Some("custom") => ValidationKind::Custom,
        _ => ValidationKind::Any,
    }
}

fn operator(node: &XmlNode) -> ValidationOperator {
    match node.attribute("operator") {
        Some("notBetween") => ValidationOperator::NotBetween,
        Some("equal") => ValidationOperator::Equal,
        Some("notEqual") => ValidationOperator::NotEqual,
        Some("greaterThan") => ValidationOperator::GreaterThan,
        Some("lessThan") => ValidationOperator::LessThan,
        Some("greaterThanOrEqual") => ValidationOperator::GreaterThanOrEqual,
        Some("lessThanOrEqual") => ValidationOperator::LessThanOrEqual,
        _ => ValidationOperator::Between,
    }
}

fn alert_style(node: &XmlNode) -> ValidationAlertStyle {
    match node.attribute("errorStyle") {
        Some("warning") => ValidationAlertStyle::Warning,
        Some("information") => ValidationAlertStyle::Information,
        _ => ValidationAlertStyle::Stop,
    }
}

/// The text of a `<formula1>`, or of the `<xm:f>` inside an `<x14:formula1>`.
fn formula(node: &XmlNode, name: &str) -> String {
    let Some(element) = node.children().find(|n| n.has_tag_name(name)) else {
        return String::new();
    };
    let inner = element.children().find(|n| n.has_tag_name("f"));
    inner
        .unwrap_or(element)
        .text()
        .unwrap_or("")
        .trim()
        .trim_start_matches('=')
        .to_string()
}

fn rule(node: &XmlNode, sheet_id: u32, sqref: String) -> Option<DataValidation> {
    if sqref.trim().is_empty() {
        return None;
    }
    let settings = ValidationSettings {
        kind: kind(node),
        operator: operator(node),
        formula1: formula(node, "formula1"),
        formula2: formula(node, "formula2"),
        ignore_blank: flag(node, "allowBlank"),
        // Despite its name, showDropDown="1" hides the dropdown
        in_cell_dropdown: !flag(node, "showDropDown"),
        show_input: flag(node, "showInputMessage"),
        input_title: text(node, "promptTitle"),
        input_message: text(node, "prompt"),
        show_error: flag(node, "showErrorMessage"),
        error_style: alert_style(node),
        error_title: text(node, "errorTitle"),
        error_message: text(node, "error"),
    };
    if settings.is_empty() {
        return None;
    }
    Some(DataValidation {
        sheet_id,
        sqref,
        settings,
    })
}

pub(super) fn load_data_validations(ws: &XmlNode, sheet_id: u32) -> Vec<DataValidation> {
    let mut rules = Vec::new();
    for group in ws.children().filter(|n| n.has_tag_name("dataValidations")) {
        for node in group
            .children()
            .filter(|n| n.has_tag_name("dataValidation"))
        {
            let sqref = node.attribute("sqref").unwrap_or("").to_string();
            rules.extend(rule(node, sheet_id, sqref));
        }
    }
    for ext_list in ws.children().filter(|n| n.has_tag_name("extLst")) {
        for ext in ext_list.children().filter(|n| n.has_tag_name("ext")) {
            for group in ext.children().filter(|n| n.has_tag_name("dataValidations")) {
                for node in group
                    .children()
                    .filter(|n| n.has_tag_name("dataValidation"))
                {
                    let sqref = node
                        .children()
                        .find(|n| n.has_tag_name("sqref"))
                        .and_then(|n| n.text())
                        .unwrap_or("")
                        .to_string();
                    rules.extend(rule(node, sheet_id, sqref));
                }
            }
        }
    }
    rules
}
