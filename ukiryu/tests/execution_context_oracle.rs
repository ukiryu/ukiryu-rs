//! `Ukiryu::ExecutionContext`, `Ukiryu::Runtime` and the `Shell.detect` /
//! `Platform.detect` paths they reach, against observations captured from
//! the pinned Ruby oracle (ukiryu ab6df353).
//!
//! Rows the decided D2 contract changes (issues #19, #20, #29: exact
//! registered concrete shell names, unknown or missing shells raise, no
//! suffix matching or default shell) keep the frozen Ruby observation and
//! declare their migrated expectation here. A migrated row must really
//! differ from Ruby, and every oracle row needs a declaration.
use serde_json::{json, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use ukiryu::environment::Environment;
use ukiryu::execution_context::{
    ContextChanges, ContextSlot, ExecutionContext, MissingSetting, RuntimeSettings,
};
use ukiryu::extractors::man::Today;
use ukiryu::runtime::{
    civil_from_unix_secs, detect_shell, Clock, Host, Platform, Runtime, RuntimeLocked, ShellError,
    ShellName, ShellSource,
};

const ORACLE: &str = include_str!("../src/environment/testdata/oracle.json");
const PLATFORM: &str = include_str!("../src/environment/testdata/platform.json");
const D2: &str = "D2 (#19/#20/#29)";

fn oracle() -> Value {
    serde_json::from_str(ORACLE).expect("oracle json")
}

fn shell(name: &str) -> ShellName {
    ShellName::lookup(name).unwrap_or_else(|| panic!("concrete shell {name}"))
}

fn platform(name: &str) -> Platform {
    Platform::parse(name).unwrap()
}

#[test]
fn shell_predicates_match_oracle() {
    let o = oracle();
    let rows: Vec<&Value> = o["context"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "shell_predicates")
        .collect();
    assert_eq!(rows.len(), 11);
    for row in rows {
        let name = row["shell"].as_str().unwrap();
        match ShellName::lookup(name) {
            Some(sh) => {
                let ctx = ExecutionContext::new().with_shell(sh);
                assert_eq!(json!(ctx.unix_shell()), row["unix_shell"], "{name}");
                assert_eq!(json!(ctx.windows_shell()), row["windows_shell"], "{name}");
                assert_eq!(json!(ctx.using_shell(sh)), row["using_self"], "{name}");
                assert_eq!(
                    json!(ctx.using_shell(ShellName::Bash)),
                    row["using_bash"],
                    "{name}"
                );
                assert_eq!(row["using_self_string"], json!(true));
            }
            None => {
                // Migrated, D2: Ruby accepts any symbol as a context shell
                // (frozen predicates all false except using_self). The port
                // holds only registered concrete shells.
                assert_eq!(row["unix_shell"], json!(false), "{name} frozen");
                assert_eq!(row["windows_shell"], json!(false), "{name} frozen");
                let err = ShellName::parse(name, ShellSource::Explicit).unwrap_err();
                let expected = match name {
                    "pwsh" => ShellError::Unknown {
                        source: ShellSource::Explicit,
                        value: "pwsh".into(),
                    },
                    "unix" | "windows" => ShellError::PlatformGroup {
                        source: ShellSource::Explicit,
                        value: name.into(),
                    },
                    other => panic!("undeclared {D2} shell row {other}"),
                };
                assert_eq!(err, expected, "{name} migrated {D2}");
            }
        }
    }
}

#[test]
fn platform_predicates_match_oracle() {
    let o = oracle();
    for row in o["context"].as_array().unwrap() {
        if row["kind"] != "platform_predicates" {
            continue;
        }
        let p = platform(row["platform"].as_str().unwrap());
        let ctx = ExecutionContext::new().with_platform(p);
        assert_eq!(json!(ctx.is_windows()), row["windows"]);
        assert_eq!(json!(ctx.is_macos()), row["macos"]);
        assert_eq!(json!(ctx.is_linux()), row["linux"]);
        assert_eq!(json!(ctx.on_platform(p)), row["on_self"]);
    }
}

#[test]
fn defaults_and_rendering_match_oracle() {
    let o = oracle();
    let rows = o["context"].as_array().unwrap();
    let d = rows.iter().find(|r| r["kind"] == "defaults").unwrap();
    let ctx = ExecutionContext::new();
    assert_eq!(json!(ctx.platform().map(Platform::as_str)), d["platform"]);
    assert_eq!(json!(ctx.shell().map(ShellName::as_str)), d["shell"]);
    assert_eq!(json!(ctx.register_path()), d["register_path"]);
    assert_eq!(json!(ctx.timeout_secs()), d["timeout"]);
    assert_eq!(json!(ctx.debug()), d["debug"]);
    assert_eq!(json!(ctx.metrics()), d["metrics"]);
    assert!(ctx.options().is_empty());
    assert_eq!(d["options"], json!({}));
    assert_eq!(json!(ctx.to_string()), d["to_s"]);
    assert_eq!(json!(ctx.inspect()), d["inspect"]);
    assert_eq!(json!(ctx.unix_shell()), d["unix_shell"]);
    assert_eq!(json!(ctx.windows_shell()), d["windows_shell"]);
    assert_eq!(json!(ctx.is_windows()), d["windows"]);

    let mut n = 0;
    for r in rows.iter().filter(|r| r["kind"] == "render") {
        let mut ctx = ExecutionContext::new();
        if let Some(p) = r["platform"].as_str() {
            ctx = ctx.with_platform(platform(p));
        }
        if let Some(s) = r["shell"].as_str() {
            ctx = ctx.with_shell(shell(s));
        }
        if let Some(rp) = r["register_path"].as_str() {
            ctx = ctx.with_register_path(rp);
        }
        assert_eq!(json!(ctx.to_string()), r["to_s"]);
        assert_eq!(json!(ctx.inspect()), r["inspect"]);
        n += 1;
    }
    assert_eq!(n, 5);
}

#[test]
fn initialize_with_values() {
    // spec '#initialize creates a context with specified values'
    let ctx = ExecutionContext::new()
        .with_platform(Platform::Macos)
        .with_shell(ShellName::Bash)
        .with_register_path("/test/register")
        .with_timeout_secs(60)
        .with_debug(true)
        .with_metrics(true);
    assert_eq!(ctx.platform(), Some(Platform::Macos));
    assert_eq!(ctx.shell(), Some(ShellName::Bash));
    assert_eq!(ctx.register_path(), Some("/test/register"));
    assert_eq!(ctx.timeout_secs(), Some(60));
    assert!(ctx.debug() && ctx.metrics());
}

fn opts(v: &Value) -> Vec<(String, Value)> {
    v.as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn opts_json(o: &[(String, Value)]) -> Value {
    Value::Object(o.iter().cloned().collect())
}

#[test]
fn merge_matches_oracle() {
    let o = oracle();
    let base = ExecutionContext::new()
        .with_platform(Platform::Macos)
        .with_shell(ShellName::Bash)
        .with_register_path("/r")
        .with_timeout_secs(30)
        .with_debug(true)
        .with_metrics(false)
        .with_options(vec![("a".into(), json!(1)), ("b".into(), json!(2))]);
    let snapshot = base.clone();
    let mut n = 0;
    for row in o["context"].as_array().unwrap() {
        if row["kind"] != "merge" {
            continue;
        }
        let id = row["id"].as_str().unwrap();
        let ch = &row["changes"];
        let mut changes = ContextChanges::default();
        for (k, v) in ch.as_object().unwrap() {
            match k.as_str() {
                "platform" => changes.platform = Some(v.as_str().map(platform)),
                "shell" => changes.shell = Some(v.as_str().map(shell)),
                "register_path" => changes.register_path = Some(v.as_str().map(str::to_string)),
                "timeout" => changes.timeout_secs = Some(v.as_u64()),
                "debug" => changes.debug = v.as_bool(),
                "metrics" => changes.metrics = v.as_bool(),
                "options" => changes.options = opts(v),
                other => panic!("{id}: change {other}"),
            }
        }
        let m = base.merge(&changes);
        let r = &row["result"];
        assert_eq!(
            json!(m.platform().map(Platform::as_str)),
            r["platform"],
            "{id}"
        );
        assert_eq!(json!(m.shell().map(ShellName::as_str)), r["shell"], "{id}");
        assert_eq!(json!(m.register_path()), r["register_path"], "{id}");
        assert_eq!(json!(m.timeout_secs()), r["timeout"], "{id}");
        assert_eq!(json!(m.debug()), r["debug"], "{id}");
        assert_eq!(json!(m.metrics()), r["metrics"], "{id}");
        assert_eq!(opts_json(m.options()), r["options"], "{id}");
        assert_eq!(base, snapshot, "{id}: receiver unchanged");
        assert_eq!(json!(base.timeout_secs()), row["base_after"]["timeout"]);
        n += 1;
    }
    assert_eq!(n, 6);
}

#[test]
fn context_slot_matches_current_semantics() {
    let o = oracle();
    let c = &o["current"];
    let runtime = Runtime::new(
        Host {
            platform: Platform::Linux,
            env: Environment::from_pairs([("SHELL", "/bin/bash")]),
        },
        None,
        Clock::Fixed(Today {
            year: 2026,
            mon: 11,
            mday: 12,
        }),
    );
    let init = || ExecutionContext::from_runtime(&runtime, RuntimeSettings::default());
    let mut slot = ContextSlot::new();
    let c1 = slot.current_or_init(init).unwrap();
    let c2 = slot.current_or_init(init).unwrap();
    assert_eq!(json!(Arc::ptr_eq(&c1, &c2)), c["same_object_twice"]);
    assert_eq!(c1.platform(), Some(Platform::Linux));
    assert_eq!(c1.shell(), Some(ShellName::Bash));

    let temp = Arc::new(
        ExecutionContext::new()
            .with_platform(Platform::Linux)
            .with_shell(ShellName::Zsh),
    );
    let inside = slot.with_context(Arc::clone(&temp), |s| s.current().unwrap());
    assert_eq!(json!(Arc::ptr_eq(&inside, &temp)), c["with_context_inside"]);
    assert_eq!(
        json!(Arc::ptr_eq(&slot.current().unwrap(), &c1)),
        c["restored"]
    );

    let panicked = catch_unwind(AssertUnwindSafe(|| {
        slot.with_context(Arc::clone(&temp), |_| panic!("test error"))
    }));
    assert!(panicked.is_err());
    assert_eq!(c["error_propagated"], json!("test error"));
    assert_eq!(
        json!(Arc::ptr_eq(&slot.current().unwrap(), &c1)),
        c["restored_after_error"]
    );

    slot.set_current(Arc::clone(&temp));
    assert_eq!(
        json!(Arc::ptr_eq(&slot.current().unwrap(), &temp)),
        c["set_current"]
    );
    slot.reset();
    let fresh = slot.current_or_init(init).unwrap();
    assert_eq!(json!(!Arc::ptr_eq(&fresh, &temp)), c["reset_creates_new"]);
}

#[test]
fn failed_init_stores_nothing() {
    let runtime = Runtime::new(
        Host {
            platform: Platform::Linux,
            env: Environment::new(),
        },
        None,
        Clock::default(),
    );
    let mut slot = ContextSlot::new();
    let err = slot
        .current_or_init(|| ExecutionContext::from_runtime(&runtime, RuntimeSettings::default()))
        .unwrap_err();
    assert_eq!(
        err,
        ShellError::Missing {
            platform: Platform::Linux
        }
    );
    assert!(slot.current().is_none());
}

fn linux_runtime(configured: Option<&str>) -> Runtime {
    Runtime::new(
        Host {
            platform: Platform::Linux,
            env: Environment::from_pairs([("SHELL", "/bin/bash")]),
        },
        configured.map(str::to_string),
        Clock::default(),
    )
}

#[test]
fn runtime_set_lock_reset_match_oracle() {
    let o = oracle();
    let r = &o["runtime"];
    let mut rt = linux_runtime(None);
    rt.set_platform(Platform::Linux).unwrap();
    rt.set_shell(ShellName::Zsh).unwrap();
    let sv = &r["set_values"];
    assert_eq!(json!(rt.platform().as_str()), sv["platform"]);
    assert_eq!(json!(rt.shell().unwrap().as_str()), sv["shell"]);
    assert_eq!(json!(rt.is_linux()), sv["linux"]);
    assert_eq!(json!(rt.unix_shell().unwrap()), sv["unix_shell"]);
    assert_eq!(
        json!(rt.using_shell(ShellName::Zsh).unwrap()),
        sv["using_zsh"]
    );
    rt.lock();
    let e = rt.set_platform(Platform::Macos).unwrap_err();
    assert_eq!(e, RuntimeLocked);
    assert_eq!(json!(e.to_string()), r["locked_platform"]["message"]);
    let e = rt.set_shell(ShellName::Bash).unwrap_err();
    assert_eq!(json!(e.to_string()), r["locked_shell"]["message"]);
    assert_eq!(
        json!(rt.platform().as_str()),
        r["after_lock_values"]["platform"]
    );
    assert_eq!(
        json!(rt.shell().unwrap().as_str()),
        r["after_lock_values"]["shell"]
    );
    rt.reset();
    rt.set_platform(Platform::Windows).unwrap();
    assert_eq!(json!(rt.platform().as_str()), r["reset_unlocks"]);
    rt.reset();
    assert_eq!(rt.platform(), Platform::Linux);
    assert_eq!(rt.shell(), Ok(ShellName::Bash));
}

#[test]
fn runtime_shell_priority_matches_oracle() {
    let o = oracle();
    let rows = o["shell_priority"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        let case = row["case"].as_str().unwrap();
        let ruby = row["result"].as_str().unwrap();
        match case {
            "config_over_detect" => {
                assert_eq!(linux_runtime(Some("fish")).shell().unwrap().as_str(), ruby)
            }
            "explicit_over_config" => {
                let mut rt = linux_runtime(Some("fish"));
                rt.set_shell(ShellName::Tcsh).unwrap();
                assert_eq!(rt.shell().unwrap().as_str(), ruby);
            }
            "config_platform_group" => {
                // Migrated, D2: Ruby keeps `unix` and maps it to Bash; a
                // platform group is not a concrete shell.
                assert_eq!(ruby, "unix");
                assert_eq!(
                    linux_runtime(Some("unix")).shell(),
                    Err(ShellError::PlatformGroup {
                        source: ShellSource::Configured,
                        value: "unix".into()
                    }),
                    "{D2}"
                );
            }
            other => panic!("undeclared priority row {other}"),
        }
    }
    assert_eq!(
        linux_runtime(Some("ksh")).shell(),
        Err(ShellError::Unknown {
            source: ShellSource::Configured,
            value: "ksh".into()
        })
    );
}

/// Declared target for every frozen `Shell.detect` observation.
enum Target {
    Parity,
    Migrated(Result<ShellName, ShellError>),
}

fn unknown(source: ShellSource, value: &str) -> Result<ShellName, ShellError> {
    Err(ShellError::Unknown {
        source,
        value: value.into(),
    })
}

fn detect_target(id: &str, shell_var: Option<&str>) -> Target {
    use ShellSource::*;
    use Target::*;
    let var = || shell_var.unwrap().to_string();
    match id {
        "unix_bash_abs"
        | "unix_bash_bare"
        | "unix_zsh"
        | "unix_fish"
        | "unix_sh"
        | "unix_trailing_slash"
        | "unix_empty"
        | "unix_test_shell_valid"
        | "win_psmodulepath"
        | "win_msystem"
        | "win_mingw_prefix"
        | "win_wsl"
        | "win_empty_psmodulepath" => Parity,
        // Ruby's suffix test maps these to :sh or :bash; exact basename wins.
        "unix_dash" => Migrated(Ok(ShellName::Dash)),
        "unix_tcsh" => Migrated(Ok(ShellName::Tcsh)),
        "unix_csh" | "unix_pwsh" | "unix_ksh_missing" | "unix_custom_exec" | "unix_bash_suffix" => {
            Migrated(unknown(ShellVariable, &var()))
        }
        // Ruby raises after its suffix/basename tests; `powershell` is a
        // registered concrete name.
        "unix_powershell" => Migrated(Ok(ShellName::PowerShell)),
        // Ruby returns the group / silently ignores an invalid override.
        "unix_test_shell_group" => Migrated(Err(ShellError::PlatformGroup {
            source: TestOverride,
            value: "unix".into(),
        })),
        "unix_test_shell_invalid" => Migrated(unknown(TestOverride, "ksh")),
        // Ruby defaults to cmd.
        "win_default" => Migrated(Err(ShellError::Missing {
            platform: Platform::Windows,
        })),
        other => panic!("undeclared detect row {other}"),
    }
}

#[test]
fn shell_detection_matches_oracle_or_declared_d2_migration() {
    let o = oracle();
    let rows = o["detect"].as_array().unwrap();
    assert_eq!(rows.len(), 24);
    let (mut parity, mut migrated) = (0, 0);
    for row in rows {
        let id = row["id"].as_str().unwrap();
        let env_pairs: Vec<(String, String)> = row["env"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p[0].as_str().unwrap().into(), p[1].as_str().unwrap().into()))
            .collect();
        let env = Environment::from_pairs(env_pairs);
        let plat = if row["windows"] == json!(true) {
            Platform::Windows
        } else {
            Platform::Linux
        };
        let got = detect_shell(plat, &env);
        let ruby = &row["ruby"];
        match detect_target(id, env.get("SHELL")) {
            Target::Parity => {
                match ruby.get("ok") {
                    Some(name) => assert_eq!(json!(got.unwrap().as_str()), *name, "{id}"),
                    None => assert_eq!(
                        got.unwrap_err().to_string(),
                        ruby["message_first_line"].as_str().unwrap(),
                        "{id}"
                    ),
                }
                parity += 1;
            }
            Target::Migrated(expected) => {
                assert_eq!(got, expected, "{id}: {D2}");
                let same_as_ruby = match (&got, ruby.get("ok")) {
                    (Ok(s), Some(name)) => json!(s.as_str()) == *name,
                    (Err(_), None) => true,
                    _ => false,
                };
                assert!(!same_as_ruby, "{id}: declared migration equals Ruby");
                migrated += 1;
            }
        }
    }
    assert_eq!((parity, migrated), (13, 11));
}

#[test]
fn shell_detection_without_shell_variable_is_missing() {
    // Ruby scans /etc/shells then PATH (host dependent, not captured);
    // the strict contract raises instead ({D2}).
    for p in [Platform::Linux, Platform::Macos] {
        assert_eq!(
            detect_shell(p, &Environment::new()),
            Err(ShellError::Missing { platform: p })
        );
    }
}

#[test]
fn windows_detection_names_are_case_insensitive() {
    let env = Environment::from_pairs([("psmodulepath", "x")]);
    assert_eq!(
        detect_shell(Platform::Windows, &env),
        Ok(ShellName::PowerShell)
    );
    assert!(detect_shell(Platform::Linux, &env).is_err());
}

#[test]
fn class_for_rows_are_frozen_and_concrete_lookup_is_exact() {
    let o = oracle();
    let cf = o["class_for"].as_object().unwrap();
    assert_eq!(cf.len(), 12);
    for (name, ruby) in cf {
        let ruby = ruby.as_str().unwrap();
        match ShellName::lookup(name) {
            Some(s) => assert!(ruby.starts_with("Ukiryu::Shell::"), "{name}: {s}"),
            None => {
                // unix/windows groups and pwsh resolve to classes in Ruby;
                // migrated ({D2}) to "not a concrete shell". ksh raises in both.
                assert!(
                    matches!(name.as_str(), "unix" | "windows" | "pwsh" | "ksh"),
                    "{name}"
                );
                if name == "ksh" {
                    assert_eq!(
                        ruby,
                        "Ukiryu::Errors::UnknownShellError: Unknown shell: ksh"
                    );
                    assert_eq!(
                        ShellName::parse("ksh", ShellSource::Explicit)
                            .unwrap_err()
                            .to_string(),
                        "Unknown shell: ksh"
                    );
                }
            }
        }
    }
}

#[test]
fn platform_from_host_os_matches_oracle() {
    let p: Value = serde_json::from_str(PLATFORM).unwrap();
    let rows = p["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 13);
    for row in rows {
        let host = row["host_os"].as_str().unwrap();
        match (Platform::from_host_os(host), row["ruby"].get("ok")) {
            (Ok(pl), Some(name)) => assert_eq!(json!(pl.as_str()), *name, "{host}"),
            (Err(e), None) => assert_eq!(
                json!(e.to_string()),
                row["ruby"]["message_first_line"],
                "{host}"
            ),
            (got, _) => panic!("{host}: {got:?} vs {}", row["ruby"]),
        }
    }
    assert!(Platform::native().is_ok());
}

#[test]
fn run_settings_require_shell_and_timeout() {
    // D1 (#20/#29): environment, shell and timeout are explicit; an empty
    // environment is allowed.
    let ctx = ExecutionContext::new();
    let e = ctx.run_settings(Environment::new()).unwrap_err();
    assert_eq!(e, MissingSetting::Shell);
    assert_eq!(
        e.to_string(),
        "shell is required - specify :shell option (e.g., shell: :bash)"
    );
    let ctx = ctx.with_shell(ShellName::Cmd);
    assert_eq!(
        ctx.run_settings(Environment::new())
            .unwrap_err()
            .to_string(),
        "timeout is required - specify :timeout option (e.g., timeout: 90)"
    );
    let s = ctx
        .with_timeout_secs(90)
        .run_settings(Environment::new())
        .unwrap();
    assert!(s.environment.is_empty());
    assert_eq!((s.shell, s.timeout_secs), (ShellName::Cmd, 90));
}

#[test]
fn clock_is_injected() {
    let fixed = Today {
        year: 2024,
        mon: 2,
        mday: 29,
    };
    let ctx = ExecutionContext::new().with_clock(Clock::Fixed(fixed));
    assert_eq!(ctx.today(), fixed);
    let merged = ctx.merge(&ContextChanges::default());
    assert_eq!(merged.today(), fixed);
    let rt = Runtime::new(
        Host {
            platform: Platform::Linux,
            env: Environment::from_pairs([("SHELL", "zsh")]),
        },
        None,
        Clock::Fixed(fixed),
    );
    assert_eq!(rt.today(), fixed);
    let from_rt = ExecutionContext::from_runtime(&rt, RuntimeSettings::default()).unwrap();
    assert_eq!(from_rt.today(), fixed);
    assert_eq!(from_rt.shell(), Some(ShellName::Zsh));

    let day = |y, m, d| Today {
        year: y,
        mon: m,
        mday: d,
    };
    assert_eq!(civil_from_unix_secs(0), day(1970, 1, 1));
    assert_eq!(civil_from_unix_secs(-1), day(1969, 12, 31));
    assert_eq!(civil_from_unix_secs(951_782_400), day(2000, 2, 29));
    assert_eq!(civil_from_unix_secs(1_709_164_800), day(2024, 2, 29));
    assert_eq!(civil_from_unix_secs(4_102_444_799), day(2099, 12, 31));
    // The system clock yields a plausible current day.
    let now = Clock::default().today();
    assert!(now.year >= 2026 && (1..=12).contains(&now.mon));
    let ahead = Clock::System {
        utc_offset_secs: 14 * 3600,
    }
    .today();
    let behind = Clock::System {
        utc_offset_secs: -12 * 3600,
    }
    .today();
    assert!((ahead.year, ahead.mon, ahead.mday) >= (behind.year, behind.mon, behind.mday));
}
