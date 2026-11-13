//! `CommandDefinition#regular_arguments` order against the pinned Ruby oracle.
//!
//! Ruby orders regular arguments with `sort_by(&:numeric_position)`, and
//! MRI's `sort_by` is not stable: arguments that share a position come back
//! in the order MRI's sort leaves them, which differs from source order once
//! there are more than 16 arguments. Each case compares the exact name order.
//!
//! Data: `models_data/regular_arguments.jsonl`. The first line is provenance
//! (ukiryu ab6df353, Ruby 4.0.1 e04267a14b, oracle Gemfile.lock fc3b12f2);
//! every other line is one case: the command YAML, the names Ruby returned,
//! their numeric positions, and whether a stable sort would order them
//! differently.
use serde_json::Value as Json;
use std::path::PathBuf;
use ukiryu::models::{CommandDefinition, Model};

fn cases() -> (Json, Vec<Json>) {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/models_data/regular_arguments.jsonl");
    let text = std::fs::read_to_string(&path).expect("regular_arguments.jsonl");
    let mut lines = text
        .lines()
        .map(|l| serde_json::from_str::<Json>(l).expect("jsonl line"));
    let provenance = lines.next().expect("provenance line");
    (provenance, lines.collect())
}

fn names(c: &CommandDefinition) -> Vec<String> {
    c.regular_arguments()
        .iter()
        .map(|a| {
            a.name
                .as_ref()
                .and_then(|n| n.as_str())
                .expect("argument name")
                .to_string()
        })
        .collect()
}

#[test]
fn regular_arguments_match_ruby_order() {
    let (provenance, cases) = cases();
    assert!(provenance["provenance"]["oracle"]
        .as_str()
        .unwrap()
        .contains("ab6df353c6dc2d7faf7817a86aafcafa82579896"));

    let mut failures = Vec::new();
    for case in &cases {
        let id = case["id"].as_str().unwrap();
        let cmd = CommandDefinition::from_yaml(case["yaml"].as_str().unwrap())
            .unwrap_or_else(|e| panic!("{id}: from_yaml: {e}"));
        let expect: Vec<String> = case["expect"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_string())
            .collect();
        let got = names(&cmd);
        if got != expect {
            failures.push(format!("{id}:\n  expect {expect:?}\n  got    {got:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
fn corpus_covers_unstable_order() {
    let (_, cases) = cases();
    let ids: Vec<&str> = cases.iter().map(|c| c["id"].as_str().unwrap()).collect();
    for required in [
        "args.mixed-positions.20",
        "args.cross-size.alternate.n17",
        "args.equal.n17",
        "args.missing.mixed.n20",
        "args.large.mixed.n20",
    ] {
        assert!(ids.contains(&required), "missing case {required}");
    }
    let mixed = cases
        .iter()
        .find(|c| c["id"] == "args.mixed-positions.20")
        .unwrap();
    assert_eq!(mixed["stable_differs"], Json::Bool(true));
    // A stable sort must fail a real share of the corpus, or it proves nothing.
    let unstable = cases
        .iter()
        .filter(|c| c["stable_differs"] == Json::Bool(true))
        .count();
    assert!(
        unstable >= 50,
        "only {unstable} cases separate MRI from a stable sort"
    );
}
