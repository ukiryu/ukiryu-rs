//! Shared row-execution types for the conformance consumer.
use std::path::PathBuf;

use super::json::Oj;

/// An op failure in the corpus error shape: the Ruby class name the core
/// maps its error to, and the exact message.
#[derive(Clone, Debug, PartialEq)]
pub struct RowError {
    pub class: String,
    pub message: String,
}

impl RowError {
    pub fn new(class: impl Into<String>, message: impl Into<String>) -> Self {
        RowError {
            class: class.into(),
            message: message.into(),
        }
    }

    /// A consumer-side problem (malformed input, unsupported value). Its
    /// class never appears in the corpus, so the row always fails.
    pub fn harness(message: impl Into<String>) -> Self {
        RowError::new(HARNESS_ERROR, message)
    }
}

pub const HARNESS_ERROR: &str = "Conformance::HarnessError";
pub const UNKNOWN_OP: &str = "Conformance::UnknownOp";

pub type Outcome = Result<Oj, RowError>;

/// What every op sees: where the corpus lives and the sandbox it runs in.
pub struct Ctx {
    /// `UKIRYU_CONFORMANCE_DIR`.
    pub dir: PathBuf,
    /// `<dir>/inputs`, as the oracle's `File.join(corpus_dir, 'inputs')`.
    pub inputs: String,
    /// The empty sandbox home (`${HOME}`).
    pub home: String,
    /// `linux`, `macos` or `windows`.
    pub platform: &'static str,
}

impl Ctx {
    /// `File.join(corpus_dir, 'inputs', rel)`.
    pub fn input_path(&self, rel: &str) -> String {
        format!("{}/{}", self.inputs, rel)
    }

    pub fn read_input(&self, rel: &str) -> Result<String, RowError> {
        std::fs::read_to_string(self.input_path(rel))
            .map_err(|e| RowError::harness(format!("read inputs/{rel}: {e}")))
    }

    /// Host substitutions applied before comparison (README "Comparison").
    pub fn scrub_str(&self, s: &str) -> String {
        s.replace(&self.inputs, "${INPUTS}")
            .replace(&self.home, "${HOME}")
    }

    pub fn scrub(&self, v: Oj) -> Oj {
        match v {
            Oj::Str(s) => Oj::Str(self.scrub_str(&s)),
            Oj::Arr(items) => Oj::Arr(items.into_iter().map(|x| self.scrub(x)).collect()),
            Oj::Obj(pairs) => Oj::Obj(
                pairs
                    .into_iter()
                    .map(|(k, x)| (self.scrub_str(&k), self.scrub(x)))
                    .collect(),
            ),
            other => other,
        }
    }
}

pub fn input_str<'a>(input: &'a Oj, key: &str) -> Result<&'a str, RowError> {
    input.field_str(key).map_err(RowError::harness)
}
