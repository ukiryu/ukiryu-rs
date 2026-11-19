//! Cross-platform discovery paths follow Ruby's File.expand_path conventions.
use ukiryu::definition::metadata::SourceKind;
use ukiryu::definition::{Discovery, DiscoveryEnv};

#[test]
fn user_source_matches_ruby_forward_slash_xdg_path() {
    // Ruby Dir.mktmpdir returns forward slashes even on Windows. Rust's
    // expanded PathBuf uses native separators, so compare the same directory
    // through both forms without requiring any files or process ENV changes.
    let root = std::env::temp_dir().join("ukiryu-discovery-source");
    let xdg = root
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    let discovery = Discovery::new(DiscoveryEnv {
        home: None,
        xdg_data_home: Some(xdg.clone()),
        xdg_data_dirs: None,
        path: None,
        opt_root: "/opt".into(),
    });
    let definitions = root.join("ukiryu").join("definitions");

    assert_eq!(
        discovery.determine_source_type(&definitions.to_string_lossy()),
        SourceKind::User
    );
    assert_eq!(
        discovery.determine_source_type(&format!("{xdg}/ukiryu/definitions")),
        SourceKind::User
    );
}
