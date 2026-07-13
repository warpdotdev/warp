use std::path::{Path, MAIN_SEPARATOR};
use std::time::Duration;

use instant::Instant;

use super::*;

/// Drives the engine until both the scan and the matcher are idle (or the
/// timeout elapses). Returns whether the engine settled.
fn drain(engine: &mut StreamingFileSearchEngine, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        let status = engine.poll(10);
        if !status.running && !engine.is_scanning() {
            // One final poll so candidates injected right before the scan
            // completed are matched.
            let status = engine.poll(10);
            if !status.running {
                return true;
            }
        }
        if start.elapsed() > timeout {
            return false;
        }
    }
}

fn write_file(root: &Path, relative: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, b"contents").unwrap();
}

fn matched_paths(engine: &StreamingFileSearchEngine, max: usize) -> Vec<String> {
    engine
        .matched(max)
        .into_iter()
        .map(|candidate| candidate.relative_path)
        .collect()
}

const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn plain_directory_scan_finds_files_and_directories() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "src/main.rs");
    write_file(dir.path(), "src/lib.rs");
    write_file(dir.path(), "README.md");

    let mut engine = StreamingFileSearchEngine::new(dir.path().to_path_buf());
    engine.update_query("");
    assert!(drain(&mut engine, DRAIN_TIMEOUT));

    let mut paths = matched_paths(&engine, usize::MAX);
    paths.sort();
    assert_eq!(
        paths,
        vec![
            "README.md".to_string(),
            format!("src{MAIN_SEPARATOR}"),
            format!("src{MAIN_SEPARATOR}lib.rs"),
            format!("src{MAIN_SEPARATOR}main.rs"),
        ]
    );

    let candidates = engine.matched(usize::MAX);
    let src_dir = candidates
        .iter()
        .find(|c| c.relative_path == format!("src{MAIN_SEPARATOR}"))
        .unwrap();
    assert!(src_dir.is_directory);
}

#[test]
fn fuzzy_query_filters_and_ranks() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "src/model.rs");
    write_file(dir.path(), "src/view.rs");
    write_file(dir.path(), "docs/model_overview.md");

    let mut engine = StreamingFileSearchEngine::new(dir.path().to_path_buf());
    engine.update_query("model");
    assert!(drain(&mut engine, DRAIN_TIMEOUT));

    let paths = matched_paths(&engine, usize::MAX);
    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&format!("src{MAIN_SEPARATOR}model.rs")));
    assert!(paths.contains(&format!("docs{MAIN_SEPARATOR}model_overview.md")));
}

#[test]
fn multi_term_queries_require_all_terms() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "src/search/model.rs");
    write_file(dir.path(), "src/other/model.rs");

    let mut engine = StreamingFileSearchEngine::new(dir.path().to_path_buf());
    engine.update_query("model search");
    assert!(drain(&mut engine, DRAIN_TIMEOUT));

    let paths = matched_paths(&engine, usize::MAX);
    assert_eq!(
        paths,
        vec![format!("src{MAIN_SEPARATOR}search{MAIN_SEPARATOR}model.rs")]
    );
}

#[test]
fn fzf_operators_are_treated_literally() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "notes!.md");
    write_file(dir.path(), "other.md");

    let mut engine = StreamingFileSearchEngine::new(dir.path().to_path_buf());
    // With fzf semantics `!other` would mean "exclude matches of `other`"
    // (matching notes!.md). Warp treats `!` literally, so nothing matches.
    engine.update_query("!other");
    assert!(drain(&mut engine, DRAIN_TIMEOUT));
    assert_eq!(engine.matched_count(), 0);

    // The literal characters still match when present in the path.
    engine.update_query("notes!");
    assert!(drain(&mut engine, DRAIN_TIMEOUT));
    assert_eq!(matched_paths(&engine, usize::MAX), vec!["notes!.md"]);
}

#[test]
fn query_narrowing_and_widening() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "alpha.rs");
    write_file(dir.path(), "beta.rs");

    let mut engine = StreamingFileSearchEngine::new(dir.path().to_path_buf());
    engine.update_query("a");
    assert!(drain(&mut engine, DRAIN_TIMEOUT));
    // Both alpha.rs and beta.rs contain "a".
    assert_eq!(engine.matched_count(), 2);

    // Appending narrows (exercises nucleo's incremental re-scoring path).
    engine.update_query("alp");
    assert!(drain(&mut engine, DRAIN_TIMEOUT));
    assert_eq!(matched_paths(&engine, usize::MAX), vec!["alpha.rs"]);

    // Clearing widens back out to everything.
    engine.update_query("");
    assert!(drain(&mut engine, DRAIN_TIMEOUT));
    assert_eq!(engine.matched_count(), 2);
}

#[test]
fn refresh_if_stale_noops_without_git_changes() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "file.rs");

    let mut engine = StreamingFileSearchEngine::new(dir.path().to_path_buf());
    engine.update_query("");
    assert!(drain(&mut engine, DRAIN_TIMEOUT));
    // Non-git root: mtime is None both times, no rescan.
    assert!(!engine.refresh_if_stale());
    assert_eq!(engine.matched_count(), 1);
}

#[test]
fn build_pattern_text_escapes_fzf_syntax() {
    assert_eq!(build_pattern_text("model"), "model");
    assert_eq!(build_pattern_text("mod view"), "mod view");
    assert_eq!(build_pattern_text("!bang"), "\\!bang");
    assert_eq!(build_pattern_text("'quote"), "\\'quote");
    assert_eq!(build_pattern_text("^start"), "\\^start");
    assert_eq!(build_pattern_text("end$"), "end\\$");
    assert_eq!(build_pattern_text("!both$"), "\\!both\\$");
    // Mid-atom occurrences are already literal in nucleo's syntax.
    assert_eq!(build_pattern_text("notes!"), "notes!");
    assert_eq!(build_pattern_text("a'b^c"), "a'b^c");
    assert_eq!(build_pattern_text("back\\slash"), "back\\slash");
}

mod git {
    use super::*;

    /// Initializes a git repo at `root` with the given tracked files
    /// committed. Returns false (skipping the test) if git is unavailable.
    fn init_git_repo(root: &Path, tracked: &[&str]) -> bool {
        let run = |args: &[&str]| {
            command::blocking::Command::new("git")
                .args(args)
                .current_dir(root)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .is_ok_and(|output| output.status.success())
        };
        if !run(&["init", "--quiet"]) {
            return false;
        }
        for file in tracked {
            write_file(root, file);
        }
        run(&["add", "."])
            && run(&[
                "-c",
                "user.email=test@warp.dev",
                "-c",
                "user.name=Test",
                "commit",
                "--quiet",
                "-m",
                "init",
            ])
    }

    #[test]
    fn git_repo_scan_includes_tracked_untracked_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        if !init_git_repo(dir.path(), &["src/main.rs", "docs/guide.md"]) {
            eprintln!("git unavailable; skipping");
            return;
        }
        // Untracked (not ignored) file.
        write_file(dir.path(), "src/untracked.rs");
        // Ignored file must not appear.
        std::fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
        write_file(dir.path(), "ignored.txt");

        let mut engine = StreamingFileSearchEngine::new(dir.path().to_path_buf());
        engine.update_query("");
        assert!(drain(&mut engine, DRAIN_TIMEOUT));

        let paths = matched_paths(&engine, usize::MAX);
        assert!(paths.contains(&format!("src{MAIN_SEPARATOR}main.rs")));
        assert!(paths.contains(&format!("docs{MAIN_SEPARATOR}guide.md")));
        assert!(paths.contains(&format!("src{MAIN_SEPARATOR}untracked.rs")));
        assert!(paths.contains(&format!("src{MAIN_SEPARATOR}")));
        assert!(paths.contains(&format!("docs{MAIN_SEPARATOR}")));
        assert!(paths.contains(&".gitignore".to_string()));
        assert!(!paths.iter().any(|p| p.contains("ignored.txt")));
        assert!(!paths.iter().any(|p| p.starts_with(".git/")));
    }

    #[test]
    fn refresh_if_stale_rescans_when_git_index_changes() {
        let dir = tempfile::tempdir().unwrap();
        if !init_git_repo(dir.path(), &["a.rs"]) {
            eprintln!("git unavailable; skipping");
            return;
        }

        let mut engine = StreamingFileSearchEngine::new(dir.path().to_path_buf());
        engine.update_query("");
        assert!(drain(&mut engine, DRAIN_TIMEOUT));
        let initial_count = engine.matched_count();

        // Add and stage a new file, bumping .git/index's mtime.
        write_file(dir.path(), "b.rs");
        let staged = command::blocking::Command::new("git")
            .args(["add", "b.rs"])
            .current_dir(dir.path())
            .output()
            .is_ok_and(|output| output.status.success());
        assert!(staged);
        // Ensure the index mtime visibly differs even on coarse filesystems.
        let stale = wait_for(|| git_index_mtime(dir.path()) != engine.scanned_git_index_mtime);
        assert!(stale, "git index mtime never changed");

        assert!(engine.refresh_if_stale());
        assert!(drain(&mut engine, DRAIN_TIMEOUT));
        assert_eq!(engine.matched_count(), initial_count + 1);
        assert!(matched_paths(&engine, usize::MAX).contains(&"b.rs".to_string()));

        // A second refresh with no changes is a no-op.
        assert!(!engine.refresh_if_stale());
    }

    fn wait_for(mut condition: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }
}
