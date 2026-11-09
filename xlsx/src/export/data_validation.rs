//! Writes a worksheet's `<dataValidations>`.

use ironcalc_base::data_validation::{
    DataValidation, ValidationAlertStyle, ValidationKind, ValidationOperator,
};
use ironcalc_base::types::{Workbook, Worksheet};

use super::escape::escape_xml;

// Excel treats a file with longer text than these as corrupt
const TITLE_LIMIT: usize = 32;
const MESSAGE_LIMIT: usize = 255;

fn attribute(name: &str, value: &str, limit: usize) -> String {
    if value.is_empty() {
        return String::new();
    }
    let value: String = value.chars().take(limit).collect();
    format!(" {name}=\"{}\"", escape_xml(&value))
}

fn flag(name: &str, on: bool) -> String {
    if on {
        format!(" {name}=\"1\"")
    } else {
        String::new()
    }
}

fn kind_name(kind: ValidationKind) -> Option<&'static str> {
    Some(match kind {
        ValidationKind::Any => return None,
        ValidationKind::Whole => "whole",
        ValidationKind::Decimal => "decimal",
        ValidationKind::List => "list",
        ValidationKind::Date => "date",
        ValidationKind::Time => "time",
        ValidationKind::TextLength => "textLength",
        ValidationKind::Custom => "custom",
    })
}

fn operator_name(operator: ValidationOperator) -> Option<&'static str> {
    Some(match operator {
        ValidationOperator::Between => return None,
        ValidationOperator::NotBetween => "notBetween",
        ValidationOperator::Equal => "equal",
        ValidationOperator::NotEqual => "notEqual",
        ValidationOperator::GreaterThan => "greaterThan",
        ValidationOperator::LessThan => "lessThan",
        ValidationOperator::GreaterThanOrEqual => "greaterThanOrEqual",
        ValidationOperator::LessThanOrEqual => "lessThanOrEqual",
    })
}

fn rule_xml(rule: &DataValidation) -> String {
    let settings = &rule.settings;
    let kind = kind_name(settings.kind)
        .map(|name| format!(" type=\"{name}\""))
        .unwrap_or_default();
    let has_operator = !matches!(
        settings.kind,
        ValidationKind::Any | ValidationKind::List | ValidationKind::Custom
    );
    let operator = operator_name(settings.operator)
        .filter(|_| has_operator)
        .map(|name| format!(" operator=\"{name}\""))
        .unwrap_or_default();
    let style = match settings.error_style {
        ValidationAlertStyle::Stop => "",
        ValidationAlertStyle::Warning => " errorStyle=\"warning\"",
        ValidationAlertStyle::Information => " errorStyle=\"information\"",
    };
    let hide_dropdown = settings.kind == ValidationKind::List && !settings.in_cell_dropdown;
    let mut formulas = String::new();
    if !settings.formula1.is_empty() {
        formulas.push_str(&format!(
            "<formula1>{}</formula1>",
            escape_xml(&settings.formula1)
        ));
    }
    if !settings.formula2.is_empty() {
        formulas.push_str(&format!(
            "<formula2>{}</formula2>",
            escape_xml(&settings.formula2)
        ));
    }
    format!(
        "<dataValidation{kind}{style}{operator}{blank}{dropdown}{input}{error}{error_title}{error_text}{prompt_title}{prompt} sqref=\"{sqref}\">{formulas}</dataValidation>",
        blank = flag("allowBlank", settings.ignore_blank),
        dropdown = flag("showDropDown", hide_dropdown),
        input = flag("showInputMessage", settings.show_input),
        error = flag("showErrorMessage", settings.show_error),
        error_title = attribute("errorTitle", &settings.error_title, TITLE_LIMIT),
        error_text = attribute("error", &settings.error_message, MESSAGE_LIMIT),
        prompt_title = attribute("promptTitle", &settings.input_title, TITLE_LIMIT),
        prompt = attribute("prompt", &settings.input_message, MESSAGE_LIMIT),
        sqref = escape_xml(&rule.sqref),
    )
}

pub(crate) fn get_data_validations_xml(workbook: &Workbook, worksheet: &Worksheet) -> String {
    let rules: Vec<String> = workbook
        .data_validations
        .iter()
        .filter(|rule| rule.sheet_id == worksheet.sheet_id)
        .map(rule_xml)
        .collect();
    if rules.is_empty() {
        return String::new();
    }
    format!(
        "<dataValidations count=\"{}\">{}</dataValidations>",
        rules.len(),
        rules.join("")
    )
}
