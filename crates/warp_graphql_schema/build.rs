use anyhow::Result;

fn main() -> Result<()> {
    // If this file changes, regenerate the Rust sources.
    println!("cargo:rerun-if-changed=build.rs");

    cynic_codegen::register_schema("warp-server")
        .from_sdl_file("api/schema.graphql")
        .expect("Should be able to register schema")
        .as_default()?;

    Ok(())
}
