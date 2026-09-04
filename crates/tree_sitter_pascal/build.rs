fn main() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("grammar/src");
    let parser = src_dir.join("parser.c");

    println!("cargo:rerun-if-changed={}", parser.display());

    let mut build = cc::Build::new();
    build
        .include(&src_dir)
        // Matches how arborium builds its grammars: these files are huge and never
        // hot, so size wins over speed.
        .opt_level_str("z")
        .warnings(false);

    // wasm32-unknown-unknown has no libc, so the `<stdint.h>`/`<stdlib.h>` includes in
    // `grammar/src/tree_sitter/parser.h` resolve only against the sysroot that
    // `arborium-sysroot` unpacks. The arborium grammar crates do the same thing;
    // staying in step with them is what lets Pascal build everywhere they do.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("wasm")
        && let Ok(sysroot) = std::env::var("DEP_ARBORIUM_SYSROOT_PATH")
    {
        build.include(&sysroot);
    }

    build.file(&parser);
    build.compile("tree_sitter_pascal");
}
