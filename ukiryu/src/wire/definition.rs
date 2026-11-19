//! Native Definition facade: all parsing, validation, discovery and rendering
//! use the existing core implementations; adapters only wrap these replies.
use super::error::{model_error, ruby};
use crate::definition::{DefinitionDocument, DocumentParseError};
use crate::definition::{DefinitionMetadata, Discovery, Loader, ValidationMode, VersionResolver};
use crate::models::{Model, ModelErrorKind, ToolDefinition};
use serde::Deserialize;
use serde_json::{json, Value as Json};

#[derive(Debug, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum DefinitionRequest {
    Load {
        yaml: String,
        #[serde(default)]
        path: Option<String>,
        #[serde(default = "strict")]
        validation: String,
    },
    ClearCache {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        yaml: Option<String>,
    },
    Discover,
    ResolveVersion {
        constraint: String,
        versions: Vec<String>,
    },
    Documentation {
        value: super::value::Typed,
        format: String,
    },
}
fn strict() -> String {
    "strict".to_owned()
}
thread_local! {
    static LOADER: Loader<ToolDocument> = Loader::new();
}
struct ToolDocument {
    inner: ToolDefinition,
    name: Option<String>,
    version: Option<String>,
    ukiryu_schema: Option<String>,
    self_uri: Option<String>,
}

impl DefinitionDocument for ToolDocument {
    fn parse_definition(yaml: &str) -> Result<Self, DocumentParseError> {
        let inner = ToolDefinition::from_yaml(yaml).map_err(|e| match e.kind {
            ModelErrorKind::Syntax => {
                // The oracle's Lutaml YAML adapter wraps the parser failure
                // as InvalidFormatError before Loader adds source context.
                let diagnostic = regex::Regex::new(r"^(.*) \(line (\d+), column (\d+)\)$")
                    .expect("static diagnostic pattern")
                    .captures(&e.message)
                    .map(|c| {
                        format!(
                            "(<unknown>): {} at line {} column {}\n",
                            &c[1], &c[2], &c[3]
                        )
                    })
                    .unwrap_or(e.message);
                DocumentParseError::Format(format!(
                    "input format is invalid, try to pass correct `yaml` format \n{diagnostic}\n"
                ))
            }
            ModelErrorKind::InvalidFormat => DocumentParseError::Format(e.message),
            _ => DocumentParseError::Other(e.message),
        })?;
        let text = |t: &Option<crate::models::value::Text>| t.as_ref().map(|t| t.to_ruby_s());
        Ok(ToolDocument {
            name: text(&inner.name),
            version: text(&inner.version),
            ukiryu_schema: text(&inner.ukiryu_schema),
            self_uri: text(&inner.self_uri),
            inner,
        })
    }

    fn definition_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn definition_version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    fn has_profiles(&self) -> bool {
        self.inner.profiles.as_ref().is_some_and(|p| !p.is_empty())
    }

    fn ukiryu_schema(&self) -> Option<&str> {
        self.ukiryu_schema.as_deref()
    }

    fn self_uri(&self) -> Option<&str> {
        self.self_uri.as_deref()
    }

    fn resolve_inheritance(&mut self) {
        self.inner.resolve_inheritance();
    }
}

fn metadata(value: DefinitionMetadata) -> Json {
    json!({"name":value.name,"version":value.version,"path":value.path,"source_type":value.source_type.as_str()})
}
pub fn process(request: DefinitionRequest) -> Json {
    let value = LOADER.with(|loader| match request {
        DefinitionRequest::Load {
            yaml,
            path,
            validation,
        } => {
            let mode = match validation.as_str() {
                "strict" => ValidationMode::Strict,
                "none" => ValidationMode::None,
                _ => ValidationMode::Lenient,
            };
            let result = if let Some(path) = path {
                loader.load_from_file(path, mode)
            } else {
                loader.load_from_string(&yaml, mode)
            };
            match result {
                Ok(model) => model
                    .inner
                    .to_hash()
                    .map_err(|e| model_error(&e))
                    .and_then(|v| {
                        super::value::encode(&v)
                            .map_err(|e| ruby(super::value::EncodeError::CLASS, e))
                    }),
                Err(e) => Err(ruby(e.ruby_class(), e.to_string())),
            }
        }
        DefinitionRequest::ClearCache { path, yaml } => {
            use crate::definition::{FileSource, Source, StringSource};
            let source: Result<Option<Box<dyn Source>>, _> = if let Some(path) = path {
                FileSource::new(path).map(|s| Some(Box::new(s) as Box<dyn Source>))
            } else if let Some(yaml) = yaml {
                StringSource::new(&yaml).map(|s| Some(Box::new(s) as Box<dyn Source>))
            } else {
                Ok(None)
            };
            source
                .and_then(|s| loader.clear_cache(s.as_deref()))
                .map(|_| json!({}))
                .map_err(|e| ruby(e.ruby_class(), e.to_string()))
        }
        DefinitionRequest::Discover => Ok(Json::Object(
            Discovery::from_process()
                .discover()
                .into_iter()
                .map(|(name, values)| {
                    (
                        name,
                        Json::Array(values.into_iter().map(metadata).collect()),
                    )
                })
                .collect(),
        )),
        DefinitionRequest::ResolveVersion {
            constraint,
            versions,
        } => VersionResolver::resolve(&constraint, Some(&versions))
            .map(|v| json!(v))
            .map_err(|e| ruby(e.ruby_class(), e.to_string())),
        DefinitionRequest::Documentation { value, format } => {
            crate::definition::documentation::DocumentationGenerator::generate(
                &value.0.to_yaml_value(),
                &format,
            )
            .map(|s| json!(s))
            .map_err(|e| ruby(e.ruby_class(), e.to_string()))
        }
    });
    match value {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(error) => error,
    }
}
