//! The part of Thor's command-line handling that decides what a command line
//! means before any command runs: `help VERB`, the `-h`/`--help` switches,
//! unknown and ambiguous commands, unknown switches and wrong argument counts.
//!
//! The oracle is the pinned exe/ukiryu (ukiryu/ukiryu ab6df35, Thor 1.5.0,
//! Ruby 4.0.1). This module models Thor's `dispatch`, `Thor::Options` and the
//! `FriendlyCLI` extension for the cases the oracle captures
//! (`tests/thor_data/cases.json`). Anything it cannot model with certainty
//! (`--`, numeric switches, non-ASCII or odd switch spellings, switches given
//! to a command group) is passed on to the clap grammar unchanged.

use crate::thor_help::{SUB_HELP, VERB_HELP};

/// What to do with a command line.
pub enum Preempt {
    /// Thor's complete answer.
    Output {
        stdout: String,
        stderr: String,
        code: i32,
    },
    /// Hand the (possibly respelled) arguments to the clap grammar.
    Pass(Vec<String>),
}

/// One Thor option of a command.
struct Opt {
    long: &'static str,
    short: Option<char>,
    boolean: bool,
}

const fn string(long: &'static str, short: Option<char>) -> Opt {
    Opt {
        long,
        short,
        boolean: false,
    }
}

const fn boolean(long: &'static str, short: Option<char>) -> Opt {
    Opt {
        long,
        short,
        boolean: true,
    }
}

/// A top-level command: its Thor name, option set and argument counts.
struct Verb {
    name: &'static str,
    opts: &'static [Opt],
    min: usize,
    max: Option<usize>,
}

const REGISTER: Opt = string("register", Some('r'));

const VERBS: &[Verb] = &[
    Verb {
        name: "cache",
        opts: &[],
        min: 0,
        max: None,
    },
    Verb {
        name: "commands",
        opts: &[REGISTER],
        min: 1,
        max: Some(1),
    },
    Verb {
        name: "config",
        opts: &[REGISTER],
        min: 0,
        max: Some(3),
    },
    Verb {
        name: "definitions",
        opts: &[],
        min: 0,
        max: None,
    },
    Verb {
        name: "describe",
        opts: &[REGISTER, string("format", Some('f'))],
        min: 1,
        max: Some(2),
    },
    Verb {
        name: "exec",
        opts: &[
            REGISTER,
            string("definition", Some('d')),
            string("format", Some('f')),
            string("output", Some('o')),
            boolean("dry-run", Some('D')),
            string("shell", Some('s')),
            boolean("stdin", None),
            boolean("raw", None),
        ],
        min: 1,
        max: None,
    },
    Verb {
        name: "extract",
        opts: &[
            string("output", Some('o')),
            string("method", Some('m')),
            boolean("verbose", Some('v')),
        ],
        min: 1,
        max: Some(1),
    },
    Verb {
        name: "help",
        opts: &[],
        min: 0,
        max: Some(2),
    },
    Verb {
        name: "info",
        opts: &[REGISTER, boolean("all", None)],
        min: 0,
        max: Some(1),
    },
    Verb {
        name: "list",
        opts: &[REGISTER],
        min: 0,
        max: Some(0),
    },
    Verb {
        name: "opts",
        opts: &[REGISTER],
        min: 1,
        max: Some(2),
    },
    Verb {
        name: "register",
        opts: &[boolean("force", Some('f')), boolean("verbose", Some('v'))],
        min: 0,
        max: Some(1),
    },
    Verb {
        name: "resolve",
        opts: &[],
        min: 1,
        max: Some(2),
    },
    Verb {
        name: "run_file",
        opts: &[
            REGISTER,
            string("output", Some('o')),
            string("format", Some('f')),
            boolean("dry-run", Some('d')),
            string("shell", None),
        ],
        min: 1,
        max: Some(1),
    },
    Verb {
        name: "system",
        opts: &[REGISTER],
        min: 0,
        max: Some(1),
    },
    Verb {
        name: "tree",
        opts: &[],
        min: 0,
        max: Some(0),
    },
    Verb {
        name: "validate",
        opts: &[],
        min: 0,
        max: None,
    },
    Verb {
        name: "version",
        opts: &[],
        min: 0,
        max: Some(0),
    },
    Verb {
        name: "which",
        opts: &[REGISTER, string("platform", None), string("shell", None)],
        min: 1,
        max: Some(1),
    },
];

/// A subcommand of a command group.
struct Sub {
    name: &'static str,
    /// The argument placeholders of its usage line (`PATH`), or empty.
    usage: &'static str,
    max: usize,
}

struct Group {
    name: &'static str,
    subs: &'static [Sub],
    /// The command Thor runs when the first word names none.
    default: Option<&'static str>,
}

const fn sub(name: &'static str, usage: &'static str) -> Sub {
    Sub {
        name,
        usage,
        max: if usage.is_empty() { 0 } else { 1 },
    }
}

/// Names a group's `help` and `tree` commands add to its own.
const GROUPS: &[Group] = &[
    Group {
        name: "cache",
        subs: &[
            sub("clear", ""),
            sub("help", "[COMMAND]"),
            sub("info", ""),
            sub("stats", ""),
            sub("tree", ""),
        ],
        default: None,
    },
    Group {
        name: "definitions",
        subs: &[
            sub("add", "SOURCE"),
            sub("help", "[COMMAND]"),
            sub("info", "TOOL"),
            sub("list", ""),
            sub("path", ""),
            sub("remove", "TOOL"),
            sub("tree", ""),
        ],
        default: Some("list"),
    },
    Group {
        name: "validate",
        subs: &[
            sub("all", ""),
            sub("file", "PATH"),
            sub("help", "[COMMAND]"),
            sub("string", "YAML"),
            sub("tree", ""),
        ],
        default: None,
    },
];

/// `Thor::HELP_MAPPINGS`: first words that mean `help`.
const HELP_MAPPINGS: [&str; 4] = ["-h", "-?", "--help", "-D"];

/// The result of one pass over a command line.
enum Run {
    Done(Preempt),
    /// `check_unknown_options!` found these switches.
    Unknown(Vec<String>),
}

/// Interprets `args` (the command line without the program name).
pub fn preempt(args: &[String], columns: Option<&str>) -> Preempt {
    // Only printable ASCII is modeled; anything else is the clap grammar's.
    let printable = |a: &String| a.bytes().all(|b| (0x20..0x7f).contains(&b));
    // The Ruby CLI rejects `--version`; the binary keeps it for release checks.
    if args.len() == 1 && args[0] == "--version" {
        return Preempt::Pass(args.to_vec());
    }
    if !args.iter().all(printable) {
        return Preempt::Pass(args.to_vec());
    }
    match run(args, columns) {
        Run::Done(done) => done,
        Run::Unknown(unknown) => {
            // `FriendlyCLI#handle_help_switches`.
            let retry = if args.first().is_some_and(|a| a == "help") && args.len() > 1 {
                Some(vec!["help".to_owned()])
            } else if unknown.iter().any(|u| u == "-h" || u == "--help") {
                let mut retry = vec!["help".to_owned()];
                if let Some(first) = args.iter().find(|a| !unknown.contains(a)) {
                    retry.push(first.clone());
                }
                Some(retry)
            } else {
                None
            };
            match retry.map(|r| run(&r, columns)) {
                Some(Run::Done(done)) => done,
                _ => {
                    let dictionary = switch_names(args);
                    error(unknown_switches(&unknown, &dictionary))
                }
            }
        }
    }
}

/// The long switches of the command `args` names, for `Did you mean?`.
fn switch_names(args: &[String]) -> Vec<String> {
    let name = match args.first() {
        Some(first) if !first.starts_with('-') => match resolve(&verb_names(), first) {
            Resolved::Name(name) => name,
            _ => return Vec::new(),
        },
        _ => "help".to_owned(),
    };
    VERBS
        .iter()
        .find(|v| v.name == name)
        .map(|v| v.opts.iter().map(|o| format!("--{}", o.long)).collect())
        .unwrap_or_default()
}

fn error(message: String) -> Preempt {
    Preempt::Output {
        stdout: String::new(),
        stderr: format!("{message}\n"),
        code: 1,
    }
}

fn output(stdout: String) -> Preempt {
    Preempt::Output {
        stdout,
        stderr: String::new(),
        code: 0,
    }
}

fn verb_names() -> Vec<&'static str> {
    VERBS.iter().map(|v| v.name).collect()
}

fn run(args: &[String], columns: Option<&str>) -> Run {
    let Some(first) = args.first() else {
        return Run::Done(output(crate::dispatch::thor_command_list(columns)));
    };
    // `retrieve_command_name`: a first word that is not a switch, or one of
    // the help mappings, is the command; otherwise `help` runs on all words.
    let (name, rest): (String, &[String]) = if HELP_MAPPINGS.contains(&first.as_str())
        || !first.starts_with('-')
    {
        if HELP_MAPPINGS.contains(&first.as_str()) {
            ("help".to_owned(), &args[1..])
        } else if first == "lint" {
            // The port's own `lint` verb; Thor has none.
            return Run::Done(Preempt::Pass(args.to_vec()));
        } else {
            match resolve(&verb_names(), first) {
                Resolved::Name(name) => (name, &args[1..]),
                Resolved::Ambiguous(names) => return Run::Done(ambiguous(first, &names)),
                Resolved::Missing(_) => return Run::Done(error(undefined(first, &verb_names()))),
            }
        }
    } else {
        ("help".to_owned(), args)
    };
    if GROUPS.iter().any(|g| g.name == name) {
        return Run::Done(group(&name, rest));
    }
    let verb = VERBS
        .iter()
        .find(|v| v.name == name)
        .expect("resolved names are verbs");
    let Some(parsed) = parse(rest, verb.opts) else {
        return Run::Done(Preempt::Pass(args.to_vec()));
    };
    if !parsed.unknown.is_empty() {
        return Run::Unknown(parsed.unknown);
    }
    let count = parsed.positional.len();
    if count == 0 && verb.min > 0 {
        return Run::Done(error(format!(
            "Missing required argument for '{name}'. Check the command syntax with: ukiryu help {name}"
        )));
    }
    if count < verb.min || verb.max.is_some_and(|max| count > max) {
        return Run::Done(error(format!(
            "Invalid number of arguments for '{name}'. Check the command syntax with: ukiryu help {name}"
        )));
    }
    if name == "help" {
        return Run::Done(match parsed.positional.first() {
            None => output(crate::dispatch::thor_command_list(columns)),
            Some(target) => match resolve(&verb_names(), target) {
                Resolved::Name(found) => output(verb_help(&found)),
                Resolved::Ambiguous(names) => ambiguous(target, &names),
                Resolved::Missing(missing) => error(undefined(&missing, &verb_names())),
            },
        });
    }
    // Hand clap the hyphenated spelling the grammar registers.
    let mut pass = vec![name.replace('_', "-")];
    pass.extend(rest.iter().cloned());
    Run::Done(Preempt::Pass(pass))
}

fn verb_help(name: &str) -> String {
    let key = name.replace('_', "-");
    VERB_HELP
        .iter()
        .find(|(verb, _)| *verb == key)
        .map(|(_, text)| (*text).to_owned())
        .expect("every verb has a captured help screen")
}

/// A command group: `cache`, `definitions` or `validate`. Switches other than
/// the help switches are left to the clap grammar.
fn group(name: &str, rest: &[String]) -> Preempt {
    let group = GROUPS
        .iter()
        .find(|g| g.name == name)
        .expect("a known group");
    let pass = || {
        let mut pass = vec![name.to_owned()];
        pass.extend(rest.iter().cloned());
        Preempt::Pass(pass)
    };
    let is_help = |w: &String| w == "-h" || w == "--help";
    if rest.iter().any(|w| w.starts_with('-') && !is_help(w)) {
        return pass();
    }
    let help = rest.iter().any(is_help);
    let words: Vec<&String> = rest.iter().filter(|w| !is_help(w)).collect();
    let names: Vec<&str> = group.subs.iter().map(|s| s.name).collect();
    let group_help = || output(verb_help(name));
    let Some(first) = words.first() else {
        if !help && group.default.is_some() {
            // No words: the default command runs.
            return pass();
        }
        return group_help();
    };
    let found = match resolve(&names, first) {
        Resolved::Name(found) => found,
        Resolved::Ambiguous(found) => return ambiguous(first, &found),
        Resolved::Missing(missing) => match group.default.filter(|_| !help) {
            Some(default) => {
                // The default command runs on every word.
                return sub_arguments(group, default, &words);
            }
            None => return error(undefined(&missing, &names)),
        },
    };
    let words_after: Vec<&String> = words[1..].to_vec();
    if found == "help" {
        if help {
            return pass();
        }
        return match words_after.first() {
            None => group_help(),
            Some(target) => match resolve(&names, target) {
                Resolved::Name(target) => output(sub_help(name, &target)),
                Resolved::Ambiguous(found) => ambiguous(target, &found),
                Resolved::Missing(missing) => error(undefined(&missing, &names)),
            },
        };
    }
    if found == "tree" {
        return pass();
    }
    if help {
        return output(sub_help(name, &found));
    }
    match sub_arguments(group, &found, &words_after) {
        Preempt::Pass(_) => {
            let mut pass = vec![name.to_owned(), found];
            pass.extend(words_after.iter().map(|w| (*w).clone()));
            Preempt::Pass(pass)
        }
        other => other,
    }
}

fn sub_help(group: &str, sub: &str) -> String {
    SUB_HELP
        .iter()
        .find(|(g, s, _)| *g == group && *s == sub)
        .map(|(_, _, text)| (*text).to_owned())
        .expect("every subcommand has a captured help screen")
}

/// The stock Thor argument-count error of a group subcommand, or a pass when
/// the count is right.
fn sub_arguments(group: &Group, sub_name: &str, words: &[&String]) -> Preempt {
    let sub = group
        .subs
        .iter()
        .find(|s| s.name == sub_name)
        .expect("a known subcommand");
    let wanted = usize::from(sub.max > 0);
    if words.len() == wanted {
        return Preempt::Pass(Vec::new());
    }
    let usage = if sub.usage.is_empty() {
        format!("ukiryu {} {}", group.name, sub.name)
    } else {
        format!("ukiryu {} {} {}", group.name, sub.name, sub.usage)
    };
    let called = if words.is_empty() {
        "no arguments".to_owned()
    } else {
        let list: Vec<String> = words.iter().map(|w| inspect(w)).collect();
        format!("arguments [{}]", list.join(", "))
    };
    error(format!(
        "ERROR: \"ukiryu {} {}\" was called with {called}\nUsage: \"{usage}\"",
        group.name, sub.name
    ))
}

enum Resolved {
    Name(String),
    Ambiguous(Vec<String>),
    /// Not a command; the name as Thor reports it (`-` read as `_`).
    Missing(String),
}

/// `Thor.normalize_command_name`: an exact name, or the one command a prefix
/// selects; several matches are ambiguous.
fn resolve(names: &[&str], word: &str) -> Resolved {
    let mut possible: Vec<String> = names
        .iter()
        .filter(|n| n.starts_with(word))
        .map(|n| (*n).to_owned())
        .collect();
    possible.sort();
    if possible.iter().any(|n| n == word) {
        return Resolved::Name(word.to_owned());
    }
    match possible.len() {
        1 => Resolved::Name(possible.remove(0)),
        0 => {
            let normal = word.replace('-', "_");
            if names.contains(&normal.as_str()) {
                Resolved::Name(normal)
            } else {
                Resolved::Missing(normal)
            }
        }
        _ => Resolved::Ambiguous(possible),
    }
}

fn ambiguous(word: &str, names: &[String]) -> Preempt {
    error(format!(
        "Ambiguous command {word} matches [{}]",
        names.join(", ")
    ))
}

fn undefined(name: &str, names: &[&str]) -> String {
    let dictionary: Vec<String> = names.iter().map(|n| (*n).to_owned()).collect();
    let corrections = corrections(&[name.to_owned()], &dictionary);
    format!(
        "Could not find command {}.{}",
        inspect(name),
        did_you_mean(&corrections)
    )
}

fn unknown_switches(unknown: &[String], dictionary: &[String]) -> String {
    let list: Vec<String> = unknown.iter().map(|u| inspect(u)).collect();
    format!(
        "Unknown switches {}{}",
        list.join(", "),
        did_you_mean(&corrections(unknown, dictionary))
    )
}

fn did_you_mean(corrections: &[String]) -> String {
    if corrections.is_empty() {
        String::new()
    } else {
        format!("\nDid you mean?  {}", corrections.join("\n               "))
    }
}

/// Ruby's `String#inspect` for printable ASCII.
fn inspect(text: &str) -> String {
    let mut out = String::from("\"");
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '#' if matches!(chars.get(i + 1), Some('{' | '$' | '@')) => out.push_str("\\#"),
            other => out.push(*other),
        }
    }
    out.push('"');
    out
}

// ------------------------------------------------------------ option parsing

struct Parsed {
    positional: Vec<String>,
    unknown: Vec<String>,
}

enum Word {
    Plain,
    /// `--name` or `--name=value`.
    Long(String, Option<String>),
    /// `-x` or `-x=value`.
    Short(char, Option<String>),
    /// `-xyz`.
    Squeezed(Vec<char>),
    /// Starts with `-` but is no switch spelling.
    Other,
}

fn is_name(text: &str) -> bool {
    !text.is_empty()
        && text.split('-').all(|part| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
}

/// `None` when the word is a spelling the model does not cover.
fn classify(word: &str) -> Option<Word> {
    if !word.starts_with('-') {
        return Some(Word::Plain);
    }
    // `--`, and words with a later `--`, are not modeled.
    if word == "--" || word[1..].contains("--") {
        return None;
    }
    let (head, value) = match word.split_once('=') {
        Some((head, value)) => (head, Some(value.to_owned())),
        None => (word, None),
    };
    if let Some(name) = head.strip_prefix("--") {
        if is_name(name) {
            return Some(Word::Long(head.to_owned(), value));
        }
        return Some(Word::Other);
    }
    let letters: Vec<char> = head[1..].chars().collect();
    if letters.first().is_some_and(char::is_ascii_alphabetic) {
        // `-r5` is a numeric short switch.
        if letters.iter().skip(1).any(char::is_ascii_digit) {
            return None;
        }
        if letters.len() == 1 {
            return Some(Word::Short(letters[0], value));
        }
        if value.is_none() && letters.iter().all(char::is_ascii_alphabetic) {
            return Some(Word::Squeezed(letters));
        }
    }
    Some(Word::Other)
}

/// Whether `word` has a switch spelling (`current_is_switch_formatted?`).
fn switch_formatted(word: &str) -> bool {
    matches!(
        classify(word),
        Some(Word::Long(..) | Word::Short(..) | Word::Squeezed(_))
    )
}

const TRUE_WORDS: [&str; 4] = ["true", "TRUE", "t", "T"];
const FALSE_WORDS: [&str; 4] = ["false", "FALSE", "f", "F"];

/// `Thor::Options.split` and `#parse` for one command. `None` when a word
/// falls outside the model.
fn parse(tokens: &[String], opts: &[Opt]) -> Option<Parsed> {
    let split = tokens
        .iter()
        .position(|t| t.starts_with('-'))
        .unwrap_or(tokens.len());
    let mut positional: Vec<String> = tokens[..split].to_vec();
    let mut pile: std::collections::VecDeque<String> = tokens[split..].iter().cloned().collect();
    let mut extra: Vec<String> = Vec::new();
    while let Some(word) = pile.pop_front() {
        match classify(&word)? {
            Word::Plain | Word::Other => extra.push(word),
            Word::Squeezed(letters) => {
                if letters
                    .iter()
                    .any(|c| opts.iter().any(|o| o.short == Some(*c)))
                {
                    for letter in letters.into_iter().rev() {
                        pile.push_front(format!("-{letter}"));
                    }
                } else {
                    extra.push(word);
                    swallow(&mut pile, &mut extra);
                }
            }
            Word::Long(head, value) => {
                let name = head[2..].replace('_', "-");
                let known = opts.iter().find(|o| o.long == name);
                let (opt, negated) = match known {
                    Some(opt) => (Some(opt), false),
                    None => {
                        let stripped = name
                            .strip_prefix("no-")
                            .or_else(|| name.strip_prefix("skip-"));
                        (
                            stripped.and_then(|s| opts.iter().find(|o| o.long == s)),
                            true,
                        )
                    }
                };
                match opt {
                    None => {
                        extra.push(word);
                        swallow(&mut pile, &mut extra);
                    }
                    Some(opt) => {
                        if !consume(opt, negated, value, &mut pile) {
                            return None;
                        }
                    }
                }
            }
            Word::Short(letter, value) => match opts.iter().find(|o| o.short == Some(letter)) {
                None => {
                    extra.push(word);
                    swallow(&mut pile, &mut extra);
                }
                Some(opt) => {
                    if !consume(opt, false, value, &mut pile) {
                        return None;
                    }
                }
            },
        }
    }
    let unknown: Vec<String> = extra
        .iter()
        .filter(|e| e.starts_with('-'))
        .cloned()
        .collect();
    positional.extend(extra);
    Some(Parsed {
        positional,
        unknown,
    })
}

/// After an unknown switch, the plain words that follow are its extras.
fn swallow(pile: &mut std::collections::VecDeque<String>, extra: &mut Vec<String>) {
    while pile.front().is_some_and(|next| !next.starts_with('-')) {
        extra.extend(pile.pop_front());
    }
}

/// Takes the value of a known switch off the pile; `false` when the form is
/// not modeled.
fn consume(
    opt: &Opt,
    negated: bool,
    value: Option<String>,
    pile: &mut std::collections::VecDeque<String>,
) -> bool {
    if opt.boolean {
        return match value {
            Some(value) => {
                !negated
                    && (TRUE_WORDS.contains(&value.as_str())
                        || FALSE_WORDS.contains(&value.as_str()))
            }
            None => {
                let takes = pile.front().is_some_and(|next| {
                    !next.starts_with('-')
                        && (TRUE_WORDS.contains(&next.as_str())
                            || FALSE_WORDS.contains(&next.as_str()))
                });
                if takes {
                    pile.pop_front();
                }
                true
            }
        };
    }
    if negated {
        return false;
    }
    if value.is_none() {
        // A string switch takes the next word unless that is itself a switch.
        let takes = pile.front().is_some_and(|next| !switch_formatted(next));
        if takes {
            pile.pop_front();
        }
    }
    true
}

// ------------------------------------------------------------ did you mean

fn normalize(text: &str) -> Vec<char> {
    text.to_lowercase().chars().filter(|c| *c != '@').collect()
}

fn jaro(a: &[char], b: &[char]) -> f64 {
    let (first, second) = if a.len() > b.len() { (b, a) } else { (a, b) };
    let (len1, len2) = (first.len(), second.len());
    let range = if len2 > 3 { len2 / 2 - 1 } else { 0 };
    let mut flags1 = vec![false; len1];
    let mut flags2 = vec![false; len2];
    let mut matches = 0.0_f64;
    for i in 0..len1 {
        let last = i + range;
        let mut j = i.saturating_sub(range);
        while j <= last {
            if j < len2 && !flags2[j] && first[i] == second[j] {
                flags2[j] = true;
                flags1[i] = true;
                matches += 1.0;
                break;
            }
            j += 1;
        }
    }
    let mut transpositions = 0.0_f64;
    let mut k = 0;
    for (i, ch) in first.iter().enumerate() {
        if flags1[i] {
            let mut j = k;
            let mut index = k;
            while j < len2 {
                index = j;
                if flags2[j] {
                    k = j + 1;
                    break;
                }
                j += 1;
            }
            if second.get(index) != Some(ch) {
                transpositions += 1.0;
            }
        }
    }
    transpositions = (transpositions / 2.0).floor();
    if matches == 0.0 {
        0.0
    } else {
        (matches / len1 as f64 + matches / len2 as f64 + (matches - transpositions) / matches) / 3.0
    }
}

fn jaro_winkler(a: &[char], b: &[char]) -> f64 {
    let distance = jaro(a, b);
    if distance > 0.7 {
        let mut prefix = 0;
        for c in a {
            if prefix < 4 && b.get(prefix) == Some(c) {
                prefix += 1;
            } else {
                break;
            }
        }
        distance + (prefix as f64 * 0.1 * (1.0 - distance))
    } else {
        distance
    }
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut d: Vec<usize> = (0..=m).collect();
    let mut x = 0;
    for (index, ch) in a.iter().enumerate() {
        let mut i = index + 1;
        for j in 0..m {
            let cost = usize::from(*ch != b[j]);
            let (p, q, r) = (d[j + 1] + 1, i + 1, d[j] + cost);
            x = if p < q && p < r {
                p
            } else if q < r {
                q
            } else {
                r
            };
            d[j] = i;
            i = x;
        }
        d[m] = x;
    }
    x
}

/// `DidYouMean::SpellChecker#correct` over several inputs, as Thor's
/// `SpellChecker#corrections` collects them (`flat_map`, `uniq`, `inspect`).
fn corrections(inputs: &[String], dictionary: &[String]) -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    for input in inputs {
        let wanted = normalize(input);
        let threshold = if wanted.len() > 3 { 0.834 } else { 0.77 };
        let mut words: Vec<&String> = dictionary
            .iter()
            .filter(|w| jaro_winkler(&normalize(w), &wanted) >= threshold)
            .filter(|w| *w != input)
            .collect();
        words.sort_by(|a, b| {
            let da = jaro_winkler(&a.chars().collect::<Vec<_>>(), &wanted);
            let db = jaro_winkler(&b.chars().collect::<Vec<_>>(), &wanted);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });
        words.reverse();
        let limit = (wanted.len() as f64 * 0.25).ceil() as usize;
        let mut found: Vec<&String> = words
            .iter()
            .copied()
            .filter(|w| levenshtein(&normalize(w), &wanted) <= limit)
            .collect();
        if found.is_empty() {
            found = words
                .iter()
                .copied()
                .filter(|w| {
                    let word = normalize(w);
                    levenshtein(&word, &wanted) < wanted.len().min(word.len())
                })
                .take(1)
                .collect();
        }
        for word in found {
            let shown = inspect(word);
            if !all.contains(&shown) {
                all.push(shown);
            }
        }
    }
    all
}
