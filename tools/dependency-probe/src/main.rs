//! Exercise the published vendored YAML dependency before the core relies on it.
//! This is a dependency/build probe, not Ruby parity or a Ukiryu runtime.
use std::path::{Path, PathBuf};

fn roundtrip(input: &str) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let value: yeptris::Value = yeptris::from_str(input)?;
    let output = yeptris::to_string(&value)?;
    let repeated = yeptris::to_string(&value)?;
    if output != repeated {
        return Err("repeated YAML serialization produced different bytes".into());
    }
    let reparsed: yeptris::Value = yeptris::from_str(&output)?;
    if value != reparsed {
        return Err("YAML value changed after serialization".into());
    }
    Ok(serde_json::to_value(value)?)
}

fn yaml_files(root: &Path, paths: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            yaml_files(&path, paths)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "yaml")
        {
            paths.push(path);
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        return Err("usage: ukiryu-dependency-probe PINNED_RUBY_CHECKOUT".into());
    }
    let source = PathBuf::from(&args[0]);
    let mut paths = Vec::new();
    yaml_files(&source.join("spec/fixtures"), &mut paths)?;
    yaml_files(&source.join("lib/ukiryu/schemas"), &mut paths)?;
    paths.sort();
    // Source inventory at ab6df353: 65 YAML spec files plus 4 schemas.
    // Tool families and interface subsets overlap that inventory.
    if paths.len() != 69 {
        return Err(format!(
            "expected 69 unique pinned YAML files, found {}",
            paths.len()
        )
        .into());
    }
    for path in &paths {
        let input = std::fs::read_to_string(path)?;
        let value = roundtrip(&input).map_err(|error| format!("{}: {error}", path.display()))?;
        println!(
            "{}",
            serde_json::json!({
                "path": path.strip_prefix(&source)?.to_string_lossy().replace('\\', "/"),
                "value": value,
            })
        );
    }
    eprintln!(
        "{} vendored YAML parse/emit value checks passed",
        paths.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::roundtrip;

    #[test]
    fn preserves_quoted_profile_versions_and_yaml_scalar_types() {
        let value = roundtrip(
            "ukiryu_schema: '1.0'\nversion: '2024.01.17'\ncount: 3\nflag: false\nvalue: null\n",
        )
        .unwrap();
        assert_eq!(value["ukiryu_schema"], "1.0");
        assert_eq!(value["version"], "2024.01.17");
        assert_eq!(value["count"], 3);
        assert_eq!(value["flag"], false);
        assert!(value["value"].is_null());
    }

    #[test]
    fn rejects_malformed_profile_yaml() {
        assert!(roundtrip("name: [broken\n").is_err());
    }
}
