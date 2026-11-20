//! Non-file URI schemes reach Ruby's local file opener for String `extends`.
// Windows cannot create colon-named local files. Its missing-file behavior is
// already covered by schema_keywords; these existing-file probes are Unix-only.
#![cfg(unix)]
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use ukiryu::schema::{self, Data, Permitted};

struct Resource(PathBuf);
impl Drop for Resource {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).unwrap();
    }
}

fn data(value: Value) -> Data {
    schema::safe_load(&value.to_string(), Permitted::DOCUMENT).unwrap()
}

#[test]
fn uri_named_local_extensions_match_ruby() {
    let fixture: Value =
        serde_json::from_str(include_str!("schema_golden/keywords/extends.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 14);
    for (index, case) in cases.iter().enumerate() {
        let path = format!(
            "{}:ukiryu-schema-{}-{index}",
            case["scheme"].as_str().unwrap(),
            std::process::id()
        );
        assert!(!std::path::Path::new(&path).exists());
        let _resource = case["contents"].as_str().map(|contents| {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            let resource = Resource(PathBuf::from(&path));
            file.write_all(contents.as_bytes()).unwrap();
            resource
        });
        let result = schema::fully_validate(
            &data(json!({"id":"urn:root", "extends":path})),
            &data(json!(1)),
            false,
        );
        let actual = match result {
            Ok(messages) => json!({"ok":messages}),
            Err(error) => json!({"error":{
                "class":error.ruby_class(),
                "message":error.message().replace(&path, "${RESOURCE}")
            }}),
        };
        assert_eq!(actual, case["expect"], "extends case {index}: {case}");
    }
}
