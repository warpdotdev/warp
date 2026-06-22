fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/remote_server.proto");
    println!("cargo:rerun-if-changed=proto/diff_state.proto");
    prost_build::compile_protos(
        &["proto/remote_server.proto", "proto/diff_state.proto"],
        &["proto/"],
    )?;
    Ok(())
}
