//! Source: lib/ukiryu/models/components.rb (`Ukiryu::Models::Components`).
use super::attr::{de_fns as d, omit_model, Attr, Model};
use super::exit_codes::ExitCodes;
use super::value::RubyValue;
use serde::{Deserialize, Serialize};

fn default_hash() -> Attr<RubyValue> {
    Attr::Set(RubyValue::Hash(Vec::new()))
}

/// Reusable definitions referenced as `#/components/<type>/<name>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Components {
    #[serde(default = "default_hash", skip_serializing_if = "Attr::omit")]
    pub options: Attr<RubyValue>,
    #[serde(default = "default_hash", skip_serializing_if = "Attr::omit")]
    pub flags: Attr<RubyValue>,
    #[serde(default = "default_hash", skip_serializing_if = "Attr::omit")]
    pub arguments: Attr<RubyValue>,
    #[serde(
        default,
        deserialize_with = "d::model",
        skip_serializing_if = "omit_model"
    )]
    pub exit_codes: Option<Box<ExitCodes>>,
}

impl Default for Components {
    fn default() -> Self {
        Components {
            options: Attr::Default(RubyValue::Hash(Vec::new())),
            flags: Attr::Default(RubyValue::Hash(Vec::new())),
            arguments: Attr::Default(RubyValue::Hash(Vec::new())),
            exit_codes: None,
        }
    }
}

/// What `#resolve(ref)` returns.
#[derive(Clone, Debug, PartialEq)]
pub enum ComponentRef<'a> {
    Value(&'a RubyValue),
    ExitCodes(&'a ExitCodes),
}

/// Matches `^#/components/(options|flags|arguments|exit_codes)/(.+)$`.
fn parse_ref(reference: &str) -> Option<(&str, &str)> {
    reference.split('\n').find_map(|line| {
        let rest = line.strip_prefix("#/components/")?;
        let (kind, name) = rest.split_once('/')?;
        let kind_ok = matches!(kind, "options" | "flags" | "arguments" | "exit_codes");
        (kind_ok && !name.is_empty()).then_some((kind, name))
    })
}

fn dig<'a>(h: &'a Attr<RubyValue>, name: &str) -> Option<&'a RubyValue> {
    h.get().and_then(|h| h.get_str(name))
}

impl Components {
    /// `#option(name)`
    pub fn option(&self, name: &str) -> Option<&RubyValue> {
        dig(&self.options, name)
    }

    /// `#flag(name)`
    pub fn flag(&self, name: &str) -> Option<&RubyValue> {
        dig(&self.flags, name)
    }

    /// `#argument(name)`
    pub fn argument(&self, name: &str) -> Option<&RubyValue> {
        dig(&self.arguments, name)
    }

    /// `#can_resolve?(ref)`
    pub fn can_resolve(&self, reference: &str) -> bool {
        let Some((kind, name)) = parse_ref(reference) else {
            return false;
        };
        let has = |h: &Attr<RubyValue>| {
            h.get()
                .is_some_and(|h| h.get(&RubyValue::str(name)).is_some())
        };
        match kind {
            "options" => has(&self.options),
            "flags" => has(&self.flags),
            "arguments" => has(&self.arguments),
            _ => self.exit_codes.is_some(),
        }
    }

    /// `#resolve(ref)`
    pub fn resolve(&self, reference: &str) -> Option<ComponentRef<'_>> {
        let (kind, name) = parse_ref(reference)?;
        match kind {
            "options" => self.option(name).map(ComponentRef::Value),
            "flags" => self.flag(name).map(ComponentRef::Value),
            "arguments" => self.argument(name).map(ComponentRef::Value),
            _ => self.exit_codes.as_deref().map(ComponentRef::ExitCodes),
        }
    }
}

impl Model for Components {}
