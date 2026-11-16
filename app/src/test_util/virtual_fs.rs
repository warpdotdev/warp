use std::path::PathBuf;

pub use virtual_fs::{Dirs, Stub, VirtualFS};

pub trait WarpDirs {
    #[allow(dead_code)]
    fn git_repository_fixture(&self) -> PathBuf {
        Warp::fixtures().join("git_repository")
    }
}

impl WarpDirs for Dirs {}

pub struct Warp;

impl Warp {
    #[allow(dead_code)]
    pub fn executable() -> PathBuf {
        let mut path = {
            let mut build = "debug";

            if !cfg!(debug_assertions) {
                build = "release";
            }

            std::env::var("CARGO_TARGET_DIR")
                .ok()
                .map(|directory| PathBuf::from(directory).join(build))
                .unwrap_or_else(|| Self::root().join(format!("target/{}", &build)))
        };

        path.push("warp");
        path
    }

    #[allow(dead_code)]
    pub fn fixtures() -> PathBuf {
        Self::root().join("tests/fixtures")
    }

    pub fn root() -> PathBuf {
        let manifest_dir = if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
            PathBuf::from(manifest_dir)
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        };

        if manifest_dir.join("Cargo.lock").exists() {
            manifest_dir
        } else {
            manifest_dir
                .parent()
                .expect("Could not find the debug binaries directory")
                .parent()
                .expect("Could not find the debug binaries directory")
                .to_path_buf()
        }
    }
}
