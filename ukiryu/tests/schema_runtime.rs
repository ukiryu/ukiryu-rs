//! Bounded Ruby BigDecimal and Onigmo runtime failure observations.
use serde_json::{json, Value};
use ukiryu::schema::{self, Data, Permitted};

// Test-only use of the bundled engine's C limit. This executable has one test,
// so no other regex search can race the global setter. Production stays at 0.
extern "C" {
    fn onig_get_match_stack_limit_size() -> u32;
    fn onig_set_match_stack_limit_size(size: u32) -> i32;
}

struct StackLimit(u32);
impl StackLimit {
    fn set(limit: u32) -> Self {
        // SAFETY: no other test runs inside this executable; both functions
        // belong to the same engine used by schema validation below.
        unsafe {
            let old = onig_get_match_stack_limit_size();
            assert_eq!(onig_set_match_stack_limit_size(limit), 0);
            Self(old)
        }
    }
}
impl Drop for StackLimit {
    fn drop(&mut self) {
        // SAFETY: restore the original engine state even if a case panics.
        unsafe { onig_set_match_stack_limit_size(self.0) };
    }
}

fn data(value: &Value) -> Data {
    schema::safe_load(&value.to_string(), Permitted::DOCUMENT).unwrap()
}

#[test]
fn schema_runtime_failures_match_the_pinned_ruby_oracle() {
    let fixture: Value =
        serde_json::from_str(include_str!("schema_golden/keywords/runtime.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 478, "runtime capture changed unexpectedly");
    assert!(cases.iter().any(|case| case["expect"]["ok"] == json!([])));
    for class in ["RegexpError", "NoMemoryError", "ZeroDivisionError"] {
        assert!(cases
            .iter()
            .any(|case| case["expect"]["error"]["class"] == class));
    }
    let mut failures = Vec::new();
    for case in cases {
        let limit = case["stack_limit"].as_u64().unwrap_or(0);
        let _limit = StackLimit::set(u32::try_from(limit).unwrap());
        let document = match case.get("repeat") {
            Some(repeat) => json!({"v": repeat["unit"].as_str().unwrap()
                .repeat(usize::try_from(repeat["count"].as_u64().unwrap()).unwrap())}),
            None => case["data"].clone(),
        };
        let result = schema::fully_validate(&data(&case["schema"]), &data(&document), false);
        let actual = match result {
            Ok(messages) => json!({"ok": messages}),
            Err(error) => {
                json!({"error": {"class": error.ruby_class(), "message": error.message()}})
            }
        };
        if actual != case["expect"] {
            failures.push(json!({"id":case["id"], "schema":case["schema"],
                "document":document, "want":case["expect"], "got":actual}));
        }
    }
    eprintln!(
        "{} runtime cases, {} mismatches",
        cases.len(),
        failures.len()
    );
    for failure in &failures {
        eprintln!("RUNTIME_FAILURE {failure}");
    }
    assert!(failures.is_empty(), "{} runtime mismatches", failures.len());
}
