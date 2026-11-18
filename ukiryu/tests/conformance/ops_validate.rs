//! `validate` kind: `type.validate` through the core's
//! `Ukiryu::Type.validate` port (`ukiryu::types::validate`).
use ukiryu::runtime::Clock;
use ukiryu::types::{validate, Error, ErrorClass, Host, Now, Regexp, StdHost, Value};

use super::codec::{decode, encode};
use super::harness::{Ctx, Outcome, RowError};
use super::json::Oj;

/// The corpus environment (README "Comparison"): the real filesystem, the
/// default regexp engine, and `TZ=UTC` (local zone "UTC", offset 0, the
/// system clock read in UTC), independent of the test process's own `TZ`.
struct CorpusHost;

impl Host for CorpusHost {
    fn file_exists(&self, path: &str) -> bool {
        StdHost.file_exists(path)
    }

    fn regexp_match(&self, regexp: &Regexp, string: &str) -> Option<bool> {
        StdHost.regexp_match(regexp, string)
    }

    fn local_offset(&self, _: i64, _: u32, _: u32, _: u32, _: u32, _: u32) -> Option<i32> {
        Some(0)
    }

    fn local_zone_abbreviation(&self, _: i64, _: u32, _: u32) -> Option<String> {
        Some("UTC".into())
    }

    fn now(&self) -> Option<Now> {
        Some(Now::from_clock(Clock::System { utc_offset_secs: 0 }, 0))
    }
}

fn decode_field(input: &Oj, key: &str) -> Result<Value, RowError> {
    match input.get(key) {
        Some(j) => decode(j).map_err(|e| RowError::harness(format!("{key}: {e}"))),
        None => Ok(Value::Nil),
    }
}

/// The core error as the corpus error shape. `Unsupported` is the core
/// declining to decide, never a Ruby class, so the row fails visibly.
fn row_error(e: Error) -> RowError {
    match e.class {
        ErrorClass::Unsupported => RowError::harness(format!("core Unsupported: {}", e.message)),
        class => RowError::new(class.ruby_name(), e.message),
    }
}

/// `Ukiryu::Type.validate(value, type, options || {})`.
pub fn type_validate(input: &Oj, _ctx: &Ctx) -> Outcome {
    let value = decode_field(input, "value")?;
    let spec = decode_field(input, "type")?;
    let options = match decode_field(input, "options")? {
        Value::Nil | Value::Bool(false) => Value::Hash(Vec::new()),
        other => other,
    };
    validate(&value, &spec, &options, &CorpusHost)
        .map(|v| encode(&v))
        .map_err(row_error)
}
