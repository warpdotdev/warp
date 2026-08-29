use std::env;

// This mirrors the `add_features` logic in app/build.rs: on non-wasm targets, it emits
// `cfg(feature = "local_tty")` so code moved from the app compiles with its original cfg gates,
// without dependents having to enable the Cargo feature.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_FAMILY");

    let target_family =
        env::var("CARGO_CFG_TARGET_FAMILY").expect("CARGO_CFG_TARGET_FAMILY not set");
    if target_family != "wasm" {
        println!("cargo:rustc-cfg=feature=\"local_tty\"");
    }
}
