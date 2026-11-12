//! Oracle tests for `ukiryu::options` against captures of the pinned Ruby
//! `Ukiryu::OptionsBuilder` (ukiryu/ukiryu ab6df353).
//!
//! Golden data: `src/options/testdata/options.json`, produced by running the
//! original Ruby code (capture script recorded in its `provenance`). Class
//! cases parse each YAML with the real `Ukiryu::Models::CommandDefinition`
//! and record the attributes the builder reads, every `Ukiryu::Type.validate`
//! call a setter makes (value, type, definition, result), and the argv handed
//! to the shell's `join`. Coercion results are replayed from the capture: the
//! type rules belong to the types module, the call sequence belongs here.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::Value as J;
use ukiryu::command_builder::{build_args, BuildContext, CommandSpec};
use ukiryu::options::formatter::{self, Formatted};
use ukiryu::options::integration::TypeCoercer;
use ukiryu::options::{
    missing_format, validator, ArgumentDef, Cache, Clock, Coercer, CommandDef, DefRef, FlagDef,
    OptionDef, OptionValue, Options, OptionsClass, OptionsClassCache, OptionsError, Position,
    TypeRef,
};
use ukiryu::shell::ShellFormatting;
use ukiryu::types::{StdHost, Value};

const GOLDEN: &str = include_str!("../src/options/testdata/options.json");

fn golden() -> J {
    serde_json::from_str(GOLDEN).expect("golden json")
}

// ---------------------------------------------------------------- values

#[derive(Debug, Clone, PartialEq)]
enum TV {
    Nil,
    Bool(bool),
    Int(i64),
    Float(String),
    Str(String),
    Sym(String),
    Arr(Vec<TV>),
    Other(String),
}

fn dec(j: &J) -> TV {
    if j.is_null() {
        return TV::Nil;
    }
    let o = j.as_object().expect("tagged value");
    if let Some(b) = o.get("b") {
        TV::Bool(b.as_bool().unwrap())
    } else if let Some(i) = o.get("i") {
        TV::Int(i.as_i64().unwrap())
    } else if o.contains_key("f") {
        TV::Float(o["to_s"].as_str().unwrap().to_owned())
    } else if let Some(s) = o.get("s") {
        TV::Str(s.as_str().unwrap().to_owned())
    } else if let Some(s) = o.get("sym") {
        TV::Sym(s.as_str().unwrap().to_owned())
    } else if let Some(a) = o.get("a") {
        TV::Arr(a.as_array().unwrap().iter().map(dec).collect())
    } else if o.contains_key("other") {
        TV::Other(o["to_s"].as_str().unwrap().to_owned())
    } else {
        panic!("unknown value {j}")
    }
}

fn str_inspect(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '#' => out.push('#'),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl TV {
    fn inspect(&self) -> String {
        match self {
            TV::Nil => "nil".into(),
            TV::Str(s) => str_inspect(s),
            TV::Sym(s) => format!(":{s}"),
            TV::Arr(items) => format!(
                "[{}]",
                items.iter().map(TV::inspect).collect::<Vec<_>>().join(", ")
            ),
            other => other.to_ruby_s(),
        }
    }
}

impl OptionValue for TV {
    fn nil() -> Self {
        TV::Nil
    }
    fn from_bool(value: bool) -> Self {
        TV::Bool(value)
    }
    fn is_nil(&self) -> bool {
        matches!(self, TV::Nil)
    }
    fn truthy(&self) -> bool {
        !matches!(self, TV::Nil | TV::Bool(false))
    }
    fn as_array(&self) -> Option<&[Self]> {
        match self {
            TV::Arr(items) => Some(items),
            _ => None,
        }
    }
    fn to_ruby_s(&self) -> String {
        match self {
            TV::Nil => String::new(),
            TV::Bool(b) => b.to_string(),
            TV::Int(i) => i.to_string(),
            TV::Float(s) | TV::Str(s) | TV::Sym(s) | TV::Other(s) => s.clone(),
            TV::Arr(_) => self.inspect(),
        }
    }
    fn from_array(items: Vec<Self>) -> Self {
        TV::Arr(items)
    }
}

// ----------------------------------------------------------- definitions

#[derive(Debug, Clone)]
enum TRef {
    Str(String),
    Sym(String),
}

impl TRef {
    fn as_ref(&self) -> TypeRef<'_> {
        match self {
            TRef::Str(s) => TypeRef::Str(s),
            TRef::Sym(s) => TypeRef::Sym(s),
        }
    }
}

fn tref(j: &J) -> Option<TRef> {
    let o = j.as_object()?;
    if let Some(s) = o.get("s") {
        Some(TRef::Str(s.as_str().unwrap().to_owned()))
    } else if let Some(s) = o.get("sym") {
        Some(TRef::Sym(s.as_str().unwrap().to_owned()))
    } else if o.contains_key("true_class") {
        // `TrueClass` as a type designator takes the boolean branch.
        Some(TRef::Sym("boolean".into()))
    } else {
        panic!("type ref {j}")
    }
}

fn opt_str(j: &J) -> Option<String> {
    j.as_str().map(str::to_owned)
}

#[derive(Debug)]
enum FormatSrc {
    /// The typed `OptionDefinition` model: no `format` attribute.
    Model,
    Duck(Option<TRef>),
}

#[derive(Debug)]
struct TOpt {
    name: Option<String>,
    cli: Option<String>,
    ty: Option<TRef>,
    format: FormatSrc,
    separator: Option<String>,
}

impl OptionDef for TOpt {
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    fn cli(&self) -> Option<&str> {
        self.cli.as_deref()
    }
    fn type_ref(&self) -> Option<TypeRef<'_>> {
        self.ty.as_ref().map(TRef::as_ref)
    }
    fn format(&self) -> Result<Option<TypeRef<'_>>, OptionsError> {
        match &self.format {
            FormatSrc::Model => Err(missing_format("Ukiryu::Models::OptionDefinition")),
            FormatSrc::Duck(f) => Ok(f.as_ref().map(TRef::as_ref)),
        }
    }
    fn separator(&self) -> Option<&str> {
        self.separator.as_deref()
    }
}

#[derive(Debug)]
enum TPos {
    Nil,
    Int(i64),
    R(TRef),
}

#[derive(Debug)]
struct TArg {
    name: Option<String>,
    ty: Option<TRef>,
    position: TPos,
    variadic: bool,
    min: Option<i64>,
}

impl ArgumentDef for TArg {
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    fn type_ref(&self) -> Option<TypeRef<'_>> {
        self.ty.as_ref().map(TRef::as_ref)
    }
    fn position(&self) -> Position<'_> {
        match &self.position {
            TPos::Nil => Position::Nil,
            TPos::Int(i) => Position::Int(*i),
            TPos::R(TRef::Str(s)) => Position::Str(s),
            TPos::R(TRef::Sym(s)) => Position::Sym(s),
        }
    }
    fn variadic(&self) -> bool {
        self.variadic
    }
    fn min(&self) -> Option<i64> {
        self.min
    }
}

#[derive(Debug)]
struct TFlag {
    name: Option<String>,
    cli: Option<String>,
    default: bool,
}

impl FlagDef for TFlag {
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    fn cli(&self) -> Option<&str> {
        self.cli.as_deref()
    }
    fn default_truthy(&self) -> bool {
        self.default
    }
}

#[derive(Debug)]
struct TCmd {
    subcommand: Option<String>,
    options: Vec<TOpt>,
    flags: Vec<TFlag>,
    arguments: Vec<TArg>,
    post_options: Vec<TOpt>,
}

impl CommandDef for TCmd {
    type Option = TOpt;
    type Argument = TArg;
    type Flag = TFlag;
    fn subcommand(&self) -> Option<&str> {
        self.subcommand.as_deref()
    }
    fn options(&self) -> &[TOpt] {
        &self.options
    }
    fn flags(&self) -> &[TFlag] {
        &self.flags
    }
    fn arguments(&self) -> &[TArg] {
        &self.arguments
    }
    fn post_options(&self) -> &[TOpt] {
        &self.post_options
    }
}

fn model_opt(j: &J) -> TOpt {
    TOpt {
        name: opt_str(&j["name"]),
        cli: opt_str(&j["cli"]),
        ty: tref(&j["type"]),
        format: FormatSrc::Model,
        separator: opt_str(&j["separator"]),
    }
}

fn command(j: &J) -> TCmd {
    let list = |k: &str| j[k].as_array().cloned().unwrap_or_default();
    TCmd {
        subcommand: opt_str(&j["subcommand"]),
        options: list("options").iter().map(model_opt).collect(),
        post_options: list("post_options").iter().map(model_opt).collect(),
        flags: list("flags")
            .iter()
            .map(|f| TFlag {
                name: opt_str(&f["name"]),
                cli: opt_str(&f["cli"]),
                default: f["default"].as_bool().unwrap_or(false),
            })
            .collect(),
        arguments: list("arguments")
            .iter()
            .map(|a| TArg {
                name: opt_str(&a["name"]),
                ty: tref(&a["type"]),
                position: match &a["position"] {
                    J::Null => TPos::Nil,
                    p if p.get("i").is_some() => TPos::Int(p["i"].as_i64().unwrap()),
                    p => TPos::R(tref(p).unwrap()),
                },
                variadic: a["variadic"].as_bool().unwrap_or(false),
                min: a["min"].as_i64(),
            })
            .collect(),
    }
}

// ------------------------------------------------------- coercion replay

struct Replay {
    calls: RefCell<VecDeque<J>>,
    case: String,
}

impl Drop for Replay {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert!(
                self.calls.borrow().is_empty(),
                "{}: unused coerce calls",
                self.case
            );
        }
    }
}

impl Coercer<TCmd, TV> for Replay {
    fn coerce(
        &self,
        value: &TV,
        type_ref: TypeRef<'_>,
        def: DefRef<'_, TCmd>,
    ) -> Result<TV, OptionsError> {
        let call = self
            .calls
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("{}: unexpected coerce of {value:?}", self.case));
        assert_eq!(*value, dec(&call["value"]), "{}: coerce value", self.case);
        let want = tref(&call["type"]).expect("coerce type");
        assert_eq!(type_ref, want.as_ref(), "{}: coerce type", self.case);
        let def_name = match def {
            DefRef::Argument(a) => a.name(),
            DefRef::Option(o) => o.name(),
        };
        assert_eq!(def_name, call["def"].as_str(), "{}: coerce def", self.case);
        match call["result"].get("ok") {
            Some(v) => Ok(dec(v)),
            None => Err(err_of(&call["result"])),
        }
    }
}

fn err_of(j: &J) -> OptionsError {
    let e = &j["error"];
    OptionsError::new(e["class"].as_str().unwrap(), e["message"].as_str().unwrap())
}

// ------------------------------------------------------------ test shell

/// Records the argv `to_shell` joins; `format_path` is visibly marked.
struct RecordingShell;

impl ShellFormatting for RecordingShell {
    fn name(&self) -> &'static str {
        "recording"
    }
    fn escape(&self, value: &str) -> String {
        value.to_owned()
    }
    fn quote(&self, value: &str) -> String {
        value.to_owned()
    }
    fn needs_quoting(&self, _: &str) -> bool {
        false
    }
    fn format_path(&self, path: &str) -> String {
        format!("P({path})")
    }
    fn env_var(&self, name: &str) -> String {
        name.to_owned()
    }
    fn join(&self, executable: &str, arguments: &[String]) -> String {
        std::iter::once(executable.to_owned())
            .chain(arguments.iter().cloned())
            .collect::<Vec<_>>()
            .join("\u{1f}")
    }
}

// ----------------------------------------------------------------- tests

fn expect_result<T: PartialEq + std::fmt::Debug>(
    case: &str,
    what: &str,
    got: Result<T, OptionsError>,
    want: &J,
    ok: impl Fn(&J) -> T,
) {
    match want.get("ok") {
        Some(v) => assert_eq!(got, Ok(ok(v)), "{case}: {what}"),
        None => assert_eq!(got, Err(err_of(want)), "{case}: {what}"),
    }
}

type CoercerFor<'a, C, V> = &'a dyn Fn(&J, &str) -> Box<dyn Coercer<C, V>>;
type BuildArgs<'a, C, V> = &'a dyn Fn(&Options<C, V>) -> Result<Vec<String>, OptionsError>;

/// Replays every class case through one value/definition/coercer stack.
/// Returns (steps run, build_args steps checked).
fn run_class_cases<C: CommandDef, V: OptionValue + PartialEq + std::fmt::Debug>(
    make_def: &dyn Fn(&J) -> C,
    conv: &dyn Fn(&J) -> V,
    coercer_for: CoercerFor<'_, C, V>,
    build_args: Option<BuildArgs<'_, C, V>>,
) -> (usize, usize) {
    let g = golden();
    let cases = g["classes"].as_array().unwrap();
    assert_eq!(cases.len(), 24, "pinned class case count");
    let (mut steps, mut built) = (0, 0);
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let class = OptionsClass::new("tool", "cmd", Arc::new(make_def(case)));
        let class = match (class, case["create"].get("ok")) {
            (Ok(c), Some(_)) => Arc::new(c),
            (Err(e), None) => {
                assert_eq!(e, err_of(&case["create"]), "{id}: create");
                continue;
            }
            (got, _) => panic!("{id}: create mismatch, got ok={}", got.is_ok()),
        };
        let mut obj: Options<C, V> = Options::new(class);
        for st in case["steps"].as_array().unwrap() {
            steps += 1;
            let want = &st["expect"];
            let strings = |v: &J| {
                v.as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            };
            match st["op"].as_str().unwrap() {
                "set" => {
                    let coercer = coercer_for(st, id);
                    let name = st["name"].as_str().unwrap();
                    let got = obj.set(name, conv(&st["value"]), coercer.as_ref());
                    expect_result(id, &format!("set {name}"), got, want, |_| ());
                }
                "get" => {
                    let name = st["name"].as_str().unwrap();
                    expect_result(id, &format!("get {name}"), obj.get(name), want, conv);
                }
                "argv" => expect_result(id, "argv", obj.argv(&RecordingShell), want, strings),
                "bash" => {
                    // Bash quoting belongs to the shell owners; here the join
                    // plumbing (first piece is the executable, empty argv is
                    // an ArgumentError) is checked against the same capture.
                    let got = obj.to_shell(&RecordingShell);
                    match want.get("ok") {
                        Some(_) => {
                            let argv = obj.argv(&RecordingShell).unwrap();
                            assert_eq!(got, Ok(argv.join("\u{1f}")), "{id}: to_shell");
                        }
                        None => assert_eq!(got, Err(err_of(want)), "{id}: to_shell"),
                    }
                }
                "to_hash" => {
                    let got: Result<_, OptionsError> = Ok(obj.to_hash());
                    expect_result(id, "to_hash", got, want, |v| {
                        v.as_array()
                            .unwrap()
                            .iter()
                            .map(|p| (p[0].as_str().unwrap().to_owned(), conv(&p[1])))
                            .collect::<Vec<_>>()
                    });
                }
                "build_args" => {
                    if let Some(build) = build_args {
                        built += 1;
                        expect_result(id, "build_args", build(&obj), want, strings);
                    }
                }
                "validate" => expect_result(id, "validate", obj.validate(), want, |_| ()),
                op => panic!("unknown op {op}"),
            }
        }
    }
    (steps, built)
}

/// Coercion replayed from the capture: checks the exact `Type.validate`
/// calls the setters make.
#[test]
fn class_cases_match_oracle_with_replayed_coercion() {
    let (steps, _) = run_class_cases::<TCmd, TV>(
        &|case| command(&case["definition"]),
        &dec,
        &|st, id| {
            Box::new(Replay {
                calls: RefCell::new(st["coerce"].as_array().unwrap().iter().cloned().collect()),
                case: id.to_owned(),
            })
        },
        None,
    );
    assert_eq!(steps, 150, "pinned class step count");
}

fn core_val(j: &J) -> Value {
    if j.is_null() {
        return Value::Nil;
    }
    let o = j.as_object().expect("tagged value");
    if let Some(b) = o.get("b") {
        Value::Bool(b.as_bool().unwrap())
    } else if let Some(i) = o.get("i") {
        Value::int(i.as_i64().unwrap())
    } else if let Some(f) = o.get("f") {
        Value::Float(f.as_f64().unwrap())
    } else if let Some(s) = o.get("s") {
        Value::str(s.as_str().unwrap())
    } else if let Some(s) = o.get("sym") {
        Value::sym(s.as_str().unwrap())
    } else if let Some(a) = o.get("a") {
        Value::Array(a.as_array().unwrap().iter().map(core_val).collect())
    } else {
        panic!("no core value for {j}")
    }
}

/// The actual consumer stack: definitions as command assembly reads them,
/// `types::Value`, coercion by `types::validate`, and the `to_hash` result
/// handed to `command_builder::build_args` (Ruby `Tool#execute`).
#[test]
fn class_cases_match_oracle_through_core_types() {
    let build = |obj: &Options<CommandSpec, Value>| {
        let spec = obj.class().command_def();
        let ctx = BuildContext {
            shell: &RecordingShell,
            platform: "linux",
            host: &StdHost,
        };
        build_args(spec, &obj.to_params(), &ctx).map_err(OptionsError::from)
    };
    let (steps, built) = run_class_cases::<CommandSpec, Value>(
        &|case| serde_json::from_value(case["spec"].clone()).expect("command spec"),
        &core_val,
        &|_, _| Box::new(TypeCoercer { host: &StdHost }),
        Some(&build),
    );
    assert_eq!(
        (steps, built),
        (150, 10),
        "pinned class/build_args step counts"
    );
}

#[test]
fn formatter_cases_match_oracle() {
    let g = golden();
    let cases = g["formatter"].as_array().unwrap();
    let (mut options, mut flags, mut args) = (0, 0, 0);
    for (i, case) in cases.iter().enumerate() {
        let d = &case["def"];
        let want = &case["expect"];
        let ctx = format!("formatter[{i}] {case}");
        match case["call"].as_str().unwrap() {
            "format_option" => {
                options += 1;
                let def = TOpt {
                    name: Some("o".into()),
                    cli: opt_str(&d["cli"]),
                    ty: tref(&d["type"]),
                    format: FormatSrc::Duck(tref(&d["format"])),
                    separator: opt_str(&d["separator"]),
                };
                let got = formatter::format_option_value(&def, &dec(&case["value"]));
                expect_result(&ctx, "format_option", got, want, |v| match v {
                    J::Null => Formatted::Nil,
                    J::String(s) => Formatted::One(s.clone()),
                    J::Array(a) => Formatted::Pair(
                        a[0].as_str().unwrap().to_owned(),
                        a[1].as_str().unwrap().to_owned(),
                    ),
                    other => panic!("{other}"),
                });
            }
            "format_flag" => {
                flags += 1;
                let def = TFlag {
                    name: Some("f".into()),
                    cli: opt_str(&d["cli"]),
                    default: false,
                };
                let got: Result<_, OptionsError> = Ok(formatter::format_flag(&def));
                expect_result(&ctx, "format_flag", got, want, |v| {
                    v.as_str().unwrap().to_owned()
                });
            }
            "format_arg" => {
                args += 1;
                let ty = tref(&d["type"]);
                let got: Result<_, OptionsError> = Ok(formatter::format_arg(
                    &dec(&case["value"]),
                    ty.as_ref().map(TRef::as_ref),
                    &RecordingShell,
                ));
                expect_result(&ctx, "format_arg", got, want, |v| {
                    v.as_str().unwrap().to_owned()
                });
            }
            other => panic!("unknown call {other}"),
        }
    }
    assert_eq!(
        (options, flags, args),
        (515, 3, 24),
        "pinned formatter counts"
    );
}

#[test]
fn model_option_format_is_ruby_no_method_error() {
    let def = TOpt {
        name: Some("resize".into()),
        cli: Some("--resize".into()),
        ty: Some(TRef::Str("string".into())),
        format: FormatSrc::Model,
        separator: None,
    };
    let err = formatter::format_option(&def, &TV::Str("50x50".into())).unwrap_err();
    assert_eq!(err.class, "NoMethodError");
    // A boolean option never reaches `format`.
    let def = TOpt {
        ty: Some(TRef::Str("boolean".into())),
        ..def
    };
    assert_eq!(
        formatter::format_option(&def, &TV::Bool(true)),
        Ok(vec!["--resize".to_owned()])
    );
}

#[test]
fn validator_message_shapes() {
    assert_eq!(
        validator::validation_failed_message(&["a", "b"]),
        "Validation failed: a, b"
    );
}

struct FakeClock(Mutex<f64>);

impl Clock for FakeClock {
    fn now(&self) -> f64 {
        *self.0.lock().unwrap()
    }
}

#[test]
fn cache_trace_matches_oracle() {
    let g = golden();
    let c = &g["cache"];
    let clock = Arc::new(FakeClock(Mutex::new(0.0)));
    let cache: Cache<String, i64> = Cache::with_clock(
        c["max_size"].as_u64().unwrap() as usize,
        c["ttl"].as_f64(),
        clock.clone(),
    );
    let trace = c["trace"].as_array().unwrap();
    assert_eq!(trace.len(), 26, "pinned cache trace length");
    for (i, st) in trace.iter().enumerate() {
        *clock.0.lock().unwrap() = 1_000_000.0 + st["t"].as_f64().unwrap();
        let key = st["key"].as_str().map(str::to_owned);
        let r = &st["result"];
        let ctx = format!("cache[{i}] {st}");
        match st["op"].as_str().unwrap() {
            "set" => cache.insert(key.unwrap(), st["value"].as_i64().unwrap()),
            "get" => assert_eq!(cache.get(&key.unwrap()), r.as_i64(), "{ctx}"),
            "key" => assert_eq!(
                cache.contains_key(&key.unwrap()),
                r.as_bool().unwrap(),
                "{ctx}"
            ),
            "keys" => {
                let want: Vec<String> = r
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|k| k.as_str().unwrap().to_owned())
                    .collect();
                assert_eq!(cache.keys(), want, "{ctx}");
            }
            "size" => assert_eq!(cache.len() as u64, r.as_u64().unwrap(), "{ctx}"),
            "delete" => assert_eq!(cache.delete(&key.unwrap()), r.as_i64(), "{ctx}"),
            "clear" => cache.clear(),
            op => panic!("unknown cache op {op}"),
        }
    }
    let cfg = &g["options_cache_config"];
    assert_eq!(
        OptionsClassCache::<TCmd>::MAX_SIZE as u64,
        cfg["max_size"].as_u64().unwrap()
    );
    assert_eq!(OptionsClassCache::<TCmd>::TTL, cfg["ttl"].as_f64().unwrap());
}

#[test]
fn options_for_trace_matches_oracle() {
    let g = golden();
    let trace = g["for"].as_array().unwrap();
    let registry: OptionsClassCache<TCmd> = OptionsClassCache::new();
    let calls = RefCell::new(0u64);
    let mut first: Option<Arc<OptionsClass<TCmd>>> = None;
    for (i, st) in trace.iter().enumerate() {
        let (tool, cmd) = (
            st["tool"].as_str().unwrap(),
            st["command"].as_str().unwrap(),
        );
        let got = registry.options_for(tool, cmd, |t, c| {
            *calls.borrow_mut() += 1;
            if t == "missing" {
                return Err(OptionsError::new(
                    "Ukiryu::Errors::ToolNotFoundError",
                    format!("Tool not found: {t}"),
                ));
            }
            Ok((c == "run").then(|| TCmd {
                subcommand: None,
                options: vec![],
                flags: vec![TFlag {
                    name: Some("v".into()),
                    cli: Some("-v".into()),
                    default: false,
                }],
                arguments: vec![],
                post_options: vec![],
            }))
        });
        let r = &st["result"];
        match (got, r.get("ok")) {
            (Ok(class), Some(ok)) => {
                let first = first.get_or_insert_with(|| class.clone());
                assert_eq!(
                    Arc::ptr_eq(first, &class),
                    ok["same_as_first"].as_bool().unwrap(),
                    "for[{i}]"
                );
                assert_eq!(class.tool_name(), ok["tool_name"].as_str().unwrap());
                assert_eq!(class.command_name(), ok["command_name"].as_str().unwrap());
            }
            (Err(e), None) => assert_eq!(e, err_of(r), "for[{i}]"),
            (got, _) => panic!("for[{i}]: mismatch ok={}", got.is_ok()),
        }
        assert_eq!(
            *calls.borrow(),
            st["resolver_calls"].as_u64().unwrap(),
            "for[{i}]"
        );
        if i == 3 {
            registry.clear();
        }
    }
}

#[test]
fn golden_provenance_is_pinned() {
    let g = golden();
    assert_eq!(
        g["provenance"]["sha"].as_str(),
        Some("ab6df353c6dc2d7faf7817a86aafcafa82579896")
    );
}
