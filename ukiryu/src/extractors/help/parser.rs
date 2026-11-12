//! Pure parts of `Ukiryu::Extractors::HelpParser` (help_parser.rb): option
//! line recognition, type inference, version extraction, the definition
//! skeleton and the command-not-found heuristic. Nothing here runs a process.

use yeptris::{Mapping, Value};

use super::text;

/// Which of the two source patterns recognised the line. The source builds
/// the option hash in a different key order for each pattern, and the
/// emitted YAML keeps that order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionForm {
    /// `^\s*(-[a-z]),?\s*\[--([a-z-]+)(?:[=\s]+([A-Z_]+))?\]` (keys: name,
    /// description, cli, type; `cli` is the short flag).
    ShortAndLong,
    /// `^\s*\[--([a-z-]+)(?:[=\s]+([A-Z_]+))?\]` (keys: name, cli,
    /// description, type; `cli` is `--long`).
    LongOnly,
}

/// Type inferred for an option, as the strings the source writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionType {
    Boolean,
    File,
    Integer,
    Directory,
    String,
}

impl OptionType {
    pub fn as_str(self) -> &'static str {
        match self {
            OptionType::Boolean => "boolean",
            OptionType::File => "file",
            OptionType::Integer => "integer",
            OptionType::Directory => "directory",
            OptionType::String => "string",
        }
    }
}

/// One option recognised in help text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpOption {
    /// Long option name with `-` replaced by `_`.
    pub name: String,
    /// The short flag (`-v`) or `--long`, depending on [`OptionForm`].
    pub cli: String,
    /// The whole help line, stripped.
    pub description: String,
    pub option_type: OptionType,
    pub form: OptionForm,
}

impl HelpOption {
    /// The option as the source's ordered hash.
    pub fn to_value(&self) -> Value {
        let s = |v: &str| Value::String(v.to_owned());
        let entries = match self.form {
            OptionForm::ShortAndLong => vec![
                (s("name"), s(&self.name)),
                (s("description"), s(&self.description)),
                (s("cli"), s(&self.cli)),
                (s("type"), s(self.option_type.as_str())),
            ],
            OptionForm::LongOnly => vec![
                (s("name"), s(&self.name)),
                (s("cli"), s(&self.cli)),
                (s("description"), s(&self.description)),
                (s("type"), s(self.option_type.as_str())),
            ],
        };
        Value::Mapping(Mapping(entries))
    }
}

/// Source `infer_type`: upcase the parameter placeholder, then test
/// `FILE|PATH`, `NUM|COUNT|LEVEL`, `DIR|FOLDER` in that order.
pub fn infer_type(param: &str) -> OptionType {
    let up = text::upcase(param);
    let has = |words: &[&str]| words.iter().any(|w| up.contains(w));
    if has(&["FILE", "PATH"]) {
        OptionType::File
    } else if has(&["NUM", "COUNT", "LEVEL"]) {
        OptionType::Integer
    } else if has(&["DIR", "FOLDER"]) {
        OptionType::Directory
    } else {
        OptionType::String
    }
}

fn take_while(s: &str, pred: impl Fn(char) -> bool) -> (&str, &str) {
    let end = s.find(|c: char| !pred(c)).unwrap_or(s.len());
    s.split_at(end)
}

/// `--([a-z-]+)(?:[=\s]+([A-Z_]+))?\]` after the literal `[`. Every
/// quantifier here is forced to its greedy extent, so no backtracking is
/// needed to reproduce the regexp.
fn bracket_tail(rest: &str) -> Option<(&str, Option<&str>)> {
    let rest = rest.strip_prefix("--")?;
    let (long, rest) = take_while(rest, |c| c == '-' || text::is_letter_i(c));
    if long.is_empty() {
        return None;
    }
    if rest.starts_with(']') {
        return Some((long, None));
    }
    let (sep, rest) = take_while(rest, |c| c == '=' || text::is_space(c));
    if sep.is_empty() {
        return None;
    }
    let (param, rest) = take_while(rest, |c| c == '_' || text::is_letter_i(c));
    if param.is_empty() || !rest.starts_with(']') {
        return None;
    }
    Some((long, Some(param)))
}

fn option(
    line: &str,
    long: &str,
    param: Option<&str>,
    cli: String,
    form: OptionForm,
) -> HelpOption {
    HelpOption {
        name: long.replace('-', "_"),
        cli,
        description: text::strip(line).to_owned(),
        option_type: param.map_or(OptionType::Boolean, infer_type),
        form,
    }
}

/// Recognise one help line with the source's two patterns, in order.
pub fn parse_help_line(line: &str) -> Option<HelpOption> {
    let (_, rest) = take_while(line, text::is_space);
    // Pattern 1: short flag, optional comma, then the bracketed long option.
    if let Some(after_dash) = rest.strip_prefix('-') {
        if let Some(letter) = after_dash.chars().next().filter(|&c| text::is_letter_i(c)) {
            let short = &rest[..1 + letter.len_utf8()];
            let tail = &after_dash[letter.len_utf8()..];
            let tail = tail.strip_prefix(',').unwrap_or(tail);
            let (_, tail) = take_while(tail, text::is_space);
            if let Some((long, param)) = tail.strip_prefix('[').and_then(bracket_tail) {
                return Some(option(
                    line,
                    long,
                    param,
                    short.to_owned(),
                    OptionForm::ShortAndLong,
                ));
            }
        }
    }
    // Pattern 2: the bracketed long option alone.
    let (long, param) = rest.strip_prefix('[').and_then(bracket_tail)?;
    Some(option(
        line,
        long,
        param,
        format!("--{long}"),
        OptionForm::LongOnly,
    ))
}

/// Source `parse_help_elements`: every recognised line in order, keeping the
/// first option for each name (`uniq { |opt| opt['name'] }`).
pub fn parse_help_options(help_text: &str) -> Vec<HelpOption> {
    let mut out: Vec<HelpOption> = Vec::new();
    for option in help_text.split('\n').filter_map(parse_help_line) {
        if !out.iter().any(|o| o.name == option.name) {
            out.push(option);
        }
    }
    out
}

/// Source `extract_version`'s match: the first `(\d+\.\d+(?:\.\d+)?)` in
/// the `--version` stdout (`\d` is ASCII-only in Ruby).
pub fn extract_version_number(version_text: &str) -> Option<&str> {
    let bytes = version_text.as_bytes();
    let digits_end = |from: usize| {
        bytes[from..]
            .iter()
            .position(|b| !b.is_ascii_digit())
            .map_or(bytes.len(), |n| from + n)
    };
    let dot_digit = |at: usize| {
        bytes.get(at) == Some(&b'.') && bytes.get(at + 1).is_some_and(u8::is_ascii_digit)
    };
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let first_end = digits_end(i);
        if dot_digit(first_end) {
            let mut end = digits_end(first_end + 1);
            if dot_digit(end) {
                end = digits_end(end + 1);
            }
            return Some(&version_text[i..end]);
        }
        i = first_end;
    }
    None
}

/// The definition the source builds before `to_yaml`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelpDefinition {
    /// `@tool_name.to_s`.
    pub name: String,
    /// The detected version, if `--version` produced one.
    pub version: Option<String>,
    /// Options after de-duplication.
    pub options: Vec<HelpOption>,
}

impl HelpDefinition {
    /// Source `parse_help_to_yaml` without the version probe.
    pub fn from_help_text(
        tool_name: &str,
        version: Option<&str>,
        help_text: &str,
    ) -> HelpDefinition {
        HelpDefinition {
            name: tool_name.to_owned(),
            version: version.map(str::to_owned),
            options: parse_help_options(help_text),
        }
    }

    /// `name.capitalize`, with Ruby's case mapping.
    pub fn display_name(&self) -> String {
        text::capitalize(&self.name)
    }

    /// The source's ordered definition hash.
    pub fn to_value(&self) -> Value {
        let s = |v: &str| Value::String(v.to_owned());
        let seq = |items: &[&str]| Value::Sequence(items.iter().map(|v| s(v)).collect());
        let map = |entries: Vec<(&str, Value)>| {
            Value::Mapping(Mapping(
                entries.into_iter().map(|(k, v)| (s(k), v)).collect(),
            ))
        };
        let version = self.version.as_deref().unwrap_or("1.0");
        let command = map(vec![
            ("name", s("default")),
            ("description", s(&format!("Default {} command", self.name))),
            (
                "options",
                Value::Sequence(self.options.iter().map(HelpOption::to_value).collect()),
            ),
        ]);
        let profile = map(vec![
            ("name", s("default")),
            ("platforms", seq(&["macos", "linux", "windows"])),
            ("shells", seq(&["bash", "zsh", "fish", "powershell", "cmd"])),
            ("commands", Value::Sequence(vec![command])),
        ]);
        map(vec![
            ("ukiryu_schema", s("1.0")),
            (
                "$self",
                s(&format!(
                    "https://www.ukiryu.com/register/1.0/{}/{}",
                    self.name, version
                )),
            ),
            ("name", s(&self.name)),
            ("version", s(version)),
            ("display_name", s(&self.display_name())),
            ("profiles", Value::Sequence(vec![profile])),
        ])
    }

    /// The definition as YAML, through the crate's YAML stack. The data
    /// matches the source; the text is yeptris' emitter style, not Psych's.
    pub fn to_yaml(&self) -> Result<String, yeptris::Error> {
        yeptris::to_string(&self.to_value())
    }
}

/// Literal forms of the source's command-not-found patterns (all `/i`).
/// `is not recognized as (?:a |the )?(?:name of a )?cmdlet` is expanded into
/// its six literal alternatives.
const NOT_FOUND_LITERALS: &[&str] = &[
    "command not found",
    "no such file or directory",
    "not found",
    "is not recognized as an internal or external command",
    "is not recognized as an operable program or batch file",
    "is not recognized as cmdlet",
    "is not recognized as name of a cmdlet",
    "is not recognized as a cmdlet",
    "is not recognized as a name of a cmdlet",
    "is not recognized as the cmdlet",
    "is not recognized as the name of a cmdlet",
    "cannot be found",
    "because it does not exist",
    "the system cannot find the file specified",
    "cannot find the file",
    "executable not found",
    "bad command or file name",
];

/// Onigmo's case fold of a non-ASCII character, where it can meet the ASCII
/// pattern literals above (measured: two single-character and nine
/// multi-character folds to ASCII).
fn fold_non_ascii(c: char) -> Option<&'static str> {
    match c {
        '\u{17F}' => Some("s"),
        '\u{212A}' => Some("k"),
        '\u{DF}' | '\u{1E9E}' => Some("ss"),
        '\u{FB00}' => Some("ff"),
        '\u{FB01}' => Some("fi"),
        '\u{FB02}' => Some("fl"),
        '\u{FB03}' => Some("ffi"),
        '\u{FB04}' => Some("ffl"),
        '\u{FB05}' | '\u{FB06}' => Some("st"),
        _ => None,
    }
}

/// Case-insensitive search of a lowercase ASCII literal. A folded character
/// matches only as a whole, so a match never starts or ends inside a
/// ligature.
fn contains_folded(haystack: &str, literal: &str) -> bool {
    haystack.char_indices().any(|(start, _)| {
        let mut rest = literal;
        for c in haystack[start..].chars() {
            let next = if c.is_ascii() {
                let mut buf = [0u8; 1];
                rest.strip_prefix(&*c.to_ascii_lowercase().encode_utf8(&mut buf))
            } else {
                fold_non_ascii(c).and_then(|f| rest.strip_prefix(f))
            };
            match next {
                Some("") => return true,
                Some(r) => rest = r,
                None => return false,
            }
        }
        false
    })
}

/// Source `command_not_found?` decision on one `--help` result: any pattern
/// matches `"#{stdout} #{stderr}"` and that text is not blank.
pub fn is_command_not_found(stdout: &str, stderr: &str) -> bool {
    let combined = format!("{stdout} {stderr}");
    NOT_FOUND_LITERALS
        .iter()
        .any(|literal| contains_folded(&combined, literal))
        && !text::strip(&combined).is_empty()
}
