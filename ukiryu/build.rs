//! Compile the Onigmo regular expression engine of Ruby 4.0.1.
//!
//! `vendor/onigmo` holds Ruby's Onigmo sources unchanged (see its
//! PROVENANCE and NOTICE). They are built the way Ruby builds them, with
//! `RUBY` defined; `onigmo-glue` supplies the few Ruby runtime hooks and the
//! group-name table they call, plus the scalar API `types::onig` uses.
//!
//! On `wasm32-unknown-unknown` the C code is compiled by Emscripten 3.1.50
//! (the same pinned compiler and scalar single-thread libc the vendored YAML
//! engine uses) into the Rust-owned module; malloc/free there are the Rust
//! allocator, so the module keeps one allocator.
use std::env;
use std::path::PathBuf;

const ONIGMO: &[&str] = &[
    "regcomp.c",
    "regenc.c",
    "regerror.c",
    "regexec.c",
    "regparse.c",
    "regsyntax.c",
    "enc/ascii.c",
    "enc/us_ascii.c",
    "enc/unicode.c",
    "enc/utf_8.c",
];

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let vendor = manifest.join("vendor/onigmo");
    let glue = manifest.join("onigmo-glue");
    println!("cargo:rerun-if-changed={}", vendor.display());
    println!("cargo:rerun-if-changed={}", glue.display());
    println!("cargo:rerun-if-env-changed=UKIRYU_EMCC");
    println!("cargo:rerun-if-env-changed=YEPTRIS_EMCC");

    let browser = env::var("TARGET").as_deref() == Ok("wasm32-unknown-unknown");
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");

    let mut build = cc::Build::new();
    if browser {
        let features = env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default();
        assert!(
            !features.split(',').any(|f| f == "atomics"),
            "the Onigmo browser build is single-thread WebAssembly without atomics"
        );
        build.compiler(emcc()).target("wasm32-unknown-emscripten");
    }
    build
        .include(glue.join("include"))
        .include(&vendor)
        .include(vendor.join("include"))
        .include(vendor.join("enc/unicode/17.0.0"))
        .define("RUBY", None)
        .define("RUBY_EXTERN", Some("extern"))
        .warnings(false);
    if !msvc {
        // Keep every Onigmo symbol private to this library, so a host that
        // also loads Ruby's own Onigmo (a Ruby extension) cannot interpose.
        build.flag("-fvisibility=hidden");
    }
    for file in ONIGMO {
        build.file(vendor.join(file));
    }
    build.file(glue.join("ukiryu_onigmo.c"));
    build.compile("ukiryu_onigmo");
}

fn emcc() -> std::ffi::OsString {
    env::var_os("UKIRYU_EMCC")
        .or_else(|| env::var_os("YEPTRIS_EMCC"))
        .unwrap_or_else(|| "emcc".into())
}
