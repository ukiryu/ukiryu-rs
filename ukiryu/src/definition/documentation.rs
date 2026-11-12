//! Human-readable documentation (Markdown, AsciiDoc) for tool definitions.
//!
//! Source: `lib/ukiryu/definition/documentation_generator.rb`
//! (`Ukiryu::Definition::DocumentationGenerator`) at the pinned Ruby oracle.
//!
//! The Ruby generator reads a plain Ruby `Hash` with symbol keys, which is
//! what `YAML.safe_load(yaml, symbolize_names: true)` returns. The port takes
//! the same document as a [`yeptris::Value`]: a string mapping key stands for
//! the Ruby symbol of that name, every other value keeps its YAML type. Values
//! are rendered with Ruby's `to_s`/`inspect`/`Array#join` rules, and
//! malformed documents fail with the Ruby exception class and message
//! (`NoMethodError`, `TypeError`, `ArgumentError`).
//!
//! Output conventions (unchanged from Ruby): lines are joined with `"\n"` on
//! every platform with nothing appended after the last line (a document
//! whose last line is empty therefore ends in `"\n"`), and no Markdown or
//! AsciiDoc escaping is applied to values. [`DocumentationGenerator::generate_to_file`]
//! writes those bytes unchanged (no CRLF translation).
use super::ruby_inspect_str;
use std::cmp::Ordering;
use std::fmt;
use std::path::Path;
use yeptris::{Mapping, Number, Value};

/// `DocumentationGenerator::FORMATS` (symbol names).
pub const FORMATS: [&str; 3] = ["markdown", "asciidoc", "md"];

/// Output format after `normalize_format` (`:md` becomes `:markdown`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Markdown,
    Asciidoc,
}

impl Format {
    /// `normalize_format` followed by the format dispatch: `"md"` and
    /// `"markdown"` select Markdown, `"asciidoc"` AsciiDoc; any other symbol
    /// name raises `ArgumentError, "Unsupported format: #{format}"`.
    pub fn from_symbol(name: &str) -> Result<Format> {
        match name {
            "markdown" | "md" => Ok(Format::Markdown),
            "asciidoc" => Ok(Format::Asciidoc),
            other => Err(DocumentationError::new(
                DocumentationErrorKind::Argument,
                format!("Unsupported format: {other}"),
            )),
        }
    }
}

/// Ruby exception class a [`DocumentationError`] corresponds to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DocumentationErrorKind {
    /// `ArgumentError` (unsupported format, failed `sort` comparison).
    Argument,
    /// `NoMethodError` (method called on a value that lacks it).
    NoMethod,
    /// `TypeError` (`[]` with a symbol on an Array, String or Integer).
    Type,
    /// `SystemCallError` from `File.write`; see [`DocumentationError::ruby_class`].
    Io(IoErrno),
}

/// Errno class of a failed `File.write`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IoErrno {
    /// `Errno::ENOENT`
    NoEntry,
    /// `Errno::EACCES`
    Access,
    /// `Errno::EISDIR`
    IsDirectory,
    /// Any other `SystemCallError`.
    Other,
}

/// An error raised while generating documentation, carrying the Ruby class
/// and the exact Ruby message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentationError {
    pub kind: DocumentationErrorKind,
    pub message: String,
}

impl DocumentationError {
    pub fn new(kind: DocumentationErrorKind, message: impl Into<String>) -> Self {
        DocumentationError {
            kind,
            message: message.into(),
        }
    }

    /// Fully qualified Ruby class name.
    pub fn ruby_class(&self) -> &'static str {
        match self.kind {
            DocumentationErrorKind::Argument => "ArgumentError",
            DocumentationErrorKind::NoMethod => "NoMethodError",
            DocumentationErrorKind::Type => "TypeError",
            DocumentationErrorKind::Io(IoErrno::NoEntry) => "Errno::ENOENT",
            DocumentationErrorKind::Io(IoErrno::Access) => "Errno::EACCES",
            DocumentationErrorKind::Io(IoErrno::IsDirectory) => "Errno::EISDIR",
            DocumentationErrorKind::Io(IoErrno::Other) => "SystemCallError",
        }
    }
}

impl fmt::Display for DocumentationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.ruby_class(), self.message)
    }
}

impl std::error::Error for DocumentationError {}

pub type Result<T> = std::result::Result<T, DocumentationError>;

/// `Ukiryu::Definition::DocumentationGenerator` (class methods only).
pub struct DocumentationGenerator;

impl DocumentationGenerator {
    /// `generate(definition, format:)`; `format` is the Ruby symbol name.
    pub fn generate(definition: &Value, format: &str) -> Result<String> {
        match Format::from_symbol(format)? {
            Format::Markdown => generate_markdown(&val(definition)),
            Format::Asciidoc => generate_asciidoc(&val(definition)),
        }
    }

    /// `generate_to_file(definition, file_path, format:)`: `File.write` of
    /// the generated text, bytes unchanged.
    pub fn generate_to_file(
        definition: &Value,
        path: impl AsRef<Path>,
        format: &str,
    ) -> Result<()> {
        let content = Self::generate(definition, format)?;
        let path = path.as_ref();
        std::fs::write(path, content).map_err(|e| io_error(&e, path))
    }

    /// `generate_command_docs(command_name, command_def, format:)`.
    pub fn generate_command_docs(
        command_name: &str,
        command_def: &Value,
        format: &str,
    ) -> Result<String> {
        let name = Rv::Str(command_name);
        match Format::from_symbol(format)? {
            Format::Markdown => generate_command_markdown(&name, &val(command_def)),
            Format::Asciidoc => generate_command_asciidoc(&name, &val(command_def)),
        }
    }
}

/// Ruby's `"#{strerror} @ rb_sysopen - #{path}"` for `File.write` failures.
fn io_error(e: &std::io::Error, path: &Path) -> DocumentationError {
    let errno = match e.kind() {
        std::io::ErrorKind::NotFound => IoErrno::NoEntry,
        std::io::ErrorKind::PermissionDenied => IoErrno::Access,
        std::io::ErrorKind::IsADirectory => IoErrno::IsDirectory,
        _ => IoErrno::Other,
    };
    let text = e.to_string();
    let text = match text.rfind(" (os error ") {
        Some(i) => text[..i].to_string(),
        None => text,
    };
    DocumentationError::new(
        DocumentationErrorKind::Io(errno),
        format!("{text} @ rb_sysopen - {}", path.display()),
    )
}

// ---------------------------------------------------------------------------
// Generators (line for line with the Ruby methods)
// ---------------------------------------------------------------------------

fn generate_markdown(definition: &Rv<'_>) -> Result<String> {
    let mut output: Vec<String> = Vec::new();
    let name = index(definition, "name")?;
    let tool_name = if name.truthy() {
        name.to_s()
    } else {
        "Unknown".to_string()
    };
    let version = index(definition, "version")?;

    output.push(format!("# {tool_name}"));
    if version.truthy() {
        output.push(String::new());
        output.push(format!("Version: {}", version.to_s()));
    }
    output.push(String::new());

    let description = index(definition, "description")?;
    if description.truthy() {
        output.push(join_element(&description, "\n"));
        output.push(String::new());
    }

    output.push("## Overview".into());
    output.push(String::new());
    output.push("| Property | Value |".into());
    output.push("|----------|-------|".into());
    output.push(format!("| Name | `{tool_name}` |"));
    if version.truthy() {
        output.push(format!("| Version | `{}` |", version.to_s()));
    }
    let homepage = index(definition, "homepage")?;
    if homepage.truthy() {
        output.push(format!("| Homepage | {} |", homepage.to_s()));
    }
    output.push(String::new());

    let platforms = extract_platforms(definition)?;
    if any(&platforms) {
        output.push("### Supported Platforms".into());
        output.push(String::new());
        for platform in &platforms {
            let shells = extract_shells_for_platform(definition, platform)?;
            output.push(format!(
                "- **{}**: {}",
                capitalize(&platform.to_s()),
                join(&shells, ", ")
            ));
        }
        output.push(String::new());
    }

    let install = index(definition, "install")?;
    if install.truthy() {
        output.push("## Installation".into());
        output.push(String::new());
        output.push(render_installation_markdown(&install)?);
        output.push(String::new());
    }

    let commands = extract_commands(definition)?;
    if !commands.is_empty() {
        output.push("## Commands".into());
        output.push(String::new());
        for (cmd_name, cmd_def) in &commands {
            output.push(generate_command_markdown(cmd_name, cmd_def)?);
        }
    }

    if !commands.is_empty() {
        output.push("## Options Reference".into());
        output.push(String::new());
        for (cmd_name, cmd_def) in &commands {
            let options = or_empty(index(cmd_def, "options")?);
            let flags = or_empty(index(cmd_def, "flags")?);

            let has_options = rb_any(&options)?;
            if !has_options && !rb_any(&flags)? {
                continue;
            }

            output.push(format!("### `{}`", cmd_name.to_s()));
            output.push(String::new());

            if has_options {
                output.push("#### Options".into());
                output.push(String::new());
                for opt in each(&options)? {
                    output.push(render_option_markdown(&opt)?);
                }
            }

            if rb_any(&flags)? {
                output.push("#### Flags".into());
                output.push(String::new());
                for flag in each(&flags)? {
                    output.push(render_flag_markdown(&flag)?);
                }
            }

            output.push(String::new());
        }
    }

    Ok(output.join("\n"))
}

fn generate_command_markdown(command_name: &Rv<'_>, command_def: &Rv<'_>) -> Result<String> {
    let mut output: Vec<String> = vec![format!("### `{}`", command_name.to_s()), String::new()];

    let description = index(command_def, "description")?;
    if description.truthy() {
        output.push(join_element(&description, "\n"));
        output.push(String::new());
    }

    let arguments = index(command_def, "arguments")?;
    if arguments.truthy() {
        output.push("#### Arguments".into());
        output.push(String::new());
        for arg in each(&arguments)? {
            output.push(render_argument_markdown(&arg)?);
        }
        output.push(String::new());
    }

    Ok(output.join("\n"))
}

fn render_option_markdown(opt: &Rv<'_>) -> Result<String> {
    let cli = or_else(index(opt, "cli")?, || index(opt, "name"))?;
    let required = if index(opt, "required")?.truthy() {
        "**(required)**"
    } else {
        "(optional)"
    };
    let desc = description_or_empty(opt)?;

    let mut output = format!("- `{}` {required}", cli.to_s());
    output.push_str(&format!(" - {}", desc.to_s()));
    let ty = index(opt, "type")?;
    if ty.truthy() {
        output.push_str(&format!("\n\n  Type: `{}`", ty.to_s()));
    }
    Ok(output)
}

fn render_flag_markdown(flag: &Rv<'_>) -> Result<String> {
    let cli = or_else(index(flag, "cli")?, || index(flag, "name"))?;
    let desc = description_or_empty(flag)?;
    Ok(format!("- `{}` - {}", cli.to_s(), desc.to_s()))
}

fn render_argument_markdown(arg: &Rv<'_>) -> Result<String> {
    let (name, ty, desc, required) = argument_parts(arg, "**required**")?;
    Ok(format!("- **`{name}`** ({ty}, {required}) - {desc}"))
}

/// `name`, `type || 'any'`, `description || ''` and the required marker of
/// an argument, in Ruby evaluation order.
fn argument_parts(arg: &Rv<'_>, required_text: &str) -> Result<(String, String, String, String)> {
    let name = index(arg, "name")?.to_s();
    let ty = index(arg, "type")?;
    let ty = if ty.truthy() {
        ty.to_s()
    } else {
        "any".to_string()
    };
    let desc = description_or_empty(arg)?.to_s();
    let required = if index(arg, "required")?.truthy() {
        required_text.to_string()
    } else {
        "optional".to_string()
    };
    Ok((name, ty, desc, required))
}

fn render_installation_markdown(install: &Rv<'_>) -> Result<String> {
    let mut output: Vec<String> = Vec::new();
    for (key, title, lang) in [
        ("macos", "macOS", "bash"),
        ("linux", "Linux", "bash"),
        ("windows", "Windows", "powershell"),
    ] {
        let script = index(install, key)?;
        if script.truthy() {
            output.push(format!("#### {title}"));
            output.push(format!("```{lang}"));
            output.push(join_element(&script, "\n"));
            output.push("```".into());
            output.push(String::new());
        }
    }
    Ok(output.join("\n"))
}

fn generate_asciidoc(definition: &Rv<'_>) -> Result<String> {
    let mut output: Vec<String> = Vec::new();
    let name = index(definition, "name")?;
    let tool_name = if name.truthy() {
        name.to_s()
    } else {
        "Unknown".to_string()
    };
    let version = index(definition, "version")?;

    output.push(format!("= {tool_name}"));
    if version.truthy() {
        output.push(String::new());
    }
    if index(definition, "profiles")?.truthy() {
        output.push(":toc:".into());
    }
    output.push(String::new());

    if version.truthy() {
        output.push(format!("Version: {}", version.to_s()));
        output.push(String::new());
    }

    let description = index(definition, "description")?;
    if description.truthy() {
        output.push(join_element(&description, "\n"));
        output.push(String::new());
    }

    output.push("== Overview".into());
    output.push(String::new());
    output.push("[cols=\"1,2\"]".into());
    output.push("|===".into());
    output.push("| Property | Value".into());
    output.push(format!("| Name | `{tool_name}`"));
    if version.truthy() {
        output.push(format!("| Version | `{}`", version.to_s()));
    }
    let homepage = index(definition, "homepage")?;
    if homepage.truthy() {
        output.push(format!("| Homepage | {}", homepage.to_s()));
    }
    output.push("|===".into());
    output.push(String::new());

    let platforms = extract_platforms(definition)?;
    if any(&platforms) {
        output.push("=== Supported Platforms".into());
        output.push(String::new());
        for platform in &platforms {
            let shells = extract_shells_for_platform(definition, platform)?;
            output.push(format!(
                "* *{}*: {}",
                capitalize(&platform.to_s()),
                join(&shells, ", ")
            ));
        }
        output.push(String::new());
    }

    let commands = extract_commands(definition)?;
    if !commands.is_empty() {
        output.push("== Commands".into());
        output.push(String::new());
        for (cmd_name, cmd_def) in &commands {
            output.push(generate_command_asciidoc(cmd_name, cmd_def)?);
        }
    }

    Ok(output.join("\n"))
}

fn generate_command_asciidoc(command_name: &Rv<'_>, command_def: &Rv<'_>) -> Result<String> {
    let mut output: Vec<String> = vec![format!("=== `{}`", command_name.to_s()), String::new()];

    let description = index(command_def, "description")?;
    if description.truthy() {
        output.push(join_element(&description, "\n"));
        output.push(String::new());
    }

    let arguments = index(command_def, "arguments")?;
    if arguments.truthy() {
        output.push("==== Arguments".into());
        output.push(String::new());
        for arg in each(&arguments)? {
            let (name, ty, desc, required) = argument_parts(&arg, "*required*")?;
            output.push(format!("* **`{name}`** ({ty}, {required})"));
            output.push(format!(": {desc}"));
        }
        output.push(String::new());
    }

    Ok(output.join("\n"))
}

/// `extract_platforms`: union of every profile's `platforms`, first-seen
/// order, deduplicated with `eql?` (a Ruby `Set`).
fn extract_platforms<'a>(definition: &Rv<'a>) -> Result<Vec<Rv<'a>>> {
    let mut platforms: Vec<Rv<'a>> = Vec::new();
    let profiles = index(definition, "profiles")?;
    if !profiles.truthy() {
        return Ok(platforms);
    }
    for profile in each(&profiles)? {
        let list = index(&profile, "platforms")?;
        if !list.truthy() {
            continue;
        }
        for p in each(&list)? {
            set_add(&mut platforms, p);
        }
    }
    Ok(platforms)
}

/// `extract_shells_for_platform`: union of the `shells` of every profile
/// whose `platforms` include the platform, then `sort`.
fn extract_shells_for_platform<'a>(definition: &Rv<'a>, platform: &Rv<'a>) -> Result<Vec<Rv<'a>>> {
    let mut shells: Vec<Rv<'a>> = Vec::new();
    let profiles = index(definition, "profiles")?;
    if !profiles.truthy() {
        return Ok(shells);
    }
    for profile in each(&profiles)? {
        let list = index(&profile, "platforms")?;
        // `profile[:platforms]&.include?(platform)`: only nil is skipped.
        if matches!(list, Rv::Nil) || !include(&list, platform)? {
            continue;
        }
        let profile_shells = index(&profile, "shells")?;
        if !profile_shells.truthy() {
            continue;
        }
        for s in each(&profile_shells)? {
            set_add(&mut shells, s);
        }
    }
    sort(shells)
}

/// `extract_commands`: first definition of each command name across
/// profiles, in first-seen order (a Ruby Hash keyed with `eql?`).
fn extract_commands<'a>(definition: &Rv<'a>) -> Result<Vec<(Rv<'a>, Rv<'a>)>> {
    let mut commands: Vec<(Rv<'a>, Rv<'a>)> = Vec::new();
    let profiles = index(definition, "profiles")?;
    if !profiles.truthy() {
        return Ok(commands);
    }
    for profile in each(&profiles)? {
        let cmds = index(&profile, "commands")?;
        if !cmds.truthy() {
            continue;
        }
        for (name, cmd_def) in each2(&cmds)? {
            if !commands.iter().any(|(k, _)| eql(k, &name)) {
                commands.push((name, cmd_def));
            }
        }
    }
    Ok(commands)
}

fn description_or_empty<'a>(item: &Rv<'a>) -> Result<Rv<'a>> {
    let desc = index(item, "description")?;
    Ok(if desc.truthy() { desc } else { Rv::Str("") })
}

fn or_empty(v: Rv<'_>) -> Rv<'_> {
    if v.truthy() {
        v
    } else {
        Rv::Seq(&[])
    }
}

fn or_else<'a>(v: Rv<'a>, other: impl FnOnce() -> Result<Rv<'a>>) -> Result<Rv<'a>> {
    if v.truthy() {
        Ok(v)
    } else {
        other()
    }
}

// ---------------------------------------------------------------------------
// Ruby value semantics over YAML values
// ---------------------------------------------------------------------------

/// A YAML value as the Ruby generator sees it.
#[derive(Clone, Debug)]
enum Rv<'a> {
    Nil,
    Bool(bool),
    Int(i128),
    Float(f64),
    Str(&'a str),
    /// A string mapping key (`symbolize_names: true`).
    Sym(&'a str),
    Seq(&'a [Value]),
    Map(&'a Mapping),
    /// The `[key, value]` Array yielded by `Hash#each`.
    Pair(Box<Rv<'a>>, Box<Rv<'a>>),
}

fn val(v: &Value) -> Rv<'_> {
    match v {
        Value::Null => Rv::Nil,
        Value::Bool(b) => Rv::Bool(*b),
        Value::Number(Number::I64(i)) => Rv::Int(i128::from(*i)),
        Value::Number(Number::U64(u)) => Rv::Int(i128::from(*u)),
        Value::Number(Number::F64(f)) => Rv::Float(*f),
        Value::String(s) => Rv::Str(s),
        Value::Sequence(items) => Rv::Seq(items),
        Value::Mapping(m) => Rv::Map(m),
        Value::Tagged(t) => val(&t.value),
    }
}

fn key(v: &Value) -> Rv<'_> {
    match val(v) {
        Rv::Str(s) => Rv::Sym(s),
        other => other,
    }
}

/// Ruby Hash entries: unique keys (`eql?`) in first-seen order, last value.
fn entries(m: &Mapping) -> Vec<(Rv<'_>, Rv<'_>)> {
    let mut out: Vec<(Rv<'_>, Rv<'_>)> = Vec::new();
    for (k, v) in &m.0 {
        let k = key(k);
        let v = val(v);
        match out.iter_mut().find(|(ek, _)| eql(ek, &k)) {
            Some(slot) => slot.1 = v,
            None => out.push((k, v)),
        }
    }
    out
}

impl<'a> Rv<'a> {
    fn truthy(&self) -> bool {
        !matches!(self, Rv::Nil | Rv::Bool(false))
    }

    fn class_name(&self) -> &'static str {
        match self {
            Rv::Nil => "NilClass",
            Rv::Bool(true) => "TrueClass",
            Rv::Bool(false) => "FalseClass",
            Rv::Int(_) => "Integer",
            Rv::Float(_) => "Float",
            Rv::Str(_) => "String",
            Rv::Sym(_) => "Symbol",
            Rv::Seq(_) | Rv::Pair(..) => "Array",
            Rv::Map(_) => "Hash",
        }
    }

    /// Receiver text of Ruby 3.4+ `NoMethodError` messages.
    fn receiver(&self) -> String {
        match self {
            Rv::Nil => "nil".into(),
            Rv::Bool(b) => b.to_string(),
            other => format!("an instance of {}", other.class_name()),
        }
    }

    /// Array elements (`Seq` or `Pair`).
    fn elements(&self) -> Option<Vec<Rv<'a>>> {
        match self {
            &Rv::Seq(items) => Some(items.iter().map(val).collect()),
            Rv::Pair(k, v) => Some(vec![(**k).clone(), (**v).clone()]),
            _ => None,
        }
    }

    fn to_s(&self) -> String {
        match self {
            Rv::Nil => String::new(),
            Rv::Str(s) | Rv::Sym(s) => (*s).to_string(),
            _ => self.inspect(),
        }
    }

    fn inspect(&self) -> String {
        match self {
            Rv::Nil => "nil".into(),
            Rv::Bool(b) => b.to_string(),
            Rv::Int(i) => i.to_string(),
            Rv::Float(f) => ruby_float_to_s(*f),
            Rv::Str(s) => ruby_inspect_str(s),
            Rv::Sym(s) => symbol_inspect(s),
            Rv::Seq(_) | Rv::Pair(..) => {
                let items = self.elements().unwrap_or_default();
                let parts: Vec<String> = items.iter().map(Rv::inspect).collect();
                format!("[{}]", parts.join(", "))
            }
            Rv::Map(m) => {
                let parts: Vec<String> = entries(m)
                    .iter()
                    .map(|(k, v)| match k {
                        Rv::Sym(s) if hash_key_needs_quote(s) => {
                            format!("{}: {}", ruby_inspect_str(s), v.inspect())
                        }
                        Rv::Sym(s) => format!("{s}: {}", v.inspect()),
                        other => format!("{} => {}", other.inspect(), v.inspect()),
                    })
                    .collect();
                if parts.is_empty() {
                    "{}".into()
                } else {
                    format!("{{{}}}", parts.join(", "))
                }
            }
        }
    }
}

fn no_method(method: &str, recv: &Rv<'_>) -> DocumentationError {
    DocumentationError::new(
        DocumentationErrorKind::NoMethod,
        format!("undefined method '{method}' for {}", recv.receiver()),
    )
}

/// `recv[:name]`.
fn index<'a>(recv: &Rv<'a>, name: &str) -> Result<Rv<'a>> {
    match recv {
        &Rv::Map(m) => Ok(m
            .0
            .iter()
            .rev()
            .find(|(k, _)| matches!(key(k), Rv::Sym(s) if s == name))
            .map(|(_, v)| val(v))
            .unwrap_or(Rv::Nil)),
        Rv::Seq(_) | Rv::Pair(..) | Rv::Str(_) | Rv::Sym(_) | Rv::Int(_) => {
            Err(DocumentationError::new(
                DocumentationErrorKind::Type,
                "no implicit conversion of Symbol into Integer",
            ))
        }
        other => Err(no_method("[]", other)),
    }
}

/// `recv.each { |x| }`.
fn each<'a>(recv: &Rv<'a>) -> Result<Vec<Rv<'a>>> {
    match recv {
        &Rv::Map(m) => Ok(entries(m)
            .into_iter()
            .map(|(k, v)| Rv::Pair(Box::new(k), Box::new(v)))
            .collect()),
        other => other.elements().ok_or_else(|| no_method("each", other)),
    }
}

/// `recv.each { |a, b| }`: a Hash yields key and value, an Array element
/// that is itself an Array is splatted, anything else binds `a` only.
fn each2<'a>(recv: &Rv<'a>) -> Result<Vec<(Rv<'a>, Rv<'a>)>> {
    match recv {
        &Rv::Map(m) => Ok(entries(m)),
        other => {
            let items = other.elements().ok_or_else(|| no_method("each", other))?;
            Ok(items
                .into_iter()
                .map(|item| match item.elements() {
                    Some(parts) => {
                        let mut it = parts.into_iter();
                        (it.next().unwrap_or(Rv::Nil), it.next().unwrap_or(Rv::Nil))
                    }
                    None => (item, Rv::Nil),
                })
                .collect())
        }
    }
}

/// `recv.any?` without a block.
fn rb_any(recv: &Rv<'_>) -> Result<bool> {
    match recv {
        Rv::Map(m) => Ok(!m.0.is_empty()),
        other => match other.elements() {
            Some(items) => Ok(any(&items)),
            None => Err(no_method("any?", other)),
        },
    }
}

fn any(items: &[Rv<'_>]) -> bool {
    items.iter().any(Rv::truthy)
}

/// `recv.include?(x)`: `Array` uses `==`, `Hash` checks keys with `eql?`.
fn include(recv: &Rv<'_>, x: &Rv<'_>) -> Result<bool> {
    match recv {
        Rv::Map(m) => Ok(entries(m).iter().any(|(k, _)| eql(k, x))),
        other => match other.elements() {
            Some(items) => Ok(items.iter().any(|e| eq(e, x))),
            None => Err(no_method("include?", other)),
        },
    }
}

fn set_add<'a>(set: &mut Vec<Rv<'a>>, item: Rv<'a>) {
    if !set.iter().any(|e| eql(e, &item)) {
        set.push(item);
    }
}

/// `Array#join(sep)` element rule: Strings as-is, Arrays joined
/// recursively with the same separator, anything else via `to_s`.
fn join_element(item: &Rv<'_>, sep: &str) -> String {
    match item.elements() {
        Some(items) => join(&items, sep),
        None => item.to_s(),
    }
}

fn join(items: &[Rv<'_>], sep: &str) -> String {
    items
        .iter()
        .map(|i| join_element(i, sep))
        .collect::<Vec<_>>()
        .join(sep)
}

/// `==`.
fn eq(a: &Rv<'_>, b: &Rv<'_>) -> bool {
    match (a, b) {
        (Rv::Int(_) | Rv::Float(_), Rv::Int(_) | Rv::Float(_)) => {
            num_cmp(a, b) == Some(Ordering::Equal)
        }
        (Rv::Map(x), Rv::Map(y)) => hash_equal(x, y, eq),
        _ => match (a.elements(), b.elements()) {
            (Some(x), Some(y)) => x.len() == y.len() && x.iter().zip(&y).all(|(p, q)| eq(p, q)),
            (None, None) => scalar_same(a, b),
            _ => false,
        },
    }
}

/// `eql?` (Hash keys, Set members).
fn eql(a: &Rv<'_>, b: &Rv<'_>) -> bool {
    match (a, b) {
        (Rv::Int(x), Rv::Int(y)) => x == y,
        (Rv::Float(x), Rv::Float(y)) => x == y,
        (Rv::Map(x), Rv::Map(y)) => hash_equal(x, y, eql),
        _ => match (a.elements(), b.elements()) {
            (Some(x), Some(y)) => x.len() == y.len() && x.iter().zip(&y).all(|(p, q)| eql(p, q)),
            (None, None) => scalar_same(a, b),
            _ => false,
        },
    }
}

fn scalar_same(a: &Rv<'_>, b: &Rv<'_>) -> bool {
    match (a, b) {
        (Rv::Nil, Rv::Nil) => true,
        (Rv::Bool(x), Rv::Bool(y)) => x == y,
        (Rv::Str(x), Rv::Str(y)) | (Rv::Sym(x), Rv::Sym(y)) => x == y,
        _ => false,
    }
}

fn hash_equal(x: &Mapping, y: &Mapping, value_eq: fn(&Rv<'_>, &Rv<'_>) -> bool) -> bool {
    let (x, y) = (entries(x), entries(y));
    x.len() == y.len()
        && x.iter().all(|(k, v)| {
            y.iter()
                .find(|(k2, _)| eql(k, k2))
                .is_some_and(|(_, v2)| value_eq(v, v2))
        })
}

fn num_cmp(a: &Rv<'_>, b: &Rv<'_>) -> Option<Ordering> {
    match (a, b) {
        (Rv::Int(x), Rv::Int(y)) => Some(x.cmp(y)),
        (Rv::Float(x), Rv::Float(y)) => x.partial_cmp(y),
        (Rv::Int(i), Rv::Float(f)) => int_float_cmp(*i, *f),
        (Rv::Float(f), Rv::Int(i)) => int_float_cmp(*i, *f).map(Ordering::reverse),
        _ => None,
    }
}

/// Exact Integer <=> Float.
fn int_float_cmp(i: i128, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    if f >= 1.0e38 {
        return Some(Ordering::Less);
    }
    if f <= -1.0e38 {
        return Some(Ordering::Greater);
    }
    let floor = f.floor();
    match i.cmp(&(floor as i128)) {
        Ordering::Equal if f > floor => Some(Ordering::Less),
        other => Some(other),
    }
}

/// `<=>` as used by `Array#sort`.
fn cmp(a: &Rv<'_>, b: &Rv<'_>) -> Option<Ordering> {
    match (a, b) {
        (Rv::Int(_) | Rv::Float(_), Rv::Int(_) | Rv::Float(_)) => num_cmp(a, b),
        (Rv::Str(x), Rv::Str(y)) | (Rv::Sym(x), Rv::Sym(y)) => Some(x.as_bytes().cmp(y.as_bytes())),
        _ => match (a.elements(), b.elements()) {
            (Some(x), Some(y)) => {
                for (p, q) in x.iter().zip(&y) {
                    match cmp(p, q)? {
                        Ordering::Equal => {}
                        other => return Some(other),
                    }
                }
                Some(x.len().cmp(&y.len()))
            }
            // Object#<=>: 0 when ==, otherwise nil.
            _ if eq(a, b) => Some(Ordering::Equal),
            _ => None,
        },
    }
}

/// `rb_cmperr`: special constants and Floats are shown with `inspect`,
/// everything else by class.
fn cmp_error(a: &Rv<'_>, b: &Rv<'_>) -> DocumentationError {
    const FIXNUM_MIN: i128 = -(1 << 62);
    const FIXNUM_MAX: i128 = (1 << 62) - 1;
    let shown = match b {
        Rv::Nil | Rv::Bool(_) | Rv::Float(_) => b.inspect(),
        Rv::Int(i) if (FIXNUM_MIN..=FIXNUM_MAX).contains(i) => b.inspect(),
        other => other.class_name().to_string(),
    };
    DocumentationError::new(
        DocumentationErrorKind::Argument,
        format!("comparison of {} with {shown} failed", a.class_name()),
    )
}

/// `Array#sort`. A top-down merge sort: the first failing comparison
/// matches the Ruby oracle for the captured 2-4 element cases; for longer
/// mixed-type lists the reported pair may differ from Ruby's.
fn sort(mut items: Vec<Rv<'_>>) -> Result<Vec<Rv<'_>>> {
    if items.len() <= 1 {
        return Ok(items);
    }
    let right = items.split_off(items.len() / 2);
    let left = sort(items)?;
    let right = sort(right)?;
    let mut out = Vec::with_capacity(left.len() + right.len());
    let mut l = left.into_iter().peekable();
    let mut r = right.into_iter().peekable();
    loop {
        let order = match (l.peek(), r.peek()) {
            (Some(x), Some(y)) => match cmp(x, y) {
                Some(order) => order,
                None => return Err(cmp_error(x, y)),
            },
            _ => break,
        };
        if order == Ordering::Greater {
            out.extend(r.next());
        } else {
            out.extend(l.next());
        }
    }
    out.extend(l);
    out.extend(r);
    Ok(out)
}

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

/// Ruby `String#capitalize`: first character to titlecase (full Unicode
/// mapping), the rest to lowercase per character (no final-sigma rule).
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = titlecase(first);
    for c in chars {
        out.extend(c.to_lowercase());
    }
    out
}

fn titlecase(c: char) -> String {
    let special: Option<&str> = match c {
        'Ǆ' | 'ǅ' | 'ǆ' => Some("ǅ"),
        'Ǉ' | 'ǈ' | 'ǉ' => Some("ǈ"),
        'Ǌ' | 'ǋ' | 'ǌ' => Some("ǋ"),
        'Ǳ' | 'ǲ' | 'ǳ' => Some("ǲ"),
        'ŉ' => Some("\u{2BC}N"),
        '\u{1FB2}' => Some("\u{1FBA}\u{345}"),
        '\u{1FB4}' => Some("\u{386}\u{345}"),
        '\u{1FB7}' => Some("\u{391}\u{342}\u{345}"),
        '\u{1FC2}' => Some("\u{1FCA}\u{345}"),
        '\u{1FC4}' => Some("\u{389}\u{345}"),
        '\u{1FC7}' => Some("\u{397}\u{342}\u{345}"),
        '\u{1FF2}' => Some("\u{1FFA}\u{345}"),
        '\u{1FF4}' => Some("\u{38F}\u{345}"),
        '\u{1FF7}' => Some("\u{3A9}\u{342}\u{345}"),
        _ => None,
    };
    if let Some(s) = special {
        return s.to_string();
    }
    let cp = c as u32;
    let single = match cp {
        // Greek with ypogegrammeni: titlecase is the prosgegrammeni form.
        0x1F80..=0x1F87 | 0x1F90..=0x1F97 | 0x1FA0..=0x1FA7 => char::from_u32(cp + 8),
        0x1F88..=0x1F8F | 0x1F98..=0x1F9F | 0x1FA8..=0x1FAF => Some(c),
        0x1FB3 | 0x1FBC => Some('\u{1FBC}'),
        0x1FC3 | 0x1FCC => Some('\u{1FCC}'),
        0x1FF3 | 0x1FFC => Some('\u{1FFC}'),
        // Georgian Mkhedruli titlecases to itself (uppercase is Mtavruli).
        0x10D0..=0x10FA | 0x10FD..=0x10FF => Some(c),
        _ => None,
    };
    if let Some(t) = single {
        return t.to_string();
    }
    // SpecialCasing titlecase = first uppercase character, rest lowercase
    // (ß -> Ss, ﬁ -> Fi, ﬓ -> Մն, ǰ -> J̌, ...).
    let mut upper = c.to_uppercase();
    let mut out = String::new();
    out.extend(upper.next());
    for u in upper {
        out.extend(u.to_lowercase());
    }
    out
}

/// Identifier-like symbol name (`[A-Za-z_\u0080-][A-Za-z0-9_\u0080-]*`).
fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() || !c.is_ascii() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric() || !c.is_ascii())
}

/// `rb_str_symname_p`: the name can be written as a bare `:symbol`.
fn is_symname(s: &str) -> bool {
    const OPERATORS: [&str; 28] = [
        "|", "^", "&", "<=>", "==", "===", "=~", ">", ">=", "<", "<=", "<<", ">>", "+", "-", "*",
        "/", "%", "**", "~", "+@", "-@", "[]", "[]=", "`", "!", "!=", "!~",
    ];
    if OPERATORS.contains(&s) {
        return true;
    }
    if let Some(rest) = s.strip_prefix("@@") {
        return is_ident(rest);
    }
    if let Some(rest) = s.strip_prefix('@') {
        return is_ident(rest);
    }
    if let Some(rest) = s.strip_prefix('$') {
        let mut chars = rest.chars();
        return match (chars.next(), chars.as_str()) {
            (Some(c), "") if "~*$?!@/\\;,.=:<>\"&`'+0_".contains(c) => true,
            (Some('-'), tail) => tail.chars().count() == 1 && is_ident(tail),
            (Some(c), _) if c.is_ascii_digit() => rest.bytes().all(|b| b.is_ascii_digit()),
            _ => is_ident(rest),
        };
    }
    let base = s
        .strip_suffix('?')
        .or_else(|| s.strip_suffix('!'))
        .or_else(|| s.strip_suffix('='))
        .unwrap_or(s);
    is_ident(base)
}

/// `Symbol#inspect`.
fn symbol_inspect(s: &str) -> String {
    if is_symname(s) {
        format!(":{s}")
    } else {
        format!(":{}", ruby_inspect_str(s))
    }
}

/// Ruby 3.4+ `Hash#inspect`: whether a symbol key needs `"name": value`
/// instead of `name: value`.
fn hash_key_needs_quote(s: &str) -> bool {
    if s.is_empty() || !is_symname(s) {
        return true;
    }
    if matches!(s.chars().next(), Some('@' | '$' | '!')) {
        return true;
    }
    matches!(
        s.chars().last(),
        Some(
            '+' | '-' | '*' | '/' | '`' | '%' | '^' | '&' | '|' | ']' | '<' | '=' | '>' | '~' | '@'
        )
    )
}

/// Ruby `Float#to_s`.
fn ruby_float_to_s(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    let abs = f.abs();
    if abs != 0.0 && !(1e-4..1e16).contains(&abs) {
        let s = format!("{f:e}");
        let (mantissa, exp) = s.split_once('e').unwrap_or((&s, "0"));
        let mantissa = if mantissa.contains('.') {
            mantissa.to_string()
        } else {
            format!("{mantissa}.0")
        };
        let exp: i32 = exp.parse().unwrap_or(0);
        return format!(
            "{mantissa}e{}{:02}",
            if exp < 0 { '-' } else { '+' },
            exp.abs()
        );
    }
    let s = format!("{f}");
    if s.contains('.') {
        s
    } else {
        format!("{s}.0")
    }
}
