use clap::error::ErrorKind;
use std::ffi::OsString;
use ukiryu_cli::arguments::command;

#[test]
fn all_source_verbs_and_nested_grammars_are_consistent() {
    command().debug_assert();
    for argv in [
        vec!["run-file", "request.yaml", "-d", "--shell", "bash"],
        vec!["exec", "ping", "host=localhost", "count=1", "--stdin"],
        vec!["list", "-r", "register"],
        vec!["info", "ping", "--all"],
        vec!["commands", "ping"],
        vec!["opts", "ping", "default"],
        vec!["describe", "ping", "-f", "json"],
        vec!["system", "shells"],
        vec!["version"],
        vec!["which", "ping", "--platform", "linux"],
        vec!["config", "set", "format", "json"],
        vec!["validate", "file", "profile.yaml", "--strict"],
        vec!["validate", "--format", "json", "all"],
        vec!["validate", "string", "name: ping"],
        vec!["extract", "ping", "-m", "help", "-v"],
        vec!["definitions", "list", "-v"],
        vec!["definitions", "path"],
        vec!["definitions", "add", "profile.yaml", "-V", "1"],
        vec!["definitions", "remove", "ping", "-v", "1", "-f"],
        vec!["definitions", "info", "ping", "-v", "1"],
        vec!["cache", "--verbose", "stats"],
        vec!["cache", "clear", "--dry-run"],
        vec!["resolve", "ping", ">=1"],
        vec!["register", "clone", "-f", "-v"],
    ] {
        let result = command().try_get_matches_from(std::iter::once("ukiryu").chain(argv));
        assert!(result.is_ok(), "{}", result.unwrap_err());
    }
}

#[test]
fn execution_preserves_parameter_order_and_explicit_passthrough_boundary() {
    let matches = command()
        .try_get_matches_from([
            "ukiryu", "exec", "tool", "action", "x=a=b", "-f", "json", "--", "--format", "raw",
            "-D", "", "--",
        ])
        .unwrap();
    let (_, args) = matches.subcommand().unwrap();
    assert_eq!(args.get_one::<OsString>("format"), Some(&"json".into()));
    assert!(!args.get_flag("dry-run"));
    let tokens: Vec<_> = args
        .get_many::<OsString>("arguments")
        .unwrap()
        .cloned()
        .collect();
    assert_eq!(
        tokens,
        ["action", "x=a=b", "--format", "raw", "-D", "", "--"].map(OsString::from)
    );
}

#[test]
fn unknown_flags_and_missing_values_are_errors_before_separator() {
    for argv in [
        vec!["exec", "tool", "--not-an-ukiryu-option"],
        vec!["list", "--raw"],
        vec!["run-file", "a.yaml", "--format"],
        vec!["exec"],
        vec!["definitions", "not-a-command"],
    ] {
        assert!(command()
            .try_get_matches_from(std::iter::once("ukiryu").chain(argv))
            .is_err());
    }
}

#[test]
fn absent_options_do_not_override_configuration_environment_defaults() {
    let matches = command()
        .try_get_matches_from(["ukiryu", "exec", "tool"])
        .unwrap();
    let (_, args) = matches.subcommand().unwrap();
    for name in ["format", "output", "shell", "register", "definition"] {
        assert!(args.get_one::<OsString>(name).is_none());
    }
    assert!(!args.get_flag("raw"));
}

#[test]
fn version_and_help_exit_without_execution() {
    for argv in [
        vec!["--version"],
        vec!["--help"],
        vec!["help", "exec"],
        vec!["exec", "--help"],
    ] {
        let error = command()
            .try_get_matches_from(std::iter::once("ukiryu").chain(argv))
            .unwrap_err();
        assert!(matches!(
            error.kind(),
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
        ));
        assert_eq!(error.exit_code(), 0);
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_paths_are_not_lossily_decoded() {
    use std::os::unix::ffi::OsStringExt;
    let path = OsString::from_vec(vec![b'r', 0xff, b'.', b'y']);
    let matches = command()
        .try_get_matches_from(["ukiryu".into(), "run-file".into(), path.clone()])
        .unwrap();
    assert_eq!(
        matches
            .subcommand()
            .unwrap()
            .1
            .get_one::<OsString>("request-file"),
        Some(&path)
    );
}
