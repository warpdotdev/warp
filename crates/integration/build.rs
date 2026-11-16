use std::path::PathBuf;
use std::{env, fs};

fn main() {
    cargo_target_tmpdir();
}

fn cargo_target_tmpdir() {
    let out_dir = env::var("OUT_DIR").expect("Build script should have out dir");
    let dest_path = PathBuf::from(&out_dir).join("cargo_target_tmpdir.rs");
    let tmp_path = match env::var("CARGO_TARGET_TMPDIR") {
        Ok(path) => PathBuf::from(path),
        Err(_) => PathBuf::from(out_dir).join("tmp/"),
    };
    fs::write(
        dest_path,
        format!(
            "pub mod cargo_target_tmpdir {{
    pub fn get() -> String {{
        r\"{}\".to_string()
    }}
}}",
            tmp_path
                .to_str()
                .expect("Should be able to convert the path to a string")
        ),
    )
    .expect("Could not write code snippet");
}
