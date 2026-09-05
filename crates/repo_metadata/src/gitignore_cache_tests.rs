use std::sync::{Arc, Barrier};

use parking_lot::Mutex;

use super::{
    Cache, CacheKey, EFFECTIVE_MAX_LIVE_MATCHERS, GitignoreRules, SourceSnapshot,
    cache_live_matcher_counts,
};

fn match_file(
    cache: &mut Cache,
    gitignore_path: &std::path::Path,
    target: &std::path::Path,
) -> bool {
    cache.match_source(
        &SourceSnapshot::file(gitignore_path.to_path_buf()),
        target,
        false,
        true,
    )
}

#[test]
fn reuses_cached_entry_for_unchanged_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join(".gitignore");
    std::fs::write(&path, "target/\n").unwrap();
    let mut cache = Cache::default();

    assert!(match_file(
        &mut cache,
        &path,
        &temp_dir.path().join("target/file")
    ));
    assert!(match_file(
        &mut cache,
        &path,
        &temp_dir.path().join("target/file")
    ));

    assert_eq!(cache.parse_count, 1);
}

#[test]
fn evicts_least_recently_used_entry_over_matcher_count_limit() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut cache = Cache::default();
    let paths: Vec<_> = (0..=EFFECTIVE_MAX_LIVE_MATCHERS)
        .map(|index| {
            let path = temp_dir.path().join(format!("gitignore_{index}"));
            std::fs::write(&path, format!("i{index}\n")).unwrap();
            match_file(&mut cache, &path, &temp_dir.path().join("unmatched"));
            path
        })
        .collect();

    assert_eq!(cache.entries.len(), EFFECTIVE_MAX_LIVE_MATCHERS);
    assert!(
        !cache
            .entries
            .contains_key(&CacheKey::File(paths[0].clone()))
    );
    assert!(cache.peak_live_matchers <= EFFECTIVE_MAX_LIVE_MATCHERS);
}

#[test]
fn rebuilds_when_content_changes_at_the_same_length() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join(".gitignore");
    std::fs::write(&path, "target/\n").unwrap();
    let original_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    let mut cache = Cache::default();

    assert!(match_file(
        &mut cache,
        &path,
        &temp_dir.path().join("target/file")
    ));
    std::fs::write(&path, "assets/\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(original_mtime)
        .unwrap();

    assert!(match_file(
        &mut cache,
        &path,
        &temp_dir.path().join("assets/file")
    ));
    assert!(!match_file(
        &mut cache,
        &path,
        &temp_dir.path().join("target/file")
    ));
    assert_eq!(cache.parse_count, 2);
}

#[cfg(unix)]
#[test]
fn recovers_after_a_transient_read_failure() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join(".gitignore");
    let target = temp_dir.path().join("target/file");
    std::fs::write(&path, "target/\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&path).is_ok() {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        return;
    }
    let mut cache = Cache::default();

    assert!(!match_file(&mut cache, &path, &target));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(match_file(&mut cache, &path, &target));
}

#[test]
fn does_not_cache_a_failed_parse() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join(".gitignore");
    let target = temp_dir.path().join("target/file");
    std::fs::write(&path, "target/\n[z-a]\n").unwrap();
    let mut cache = Cache::default();

    match_file(&mut cache, &path, &target);
    match_file(&mut cache, &path, &target);
    assert_eq!(cache.parse_count, 2);
    assert!(!cache.entries.contains_key(&CacheKey::File(path.clone())));

    std::fs::write(&path, "target/\n").unwrap();
    assert!(match_file(&mut cache, &path, &target));
    assert!(match_file(&mut cache, &path, &target));
    assert_eq!(cache.parse_count, 3);
}

#[test]
fn evicts_least_recently_used_entry_over_source_byte_budget() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut cache = Cache::default();
    let paths: Vec<_> = (0..3)
        .map(|index| {
            let path = temp_dir.path().join(format!("gitignore_{index}"));
            std::fs::write(&path, "target/\n\n").unwrap();
            match_file(&mut cache, &path, &temp_dir.path().join("unmatched"));
            path
        })
        .collect();

    assert_eq!(cache.total_source_bytes, 18);
    assert!(
        !cache
            .entries
            .contains_key(&CacheKey::File(paths[0].clone()))
    );
}

#[test]
fn over_cap_concurrent_rule_sets_stay_within_live_matcher_budget() {
    let temp_dir = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..10)
        .map(|index| {
            let path = temp_dir.path().join(format!("gitignore_{index}"));
            std::fs::write(&path, format!("i{index}\n")).unwrap();
            path
        })
        .collect();
    let rules = Arc::new(GitignoreRules::default().with_cached_paths(paths));
    let cache = Arc::new(Mutex::new(Cache::default()));
    let barrier = Arc::new(Barrier::new(9));
    let target = Arc::new(temp_dir.path().join("unmatched"));

    let threads: Vec<_> = (0..8)
        .map(|_| {
            let rules = rules.clone();
            let cache = cache.clone();
            let barrier = barrier.clone();
            let target = target.clone();
            std::thread::spawn(move || {
                barrier.wait();
                assert!(
                    !rules
                        .operation()
                        .matches_with_cache(&cache, &target, false, false)
                );
            })
        })
        .collect();
    barrier.wait();
    for thread in threads {
        thread.join().unwrap();
    }

    let cache = cache.lock();
    assert!(cache.entries.len() <= EFFECTIVE_MAX_LIVE_MATCHERS);
    assert!(cache.peak_live_matchers <= EFFECTIVE_MAX_LIVE_MATCHERS);
    assert!(cache.parse_count >= 10);
}

#[test]
fn public_matches_refreshes_global_rules_for_long_lived_rule_sets() {
    let temp_dir = tempfile::tempdir().unwrap();
    let global_path = temp_dir.path().join("global-ignore");
    let rules = GitignoreRules::default()
        .with_global_source(global_path.clone(), temp_dir.path().to_path_buf());

    std::fs::write(&global_path, "first\n").unwrap();
    assert!(rules.matches(&temp_dir.path().join("first"), false, false));

    std::fs::write(&global_path, "second\n").unwrap();
    assert!(!rules.matches(&temp_dir.path().join("first"), false, false));
    assert!(rules.matches(&temp_dir.path().join("second"), false, false));
}

#[test]
fn operation_reads_each_source_only_once() {
    let temp_dir = tempfile::tempdir().unwrap();
    let paths = (0..10)
        .map(|index| {
            let path = temp_dir.path().join(format!("gitignore_{index}"));
            std::fs::write(&path, format!("ignored_{index}\n")).unwrap();
            path
        })
        .collect();
    let operation = GitignoreRules::default()
        .with_cached_paths(paths)
        .operation();

    for index in 0..100 {
        assert!(!operation.matches(
            &temp_dir.path().join(format!("unmatched_{index}")),
            false,
            false,
        ));
    }

    assert_eq!(operation.source_read_count(), 10);
}

#[test]
fn over_cap_repository_watch_operations_do_not_retain_matchers() {
    let temp_dir = tempfile::tempdir().unwrap();
    let operations = (0..10)
        .map(|index| {
            let root = temp_dir.path().join(format!("repo_{index}"));
            std::fs::create_dir(&root).unwrap();
            std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
            std::fs::create_dir(root.join("ignored")).unwrap();
            (
                root.clone(),
                GitignoreRules::for_directory(&root).operation(),
            )
        })
        .collect::<Vec<_>>();

    for (root, operation) in &operations {
        assert!(!crate::entry::should_watch_repo_directory_with_operation(
            &root.join("ignored"),
            root,
            operation,
            &[],
        ));
    }

    let (live, peak) = cache_live_matcher_counts();
    assert!(live <= EFFECTIVE_MAX_LIVE_MATCHERS);
    assert!(peak <= EFFECTIVE_MAX_LIVE_MATCHERS);
}
