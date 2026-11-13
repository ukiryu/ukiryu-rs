//! Source: lib/ukiryu/models/implementation_index.rb (`Ukiryu::Models::ImplementationIndex`).
//!
//! `from_hash` expects Symbol keys (the register symbolizes them first);
//! `from_yaml(path)` passes Psych's String-keyed Hash straight through, so
//! every `data[:key]` lookup misses there, exactly as in the source.
use super::error::ModelError;
use super::interface::{load_yaml_file, register_hash};
use super::value::RubyValue;
use std::path::Path;

/// A tool's implementation index (`index.yaml`).
#[derive(Clone, Debug, PartialEq)]
pub struct ImplementationIndex {
    pub name: RubyValue,
    pub interface: RubyValue,
    pub interface_version: RubyValue,
    /// Symbolized implementation Hashes.
    pub implementations: Vec<RubyValue>,
}

fn sym_get(data: &RubyValue, key: &str) -> Result<RubyValue, ModelError> {
    match data {
        RubyValue::Hash(_) => Ok(data.get_sym(key).cloned().unwrap_or(RubyValue::Nil)),
        other => Err(ModelError::no_method("[]", other)),
    }
}

impl ImplementationIndex {
    /// `ImplementationIndex.new(name:, interface:, implementations:, interface_version: '1.0')`.
    pub fn new(
        name: RubyValue,
        interface: RubyValue,
        implementations: &[RubyValue],
        interface_version: RubyValue,
    ) -> Result<ImplementationIndex, ModelError> {
        let implementations = implementations
            .iter()
            .map(|i| match i {
                RubyValue::Hash(_) => Ok(i.symbolize_keys()),
                other => Err(ModelError::no_method("transform_keys", other)),
            })
            .collect::<Result<_, _>>()?;
        Ok(ImplementationIndex {
            name,
            interface,
            interface_version,
            implementations,
        })
    }

    /// `ImplementationIndex.from_yaml(path)`.
    pub fn from_yaml(path: &Path) -> Result<ImplementationIndex, ModelError> {
        ImplementationIndex::from_hash(&load_yaml_file(path)?)
    }

    /// `ImplementationIndex.from_hash(data)` (Symbol keys).
    pub fn from_hash(data: &RubyValue) -> Result<ImplementationIndex, ModelError> {
        let version = match sym_get(data, "interface_version")? {
            v if v.truthy() => v,
            _ => RubyValue::str("1.0"),
        };
        let implementations = match sym_get(data, "implementations")? {
            v if v.truthy() => v,
            _ => RubyValue::Array(Vec::new()),
        };
        let items = match implementations {
            RubyValue::Array(items) => items,
            other => return Err(ModelError::no_method("map", &other)),
        };
        ImplementationIndex::new(
            sym_get(data, "name")?,
            sym_get(data, "interface")?,
            &items,
            version,
        )
    }

    /// `Register#load_implementation_index` minus file lookup.
    pub fn from_register_yaml(yaml: &str) -> Result<Option<ImplementationIndex>, ModelError> {
        register_hash(yaml)?
            .map(|h| ImplementationIndex::from_hash(&h))
            .transpose()
    }

    /// `#implementation(impl_name)`
    pub fn implementation(&self, impl_name: &RubyValue) -> Option<&RubyValue> {
        self.implementations
            .iter()
            .find(|i| i.get_sym("name").unwrap_or(&RubyValue::Nil) == impl_name)
    }

    /// `#implementation_names`
    pub fn implementation_names(&self) -> Vec<RubyValue> {
        self.implementations
            .iter()
            .map(|i| i.get_sym("name").cloned().unwrap_or(RubyValue::Nil))
            .collect()
    }

    /// `#to_s`
    pub fn to_ruby_s(&self) -> String {
        format!(
            "{} (implements {}/{})",
            self.name.to_ruby_s(),
            self.interface.to_ruby_s(),
            self.interface_version.to_ruby_s()
        )
    }

    /// `#inspect`
    pub fn inspect(&self) -> String {
        format!(
            "#<Ukiryu::Models::ImplementationIndex {} implementations={}>",
            self.to_ruby_s(),
            self.implementations.len()
        )
    }
}
