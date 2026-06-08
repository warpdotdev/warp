fn main() -> Result<(), std::env::VarError> {
    let target_family = std::env::var("CARGO_CFG_TARGET_FAMILY")?;

    if target_family != "wasm" {
        println!("cargo:rustc-cfg=feature=\"local_fs\"");
    }

    // `embed_migrations!` bakes the migration SQL into the binary at compile
    // time. Tell cargo to rebuild this crate whenever any migration file
    // changes, so editing SQL during development doesn't leave a stale,
    // cached binary behind.
    println!("cargo:rerun-if-changed=migrations");

    Ok(())
}
