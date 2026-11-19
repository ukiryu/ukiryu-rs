use serde_json::{json, Value};

fn call(request: Value) -> Value {
    ukiryu::wire::process(request.to_string().as_bytes())
}

const YAML: &str = "name: echo\nversion: '1.2'\nprofiles:\n- name: unix\n  platforms: [linux]\n  shells: [bash]\n  commands:\n  - name: say\n    arguments:\n    - name: text\n      type: string\n      position: first\n    options:\n    - name: output\n      type: string\n      cli: --output\n";

#[test]
fn resolved_definition_is_distinct_from_detected_version_and_keeps_command_models() {
    let result = call(json!({"op":"resolve", "method":"tool_profile",
        "source":{"kind":"definition", "yaml":YAML}, "platform":"linux",
        "shell":"bash", "version":"9.0"}));
    assert_eq!(result["ok"], true, "{result}");
    let definition = &result["value"]["definition"];
    assert_eq!(definition["version"], "1.2");
    assert_eq!(result["value"]["profile_index"], 0);
    assert_eq!(
        definition["profiles"][0]["commands"][0]["options"][0]["name"],
        "output"
    );
}

#[test]
fn binding_model_views_roundtrip_through_the_same_core_models() {
    for (model, yaml) in [
        ("ToolDefinition", YAML),
        ("PlatformProfile", "name: unix\nplatforms: [linux]\n"),
        (
            "CommandDefinition",
            "name: say\narguments: [{name: text, type: string}]\n",
        ),
        ("ArgumentDefinition", "name: text\ntype: string\n"),
        ("OptionDefinition", "name: output\ncli: --output\n"),
        ("FlagDefinition", "name: verbose\ncli: --verbose\n"),
    ] {
        let read = call(json!({"op":"yaml", "method":"model_read", "model":model, "yaml":yaml}));
        assert_eq!(read["ok"], true, "{model}: {read}");
        let dumped =
            call(json!({"op":"yaml", "method":"model_dump", "model":model, "value":read["value"]}));
        let existing =
            call(json!({"op":"yaml", "method":"model_to_yaml", "model":model, "yaml":yaml}));
        assert_eq!(dumped["ok"], true, "{model}: {dumped}");
        assert_eq!(dumped, existing, "{model}");
        let reread = call(
            json!({"op":"yaml", "method":"model_read", "model":model, "yaml":dumped["value"]}),
        );
        assert_eq!(reread, read, "{model}");
    }
}

#[test]
fn model_views_fail_loudly_for_unknown_models_and_invalid_shapes() {
    for method in ["model_read", "model_to_yaml"] {
        let reply = call(json!({"op":"yaml", "method":method, "model":"NoSuchModel", "yaml":"{}"}));
        assert_eq!(reply["error"]["code"], "INVALID_ARGUMENT");
    }
    let reply =
        call(json!({"op":"yaml", "method":"model_dump", "model":"ToolDefinition", "value":42}));
    assert_eq!(reply["ok"], false, "{reply}");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn generated_classes_use_the_classic_raw_loader_and_resolve_inheritance() {
    use std::fs;
    let path = std::env::temp_dir().join(format!("ukiryu-wire-models-{}", std::process::id()));
    fs::create_dir(&path).unwrap();
    struct Temp(std::path::PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _temp = Temp(path.clone());
    let dir = path.join("tools/echo/default");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        path.join("tools/echo/index.yaml"),
        "implementations:\n- name: default\n  versions:\n  - equals: '1'\n    file: '1.yaml'\n",
    )
    .unwrap();
    fs::write(dir.join("1.yaml"), "name: echo\nversion: '1'\nprofiles:\n- name: unix\n  platforms: [linux]\n  commands: [{name: say}]\n- name: windows\n  platforms: [windows]\n  inherits: unix\n").unwrap();
    let result = call(
        json!({"op":"resolve", "method":"tool_class", "register":path,
        "tool":"echo", "platform":"windows", "shell":"cmd"}),
    );
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"]["profile_index"], 1);
    assert_eq!(
        result["value"]["definition"]["profiles"][1]["commands"][0]["name"],
        "say"
    );
    // Tool.get accepts a range-only index, while the classic class generator
    // cannot list a raw version from one. Pin that observable difference.
    fs::write(
        path.join("tools/echo/index.yaml"),
        "implementations:\n- name: default\n  versions:\n  - after: '1'\n    file: '1.yaml'\n",
    )
    .unwrap();
    let result = call(
        json!({"op":"resolve", "method":"tool_class", "register":path,
        "tool":"echo", "platform":"linux", "shell":"bash"}),
    );
    assert_eq!(
        result["error"]["source_class"],
        "Ukiryu::Errors::ToolNotFoundError"
    );
    assert_eq!(result["error"]["message"], "Tool not found: echo");
}
