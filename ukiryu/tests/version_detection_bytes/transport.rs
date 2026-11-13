//! Test-only JSON adapter; all extraction semantics are in the core API.
use serde_json::{json, Value};
use ukiryu::types::onig::Encoding;
use ukiryu::version_detection::{extract_version_from_encoded, EncodedText, ExtractionStage};
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
pub fn bytes(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0);
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}
pub fn request(input: &Value) -> Value {
    let output = input.get("output").filter(|v| !v.is_null());
    let raw = output.map(|v| bytes(v["hex"].as_str().unwrap()));
    let encoding = output.map(|v| match v["encoding"].as_str().unwrap() {
        "UTF-8" => Encoding::Utf8,
        "ASCII-8BIT" => Encoding::Binary,
        "US-ASCII" => Encoding::UsAscii,
        other => panic!("unknown fixture encoding {other}"),
    });
    match extract_version_from_encoded(
        raw.as_deref()
            .zip(encoding)
            .map(|(bytes, encoding)| EncodedText { bytes, encoding }),
        input["pattern"].as_str(),
    ) {
        Ok(version) => {
            json!({"ok":version.map(|v| json!({"hex":hex(&v.bytes),"encoding":v.encoding.name()}))})
        }
        Err(error) => {
            json!({"error":{"class":error.class,"message":error.message,"stage":match error.stage {ExtractionStage::Compile=>"compile",ExtractionStage::Match=>"match"}}})
        }
    }
}
