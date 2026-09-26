//! THIS FILE IS GENERATED FROM xtask/templates/build.stpl.rs; DO NOT EDIT MANUALLY

fn main() {
    // All build-time grammar sources live inside the crate so that
    // crates.io package verification can rebuild the crate in isolation.
    //
    // Layout inside the published crate:
    //   grammar/
    //     scanner.c      (optional, hand-written)
    //     src/
    //       parser.c
    //       grammar.json
    //       node-types.json
    //       tree_sitter/* (generated headers)
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let src_dir = manifest_dir.join("grammar/src");
    let grammar_dir = manifest_dir.join("grammar");

    println!(
        "cargo:rerun-if-changed={}",
        src_dir.join("parser.c").display()
    );

    println!(
        "cargo:rerun-if-changed={}",
        grammar_dir.join("scanner.c").display()
    );

    let mut build = cc::Build::new();

    build
        .include(&src_dir)
        .include(&grammar_dir) // for common/ includes like "../common/scanner.h"
        .include(src_dir.join("tree_sitter"))
        .opt_level_str("z") // optimize aggressively for size
        .warnings(false)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-but-set-variable")
        .flag_if_supported("-Wno-trigraphs");

    // For WASM builds, use our custom sysroot (provided by arborium crate via links = "arborium")
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("wasm")
        && let Ok(sysroot) = std::env::var("DEP_ARBORIUM_SYSROOT_PATH")
    {
        build.include(&sysroot);
    }

    build.file(src_dir.join("parser.c"));

    build.file(grammar_dir.join("scanner.c"));

    build.compile("tree_sitter_sql");
}
