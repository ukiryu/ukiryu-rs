//! Lint tool definitions for best practices.
//!
//! Source: `lib/ukiryu/definition/definition_linter.rb`
//! (`Ukiryu::Definition::DefinitionLinter`, its `LintResult` and `Rules`)
//! at the pinned Ruby oracle `ab6df353`. Issues use the shared
//! [`LintIssue`] type; YAML goes through the schema module's `safe_load`
//! port; the schema version check uses [`VersionResolver`].
//!
//! The linter walks the loaded Ruby object (a [`Data`]) exactly as the Ruby
//! code does, including its limitations:
//!
//! - `lint_string` does not pass `symbolize_names: true`, so a document with
//!   ordinary keys is only checked for completeness (and for a `$schema`
//!   string key). `lint_file` symbolizes keys, so `$schema` there is never
//!   found.
//! - Values of an unexpected type make the Ruby code raise (`NoMethodError`,
//!   `TypeError`) out of `lint`, `lint_string` and `lint_file`; those are
//!   returned as [`LintError::Raised`] with the Ruby class and message
//!   (Ruby 4.0 wording).
//! - `lint_file` turns only a missing file and YAML syntax errors into an
//!   issue; other file errors and `Psych::AliasesNotEnabled` are raised.
//! - The text after `Invalid YAML: ` is the YAML engine's message, not
//!   libyaml's.
use super::lint_issue::{LintIssue, LintIssueHash, Severity};
use super::ruby_split;
use super::version_resolver::VersionResolver;
use crate::schema::{safe_load, Data, Permitted, SchemaError};
use regex::Regex;
use serde::Serialize;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::path::Path;
use std::sync::OnceLock;

/// One entry of the `Rules` constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub rule_id: &'static str,
    pub message: &'static str,
    pub suggestion: &'static str,
    /// Ruby regexp source, when the rule has one.
    pub pattern: Option<&'static str>,
    /// Version threshold, when the rule has one.
    pub threshold: Option<&'static str>,
}

/// `DefinitionLinter::Rules`.
pub mod rules {
    use super::Rule;

    pub const TOOL_NAME_FORMAT: Rule = Rule {
        rule_id: "naming_tool_name_format",
        pattern: Some("^[a-z][a-z0-9_-]*$"),
        message: "Tool name should start with a lowercase letter and contain only lowercase letters, numbers, hyphens, and underscores",
        suggestion: "Use lowercase with hyphens for multi-word names (e.g., \"my-tool\")",
        threshold: None,
    };
    pub const COMMAND_NAME_FORMAT: Rule = Rule {
        rule_id: "naming_command_name_format",
        pattern: Some("^[a-z][a-z0-9_]*$"),
        message: "Command names should be lowercase with underscores",
        suggestion: "Use snake_case for command names (e.g., \"build_command\")",
        threshold: None,
    };
    pub const MISSING_DESCRIPTION: Rule = Rule {
        rule_id: "complete_missing_description",
        message: "Tool is missing a description",
        suggestion: "Add a \"description\" field to help users understand what this tool does",
        pattern: None,
        threshold: None,
    };
    pub const MISSING_HOMEPAGE: Rule = Rule {
        rule_id: "complete_missing_homepage",
        message: "Tool is missing a homepage URL",
        suggestion: "Add a \"homepage\" field linking to the tool's website",
        pattern: None,
        threshold: None,
    };
    pub const MISSING_VERSION_DETECTION: Rule = Rule {
        rule_id: "complete_missing_version_detection",
        message: "Tool is missing version detection",
        suggestion: "Add \"version_detection\" to auto-detect installed versions",
        pattern: None,
        threshold: None,
    };
    pub const SUSPICIOUS_SUBCOMMAND: Rule = Rule {
        rule_id: "security_suspicious_subcommand",
        pattern: Some(r"(^|\s|;)\s*(rm\s+-rf|del|format|mkfs)"),
        message: "Subcommand contains potentially dangerous shell commands",
        suggestion: "Avoid using destructive commands in subcommands",
        threshold: None,
    };
    pub const UNVALIDATED_USER_INPUT: Rule = Rule {
        rule_id: "security_unvalidated_input",
        message: "Arguments should specify type validation",
        suggestion: "Add \"type\" field to all arguments for validation",
        pattern: None,
        threshold: None,
    };
    pub const OLD_SCHEMA_VERSION: Rule = Rule {
        rule_id: "deprecated_old_schema",
        threshold: Some("1.0"),
        message: "Schema version should be 1.0",
        suggestion: "Use ukiryu_schema: 1.0 (we are in 1.0 development)",
        pattern: None,
    };
    /// Inline in `check_best_practices` (not part of `Rules`).
    pub const REDUNDANT_DEFAULT_PROFILE: Rule = Rule {
        rule_id: "style_redundant_default_profile",
        message: "Single profile named \"default\" is redundant",
        suggestion:
            "For single-profile definitions, omit the profiles array and define commands directly",
        pattern: None,
        threshold: None,
    };
    /// Inline in `check_best_practices` (not part of `Rules`).
    pub const MISSING_PLATFORMS: Rule = Rule {
        rule_id: "complete_missing_platforms",
        message: "Profile missing platforms specification",
        suggestion: "Specify supported platforms (macos, linux, windows)",
        pattern: None,
        threshold: None,
    };

    /// `NAMING_RULES`, in Ruby key order.
    pub const NAMING_RULES: [(&str, Rule); 2] = [
        ("tool_name_format", TOOL_NAME_FORMAT),
        ("command_name_format", COMMAND_NAME_FORMAT),
    ];
    /// `COMPLETENESS_RULES`, in Ruby key order.
    pub const COMPLETENESS_RULES: [(&str, Rule); 3] = [
        ("missing_description", MISSING_DESCRIPTION),
        ("missing_homepage", MISSING_HOMEPAGE),
        ("missing_version_detection", MISSING_VERSION_DETECTION),
    ];
    /// `SECURITY_RULES`, in Ruby key order.
    pub const SECURITY_RULES: [(&str, Rule); 2] = [
        ("suspicious_subcommand", SUSPICIOUS_SUBCOMMAND),
        ("unvalidated_user_input", UNVALIDATED_USER_INPUT),
    ];
    /// `DEPRECATED_RULES`, in Ruby key order.
    pub const DEPRECATED_RULES: [(&str, Rule); 1] = [("old_schema_version", OLD_SCHEMA_VERSION)];
}

/// The `rules:` argument of `lint` (`{ enabled: [...], disabled: [...] }`).
/// A non-empty `disabled` list wins over `enabled`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuleFilter {
    pub enabled: Option<Vec<String>>,
    pub disabled: Option<Vec<String>>,
}

/// A failure that the Ruby code raises instead of returning a result, or an
/// input whose Ruby behaviour this port does not reproduce.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LintError {
    /// A Ruby exception (class name, message) escaping the linter.
    Raised { class: String, message: String },
    /// Behaviour of the original that is not reproduced.
    Unsupported { feature: String, detail: String },
}

impl LintError {
    fn raised(class: &str, message: impl Into<String>) -> Self {
        LintError::Raised {
            class: class.to_owned(),
            message: message.into(),
        }
    }

    /// The Ruby exception class, when there is one.
    pub fn ruby_class(&self) -> Option<&str> {
        match self {
            LintError::Raised { class, .. } => Some(class),
            LintError::Unsupported { .. } => None,
        }
    }

    pub fn message(&self) -> String {
        match self {
            LintError::Raised { message, .. } => message.clone(),
            LintError::Unsupported { feature, detail } => {
                format!("unsupported ({feature}): {detail}")
            }
        }
    }
}

impl From<SchemaError> for LintError {
    fn from(e: SchemaError) -> Self {
        match e {
            SchemaError::Unsupported { feature, detail } => {
                LintError::Unsupported { feature, detail }
            }
            other => LintError::Raised {
                class: other.ruby_class().unwrap_or_default(),
                message: other.message(),
            },
        }
    }
}

impl fmt::Display for LintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LintError::Raised { class, message } => write!(f, "{class}: {message}"),
            LintError::Unsupported { .. } => f.write_str(&self.message()),
        }
    }
}

impl std::error::Error for LintError {}

/// `DefinitionLinter::LintResult`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LintResult {
    pub issues: Vec<LintIssue>,
}

/// `LintResult#to_h`, in Ruby key order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LintResultHash {
    pub issues: Vec<LintIssueHash>,
    pub total_count: usize,
    pub error_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    pub style_count: usize,
}

impl LintResult {
    pub fn new(issues: Vec<LintIssue>) -> Self {
        LintResult { issues }
    }

    /// `by_severity(severity)`.
    pub fn by_severity(&self, severity: &Severity) -> Vec<&LintIssue> {
        self.issues
            .iter()
            .filter(|i| &i.severity == severity)
            .collect()
    }

    pub fn errors(&self) -> Vec<&LintIssue> {
        self.by_severity(&Severity::Error)
    }

    pub fn warnings(&self) -> Vec<&LintIssue> {
        self.by_severity(&Severity::Warning)
    }

    pub fn infos(&self) -> Vec<&LintIssue> {
        self.by_severity(&Severity::Info)
    }

    pub fn styles(&self) -> Vec<&LintIssue> {
        self.by_severity(&Severity::Style)
    }

    /// `has_issues?`.
    pub fn has_issues(&self) -> bool {
        !self.issues.is_empty()
    }

    /// `has_errors?`. The `ukiryu lint file` command exits 1 exactly when
    /// this is true.
    pub fn has_errors(&self) -> bool {
        !self.errors().is_empty()
    }

    pub fn count(&self) -> usize {
        self.issues.len()
    }

    pub fn count_by_severity(&self, severity: &Severity) -> usize {
        self.by_severity(severity).len()
    }

    pub fn to_h(&self) -> LintResultHash {
        LintResultHash {
            issues: self.issues.iter().map(LintIssue::to_h).collect(),
            total_count: self.count(),
            error_count: self.errors().len(),
            warning_count: self.warnings().len(),
            info_count: self.infos().len(),
            style_count: self.styles().len(),
        }
    }

    /// `to_h.to_json` (what `ukiryu lint file --format json` prints).
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.to_h()).expect("strings serialize")
    }
}

impl fmt::Display for LintResult {
    /// `LintResult#to_s`. Issues with a severity outside the four known
    /// ones are counted but not listed, as in Ruby.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.has_issues() {
            return f.write_str("No issues found");
        }
        let mut out = vec![format!("Found {} issue(s):", self.count())];
        for severity in Severity::ALL.iter() {
            let issues = self.by_severity(severity);
            if issues.is_empty() {
                continue;
            }
            out.push(String::new());
            out.push(format!("{}:", severity.as_str().to_uppercase()));
            for issue in issues {
                out.push(format!("  {issue}"));
            }
        }
        f.write_str(&out.join("\n"))
    }
}

/// `Ukiryu::Definition::DefinitionLinter`.
pub struct DefinitionLinter;

impl DefinitionLinter {
    /// `DefinitionLinter.lint(definition, rules:)`.
    pub fn lint(definition: &Data, rules: Option<&RuleFilter>) -> Result<LintResult, LintError> {
        let Data::Hash(_) = definition else {
            return Ok(LintResult::new(vec![LintIssue::error(
                "Definition must be a hash/object",
                None,
                None,
                None,
            )]));
        };
        let mut issues = Vec::new();
        check_naming_conventions(definition, &mut issues)?;
        check_completeness(definition, &mut issues);
        check_security(definition, &mut issues)?;
        check_deprecated_patterns(definition, &mut issues)?;
        check_best_practices(definition, &mut issues);

        if let Some(rules) = rules {
            let enabled = rules.enabled.as_deref().unwrap_or_default();
            let disabled = rules.disabled.as_deref().unwrap_or_default();
            let listed = |list: &[String], issue: &LintIssue| {
                issue
                    .rule_id
                    .as_ref()
                    .is_some_and(|id| list.iter().any(|r| r == id))
            };
            issues.retain(|issue| {
                if !disabled.is_empty() {
                    !listed(disabled, issue)
                } else if !enabled.is_empty() {
                    listed(enabled, issue)
                } else {
                    true
                }
            });
        }
        Ok(LintResult::new(issues))
    }

    /// `DefinitionLinter.lint_file(file_path, rules:)`: the file is loaded
    /// with `YAML.safe_load(permitted_classes: [Symbol, Date, Time],
    /// symbolize_names: true)`.
    pub fn lint_file(
        path: impl AsRef<Path>,
        rules: Option<&RuleFilter>,
    ) -> Result<LintResult, LintError> {
        let path = path.as_ref();
        let shown = path.display();
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                return match e.kind() {
                    std::io::ErrorKind::NotFound => {
                        Ok(single_error(format!("File not found: {shown}")))
                    }
                    std::io::ErrorKind::IsADirectory => Err(LintError::raised(
                        "Errno::EISDIR",
                        format!("Is a directory @ io_fread - {shown}"),
                    )),
                    std::io::ErrorKind::NotADirectory => Err(LintError::raised(
                        "Errno::ENOTDIR",
                        format!("Not a directory @ rb_sysopen - {shown}"),
                    )),
                    std::io::ErrorKind::PermissionDenied => Err(LintError::raised(
                        "Errno::EACCES",
                        format!("Permission denied @ rb_sysopen - {shown}"),
                    )),
                    _ => Err(LintError::Unsupported {
                        feature: "file read".to_owned(),
                        detail: format!("{shown}: {e}"),
                    }),
                };
            }
        };
        let text = String::from_utf8(bytes).map_err(|_| LintError::Unsupported {
            feature: "file encoding".to_owned(),
            detail: format!("{shown} is not UTF-8"),
        })?;
        Self::lint_yaml(&text, true, rules)
    }

    /// `DefinitionLinter.lint_string(yaml_string, rules:)`: loaded with
    /// `YAML.safe_load(permitted_classes: [Symbol, Date, Time])`, keys not
    /// symbolized.
    pub fn lint_string(yaml: &str, rules: Option<&RuleFilter>) -> Result<LintResult, LintError> {
        Self::lint_yaml(yaml, false, rules)
    }

    fn lint_yaml(
        text: &str,
        symbolize_names: bool,
        rules: Option<&RuleFilter>,
    ) -> Result<LintResult, LintError> {
        match load_definition(text, symbolize_names) {
            Ok(data) => Self::lint(&data, rules),
            Err(Loaded::Syntax(message)) => Ok(single_error(format!("Invalid YAML: {message}"))),
            Err(Loaded::Error(e)) => Err(e),
        }
    }
}

enum Loaded {
    Syntax(String),
    Error(LintError),
}

fn load_definition(text: &str, symbolize_names: bool) -> Result<Data, Loaded> {
    let data = match safe_load(text, Permitted::DOCUMENT) {
        Ok(d) => d,
        Err(SchemaError::YamlSyntax { message }) => return Err(Loaded::Syntax(message)),
        Err(e) => return Err(Loaded::Error(e.into())),
    };
    if symbolize_names {
        symbolize(data).map_err(Loaded::Error)
    } else {
        Ok(data)
    }
}

/// `YAML.safe_load(text, permitted_classes: [Symbol, Date, Time],
/// symbolize_names:)` as the linter calls it. A syntax error is
/// `LintError::Raised` with class `Psych::SyntaxError` (and the YAML
/// engine's message).
pub fn safe_load_definition(text: &str, symbolize_names: bool) -> Result<Data, LintError> {
    load_definition(text, symbolize_names).map_err(|e| match e {
        Loaded::Syntax(message) => LintError::raised("Psych::SyntaxError", message),
        Loaded::Error(e) => e,
    })
}

/// Psych `symbolize_names: true`: String hash keys become Symbols while the
/// mapping is built (later duplicates replace earlier ones in place).
fn symbolize(data: Data) -> Result<Data, LintError> {
    Ok(match data {
        Data::Array(items) => {
            Data::Array(items.into_iter().map(symbolize).collect::<Result<_, _>>()?)
        }
        Data::Hash(pairs) => {
            // The loaded hash has already merged duplicate keys, so when a
            // String key and the equal Symbol key both occur, the order of
            // their occurrences (which decides the value) is lost.
            for (k, _) in &pairs {
                if let Data::String(s) = k {
                    if pairs
                        .iter()
                        .any(|(k2, _)| matches!(k2, Data::Symbol(t) if t == s))
                    {
                        return Err(LintError::Unsupported {
                            feature: "symbolize_names key collision".to_owned(),
                            detail: format!(
                                "a mapping has both the String key {s:?} and the Symbol key :{s}"
                            ),
                        });
                    }
                }
            }
            let mut out = Vec::with_capacity(pairs.len());
            for (k, v) in pairs {
                let k = match k {
                    Data::String(s) => Data::Symbol(s),
                    other => other,
                };
                Data::hash_insert(&mut out, k, symbolize(v)?);
            }
            Data::Hash(out)
        }
        other => other,
    })
}

fn single_error(message: String) -> LintResult {
    LintResult::new(vec![LintIssue::error(message, None, None, None)])
}

fn issue(severity: Severity, rule: &Rule, location: String) -> LintIssue {
    LintIssue::new(
        severity,
        rule.message,
        Some(location),
        Some(rule.suggestion.to_owned()),
        Some(rule.rule_id.to_owned()),
    )
}

static NIL: Data = Data::Null;

/// Ruby class name of a value.
fn class_name(d: &Data) -> &'static str {
    match d {
        Data::Null => "NilClass",
        Data::Bool(true) => "TrueClass",
        Data::Bool(false) => "FalseClass",
        Data::Integer(_) => "Integer",
        Data::Float(_) => "Float",
        Data::String(_) => "String",
        Data::Symbol(_) => "Symbol",
        Data::Date { .. } => "Date",
        Data::Time(_) => "Time",
        Data::Array(_) => "Array",
        Data::Hash(_) => "Hash",
    }
}

/// Ruby 4.0 `NoMethodError` message.
fn no_method(method: &str, receiver: &Data) -> LintError {
    let recv = match receiver {
        Data::Null => "nil".to_owned(),
        Data::Bool(b) => b.to_string(),
        other => format!("an instance of {}", class_name(other)),
    };
    LintError::raised(
        "NoMethodError",
        format!("undefined method '{method}' for {recv}"),
    )
}

/// `Hash#[]` lookup by `eql?`.
fn hash_get<'a>(pairs: &'a [(Data, Data)], key: &Data) -> &'a Data {
    pairs
        .iter()
        .find(|(k, _)| k.eql(key))
        .map_or(&NIL, |(_, v)| v)
}

/// `receiver[:key]` with a Symbol key.
fn index_sym<'a>(receiver: &'a Data, key: &str) -> Result<&'a Data, LintError> {
    match receiver {
        Data::Hash(pairs) => Ok(hash_get(pairs, &Data::Symbol(key.to_owned()))),
        Data::Array(_) | Data::String(_) | Data::Integer(_) | Data::Symbol(_) => Err(
            LintError::raised("TypeError", "no implicit conversion of Symbol into Integer"),
        ),
        other => Err(no_method("[]", other)),
    }
}

/// `receiver.each_with_index`: an Array yields its items, a Hash yields
/// `[key, value]` pairs.
fn each_with_index(receiver: &Data) -> Result<Vec<Cow<'_, Data>>, LintError> {
    match receiver {
        Data::Array(items) => Ok(items.iter().map(Cow::Borrowed).collect()),
        Data::Hash(pairs) => Ok(pairs
            .iter()
            .map(|(k, v)| Cow::Owned(Data::Array(vec![k.clone(), v.clone()])))
            .collect()),
        other => Err(no_method("each_with_index", other)),
    }
}

/// `receiver.length` for the Array/Hash that `each_with_index` accepted.
fn length(receiver: &Data) -> usize {
    match receiver {
        Data::Array(items) => items.len(),
        Data::Hash(pairs) => pairs.len(),
        _ => 0,
    }
}

fn ruby_to_s(d: &Data) -> Result<String, LintError> {
    d.ruby_to_s().map_err(LintError::from)
}

fn tool_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Ruby `^`/`$` are line anchors.
    RE.get_or_init(|| Regex::new(r"(?m)^[a-z][a-z0-9_-]*$").expect("valid regex"))
}

fn command_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^[a-z][a-z0-9_]*$").expect("valid regex"))
}

fn suspicious_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Ruby `\s` is ASCII only: [ \t\n\v\f\r].
    RE.get_or_init(|| {
        Regex::new(
            r"(?m)(^|[ \t\n\x0B\x0C\r]|;)[ \t\n\x0B\x0C\r]*(rm[ \t\n\x0B\x0C\r]+-rf|del|format|mkfs)",
        )
        .expect("valid regex")
    })
}

/// `value.match?(pattern)`: defined on String and Symbol only.
fn ruby_match(value: &Data, re: &Regex) -> Result<bool, LintError> {
    match value {
        Data::String(s) | Data::Symbol(s) => Ok(re.is_match(s)),
        other => Err(no_method("match?", other)),
    }
}

fn def_get<'a>(definition: &'a Data, key: &str) -> &'a Data {
    match definition {
        Data::Hash(pairs) => hash_get(pairs, &Data::Symbol(key.to_owned())),
        _ => &NIL,
    }
}

/// `profile[:commands]` as a Hash: `each_key` raises for anything else.
fn command_pairs(commands: &Data) -> Result<&[(Data, Data)], LintError> {
    match commands {
        Data::Hash(pairs) => Ok(pairs),
        other => Err(no_method("each_key", other)),
    }
}

fn check_naming_conventions(
    definition: &Data,
    issues: &mut Vec<LintIssue>,
) -> Result<(), LintError> {
    let name = def_get(definition, "name");
    if name.truthy() && !ruby_match(name, tool_name_re())? {
        issues.push(issue(
            Severity::Warning,
            &rules::TOOL_NAME_FORMAT,
            "name".to_owned(),
        ));
    }

    let profiles = def_get(definition, "profiles");
    // `definition[:profiles]&.each_with_index`: only nil short-circuits.
    if matches!(profiles, Data::Null) {
        return Ok(());
    }
    for (p_idx, profile) in each_with_index(profiles)?.iter().enumerate() {
        let commands = index_sym(profile, "commands")?;
        if !commands.truthy() {
            continue;
        }
        for (cmd_name, _) in command_pairs(commands)? {
            let cmd_name = ruby_to_s(cmd_name)?;
            if command_name_re().is_match(&cmd_name) {
                continue;
            }
            issues.push(issue(
                Severity::Warning,
                &rules::COMMAND_NAME_FORMAT,
                format!("profiles[{p_idx}].commands.{cmd_name}"),
            ));
        }
    }
    Ok(())
}

fn check_completeness(definition: &Data, issues: &mut Vec<LintIssue>) {
    for (key, severity, rule) in [
        ("description", Severity::Info, &rules::MISSING_DESCRIPTION),
        ("homepage", Severity::Info, &rules::MISSING_HOMEPAGE),
        (
            "version_detection",
            Severity::Warning,
            &rules::MISSING_VERSION_DETECTION,
        ),
    ] {
        if !def_get(definition, key).truthy() {
            issues.push(issue(severity, rule, "definition".to_owned()));
        }
    }
}

fn check_security(definition: &Data, issues: &mut Vec<LintIssue>) -> Result<(), LintError> {
    let profiles = def_get(definition, "profiles");
    // `definition[:profiles]&.each_with_index`: only nil short-circuits.
    if matches!(profiles, Data::Null) {
        return Ok(());
    }
    for (p_idx, profile) in each_with_index(profiles)?.iter().enumerate() {
        let commands = index_sym(profile, "commands")?;
        if !commands.truthy() {
            continue;
        }
        // check_naming_conventions already raised for a non-Hash value.
        let pairs = command_pairs(commands)?;
        for (cmd_name, cmd_def) in pairs {
            let subcommand = index_sym(cmd_def, "subcommand")?;
            if !subcommand.truthy() {
                continue;
            }
            let subcommand = ruby_to_s(subcommand)?;
            if !suspicious_re().is_match(&subcommand) {
                continue;
            }
            issues.push(issue(
                Severity::Error,
                &rules::SUSPICIOUS_SUBCOMMAND,
                format!(
                    "profiles[{p_idx}].commands.{}.subcommand",
                    ruby_to_s(cmd_name)?
                ),
            ));
        }

        for (cmd_name, cmd_def) in pairs {
            let arguments = index_sym(cmd_def, "arguments")?;
            if !arguments.truthy() {
                continue;
            }
            for (a_idx, arg) in each_with_index(arguments)?.iter().enumerate() {
                if index_sym(arg, "type")?.truthy() {
                    continue;
                }
                issues.push(issue(
                    Severity::Warning,
                    &rules::UNVALIDATED_USER_INPUT,
                    format!(
                        "profiles[{p_idx}].commands.{}.arguments[{a_idx}]",
                        ruby_to_s(cmd_name)?
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn check_deprecated_patterns(
    definition: &Data,
    issues: &mut Vec<LintIssue>,
) -> Result<(), LintError> {
    // `definition[:schema] || definition['$schema']`
    let mut schema = def_get(definition, "schema");
    if !schema.truthy() {
        schema = match definition {
            Data::Hash(pairs) => hash_get(pairs, &Data::String("$schema".to_owned())),
            _ => &NIL,
        };
    }
    if !schema.truthy() {
        return Ok(());
    }
    let text = ruby_to_s(schema)?;
    let Some(last) = ruby_split(&text, '/').last().copied() else {
        return Err(no_method("gsub", &NIL));
    };
    let version = last.replace('v', "");
    let rule = &rules::OLD_SCHEMA_VERSION;
    if VersionResolver::compare_versions(&version, rule.threshold.unwrap_or_default())
        == Ordering::Less
    {
        issues.push(issue(Severity::Warning, rule, "schema".to_owned()));
    }
    Ok(())
}

fn check_best_practices(definition: &Data, issues: &mut Vec<LintIssue>) {
    let profiles = def_get(definition, "profiles");
    // check_naming_conventions already raised for a profiles value that
    // does not iterate, or a profile that cannot be indexed.
    let Ok(items) = each_with_index(profiles) else {
        return;
    };
    for (p_idx, profile) in items.iter().enumerate() {
        let Ok(name) = index_sym(profile, "name") else {
            continue;
        };
        if !(matches!(name, Data::String(s) if s == "default") && length(profiles) == 1) {
            continue;
        }
        issues.push(issue(
            Severity::Style,
            &rules::REDUNDANT_DEFAULT_PROFILE,
            format!("profiles[{p_idx}]"),
        ));
        if index_sym(profile, "platforms").is_ok_and(Data::truthy) {
            continue;
        }
        issues.push(issue(
            Severity::Warning,
            &rules::MISSING_PLATFORMS,
            format!("profiles[{p_idx}]"),
        ));
    }
}
