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

Most likely, this is fixed by:
    1) Ensuring you have an up-to-date Node version; 18.14.1 (required for warp-server development) should suffice.
    2) Running `corepack enable` (this can be done in any directory).
    3) Removing a conflicting yarn installed by brew by running:
        brew uninstall yarn

If you continue to encounter issues, ensure you don't have conflicting Node installations, one of which might not be a sufficiently recent version.
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
