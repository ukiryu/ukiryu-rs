//! `CommandBuilder#build_env_vars`.
use super::spec::{CommandSpec, EnvVarSets, EnvVarSpec};
use super::{param, Params};
use crate::types::ruby;

/// An insertion-ordered environment, with Ruby `Hash#[]=` semantics: setting
/// an existing name replaces its value in place. A definition without a
/// name stores under a `nil` key, as Ruby does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvVars(pub Vec<(Option<String>, String)>);

impl EnvVars {
    fn set(&mut self, name: Option<String>, value: String) {
        match self.0.iter_mut().find(|(k, _)| *k == name) {
            Some(slot) => slot.1 = value,
            None => self.0.push((name, value)),
        }
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k.as_deref() == Some(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// `CommandBuilder#build_env_vars`: first the variables of each profile set
/// named in `use_env_vars` (unknown sets are skipped), then the command's own
/// `env_vars`, which override. A variable restricted to platforms applies
/// only on those platforms. A literal `value` (even `""`) wins over `from`;
/// `from` reads a parameter; a `nil` result sets nothing, anything else is
/// set to its `to_s`.
pub fn build_env_vars(
    command: &CommandSpec,
    env_var_sets: Option<&EnvVarSets>,
    params: &dyn Params,
    platform: &str,
) -> EnvVars {
    let mut env = EnvVars::default();
    for set_name in command.use_env_vars.as_deref().unwrap_or_default() {
        if let Some(set) = env_var_sets.and_then(|s| s.get(set_name)) {
            for ev in set {
                apply(&mut env, ev, params, platform);
            }
        }
    }
    for ev in command.env_vars.as_deref().unwrap_or_default() {
        apply(&mut env, ev, params, platform);
    }
    env
}

fn apply(env: &mut EnvVars, ev: &EnvVarSpec, params: &dyn Params, platform: &str) {
    let platforms = ev.platforms.as_deref().unwrap_or_default();
    if !platforms.is_empty() && !platforms.iter().any(|p| p == platform) {
        return;
    }
    let value = match (&ev.value, &ev.from) {
        (Some(v), _) => Some(v.clone()),
        (None, Some(from)) => param(params, from).map(ruby::to_s),
        (None, None) => None,
    };
    if let Some(value) = value {
        env.set(ev.name.clone(), value);
    }
}
