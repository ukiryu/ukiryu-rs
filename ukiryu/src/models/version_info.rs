//! Source: lib/ukiryu/models/version_info.rb (`Ukiryu::Models::VersionInfo`).
use super::value::RubyValue;

/// A detected version and the method that found it (`:command`,
/// `:man_page`, `:profile`).
#[derive(Clone, Debug, PartialEq)]
pub struct VersionInfo {
    pub value: RubyValue,
    pub method_used: RubyValue,
    pub available_methods: Vec<RubyValue>,
}

impl VersionInfo {
    pub fn new(
        value: RubyValue,
        method_used: RubyValue,
        available_methods: Vec<RubyValue>,
    ) -> Self {
        VersionInfo {
            value,
            method_used,
            available_methods,
        }
    }

    fn method_is(&self, m: &str) -> bool {
        matches!(&self.method_used, RubyValue::Sym(s) if s == m)
    }

    /// `#from_command?`
    pub fn is_from_command(&self) -> bool {
        self.method_is("command")
    }

    /// `#from_man_page?`
    pub fn is_from_man_page(&self) -> bool {
        self.method_is("man_page")
    }

    /// `#from_profile?`
    pub fn is_from_profile(&self) -> bool {
        self.method_is("profile")
    }

    /// `#to_s` (returns the value itself unless it came from a man page).
    pub fn to_ruby_s(&self) -> RubyValue {
        if self.is_from_man_page() {
            RubyValue::Str(format!("{} (man page)", self.value.to_ruby_s()))
        } else {
            self.value.clone()
        }
    }

    /// `#to_h` (`{value:, method:, available_methods:}`).
    pub fn to_h(&self) -> RubyValue {
        RubyValue::Hash(vec![
            (RubyValue::sym("value"), self.value.clone()),
            (RubyValue::sym("method"), self.method_used.clone()),
            (
                RubyValue::sym("available_methods"),
                RubyValue::Array(self.available_methods.clone()),
            ),
        ])
    }

    /// `#inspect`
    pub fn inspect(&self) -> String {
        format!(
            "#<VersionInfo value=\"{}\" method={}>",
            self.value.to_ruby_s(),
            self.method_used.to_ruby_s()
        )
    }
}
