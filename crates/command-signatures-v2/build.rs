use std::path::Path;

use anyhow::anyhow;
use command::blocking::Command;

fn main() -> anyhow::Result<()> {
    println!("cargo:rerun-if-changed=js/src");
    println!("cargo:rerun-if-changed=js/build");
    println!("cargo:rerun-if-changed=js/package.json");
    println!("cargo:rerun-if-changed=js/tsconfig.json");

    if let Err(e) = build_command_signatures() {
        if !Path::new(format!("{}/js/build", env!("CARGO_MANIFEST_DIR")).as_str()).exists() {
            panic!(
                r#"Failed to build command signatures JS: {e:?}.

This usually means Node.js / yarn aren't set up for this checkout. See the
"Node.js setup" section of WARP.md for the version requirement and the
recommended install paths (nvm/fnm/volta/asdf/mise or your system package
manager). Re-running `./script/bootstrap` will also fail fast with the same
guidance.
"#
            )
        } else {
            println!("cargo:warning=Failed to build command signatures JS. Proceeding with stale command signatures!");
        }
    }
    Ok(())
}

fn build_command_signatures() -> anyhow::Result<()> {
    match Command::new("yarn")
        .arg("build")
        .current_dir(format!("{}/js", env!("CARGO_MANIFEST_DIR")))
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                Ok(())
            } else {
                Err(anyhow!(
                    "Failed to build Command Signatures JS with output: {:?}",
                    output
                ))
            }
        }
        Err(e) => Err(anyhow::Error::from(e)),
    }
}
