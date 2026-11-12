//! The four v1 schemas the original gem ships in `lib/ukiryu/schemas/v1`.
//!
//! Three are byte copies of ukiryu/ukiryu at ab6df353. `tool.schema.yaml`
//! is the copy repaired upstream by c2cb9fc21f76 (ukiryu/ukiryu#30, merged
//! as 6fa27322ae62): at ab6df353 an unquoted `description: DEPRECATED: ...`
//! on line 287 makes the file invalid YAML. The original bytes are kept as
//! a negative test input, not here.
//!
//! All four declare `$schema: http://json-schema.org/draft-07/schema#`,
//! which json-schema 6.2.1 does not implement, so the original validator
//! (and this port) reject them with `Schema not found`.

/// One bundled schema file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundledSchema {
    /// Short name, e.g. `"tool"`.
    pub name: &'static str,
    /// Path inside the original gem.
    pub source_path: &'static str,
    /// Upstream commit the bytes come from.
    pub source_commit: &'static str,
    /// File contents.
    pub contents: &'static str,
}

/// The bundled v1 schemas.
pub const SCHEMAS: [BundledSchema; 4] = [
    BundledSchema {
        name: "implementation_index",
        source_path: "lib/ukiryu/schemas/v1/implementation_index.schema.yaml",
        source_commit: "ab6df353c6dc2d7faf7817a86aafcafa82579896",
        contents: include_str!("../../schemas/v1/implementation_index.schema.yaml"),
    },
    BundledSchema {
        name: "implementation_version",
        source_path: "lib/ukiryu/schemas/v1/implementation_version.schema.yaml",
        source_commit: "ab6df353c6dc2d7faf7817a86aafcafa82579896",
        contents: include_str!("../../schemas/v1/implementation_version.schema.yaml"),
    },
    BundledSchema {
        name: "interface",
        source_path: "lib/ukiryu/schemas/v1/interface.schema.yaml",
        source_commit: "ab6df353c6dc2d7faf7817a86aafcafa82579896",
        contents: include_str!("../../schemas/v1/interface.schema.yaml"),
    },
    BundledSchema {
        name: "tool",
        source_path: "lib/ukiryu/schemas/v1/tool.schema.yaml",
        source_commit: "c2cb9fc21f76439587cd3d6b5ca67a243836a967",
        contents: include_str!("../../schemas/v1/tool.schema.yaml"),
    },
];

/// Look a bundled schema up by its short name.
pub fn get(name: &str) -> Option<&'static BundledSchema> {
    SCHEMAS.iter().find(|s| s.name == name)
}
