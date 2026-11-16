//! `ArgumentDefinition#numeric_position` / `#parsed_position` and
//! `CommandDefinition#regular_arguments` order for positions at and beyond
//! the i128 range, against the pinned Ruby oracle.
//!
//! Ruby reads a digit position with `position.to_i`, an arbitrary-precision
//! Integer, so positions past 2**127 keep their exact order instead of tying.
//!
//! Data: `models_data/numeric_position.jsonl`. The first line is provenance
//! (ukiryu ab6df353, Ruby 4.0.1 e04267a14b, oracle Gemfile.lock fc3b12f2).
//! `position` rows hold one argument: its loaded position and the
//! `numeric_position`, `parsed_position` and `last?` Ruby returned (Integers
//! as decimal strings). `order` rows hold a command: the argument names
//! `regular_arguments` returned, their numeric positions, and whether MRI's
//! sort on i128-saturated keys would give a different order.
use num_bigint::BigInt;
use serde_json::Value as Json;
use std::path::PathBuf;
use ukiryu::command_builder::ArgumentSpec;
use ukiryu::models::{ArgumentDefinition, CommandDefinition, Model, ParsedPosition};

fn rows() -> (Json, Vec<Json>) {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/models_data/numeric_position.jsonl");
    let text = std::fs::read_to_string(&path).expect("numeric_position.jsonl");
    let mut lines = text
        .lines()
        .map(|l| serde_json::from_str::<Json>(l).expect("jsonl line"));
    let provenance = lines.next().expect("provenance line");
    assert!(provenance["provenance"]["oracle"]
        .as_str()
        .unwrap()
        .contains("ab6df353c6dc2d7faf7817a86aafcafa82579896"));
    (provenance, lines.collect())
}

fn kind<'a>(rows: &'a [Json], k: &str) -> Vec<&'a Json> {
    rows.iter().filter(|r| r["kind"] == k).collect()
}

fn big(v: &Json) -> BigInt {
    v.as_str()
        .expect("decimal string")
        .parse()
        .expect("Integer")
}

fn parsed(p: &ParsedPosition) -> Json {
    match p {
        ParsedPosition::Last => serde_json::json!({"$sym": "last"}),
        ParsedPosition::First => serde_json::json!({"$sym": "first"}),
        ParsedPosition::Index(i) => Json::String(i.to_string()),
    }
}

#[test]
fn numeric_position_matches_ruby_integer() {
    let (_, rows) = rows();
    let mut failures = Vec::new();
    for row in kind(&rows, "position") {
        let id = row["id"].as_str().unwrap();
        let a = ArgumentDefinition::from_yaml(row["yaml"].as_str().unwrap())
            .unwrap_or_else(|e| panic!("{id}: from_yaml: {e}"));
        let expected = big(&row["numeric_position"]);
        let got = a.numeric_position();
        if got != expected {
            failures.push(format!("{id}: numeric_position {got}, expected {expected}"));
        }
        let got_parsed = parsed(&a.parsed_position());
        if got_parsed != row["parsed_position"] {
            failures.push(format!(
                "{id}: parsed_position {got_parsed}, expected {}",
                row["parsed_position"]
            ));
        }
        if a.is_last() != row["last"].as_bool().unwrap() {
            failures.push(format!("{id}: last? {}", a.is_last()));
        }
        // The command builder reads the same position through ArgumentSpec.
        let spec = ArgumentSpec {
            position: row["position"].as_str().map(str::to_string),
            ..ArgumentSpec::default()
        };
        if spec.numeric_position() != expected {
            failures.push(format!(
                "{id}: ArgumentSpec numeric_position {}, expected {expected}",
                spec.numeric_position()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn regular_arguments_keep_exact_order_beyond_i128() {
    let (_, rows) = rows();
    let mut failures = Vec::new();
    for row in kind(&rows, "order") {
        let id = row["id"].as_str().unwrap();
        let cmd = CommandDefinition::from_yaml(row["yaml"].as_str().unwrap())
            .unwrap_or_else(|e| panic!("{id}: from_yaml: {e}"));
        let regular = cmd.regular_arguments();
        let names: Vec<&str> = regular
            .iter()
            .map(|a| a.name.as_ref().and_then(|n| n.as_str()).expect("name"))
            .collect();
        let expect: Vec<&str> = row["expect"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        if names != expect {
            failures.push(format!("{id}:\n  expect {expect:?}\n  got    {names:?}"));
        }
        let keys: Vec<BigInt> = regular.iter().map(|a| a.numeric_position()).collect();
        let expect_keys: Vec<BigInt> = row["numeric_positions"]
            .as_array()
            .unwrap()
            .iter()
            .map(big)
            .collect();
        if keys != expect_keys {
            failures.push(format!("{id}: numeric positions differ"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn corpus_reaches_beyond_i128() {
    let (_, rows) = rows();
    let positions = kind(&rows, "position");
    let orders = kind(&rows, "order");
    let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    for required in [
        "pos.i128-max",
        "pos.i128-max+1",
        "pos.u128-max+1",
        "pos.digits-1000",
        "pos.leading-zeros.i128-max+1",
        "pos.multiline.minus-big-then-digit",
        "order.beyond-i128.descending.n20",
        "order.beyond-i128.ties.n20",
    ] {
        assert!(ids.contains(&required), "missing case {required}");
    }
    let outside = positions
        .iter()
        .filter(|r| r["within_i128"] == Json::Bool(false))
        .count();
    assert!(outside >= 15, "only {outside} positions outside i128");
    // Saturating to i128 must break a real share of the order cases.
    let saturated = orders
        .iter()
        .filter(|r| r["saturated_differs"] == Json::Bool(true))
        .count();
    assert!(
        saturated >= 10,
        "only {saturated} cases separate exact from saturated keys"
    );
}
