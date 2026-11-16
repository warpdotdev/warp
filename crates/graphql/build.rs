use anyhow::{Context as _, Result};

fn main() -> Result<()> {
    // If this file changes, regenerate the Rust sources.
    println!("cargo:rerun-if-changed=build.rs");

    // We need to register the schema here, even though the code is generated in the schema crate.
    cynic_codegen::register_schema("warp-server")
        .from_sdl_file("../warp_graphql_schema/api/schema.graphql")
        .context("Should be able to register schema")?
        .as_default()?;

    Ok(())
}
