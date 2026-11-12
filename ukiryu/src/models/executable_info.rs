//! Source: lib/ukiryu/models/executable_info.rb (`Ukiryu::Models::ExecutableInfo`).
use super::value::RubyValue;

/// Where an executable was found (`:path` or a shell `:alias`).
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutableInfo {
    pub path: RubyValue,
    pub source: RubyValue,
    pub shell: RubyValue,
    pub alias_definition: RubyValue,
}

impl ExecutableInfo {
    pub fn new(
        path: RubyValue,
        source: RubyValue,
        shell: RubyValue,
        alias_definition: RubyValue,
    ) -> Self {
        ExecutableInfo {
            path,
            source,
            shell,
            alias_definition,
        }
    }

    fn source_is(&self, s: &str) -> bool {
        matches!(&self.source, RubyValue::Sym(x) if x == s)
    }

    /// `#description` (`nil` for an unknown source).
    pub fn description(&self) -> Option<String> {
        if self.source_is("path") {
            Some(format!("Found in PATH at {}", self.path.to_ruby_s()))
        } else if self.source_is("alias") {
            Some(format!(
                "Shell alias in {}: {}",
                self.shell.to_ruby_s(),
                self.alias_definition.to_ruby_s()
            ))
        } else {
            None
        }
    }

    /// `#alias?`
    pub fn is_alias(&self) -> bool {
        self.source_is("alias")
    }

    /// `#path?`
    pub fn is_path(&self) -> bool {
        self.source_is("path")
    }
}
