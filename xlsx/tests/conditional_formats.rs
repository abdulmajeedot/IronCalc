#![allow(clippy::unwrap_used)]

use std::io::{Cursor, Read};

use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::cf_types::{
    icon_set_thresholds, CfRule, CfRuleInput, Cfvo, ColorScaleThreshold, Icon, TextOperator,
};
use ironcalc_base::types::{Color, Dxf};
use ironcalc_base::{Model, UserModel};

fn model() -> UserModel<'static> {
    UserModel::new_empty("book", "en", "UTC", "en").unwrap()
}

fn export(model: &UserModel) -> Vec<u8> {
    save_xlsx_to_writer(model.get_model(), Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn sheet_xml(bytes: &[u8]) -> String {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut file = archive.by_name("xl/worksheets/sheet1.xml").unwrap();
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    text
}

fn import(bytes: &[u8]) -> Model<'static> {
    let workbook = load_from_xlsx_bytes(bytes, "book", "en", "UTC").unwrap();
    Model::from_workbook(workbook, "en").unwrap()
}

fn icon_set(name: &str) -> CfRuleInput {
    CfRuleInput::IconSet {
        thresholds: icon_set_thresholds(name).unwrap(),
        show_value: true,
    }
}

fn rules(model: &Model) -> Vec<CfRule> {
    model.workbook.worksheets[0]
        .conditional_formatting
        .iter()
        .map(|cf| cf.cf_rule.clone())
        .collect()
}

#[test]
fn every_icon_set_in_the_gallery_keeps_its_name() {
    for name in [
        "3Arrows",
        "3ArrowsGray",
        "4Arrows",
        "4ArrowsGray",
        "5Arrows",
        "5ArrowsGray",
        "3TrafficLights1",
        "3Signs",
        "4TrafficLights",
        "4RedToBlack",
        "3Symbols",
        "3Flags",
    ] {
        let mut model = model();
        model
            .add_conditional_formatting(0, "A1:A5", icon_set(name))
            .unwrap();
        let xml = sheet_xml(&export(&model));
        assert!(
            xml.contains(&format!("<iconSet iconSet=\"{name}\"")),
            "{name} was written as {xml}"
        );
    }
}

#[test]
fn sets_only_excels_extension_knows_go_in_the_extension() {
    let mut model = model();
    model
        .add_conditional_formatting(0, "B2:B6", icon_set("3Triangles"))
        .unwrap();
    let bytes = export(&model);
    let xml = sheet_xml(&bytes);
    assert!(!xml.contains("<iconSet iconSet=\"3Triangles\""));
    assert!(!xml.contains("<conditionalFormatting"));
    assert!(xml.contains("<x14:iconSet iconSet=\"3Triangles\">"));
    assert!(xml.contains("<x14:cfRule type=\"iconSet\" priority=\"1\" id=\"{"));
    assert!(xml.contains("<xm:sqref>B2:B6</xm:sqref>"));
    match &rules(&import(&bytes))[0] {
        CfRule::IconSet { thresholds, .. } => {
            assert_eq!(thresholds.len(), 3);
            assert_eq!(thresholds[2].icon, Icon::TriangleUp);
            assert_eq!(thresholds[1].cfvo, Cfvo::Percent(33.0));
        }
        other => panic!("unexpected rule {other:?}"),
    }
}

#[test]
fn three_star_ratings_go_in_the_extension_too() {
    let mut model = model();
    let rule = CfRuleInput::IconRating {
        icon: Icon::Star,
        color: Color::Rgb("#F8CD3D".to_string()),
        thresholds: vec![
            (Cfvo::Percent(0.0), true),
            (Cfvo::Percent(33.0), true),
            (Cfvo::Percent(67.0), false),
        ],
        show_value: false,
    };
    model.add_conditional_formatting(0, "C1:C4", rule).unwrap();
    let bytes = export(&model);
    let xml = sheet_xml(&bytes);
    assert!(xml.contains("<x14:iconSet iconSet=\"3Stars\" showValue=\"0\">"));
    assert!(xml.contains("gte=\"0\""));
    assert!(matches!(
        rules(&import(&bytes))[0],
        CfRule::IconRating {
            show_value: false,
            ..
        }
    ));
}

#[test]
fn ratings_without_a_set_of_their_own_are_still_valid() {
    let mut model = model();
    let rule = CfRuleInput::IconRating {
        icon: Icon::Heart,
        color: Color::Rgb("#FF0000".to_string()),
        thresholds: vec![
            (Cfvo::Percent(0.0), true),
            (Cfvo::Percent(33.0), true),
            (Cfvo::Percent(67.0), true),
        ],
        show_value: true,
    };
    model.add_conditional_formatting(0, "C1:C4", rule).unwrap();
    assert!(!sheet_xml(&export(&model)).contains("3Rating"));
}

#[test]
fn text_rules_name_their_operator() {
    for (operator, kind, name) in [
        (TextOperator::Contains, "containsText", "containsText"),
        (
            TextOperator::DoesNotContain,
            "notContainsText",
            "notContains",
        ),
        (TextOperator::BeginsWith, "beginsWith", "beginsWith"),
        (TextOperator::EndsWith, "endsWith", "endsWith"),
    ] {
        let mut model = model();
        let rule = CfRuleInput::Text {
            operator: operator.clone(),
            value: "x".to_string(),
            format: Dxf::default(),
            stop_if_true: false,
        };
        model.add_conditional_formatting(0, "A1:A3", rule).unwrap();
        let bytes = export(&model);
        let xml = sheet_xml(&bytes);
        assert!(xml.contains(&format!("type=\"{kind}\"")), "{xml}");
        assert!(xml.contains(&format!("operator=\"{name}\"")), "{xml}");
        assert!(matches!(
            &rules(&import(&bytes))[0],
            CfRule::Text { operator: read, .. } if *read == operator
        ));
    }
}

#[test]
fn top_bottom_and_average_rules_survive() {
    let mut model = model();
    let format = Dxf::default();
    for rule in [
        CfRuleInput::Top10 {
            rank: 5,
            percent: false,
            format: format.clone(),
            stop_if_true: false,
        },
        CfRuleInput::Bottom10 {
            rank: 20,
            percent: true,
            format: format.clone(),
            stop_if_true: true,
        },
        CfRuleInput::AboveAverage {
            format: format.clone(),
            stop_if_true: false,
        },
        CfRuleInput::BelowAverage {
            format,
            stop_if_true: false,
        },
    ] {
        model.add_conditional_formatting(0, "A1:A9", rule).unwrap();
    }
    let found = rules(&import(&export(&model)));
    assert!(found.iter().any(|r| matches!(
        r,
        CfRule::Top10 {
            rank: 5,
            percent: false,
            ..
        }
    )));
    assert!(found.iter().any(|r| matches!(
        r,
        CfRule::Bottom10 {
            rank: 20,
            percent: true,
            stop_if_true: true,
            ..
        }
    )));
    assert!(found
        .iter()
        .any(|r| matches!(r, CfRule::AboveAverage { .. })));
    assert!(found
        .iter()
        .any(|r| matches!(r, CfRule::BelowAverage { .. })));
}

#[test]
fn a_color_scale_from_the_gallery_survives() {
    let mut model = model();
    let stop = |cfvo: Cfvo, color: &str| ColorScaleThreshold {
        cfvo,
        color: Color::Rgb(color.to_string()),
    };
    let rule = CfRuleInput::ColorScale {
        thresholds: vec![
            stop(Cfvo::Min, "#F8696B"),
            stop(Cfvo::Percentile(50.0), "#FFEB84"),
            stop(Cfvo::Max, "#63BE7B"),
        ],
    };
    model.add_conditional_formatting(0, "A1:A9", rule).unwrap();
    let xml = sheet_xml(&export(&model));
    assert!(xml.contains("<cfvo type=\"percentile\" val=\"50\"/>"));
    assert!(xml.contains("<color rgb=\"FF63BE7B\"/>"));
}

#[test]
fn a_rule_value_is_saved_without_its_equals_sign() {
    let mut model = model();
    let rule = CfRuleInput::CellIs {
        operator: ironcalc_base::cf_types::ValueOperator::Between,
        formula: "=$C$1".to_string(),
        formula2: Some("=$C$2".to_string()),
        format: Dxf::default(),
        stop_if_true: false,
    };
    model.add_conditional_formatting(0, "A1:A3", rule).unwrap();
    let xml = sheet_xml(&export(&model));
    assert!(
        xml.contains("<formula>$C$1</formula><formula>$C$2</formula>"),
        "{xml}"
    );
}
