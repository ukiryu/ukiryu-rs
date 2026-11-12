//! Composing definitions through `inherits`/`includes` and merge strategies.
//!
//! Source: `lib/ukiryu/definition/definition_composer.rb`
//! (`Ukiryu::Definition::DefinitionComposer`).
//!
//! The Ruby merge works on shallow copies, so merged profiles stay the same
//! objects as the base/addition profiles and later command updates are
//! visible through every alias (for example `:append` doubles the commands of
//! the appended addition profiles and also mutates both inputs).
//! [`DefinitionComposer::merge_definitions`] reproduces that within one call
//! by tracking which input profile each merged slot aliases and updating the
//! inputs in place. Aliasing that crosses calls (Ruby mutating
//! `Loader`-cached definitions during `includes`) is not reproduced: loaded
//! definitions are cloned here.
use super::discovery::Discovery;
use super::loader::{DefinitionDocument, Loader};
use super::{ruby_inspect_str, Result};
use yeptris::{Number, Value};

/// A command entry (`Models::CommandDefinition`).
pub trait ComposableCommand: Clone {
    fn command_name(&self) -> Option<&str>;
}

/// A platform profile (`Models::PlatformProfile`).
pub trait ComposableProfile: Clone {
    type Command: ComposableCommand;
    fn profile_name(&self) -> Option<&str>;
    /// `commands` (`None` = nil).
    fn commands(&self) -> Option<&Vec<Self::Command>>;
    fn commands_mut(&mut self) -> &mut Option<Vec<Self::Command>>;
    /// `option_style`, including any model default.
    fn option_style(&self) -> Option<&str>;
    fn set_option_style(&mut self, value: Option<String>);
}

/// A composable definition (`Models::ToolDefinition`, or any object with
/// `inherits`/`includes`).
pub trait ComposableDefinition: Clone {
    type Profile: ComposableProfile;
    /// `profiles` (`None` = nil).
    fn profiles(&self) -> Option<&Vec<Self::Profile>>;
    fn profiles_mut(&mut self) -> &mut Option<Vec<Self::Profile>>;
    /// `inherits` when the object responds to it and it is set.
    /// `ToolDefinition` has no such attribute (the default).
    fn inherits(&self) -> Option<Vec<Value>> {
        None
    }
    /// `includes`, as for [`ComposableDefinition::inherits`].
    fn includes(&self) -> Option<Vec<Value>> {
        None
    }
}

/// Merge strategy symbol.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MergeStrategy {
    Override,
    Replace,
    Prepend,
    Append,
    /// Unknown symbols leave profiles untouched.
    Other(String),
}

impl MergeStrategy {
    /// `MERGE_STRATEGIES`.
    pub const ALL: [MergeStrategy; 4] = [
        MergeStrategy::Override,
        MergeStrategy::Replace,
        MergeStrategy::Prepend,
        MergeStrategy::Append,
    ];

    pub fn as_str(&self) -> &str {
        match self {
            MergeStrategy::Override => "override",
            MergeStrategy::Replace => "replace",
            MergeStrategy::Prepend => "prepend",
            MergeStrategy::Append => "append",
            MergeStrategy::Other(s) => s,
        }
    }

    pub fn from_symbol(name: &str) -> Self {
        match name {
            "override" => MergeStrategy::Override,
            "replace" => MergeStrategy::Replace,
            "prepend" => MergeStrategy::Prepend,
            "append" => MergeStrategy::Append,
            other => MergeStrategy::Other(other.to_string()),
        }
    }
}

/// `Ukiryu::Definition::DefinitionComposer` class methods.
pub struct DefinitionComposer;

enum Slot<P> {
    Base(usize),
    Add(usize),
    Own(P),
}

fn slot_ref<'a, P>(slot: &'a Slot<P>, base: &'a Option<Vec<P>>, add: &'a Option<Vec<P>>) -> &'a P {
    match slot {
        Slot::Base(i) => &base.as_ref().expect("base slot")[*i],
        Slot::Add(j) => &add.as_ref().expect("addition slot")[*j],
        Slot::Own(p) => p,
    }
}

fn slot_mut<'a, P>(
    slot: &'a mut Slot<P>,
    base: &'a mut Option<Vec<P>>,
    add: &'a mut Option<Vec<P>>,
) -> &'a mut P {
    match slot {
        Slot::Base(i) => &mut base.as_mut().expect("base slot")[*i],
        Slot::Add(j) => &mut add.as_mut().expect("addition slot")[*j],
        Slot::Own(p) => p,
    }
}

/// Ruby `to_s` used when interpolating a composition spec.
fn ruby_to_s(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Tagged(t) => ruby_to_s(&t.value),
        other => ruby_inspect(other),
    }
}

/// Ruby `inspect` for YAML values (Ruby 3.4+ Hash format).
fn ruby_inspect(v: &Value) -> String {
    match v {
        Value::Null => "nil".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(Number::I64(i)) => i.to_string(),
        Value::Number(Number::U64(u)) => u.to_string(),
        Value::Number(Number::F64(f)) => format!("{f:?}"),
        Value::String(s) => ruby_inspect_str(s),
        Value::Sequence(items) => format!(
            "[{}]",
            items
                .iter()
                .map(ruby_inspect)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Mapping(m) => {
            if m.0.is_empty() {
                return "{}".into();
            }
            let pairs: Vec<String> =
                m.0.iter()
                    .map(|(k, v)| format!("{} => {}", ruby_inspect(k), ruby_inspect(v)))
                    .collect();
            format!("{{{}}}", pairs.join(", "))
        }
        Value::Tagged(t) => ruby_inspect(&t.value),
    }
}

impl DefinitionComposer {
    /// `compose(definition)`: `inherits` first, then `includes` of the
    /// result. Inheriting yields a copy of the base definition, so a
    /// definition with both `inherits` and `includes` loses its includes
    /// whenever any base is found (Ruby behavior).
    pub fn compose<T>(definition: T, discovery: &Discovery, loader: &Loader<T>) -> Result<T>
    where
        T: ComposableDefinition + DefinitionDocument,
    {
        let mut definition = definition;
        if let Some(inherits) = definition.inherits() {
            definition = Self::process_inherits(definition, &inherits, discovery, loader)?;
        }
        if let Some(includes) = definition.includes() {
            definition = Self::process_includes(definition, &includes, discovery, loader)?;
        }
        Ok(definition)
    }

    /// `process_inherits`: bases are applied last-to-first with `:replace`.
    pub fn process_inherits<T>(
        definition: T,
        inherits: &[Value],
        discovery: &Discovery,
        loader: &Loader<T>,
    ) -> Result<T>
    where
        T: ComposableDefinition + DefinitionDocument,
    {
        let mut current = definition;
        for spec in inherits.iter().rev() {
            if let Some(mut base) = Self::load_base_definition(spec, discovery, loader)? {
                current = Self::merge_definitions(&mut base, &mut current, &MergeStrategy::Replace);
            }
        }
        Ok(current)
    }

    /// `process_includes`: each include is merged in order with `:append`.
    pub fn process_includes<T>(
        definition: T,
        includes: &[Value],
        discovery: &Discovery,
        loader: &Loader<T>,
    ) -> Result<T>
    where
        T: ComposableDefinition + DefinitionDocument,
    {
        let mut current = definition;
        for spec in includes {
            if let Some(mut include) = Self::load_base_definition(spec, discovery, loader)? {
                current =
                    Self::merge_definitions(&mut current, &mut include, &MergeStrategy::Append);
            }
        }
        Ok(current)
    }

    /// `load_base_definition(spec)`: a tool name (latest by priority) or a
    /// mapping with `tool` and optional exact `version`; other specs and
    /// unknown tools give `None`. Load errors propagate.
    pub fn load_base_definition<T: DefinitionDocument + Clone>(
        spec: &Value,
        discovery: &Discovery,
        loader: &Loader<T>,
    ) -> Result<Option<T>> {
        let metadata = match spec {
            Value::String(name) => discovery.find(name, None),
            Value::Mapping(map) => {
                let tool = map.get(&Value::String("tool".into()));
                let version = map.get(&Value::String("version".into()));
                let tool = match tool {
                    Some(Value::String(t)) => Some(t.as_str()),
                    _ => None,
                };
                match (tool, version) {
                    (None, _) => None,
                    (Some(t), None | Some(Value::Null) | Some(Value::Bool(false))) => {
                        discovery.find(t, None)
                    }
                    (Some(t), Some(Value::String(v))) => discovery.find(t, Some(v)),
                    (Some(_), Some(_)) => None,
                }
            }
            _ => None,
        };
        match metadata {
            Some(md) => Ok(Some((*md.load_definition(loader)?).clone())),
            None => Ok(None),
        }
    }

    /// `merge_definitions(base, addition, strategy:)`. Returns the merged
    /// copy of `base`; `base` and `addition` are updated in place exactly as
    /// the Ruby objects are through shared profile references.
    pub fn merge_definitions<D: ComposableDefinition>(
        base: &mut D,
        addition: &mut D,
        strategy: &MergeStrategy,
    ) -> D {
        let mut merged = base.clone();
        let mut bp = base.profiles_mut().take();
        let mut ap = addition.profiles_mut().take();
        let base_len = bp.as_ref().map_or(0, Vec::len);

        let mut slots: Option<Vec<Slot<D::Profile>>> =
            bp.as_ref().map(|v| (0..v.len()).map(Slot::Base).collect());
        if let Some(add) = ap.as_ref() {
            if slots.is_none() {
                slots = Some(Vec::new());
            }
            let add_slots = || (0..add.len()).map(Slot::Add);
            let base_slots = || (0..base_len).map(Slot::Base);
            match strategy {
                MergeStrategy::Replace => slots = Some(add_slots().collect()),
                MergeStrategy::Override => {
                    let mut out: Vec<Slot<D::Profile>> = base_slots().collect();
                    for addition_profile in add {
                        let existing = out.iter().position(|s| {
                            slot_ref(s, &bp, &ap).profile_name() == addition_profile.profile_name()
                        });
                        match existing {
                            Some(i) => {
                                let content = Self::merge_profile_content(
                                    slot_ref(&out[i], &bp, &ap),
                                    addition_profile,
                                );
                                out[i] = Slot::Own(content);
                            }
                            None => out.push(Slot::Own(addition_profile.clone())),
                        }
                    }
                    slots = Some(out);
                }
                MergeStrategy::Append => slots = Some(base_slots().chain(add_slots()).collect()),
                MergeStrategy::Prepend => slots = Some(add_slots().chain(base_slots()).collect()),
                MergeStrategy::Other(_) => {}
            }
        }

        if let Some(list) = slots.as_mut() {
            for slot in list.iter_mut() {
                let name = slot_ref(slot, &bp, &ap).profile_name().map(str::to_string);
                let Some(add) = ap.as_ref() else { continue };
                let Some(found) = add.iter().find(|p| p.profile_name() == name.as_deref()) else {
                    continue;
                };
                let Some(addition_commands) = found.commands().cloned() else {
                    continue;
                };
                let profile = slot_mut(slot, &mut bp, &mut ap);
                let current = profile.commands_mut().get_or_insert_with(Vec::new).clone();
                let updated = match strategy {
                    MergeStrategy::Replace => addition_commands,
                    MergeStrategy::Override => {
                        Self::merge_commands_by_name(&current, &addition_commands)
                    }
                    MergeStrategy::Append => current.into_iter().chain(addition_commands).collect(),
                    MergeStrategy::Prepend => {
                        addition_commands.into_iter().chain(current).collect()
                    }
                    MergeStrategy::Other(_) => continue,
                };
                *profile.commands_mut() = Some(updated);
            }
        }

        let merged_profiles =
            slots.map(|list| list.iter().map(|s| slot_ref(s, &bp, &ap).clone()).collect());
        *merged.profiles_mut() = merged_profiles;
        *base.profiles_mut() = bp;
        *addition.profiles_mut() = ap;
        merged
    }

    /// `merge_profiles_by_name(base_profiles, addition_profiles)`.
    pub fn merge_profiles_by_name<P: ComposableProfile>(base: &[P], addition: &[P]) -> Vec<P> {
        let mut merged = base.to_vec();
        for addition_profile in addition {
            match merged
                .iter()
                .position(|p| p.profile_name() == addition_profile.profile_name())
            {
                Some(i) => merged[i] = Self::merge_profile_content(&merged[i], addition_profile),
                None => merged.push(addition_profile.clone()),
            }
        }
        merged
    }

    /// `merge_profile_content(base, addition)`: commands merged by name;
    /// a non-nil `option_style` of the addition overrides (model defaults
    /// count as set). `PlatformProfile` has no `description`, `environment`,
    /// `options`, `flags` or `arguments`, so those Ruby branches never run.
    pub fn merge_profile_content<P: ComposableProfile>(base: &P, addition: &P) -> P {
        let mut merged = base.clone();
        if let Some(addition_commands) = addition.commands() {
            let base_commands = base.commands().cloned().unwrap_or_default();
            *merged.commands_mut() = Some(Self::merge_commands_by_name(
                &base_commands,
                addition_commands,
            ));
        }
        if let Some(style) = addition.option_style() {
            merged.set_option_style(Some(style.to_string()));
        }
        merged
    }

    /// `merge_commands_by_name(base, addition)`: same-named commands are
    /// replaced at the first match's position, others appended.
    pub fn merge_commands_by_name<C: ComposableCommand>(base: &[C], addition: &[C]) -> Vec<C> {
        let mut merged = base.to_vec();
        for command in addition {
            match merged
                .iter()
                .position(|c| c.command_name() == command.command_name())
            {
                Some(i) => merged[i] = command.clone(),
                None => merged.push(command.clone()),
            }
        }
        merged
    }

    /// `find_profile(profiles, name)`.
    pub fn find_profile<'a, P: ComposableProfile>(
        profiles: Option<&'a [P]>,
        name: Option<&str>,
    ) -> Option<&'a P> {
        profiles?.iter().find(|p| p.profile_name() == name)
    }

    /// `validate_composable(definition)`: one message per unresolvable spec.
    pub fn validate_composable<T>(
        definition: &T,
        discovery: &Discovery,
        loader: &Loader<T>,
    ) -> Result<Vec<String>>
    where
        T: ComposableDefinition + DefinitionDocument,
    {
        let mut errors = Vec::new();
        for spec in definition.inherits().unwrap_or_default() {
            if Self::load_base_definition(&spec, discovery, loader)?.is_none() {
                errors.push(format!(
                    "Cannot inherit from '{}': definition not found",
                    ruby_to_s(&spec)
                ));
            }
        }
        for spec in definition.includes().unwrap_or_default() {
            if Self::load_base_definition(&spec, discovery, loader)?.is_none() {
                errors.push(format!(
                    "Cannot include '{}': definition not found",
                    ruby_to_s(&spec)
                ));
            }
        }
        Ok(errors)
    }
}
