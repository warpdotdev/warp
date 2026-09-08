use std::path::PathBuf;

use getset::Getters;
use tempfile::{TempDir, tempdir};

/// A virtual filesystem for testing purposes.
#[derive(Getters)]
#[get = "pub"]
pub struct VirtualFS {
    root: TempDir,
    cwd: PathBuf,
    tests: String,
}

#[derive(Default, Getters, Clone)]
#[get = "pub"]
pub struct Dirs {
    pub root: PathBuf,
    pub tests: PathBuf,
}

impl Dirs {
    #[allow(dead_code)]
    pub fn git_repository_fixture(&self) -> PathBuf {
        Warp::fixtures().join("git_repository")
    }
}

pub enum Stub<'a> {
    #[allow(dead_code)]
    FileWithContent(&'a str, &'a str),
    FileWithContentToBeTrimmed(&'a str, &'a str),
    EmptyFile(&'a str),
    #[cfg(unix)]
    MockExecutable(&'a str),
}

impl VirtualFS {
    pub fn test(tag: &str, test_callback: impl FnOnce(Dirs, VirtualFS)) {
        let root = tempdir().expect("failed create root directory.");

        let warpbox_dir = root.path().join(tag);

        if PathBuf::from(&warpbox_dir).exists() {
            std::fs::remove_dir_all(PathBuf::from(&warpbox_dir)).expect("can not remove directory");
        }

        std::fs::create_dir(PathBuf::from(&warpbox_dir)).expect("can not create directory");

        let tests = dunce::canonicalize(&warpbox_dir).unwrap_or_else(|e| {
            panic!(
                "Couldn't canonicalize test path {}: {:?}",
                warpbox_dir.display(),
                e
            )
        });

        let directories = Dirs {
            root: root.path().to_path_buf(),
            tests,
        };

        let warpbox = VirtualFS {
            root,
            cwd: warpbox_dir,
            tests: tag.to_string(),
        };

        test_callback(directories, warpbox);
    }

    pub fn back_to_root(&mut self) -> &mut Self {
        self.cwd = PathBuf::from(self.root().path()).join(self.tests.clone());
        self
    }

    pub fn mkdir(&mut self, directory: &str) -> &mut Self {
        self.cwd.push(directory);
        std::fs::create_dir_all(&self.cwd).expect("can not create directory");
        self.back_to_root();
        self
    }

    #[cfg(unix)]
    pub fn ln<T, U>(&mut self, target: T, link: U) -> &mut Self
    where
        T: AsRef<std::path::Path>,
        U: AsRef<std::path::Path>,
    {
        let mut target_path = PathBuf::from(&self.cwd);
        target_path.push(target);
        let mut link_path = PathBuf::from(&self.cwd);
        link_path.push(link);
        std::os::unix::fs::symlink(target_path, link_path)
            .expect("can not create symlink for {link} -> {target}");
        self.back_to_root();
        self
    }

    pub fn with_files(&mut self, files: Vec<Stub>) -> &mut Self {
        let endl = String::from("\n");

        files
            .iter()
            .map(|f| {
                let mut path = PathBuf::from(&self.cwd);

                let (file_name, contents) = match *f {
                    Stub::EmptyFile(name) => (name, "fake data".to_string()),
                    #[cfg(unix)]
                    Stub::MockExecutable(name) => (name, "fake data".to_string()),
                    Stub::FileWithContent(name, content) => (name, content.to_string()),
                    Stub::FileWithContentToBeTrimmed(name, content) => (
                        name,
                        content
                            .lines()
                            .skip(1)
                            .map(|line| line.trim())
                            .collect::<Vec<&str>>()
                            .join(&endl),
                    ),
                };

                path.push(file_name);

                std::fs::write(&path, contents.as_bytes()).expect("can not create file");

                #[cfg(unix)]
                {
                    if matches!(f, Stub::MockExecutable(_)) {
                        use std::os::unix::fs::PermissionsExt;
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                            .expect("can not set permissions for executable");
                    }
                }
            })
            .for_each(drop);
        self.back_to_root();
        self
    }

    pub fn touch(&mut self, files: Vec<Stub>) -> &mut Self {
        files
            .iter()
            .map(|f| {
                let mut path = PathBuf::from(&self.cwd);

                if let Stub::EmptyFile(path_to_file) = f {
                    path.push(path_to_file);
                };

                std::fs::write(path, "emptyfile".as_bytes()).expect("can not create file");
            })
            .for_each(drop);
        self.back_to_root();
        self
    }
}

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
