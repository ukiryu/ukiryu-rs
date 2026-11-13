//! `ukiryu::register_lookup`: the register scan, `Register.load_tool_metadata`,
//! `Generator.load_tool_definition` and `ToolFinder.find_by`.
//!
//! The scan and the metadata lookup are checked against
//! `routing_oracle/golden.json`, captured from the pinned Ruby source
//! (`ukiryu/ukiryu` at ab6df353): the fixture register is scanned from disk
//! and must give the register the oracle indexed; the synthetic registers are
//! written to disk and must give the oracle's index and warnings.
//! `find_by` has no captured oracle; its tests encode the order and rescue
//! rules read from `lib/ukiryu/tool_finder.rb` and use a stub
//! [`DefinitionSource`] over scratch registers.
//!
//! Scratch directories are created fresh by each test under the system temp
//! directory and removed by the test that created them.

use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use ukiryu::cache::ToolCache;
use ukiryu::executable::{DiscoveryContext, Platform};
use ukiryu::models::{Model, Text, ToolDefinition};
use ukiryu::register::Register;
use ukiryu::register_lookup::{
    find_by, load_tool_definition, load_tool_index, load_tool_metadata, scan_register, LookupError,
};
use ukiryu::routing::{RegisterFile, RegisterImplementation, RegisterTool, ToolIndex};
use ukiryu::runtime::ShellName;
use ukiryu::tool::{self, DefinitionSource, Tool, ToolError, ToolOptions};

fn manifest_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fixture() -> Register {
    Register::at(manifest_root().join("fixtures/register/spec"))
}

fn golden() -> Json {
    serde_json::from_str(include_str!("routing_oracle/golden.json")).unwrap()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Scratch {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "ukiryu-lookup-{label}-{}-{}-{nanos}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir(&dir).expect("scratch directory must be new");
        Scratch(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn register(&self) -> Register {
        Register::at(&self.0)
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn s(v: &Json) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn golden_register(v: &Json) -> Vec<RegisterTool> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|t| RegisterTool {
            name: s(&t["name"]).unwrap(),
            implementations: t["implementations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| RegisterImplementation {
                    name: s(&i["name"]).unwrap(),
                    files: i["files"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|f| RegisterFile {
                            file: s(&f["file"]).unwrap(),
                            document: match f.get("doc") {
                                Some(doc) => Ok(doc.clone()),
                                None => Err(s(&f["parse_error"]["message"]).unwrap()),
                            },
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

/// Writes a golden register as `tools/<tool>/<impl>/<file>`. JSON is YAML, so
/// documents are written as JSON text; a golden parse failure is written as
/// YAML that does not parse.
fn materialize(root: &Path, reg: &Json) {
    for t in reg.as_array().unwrap() {
        for i in t["implementations"].as_array().unwrap() {
            let dir = root
                .join("tools")
                .join(t["name"].as_str().unwrap())
                .join(i["name"].as_str().unwrap());
            fs::create_dir_all(&dir).unwrap();
            for f in i["files"].as_array().unwrap() {
                let text = match f.get("doc") {
                    Some(doc) => doc.to_string(),
                    None => "key: [unclosed\n".to_string(),
                };
                fs::write(dir.join(f["file"].as_str().unwrap()), text).unwrap();
            }
        }
    }
}

fn assert_same_register(scanned: &[RegisterTool], expected: &[RegisterTool]) {
    let shape = |r: &[RegisterTool]| -> Vec<(String, Vec<(String, Vec<String>)>)> {
        r.iter()
            .map(|t| {
                (
                    t.name.clone(),
                    t.implementations
                        .iter()
                        .map(|i| {
                            (
                                i.name.clone(),
                                i.files.iter().map(|f| f.file.clone()).collect(),
                            )
                        })
                        .collect(),
                )
            })
            .collect()
    };
    assert_eq!(shape(scanned), shape(expected));
    for (a, b) in scanned.iter().zip(expected) {
        for (ia, ib) in a.implementations.iter().zip(&b.implementations) {
            for (fa, fb) in ia.files.iter().zip(&ib.files) {
                let at = format!("{}/{}/{}", a.name, ia.name, fa.file);
                match (&fa.document, &fb.document) {
                    (Ok(x), Ok(y)) => assert_eq!(x, y, "{at}"),
                    (Err(_), Err(_)) => {}
                    other => panic!("{at}: {other:?}"),
                }
            }
        }
    }
}

fn rows(table: &[(String, Vec<String>)]) -> Json {
    Json::Array(table.iter().map(|(k, v)| json!([k, v])).collect())
}

fn warnings(index: &ToolIndex) -> Vec<(String, &'static str)> {
    index
        .warnings()
        .iter()
        .map(|w| (w.tool.clone(), w.fault.class))
        .collect()
}

#[test]
fn fixture_register_scans_to_the_register_ruby_indexed() {
    let g = golden();
    let expected = golden_register(&g["index_fixture"]["register"]);
    assert_eq!(expected.len(), 21);
    let scanned = scan_register(&fixture());
    assert_same_register(&scanned, &expected);

    let index = load_tool_index(&fixture());
    assert!(index.warnings().is_empty());
    assert_eq!(rows(index.all_tools()), g["index_fixture"]["all_tools"]);
    assert_eq!(rows(index.aliases()), g["index_fixture"]["aliases"]);
}

#[test]
fn synthetic_registers_on_disk_give_the_ruby_index_and_warnings() {
    let g = golden();
    let cases = g["index_synthetic"].as_array().unwrap();
    assert_eq!(cases.len(), 3);
    for case in cases {
        let label = case["label"].as_str().unwrap();
        let dir = Scratch::new(label);
        materialize(dir.path(), &case["register"]);
        let scanned = scan_register(&dir.register());
        let expected = golden_register(&case["register"]);
        assert_same_register(&scanned, &expected);

        let from_disk = load_tool_index(&dir.register());
        let in_memory = ToolIndex::build(expected);
        assert_eq!(
            rows(from_disk.all_tools()),
            rows(in_memory.all_tools()),
            "{label}"
        );
        assert_eq!(
            rows(from_disk.aliases()),
            rows(in_memory.aliases()),
            "{label}"
        );
        assert_eq!(warnings(&from_disk), warnings(&in_memory), "{label}");
    }
    // The shapes register is the one the Ruby run warned about.
    let shapes = Scratch::new("shapes-warn");
    materialize(shapes.path(), &cases[1]["register"]);
    let index = load_tool_index(&shapes.register());
    let warned: Vec<String> = warnings(&index).into_iter().map(|w| w.0).collect();
    assert_eq!(warned, ["eps", "eta", "kappa", "mu", "xi", "zeta"]);
    // Partial index: tools scanned before the failing one stay indexed.
    assert_eq!(index.find_all_by_interface("ife1"), ["eps"]);
    assert_eq!(index.find_all_by_interface("ifh"), ["eta"]);
    assert!(index.find_all_by_interface("ifh2").is_empty());
}

#[test]
fn scan_is_sorted_nested_and_skips_what_the_glob_skips() {
    let dir = Scratch::new("sorted");
    dir.write("tools/b2/z/b.yaml", "implements: b2\n");
    dir.write("tools/b2/z/a.yaml", "implements: b2\n");
    dir.write("tools/b2/a/1.yaml", "implements: b2\n");
    dir.write("tools/b2/z/notes.txt", "not yaml\n");
    dir.write("tools/b2/z/c.yml", "not yaml suffix\n");
    dir.write("tools/b2/z/.hidden.yaml", "implements: hidden\n");
    dir.write("tools/b2/index.yaml", "interface: b2\n");
    dir.write("tools/a10/default/1.yaml", "implements: a10\n");
    dir.write("tools/a2/default/1.yaml", "implements: a2\n");
    dir.write("tools/.hidden/default/1.yaml", "implements: hidden\n");
    dir.write("tools/stray.yaml", "implements: stray\n");
    fs::create_dir_all(dir.path().join("tools/b2/z/d.yaml")).unwrap();

    let scanned = scan_register(&dir.register());
    let names: Vec<&str> = scanned.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["a10", "a2", "b2"]);
    let b2 = &scanned[2];
    let impls: Vec<&str> = b2.implementations.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(impls, ["a", "z"]);
    let files: Vec<&str> = b2.implementations[1]
        .files
        .iter()
        .map(|f| f.file.as_str())
        .collect();
    assert_eq!(files, ["a.yaml", "b.yaml", "d.yaml"]);
    // A directory that matches `*.yaml` cannot be read: kept as a failed file.
    assert!(b2.implementations[1].files[2].document.is_err());
    assert!(b2.implementations[1].files[0].document.is_ok());
}

#[cfg(target_os = "linux")]
#[test]
fn scan_orders_by_bytes_not_case() {
    let dir = Scratch::new("bytes");
    dir.write("tools/b/default/1.yaml", "implements: b\n");
    dir.write("tools/B/default/1.yaml", "implements: B\n");
    dir.write("tools/a/default/b.yaml", "implements: a\n");
    dir.write("tools/a/default/A.yaml", "implements: a\n");
    let scanned = scan_register(&dir.register());
    let names: Vec<&str> = scanned.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["B", "a", "b"]);
    let files: Vec<&str> = scanned[1].implementations[0]
        .files
        .iter()
        .map(|f| f.file.as_str())
        .collect();
    assert_eq!(files, ["A.yaml", "b.yaml"]);
}

#[test]
fn missing_register_or_tools_directory_scans_to_nothing() {
    let dir = Scratch::new("missing");
    assert!(scan_register(&Register::at(dir.path().join("nope"))).is_empty());
    assert!(scan_register(&dir.register()).is_empty());
    let index = load_tool_index(&dir.register());
    assert!(index.all_tools().is_empty());
    assert!(index.aliases().is_empty());
    assert!(index.warnings().is_empty());
}

#[test]
fn malformed_file_stops_its_tool_but_not_the_rest() {
    let dir = Scratch::new("malformed");
    dir.write("tools/aaa/default/1.yaml", "implements: first\n");
    dir.write("tools/bbb/default/1.yaml", "implements: second\n");
    dir.write("tools/bbb/default/2.yaml", "implements: [unclosed\n");
    dir.write("tools/bbb/default/3.yaml", "implements: third\n");
    dir.write("tools/ccc/default/1.yaml", "implements: fourth\n");
    let index = load_tool_index(&dir.register());
    assert_eq!(
        warnings(&index),
        [("bbb".to_string(), "Psych::SyntaxError")]
    );
    assert_eq!(index.find_all_by_interface("first"), ["aaa"]);
    assert_eq!(index.find_all_by_interface("second"), ["bbb"]);
    assert!(index.find_all_by_interface("third").is_empty());
    assert_eq!(index.find_all_by_interface("fourth"), ["ccc"]);
}

#[test]
fn metadata_lookup_matches_the_ruby_oracle_rows() {
    let g = golden();
    let reg = fixture();
    let index = load_tool_index(&reg);
    let rows = g["load_tool_metadata"].as_array().unwrap();
    assert_eq!(rows.len(), 26);
    for row in rows {
        let name = row[0].as_str().unwrap();
        let got = load_tool_metadata(&reg, &index, name).unwrap();
        match &row[2]["ok"] {
            Json::Null => assert!(got.is_none(), "{name}"),
            ok => {
                let m = got.unwrap_or_else(|| panic!("{name}: expected metadata"));
                assert_eq!(
                    m.name.as_ref().map(Text::to_ruby_s).as_deref(),
                    ok["tool"].as_str(),
                    "{name}"
                );
                assert_eq!(
                    m.implements.as_ref().map(Text::to_ruby_s).as_deref(),
                    ok["implements"].as_str(),
                    "{name}"
                );
                assert_eq!(
                    m.version.as_ref().map(Text::to_ruby_s).as_deref(),
                    ok["version"].as_str(),
                    "{name}"
                );
            }
        }
    }
}

const OLD_TOOL: &str = "
name: old
version: '1.0'
implements: legacy
aliases: [oldie]
profiles:
- name: unix
  platforms: [linux, macos]
  shells: [bash, zsh]
  executable_name: ukiryu-lookup-old-exe
  commands:
  - name: run
    subcommand: run
- name: win
  platforms: [windows]
  shells: [cmd]
  inherits: unix
  commands: []
";

/// `tools/<tool>/index.yaml` pointing at `default/1.0.yaml`, plus that file.
fn write_tool(dir: &Scratch, tool: &str, document: &str) {
    dir.write(
        &format!("tools/{tool}/index.yaml"),
        "implementations:\n- name: default\n  versions:\n  - equals: '1.0'\n    file: 1.0.yaml\n",
    );
    dir.write(&format!("tools/{tool}/default/1.0.yaml"), document);
}

#[test]
fn tool_definition_loads_with_inheritance_and_compatible_profile() {
    let dir = Scratch::new("definition");
    write_tool(&dir, "old", OLD_TOOL);
    let definition = load_tool_definition(&dir.register(), "old", None)
        .unwrap()
        .expect("definition");
    let win = definition
        .compatible_profile("windows", "cmd")
        .unwrap()
        .expect("windows profile");
    assert_eq!(
        win.name.as_ref().map(Text::to_ruby_s).as_deref(),
        Some("win")
    );
    // `resolve_inheritance!`: the child gained the parent's command.
    assert_eq!(win.commands.as_ref().map(Vec::len), Some(1));
    let unix = definition
        .compatible_profile("linux", "bash")
        .unwrap()
        .expect("unix profile");
    assert_eq!(
        unix.name.as_ref().map(Text::to_ruby_s).as_deref(),
        Some("unix")
    );
    assert!(definition
        .compatible_profile("linux", "cmd")
        .unwrap()
        .is_none());

    assert!(load_tool_definition(&dir.register(), "absent", None)
        .unwrap()
        .is_none());
    // A listed version file that is missing is an error, not `None`.
    dir.write(
        "tools/gone/index.yaml",
        "implementations:\n- name: default\n  versions:\n  - equals: '1.0'\n    file: 1.0.yaml\n",
    );
    assert!(matches!(
        load_tool_definition(&dir.register(), "gone", None),
        Err(LookupError::Register(_))
    ));
}

/// A new-architecture definition source: tool name to definition YAML, and
/// names that fail with a given error.
struct Stub {
    yaml: HashMap<String, String>,
    errors: Vec<(String, fn() -> ToolError)>,
}

impl Stub {
    fn new(defs: &[(&str, String)]) -> Stub {
        Stub {
            yaml: defs
                .iter()
                .map(|(n, y)| (n.to_string(), y.clone()))
                .collect(),
            errors: Vec::new(),
        }
    }

    fn failing(mut self, name: &str, error: fn() -> ToolError) -> Stub {
        self.errors.push((name.to_string(), error));
        self
    }
}

impl DefinitionSource for Stub {
    fn load_definition(
        &self,
        name: &str,
        _options: &ToolOptions,
    ) -> Result<Option<ToolDefinition>, ToolError> {
        if let Some((_, error)) = self.errors.iter().find(|(n, _)| n == name) {
            return Err(error());
        }
        Ok(self
            .yaml
            .get(name)
            .map(|y| ToolDefinition::from_yaml(y).expect("stub definition")))
    }
}

/// A definition that runs on `platforms` and `shells` with executable `exe`.
fn definition(name: &str, exe: &str, platforms: &str, shells: &str) -> String {
    format!(
        "name: {name}\nversion: '1.0'\nprofiles:\n- name: default\n  platforms: [{platforms}]\n  shells: [{shells}]\n  executable_name: {exe}\n  commands:\n  - name: run\n    subcommand: run\n"
    )
}

/// An index document with only what `ToolIndex` reads.
fn index_document(implements: &str, aliases: &str, platforms: &str, shells: &str) -> String {
    format!(
        "version: '1.0'\nimplements: {implements}\naliases: [{aliases}]\nprofiles:\n- name: default\n  platforms: [{platforms}]\n  shells: [{shells}]\n"
    )
}

fn options() -> ToolOptions {
    ToolOptions::new(Platform::Linux, ShellName::Bash).discovery(DiscoveryContext::new(
        Platform::Linux,
        "bash",
        None,
        None,
    ))
}

fn runtime(platform: Platform, shell: ShellName) -> ToolOptions {
    ToolOptions::new(platform, shell)
}

fn linux_bash() -> ToolOptions {
    runtime(Platform::Linux, ShellName::Bash)
}

struct World {
    dir: Scratch,
    cache: ToolCache<Rc<Tool>>,
}

impl World {
    fn new(label: &str) -> World {
        World {
            dir: Scratch::new(label),
            cache: ToolCache::new(),
        }
    }

    fn index_doc(&self, tool: &str, document: &str) {
        self.dir
            .write(&format!("tools/{tool}/default/1.0.yaml"), document);
    }

    fn find(
        &self,
        stub: &Stub,
        identifier: &str,
        options: &ToolOptions,
        runtime: &ToolOptions,
    ) -> Result<Option<Rc<Tool>>, LookupError> {
        let register = self.dir.register();
        let index = load_tool_index(&register);
        find_by(
            &register,
            &index,
            stub,
            &self.cache,
            identifier,
            options,
            runtime,
        )
    }

    fn found(
        &self,
        stub: &Stub,
        identifier: &str,
        options: &ToolOptions,
        runtime: &ToolOptions,
    ) -> Option<String> {
        self.find(stub, identifier, options, runtime)
            .unwrap()
            .and_then(|t| t.name())
    }
}

const ANY: (&str, &str) = ("linux, macos, windows", "bash, zsh, cmd");

#[test]
fn exact_name_wins_over_interface_and_alias_without_needing_an_executable() {
    let w = World::new("exact");
    w.index_doc("alpha", &index_document("other", "", ANY.0, ANY.1));
    w.index_doc("beta", &index_document("alpha", "", ANY.0, ANY.1));
    w.index_doc("gamma", &index_document("third", "alpha", ANY.0, ANY.1));
    let stub = Stub::new(&[
        ("alpha", definition("alpha", "no-such-alpha", ANY.0, ANY.1)),
        ("beta", definition("beta", "no-such-beta", ANY.0, ANY.1)),
        ("gamma", definition("gamma", "no-such-gamma", ANY.0, ANY.1)),
    ]);
    let found = w.found(&stub, "alpha", &options(), &linux_bash());
    assert_eq!(found.as_deref(), Some("alpha"));
}

#[test]
fn unknown_identifier_is_none_not_an_error() {
    let w = World::new("unknown");
    w.index_doc("alpha", &index_document("conv", "al", ANY.0, ANY.1));
    let stub = Stub::new(&[("alpha", definition("alpha", "no-such-alpha", ANY.0, ANY.1))]);
    let found = w.find(&stub, "nothing-like-it", &options(), &linux_bash());
    assert!(found.unwrap().is_none());
    // An empty register and an empty source behave the same.
    let empty = World::new("unknown-empty");
    let found = empty.find(&Stub::new(&[]), "nothing", &options(), &linux_bash());
    assert!(found.unwrap().is_none());
}

#[test]
fn interface_candidate_missing_from_the_source_is_skipped() {
    let w = World::new("iface-missing");
    // Index order is byte order: aa (not in the source), then bb.
    w.index_doc("aa", &index_document("conv", "", ANY.0, ANY.1));
    w.index_doc("bb", &index_document("conv", "", ANY.0, ANY.1));
    // The first candidate is not available, the second is not either: with no
    // available candidate the interface step finds nothing.
    let stub = Stub::new(&[("bb", definition("bb", "no-such-bb", ANY.0, ANY.1))]);
    assert!(w
        .find(&stub, "conv", &options(), &linux_bash())
        .unwrap()
        .is_none());
}

#[cfg(unix)]
#[test]
fn interface_takes_the_first_available_candidate_in_index_order() {
    use std::os::unix::fs::PermissionsExt;
    let w = World::new("iface-first");
    let bin = w.dir.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let make = |name: &str| {
        let path = bin.join(name);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    };
    make("ukiryu-lookup-bb");
    make("ukiryu-lookup-cc");
    for tool in ["aa", "bb", "cc"] {
        w.index_doc(tool, &index_document("conv", "", ANY.0, ANY.1));
    }
    let stub = Stub::new(&[
        ("aa", definition("aa", "ukiryu-lookup-aa", ANY.0, ANY.1)),
        ("bb", definition("bb", "ukiryu-lookup-bb", ANY.0, ANY.1)),
        ("cc", definition("cc", "ukiryu-lookup-cc", ANY.0, ANY.1)),
    ]);
    let with_path = ToolOptions::new(Platform::Linux, ShellName::Bash).discovery(
        DiscoveryContext::new(Platform::Linux, "bash", bin.to_str(), None),
    );
    // aa is first but its executable is not on PATH; bb is the first available.
    let found = w.found(&stub, "conv", &with_path, &linux_bash());
    assert_eq!(found.as_deref(), Some("bb"));
}

#[test]
fn alias_collision_is_settled_by_the_runtime_platform_and_shell() {
    let w = World::new("alias");
    w.index_doc("x1", &index_document("one", "al", "macos", "zsh"));
    w.index_doc("x2", &index_document("two", "al", "linux", "bash"));
    let stub = Stub::new(&[
        (
            "x1",
            definition("x1", "no-such-x1", "macos, linux", "zsh, bash"),
        ),
        (
            "x2",
            definition("x2", "no-such-x2", "macos, linux", "zsh, bash"),
        ),
    ]);
    // Runtime linux/bash: x2 is the compatible one although x1 sorts first. An
    // alias match needs no executable.
    let found = w.found(&stub, "al", &options(), &linux_bash());
    assert_eq!(found.as_deref(), Some("x2"));
    // Runtime macos/zsh: x1 is the compatible one.
    let mac = runtime(Platform::Macos, ShellName::Zsh);
    let found = w.found(&stub, "al", &options(), &mac);
    assert_eq!(found.as_deref(), Some("x1"));
    // Neither is compatible with windows/cmd: the first candidate.
    let win = runtime(Platform::Windows, ShellName::Cmd);
    let found = w.found(&stub, "al", &options(), &win);
    assert_eq!(found.as_deref(), Some("x1"));
}

#[test]
fn alias_tool_without_a_profile_for_the_options_is_rescued_then_raised_by_exhaustive_search() {
    // The runtime says macos, so the alias resolves to the tool, but the
    // options are linux/bash.
    let mac = runtime(Platform::Macos, ShellName::Zsh);

    // A single profile is always the fallback (`find_command_profile`
    // returns `profiles.first` when there is one), so the alias step finds
    // the tool even though the profile names only macos/zsh.
    let w = World::new("alias-profile-single");
    w.index_doc("only-mac", &index_document("m", "al", "macos", "zsh"));
    let stub = Stub::new(&[(
        "only-mac",
        definition("only-mac", "no-such-only-mac", "macos", "zsh"),
    )]);
    assert_eq!(
        w.found(&stub, "al", &options(), &mac).as_deref(),
        Some("only-mac")
    );

    // With two profiles, neither linux/bash, `Tool.get` raises
    // ProfileNotFoundError. The alias step rescues it; the exhaustive step
    // then reaches the same tool and does not rescue it.
    let w = World::new("alias-profile-two");
    w.index_doc("two-mac", &index_document("m", "al", "macos", "zsh"));
    let two_profiles = "name: two-mac\nversion: '1.0'\nprofiles:\n- name: zsh\n  platforms: [macos]\n  shells: [zsh]\n  executable_name: no-such-two-mac\n  commands:\n  - name: run\n    subcommand: run\n- name: fish\n  platforms: [macos]\n  shells: [fish]\n  executable_name: no-such-two-mac\n  commands:\n  - name: run\n    subcommand: run\n";
    let stub = Stub::new(&[("two-mac", two_profiles.to_string())]);
    let err = w.find(&stub, "al", &options(), &mac).unwrap_err();
    assert!(
        matches!(err, LookupError::Tool(ToolError::ProfileNotFound { .. })),
        "{err:?}"
    );
}

#[test]
fn exhaustive_search_falls_back_to_old_architecture_definitions() {
    let w = World::new("exhaustive");
    // Sorted before `old`, but its only profile is macos: skipped.
    let mac_only = OLD_TOOL
        .replace("name: old", "name: a-mac")
        .replace("[linux, macos]", "[macos]")
        .replace("[bash, zsh]", "[zsh]")
        .replace("[windows]", "[macos]")
        .replace("[cmd]", "[zsh]");
    write_tool(&w.dir, "a-mac", &mac_only);
    write_tool(&w.dir, "old", OLD_TOOL);
    // Neither tool is known to the new-architecture source.
    let stub = Stub::new(&[]);
    for identifier in ["legacy", "oldie"] {
        let found = w.found(&stub, identifier, &options(), &linux_bash());
        assert_eq!(found.as_deref(), Some("old"), "{identifier}");
    }
    // The tool was built once and cached; a second search returns that tool.
    let key = tool::cache_key("old", &options());
    let cached = w.cache.get(&key).expect("cached");
    let again = w
        .find(&stub, "legacy", &options(), &linux_bash())
        .unwrap()
        .unwrap();
    assert!(Rc::ptr_eq(&cached, &again));
    // On windows/cmd only the `win` profile of `old` is compatible; the match
    // still works. Windows with bash has no compatible profile anywhere.
    let win = ToolOptions::new(Platform::Windows, ShellName::Cmd);
    let found = w.found(&stub, "legacy", &win, &linux_bash());
    assert_eq!(found.as_deref(), Some("old"));
    let none = ToolOptions::new(Platform::Windows, ShellName::Bash);
    assert!(w
        .find(&stub, "legacy", &none, &linux_bash())
        .unwrap()
        .is_none());
}

#[test]
fn exhaustive_search_checks_definitions_the_index_missed() {
    let w = World::new("exhaustive-def");
    // The register file says nothing about `hidden-iface`; the definition does.
    dir_tool(&w, "quiet");
    let mut def = definition("quiet", "no-such-quiet", ANY.0, ANY.1);
    def.push_str("implements: [hidden-iface]\naliases: [shy]\n");
    let stub = Stub::new(&[("quiet", def)]);
    for identifier in ["hidden-iface", "shy"] {
        let found = w.found(&stub, identifier, &options(), &linux_bash());
        assert_eq!(found.as_deref(), Some("quiet"), "{identifier}");
    }
    assert!(w
        .find(&stub, "other", &options(), &linux_bash())
        .unwrap()
        .is_none());
}

/// A listed tool (`index.yaml`) whose register file has no `implements`.
fn dir_tool(w: &World, tool: &str) {
    write_tool(&w.dir, tool, "version: '1.0'\n");
}

#[test]
fn errors_other_than_not_found_escape() {
    let w = World::new("errors");
    // Exact step: a Ruby error from the source is not rescued.
    let stub = Stub::new(&[]).failing("boom", || ToolError::Ruby {
        class: "NoMethodError",
        message: "undefined method".to_string(),
    });
    let err = w
        .find(&stub, "boom", &options(), &linux_bash())
        .unwrap_err();
    assert_eq!(err.class_name(), "NoMethodError");

    // Exhaustive step: ProfileNotFoundError is not rescued there (only
    // ToolNotFoundError is), unlike the earlier steps.
    let w = World::new("errors-exhaustive");
    dir_tool(&w, "t1");
    let stub = Stub::new(&[]).failing("t1", || ToolError::ProfileNotFound {
        message: "no profile".to_string(),
    });
    let err = w
        .find(&stub, "unrelated", &options(), &linux_bash())
        .unwrap_err();
    assert!(matches!(
        err,
        LookupError::Tool(ToolError::ProfileNotFound { .. })
    ));
    // The same error for the exact name is rescued, then the search goes on
    // and escapes in the exhaustive step as well: it comes from the same tool.
    let err = w.find(&stub, "t1", &options(), &linux_bash()).unwrap_err();
    assert!(matches!(
        err,
        LookupError::Tool(ToolError::ProfileNotFound { .. })
    ));
}
