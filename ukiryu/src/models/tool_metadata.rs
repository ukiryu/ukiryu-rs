//! Source: lib/ukiryu/models/tool_metadata.rb (`Ukiryu::ToolMetadata`).
//!
//! The Ruby class has no explicit mapping, so lutaml serializes every
//! attribute through its reader; several readers are overridden
//! (`aliases`, `tool_name`, `default_command`), which the `Serialize`
//! implementation mirrors through a derived view.
use super::attr::{de_fns as d, omit_opt, Attr, Model};
use super::error::ModelError;
use super::value::{Text, Value};
use serde::{Deserialize, Serialize, Serializer};

fn default_list() -> Attr<Text> {
    Attr::Set(Text::List(Vec::new()))
}

/// Lightweight tool metadata read from a register index.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ToolMetadata {
    #[serde(default, deserialize_with = "d::opt")]
    pub name: Option<Text>,
    #[serde(default, deserialize_with = "d::opt")]
    pub version: Option<Text>,
    #[serde(default, deserialize_with = "d::opt")]
    pub display_name: Option<Text>,
    #[serde(default, deserialize_with = "d::opt")]
    pub implements: Option<Text>,
    #[serde(default, deserialize_with = "d::opt")]
    pub implements_version: Option<Text>,
    #[serde(default, deserialize_with = "d::opt")]
    pub homepage: Option<Text>,
    #[serde(default, deserialize_with = "d::opt")]
    pub description: Option<Text>,
    /// Raw `@aliases` (`:string` attribute holding an Array by default).
    #[serde(default = "default_list")]
    pub aliases: Attr<Text>,
    /// Raw `@tool_name`; read through [`ToolMetadata::tool_name`].
    #[serde(default, deserialize_with = "d::opt")]
    pub tool_name: Option<Text>,
    #[serde(default, deserialize_with = "d::opt")]
    pub register_path: Option<Text>,
    /// Raw `@default_command`; read through [`ToolMetadata::default_command`].
    #[serde(default, deserialize_with = "d::opt")]
    pub default_command: Option<Text>,
    #[serde(default = "default_list")]
    pub may_provide: Attr<Text>,
    #[serde(default, deserialize_with = "d::opt")]
    pub backed_by: Option<Text>,
}

impl Default for ToolMetadata {
    fn default() -> Self {
        ToolMetadata {
            name: None,
            version: None,
            display_name: None,
            implements: None,
            implements_version: None,
            homepage: None,
            description: None,
            aliases: Attr::Default(Text::List(Vec::new())),
            tool_name: None,
            register_path: None,
            default_command: None,
            may_provide: Attr::Default(Text::List(Vec::new())),
            backed_by: None,
        }
    }
}

#[derive(Serialize)]
struct View {
    #[serde(skip_serializing_if = "omit_opt")]
    name: Option<Text>,
    #[serde(skip_serializing_if = "omit_opt")]
    version: Option<Text>,
    #[serde(skip_serializing_if = "omit_opt")]
    display_name: Option<Text>,
    #[serde(skip_serializing_if = "omit_opt")]
    implements: Option<Text>,
    #[serde(skip_serializing_if = "omit_opt")]
    implements_version: Option<Text>,
    #[serde(skip_serializing_if = "omit_opt")]
    homepage: Option<Text>,
    #[serde(skip_serializing_if = "omit_opt")]
    description: Option<Text>,
    #[serde(skip_serializing_if = "omit_opt")]
    aliases: Option<Text>,
    #[serde(skip_serializing_if = "omit_opt")]
    tool_name: Option<Text>,
    #[serde(skip_serializing_if = "omit_opt")]
    register_path: Option<Text>,
    /// A Symbol: written even when empty (`:""`).
    #[serde(skip_serializing_if = "Option::is_none")]
    default_command: Option<Text>,
    #[serde(skip_serializing_if = "omit_opt")]
    may_provide: Option<Text>,
    #[serde(skip_serializing_if = "omit_opt")]
    backed_by: Option<Text>,
}

impl Serialize for ToolMetadata {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let explicit = |a: &Attr<Text>| (!a.is_default()).then(|| a.get().cloned()).flatten();
        let view = View {
            name: self.name.clone(),
            version: self.version.clone(),
            display_name: self.display_name.clone(),
            implements: self.implements.clone(),
            implements_version: self.implements_version.clone(),
            homepage: self.homepage.clone(),
            description: self.description.clone(),
            aliases: explicit(&self.aliases).map(|_| Text::List(self.aliases())),
            tool_name: self.tool_name(),
            register_path: self.register_path.clone(),
            default_command: self
                .default_command()
                .map_err(|e| serde::ser::Error::custom(e.kind.tag(&e.message)))?
                .map(Text::Str),
            may_provide: explicit(&self.may_provide),
            backed_by: self.backed_by.clone(),
        };
        view.serialize(serializer)
    }
}

fn sym(t: &Option<Text>) -> Result<Option<String>, ModelError> {
    t.as_ref().map(Text::to_sym).transpose()
}

fn rv(t: &Option<Text>) -> Value {
    t.as_ref().map_or(Value::Nil, Text::to_ruby_value)
}

fn text(v: Option<&Value>) -> Result<Option<Text>, ModelError> {
    match v {
        None => Ok(None),
        Some(v) => Text::from_ruby_value(v),
    }
}

impl ToolMetadata {
    /// `#implements_ref` (`"interface@version"` when both are set).
    pub fn implements_ref(&self) -> Option<String> {
        match (&self.implements, &self.implements_version) {
            (Some(i), Some(v)) => Some(format!("{}@{}", i.to_ruby_s(), v.to_ruby_s())),
            _ => None,
        }
    }

    /// `#implements?(interface_name, version: nil)`
    pub fn is_implements(
        &self,
        interface_name: &str,
        version: Option<&str>,
    ) -> Result<bool, ModelError> {
        let Some(implements) = &self.implements else {
            return Ok(false);
        };
        let matches = implements.to_sym()? == interface_name || implements.eq_str(interface_name);
        Ok(match version {
            Some(v) => {
                matches
                    && self
                        .implements_version
                        .as_ref()
                        .is_some_and(|iv| iv.eq_str(v))
            }
            None => matches,
        })
    }

    /// `#default_command` (`@default_command || implements || name`, as a Symbol).
    pub fn default_command(&self) -> Result<Option<String>, ModelError> {
        if let Some(s) = sym(&self.default_command)? {
            return Ok(Some(s));
        }
        if let Some(s) = sym(&self.implements)? {
            return Ok(Some(s));
        }
        sym(&self.name)
    }

    /// `#may_provide_list` (`Array(may_provide).map(&:to_sym)`).
    pub fn may_provide_list(&self) -> Result<Vec<String>, ModelError> {
        self.may_provide
            .get()
            .map_or(Value::Nil, Text::to_ruby_value)
            .to_array()
            .iter()
            .map(Value::to_sym)
            .collect()
    }

    /// `#tool_name` (`@tool_name || name`).
    pub fn tool_name(&self) -> Option<Text> {
        self.tool_name.clone().or_else(|| self.name.clone())
    }

    /// `#aliases` (`Array(@aliases || [])`).
    pub fn aliases(&self) -> Vec<Value> {
        self.aliases
            .get()
            .map_or(Value::Nil, Text::to_ruby_value)
            .to_array()
    }

    /// `#backing_tool` (`backed_by&.to_sym`).
    pub fn backing_tool(&self) -> Result<Option<String>, ModelError> {
        sym(&self.backed_by)
    }

    /// `#to_s` (`"#{display_name || name} v#{version}"`).
    pub fn to_ruby_s(&self) -> String {
        let label = self.display_name.as_ref().or(self.name.as_ref());
        format!(
            "{} v{}",
            label.map_or(String::new(), Text::to_ruby_s),
            self.version.as_ref().map_or(String::new(), Text::to_ruby_s)
        )
    }

    /// `#inspect`
    pub fn inspect(&self) -> String {
        format!(
            "#<Ukiryu::ToolMetadata name={} version={} implements={}>",
            rv(&self.name).inspect(),
            rv(&self.version).inspect(),
            rv(&self.implements).inspect()
        )
    }

    /// `ToolMetadata.parse_implements(value)` → `[interface, version]`.
    pub fn parse_implements(value: Option<&Value>) -> (Option<String>, Option<String>) {
        match value {
            None | Some(Value::Nil) | Some(Value::Bool(false)) => (None, None),
            Some(v) => {
                let s = v.to_ruby_s();
                match s.split_once('@') {
                    Some((a, b)) => (Some(a.to_owned()), Some(b.to_owned())),
                    None => (Some(s), None),
                }
            }
        }
    }

    /// `ToolMetadata.from_hash(hash, tool_name:, register_path:)` (string keys).
    pub fn from_hash(
        hash: &Value,
        tool_name: &str,
        register_path: Option<&str>,
    ) -> Result<ToolMetadata, ModelError> {
        let (interface, version) = Self::parse_implements(hash.get_str("implements"));
        let or_empty = |key: &str| -> Result<Attr<Text>, ModelError> {
            Ok(Attr::Set(match hash.get_str(key).filter(|v| v.truthy()) {
                Some(v) => Text::from_ruby_value(v)?.unwrap_or(Text::List(Vec::new())),
                None => Text::List(Vec::new()),
            }))
        };
        Ok(ToolMetadata {
            name: Some(Text::new(tool_name)),
            version: text(hash.get_str("version"))?,
            display_name: text(hash.get_str("display_name"))?,
            implements: interface.map(Text::Str),
            implements_version: version.map(Text::Str),
            homepage: text(hash.get_str("homepage"))?,
            description: text(hash.get_str("description"))?,
            aliases: or_empty("aliases")?,
            tool_name: Some(Text::new(tool_name)),
            register_path: register_path.map(Text::new),
            default_command: text(hash.get_str("default_command"))?,
            may_provide: or_empty("may_provide")?,
            backed_by: text(hash.get_str("backed_by"))?,
        })
    }
}

impl Model for ToolMetadata {}
