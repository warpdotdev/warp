use std::fs;
use std::os::unix::fs::symlink;

use anyhow::Result;

use super::*;

#[test]
fn path_resolves_to_detects_matching_symlink() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let source = temp_dir.path().join("source");
    let target = temp_dir.path().join("target");

    fs::write(&source, "wrapper")?;
    assert!(!path_resolves_to(&target, &source));

    symlink(&source, &target)?;
    assert!(path_resolves_to(&target, &source));

    Ok(())
}
