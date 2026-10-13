use datatool_core::dataset::infer::InferredType;
use datatool_core::dataset::Format;
use datatool_core::profile::quality::Severity;
use datatool_core::profile::Role;
use datatool_core::{load, profile, DatasetProfile};

fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name)
}

fn profile_fixture(name: &str) -> DatasetProfile {
    profile(&load(fixture(name)).unwrap()).unwrap()
}

fn roles(p: &DatasetProfile) -> Vec<(&str, Role)> {
    p.columns
        .iter()
        .map(|c| (c.name.as_str(), c.role))
        .collect()
}

#[test]
fn clean_dataset_has_no_serious_issues() {
    let p = profile_fixture("sales.csv");
    assert_eq!((p.rows, p.cols), (200, 7));
    assert_eq!(p.duplicate_rows, 0);
    assert_eq!(p.completeness, 1.0);
    assert!(
        p.warnings.iter().all(|w| w.severity == Severity::Low),
        "unexpected warnings: {:#?}",
        p.warnings
    );
    assert_eq!(
        roles(&p),
        vec![
            ("order_id", Role::Identifier),
            ("order_date", Role::Timestamp),
            ("region", Role::Category),
            ("units", Role::Measure),
            ("unit_price", Role::Measure),
            ("revenue", Role::Measure),
            ("repeat_customer", Role::Flag),
        ]
    );
    // revenue = units × unit_price, so it should correlate most with units
    let top = &p.correlations.top_pairs[0];
    assert_eq!((top.a.as_str(), top.b.as_str()), ("units", "revenue"));
    assert!(top.pearson > 0.7);
    assert!(p
        .summary
        .iter()
        .any(|s| s.contains("strongest relationship")));
}

#[test]
fn messy_dataset_reports_each_planted_problem() {
    let p = profile_fixture("messy.csv");
    let titles: Vec<&str> = p.warnings.iter().map(|w| w.title.as_str()).collect();
    for expected in [
        "`customer_id` looks like an ID but has repeats",
        "`signup_date` mixes 3 date formats",
        "1 duplicate row",
        "`age` contains -999",
        "`income` has 1 non-numeric value",
        "`plan` has inconsistent spellings",
        "`city` has inconsistent spellings",
        "`city` has extra spaces",
        "`age` uses placeholders for missing values",
    ] {
        assert!(
            titles.contains(&expected),
            "missing {:?} in {:#?}",
            expected,
            titles
        );
    }
    // Most severe first
    let severities: Vec<Severity> = p.warnings.iter().map(|w| w.severity).collect();
    let mut sorted = severities.clone();
    sorted.sort();
    assert_eq!(severities, sorted);
}

#[test]
fn excel_matches_csv() {
    let ds = load(fixture("sales.xlsx")).unwrap();
    assert_eq!(
        ds.format,
        Format::Excel {
            sheet: "Orders".into()
        }
    );
    assert_eq!((ds.rows(), ds.cols()), (30, 7));
    let types: Vec<InferredType> = ds.columns.iter().map(|c| c.inference.inferred).collect();
    assert_eq!(
        types,
        vec![
            InferredType::Text,
            InferredType::Date,
            InferredType::Text,
            InferredType::Integer,
            InferredType::Float,
            InferredType::Float,
            InferredType::Boolean,
        ]
    );
}

#[test]
fn profile_serializes_to_json() {
    let p = profile_fixture("messy.csv");
    let json: serde_json::Value = serde_json::to_value(&p).unwrap();
    for key in [
        "name",
        "rows",
        "cols",
        "duplicate_rows",
        "completeness",
        "columns",
        "correlations",
        "warnings",
        "summary",
    ] {
        assert!(json.get(key).is_some(), "missing key {}", key);
    }
    let age = &json["columns"][3];
    assert_eq!(age["name"], "age");
    assert_eq!(age["role"], "measure");
    assert_eq!(age["inferred"], "integer");
    assert_eq!(age["cleaning"]["placeholder_nulls"], 2);
    assert_eq!(json["warnings"][0]["severity"], "high");
}
