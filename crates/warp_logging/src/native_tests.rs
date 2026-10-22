use std::io::Write;

use super::*;

fn touch(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    File::create(&path).unwrap();
    path
}

fn write_bytes(path: &Path, bytes: &[u8]) {
    let mut file = File::create(path).unwrap();
    file.write_all(bytes).unwrap();
}

fn zip_entry_names(zip_path: &Path) -> Vec<String> {
    let file = File::open(zip_path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    (0..archive.len())
        .map(|index| archive.by_index(index).unwrap().name().to_owned())
        .collect()
}
fn log_state(base_directory: &Path, frontend: LogFrontend, logfile_name: &str) -> LogState {
    LogState::new(
        true,
        base_directory.to_path_buf(),
        logfile_name.to_owned(),
        frontend,
    )
}

#[test]
fn frontend_resolves_directory_and_rotation_policy() {
    let base = PathBuf::from("/tmp/warp-logs");
    let gui = log_state(&base, LogFrontend::Gui, "warp_dev.log");
    let tui = log_state(&base, LogFrontend::Tui, "warp_dev.log");
    let cli = log_state(&base, LogFrontend::Cli, "warp_dev.log");

    assert_eq!(gui.log_directory, base);
    assert_eq!(gui.max_rotation, MAX_FILES_IN_GUI_ROTATION);
    assert_eq!(tui.log_directory, PathBuf::from("/tmp/warp-logs/warp-cli"));
    assert_eq!(tui.max_rotation, MAX_FILES_IN_CLI_ROTATION);
    assert_eq!(cli.log_directory, PathBuf::from("/tmp/warp-logs/oz"));
    assert_eq!(cli.max_rotation, MAX_FILES_IN_CLI_ROTATION);
}

#[test]
fn tui_bundle_uses_resolved_state_and_ignores_legacy_oz_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let state = log_state(tmp.path(), LogFrontend::Tui, "warp_preview.log");
    fs::create_dir(&state.log_directory).unwrap();
    write_bytes(
        &state.log_directory.join("warp_preview.log"),
        b"active tui session",
    );
    write_bytes(
        &state.log_directory.join("warp_preview.log.old.0"),
        b"previous tui session",
    );
    write_bytes(
        &state.log_directory.join("warp_preview.log.in_session.0"),
        b"mid-session tui chunk",
    );
    let legacy = tmp.path().join("oz");
    fs::create_dir(&legacy).unwrap();
    touch(&legacy, "warp_preview.log");
    let zip_path = state.create_log_bundle_zip().unwrap();

    assert_eq!(zip_path.parent(), Some(state.log_directory.as_path()));
    assert_eq!(
        zip_entry_names(&zip_path),
        vec![
            "warp_preview.log",
            "warp_preview.log.in_session.0",
            "warp_preview.log.old.0",
        ]
    );
}

#[test]
fn needs_zip64_respects_margin_below_threshold() {
    // The margin means the switch to ZIP64 happens slightly before the raw threshold, not
    // exactly at it — check the boundary sits where `ZIP64_THRESHOLD_MARGIN_BYTES` puts it.
    let boundary = zip::ZIP64_BYTES_THR - ZIP64_THRESHOLD_MARGIN_BYTES;
    assert!(!needs_zip64(boundary - 1));
    assert!(needs_zip64(boundary));
    assert!(needs_zip64(boundary + 1));
}

#[test]
#[ignore = "writes a 4 GiB entry; run manually or in a nightly job"]
fn bundle_zip_supports_entries_over_4gib() {
    // A sparse file lets this exercise the >4 GiB (ZIP64) code path on Linux without writing
    // real data: its logical length crosses `ZIP64_BYTES_THR`, and `io::copy` reads back the
    // resulting hole as zeroes. Still slow enough (real compression work over a multi-GiB
    // entry) to keep out of the default `cargo nextest` run, and not sparse on Windows/NTFS.
    let tmp = tempfile::tempdir().unwrap();
    let state = log_state(tmp.path(), LogFrontend::Gui, "warp.log");
    fs::create_dir_all(&state.log_directory).unwrap();
    touch(&state.log_directory, "warp.log");
    let huge_len = zip::ZIP64_BYTES_THR + 1;
    let huge_log = File::create(state.log_directory.join("warp.log.old.0")).unwrap();
    huge_log.set_len(huge_len).unwrap();
    drop(huge_log);

    let zip_path = state.create_log_bundle_zip().unwrap();

    assert_eq!(
        zip_entry_names(&zip_path),
        vec!["warp.log", "warp.log.old.0"]
    );
    let file = File::open(&zip_path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    assert_eq!(archive.by_name("warp.log.old.0").unwrap().size(), huge_len);
}

#[test]
fn create_log_bundle_zip_rejects_concurrent_exports_and_recovers_after_failure() {
    // `LOG_STATE` is process-global and can only be `set` once, so this is the crate's one
    // test that exercises the module-level `create_log_bundle_zip` (rather than
    // `LogState::create_log_bundle_zip` directly). Safe under `cargo nextest`, which runs each
    // test in its own process; would conflict with a second such test under plain `cargo test`.
    let tmp = tempfile::tempdir().unwrap();
    let state = log_state(tmp.path(), LogFrontend::Gui, "warp_guard_test.log");
    fs::create_dir_all(&state.log_directory).unwrap();
    LOG_STATE.set(state).unwrap();

    // No log files exist yet, so this fails before ever touching the in-flight flag's guard
    // scope — the flag must still be released afterward.
    let err = create_log_bundle_zip().unwrap_err();
    assert!(err.to_string().contains("No warp logs were found"));
    assert!(!LOG_BUNDLE_EXPORT_IN_PROGRESS.load(Ordering::Acquire));

    touch(&log_directory().unwrap(), "warp_guard_test.log");

    // Simulate a second request arriving while an export is already running.
    assert!(!LOG_BUNDLE_EXPORT_IN_PROGRESS.swap(true, Ordering::AcqRel));
    let err = create_log_bundle_zip().unwrap_err();
    assert!(err.to_string().contains("already in progress"));
    LOG_BUNDLE_EXPORT_IN_PROGRESS.store(false, Ordering::Release);

    // Once the in-progress export finishes, the next request goes through, and releases the
    // guard in turn rather than leaving it stuck on the success path.
    create_log_bundle_zip().unwrap();
    assert!(!LOG_BUNDLE_EXPORT_IN_PROGRESS.load(Ordering::Acquire));
}

#[test]
fn collects_active_in_session_and_old_logs_in_expected_order() {
    let tmp = tempfile::tempdir().unwrap();
    let active = touch(tmp.path(), "warp.log");
    let in_session_0 = touch(tmp.path(), "warp.log.in_session.0");
    let in_session_1 = touch(tmp.path(), "warp.log.in_session.1");
    let in_session_2 = touch(tmp.path(), "warp.log.in_session.2");
    let old_0 = touch(tmp.path(), "warp.log.old.0");
    let old_1 = touch(tmp.path(), "warp.log.old.1");

    let paths = collect_log_paths_in(tmp.path(), "warp.log").unwrap();

    assert_eq!(
        paths,
        vec![
            active,
            in_session_0,
            in_session_1,
            in_session_2,
            old_0,
            old_1
        ]
    );
}

#[test]
fn includes_in_session_logs_even_when_no_active_or_old_logs_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let in_session_0 = touch(tmp.path(), "warp.log.in_session.0");

    let paths = collect_log_paths_in(tmp.path(), "warp.log").unwrap();

    assert_eq!(paths, vec![in_session_0]);
}

#[test]
fn ignores_unrelated_files_and_malformed_suffixes() {
    let tmp = tempfile::tempdir().unwrap();
    let active = touch(tmp.path(), "warp.log");
    touch(tmp.path(), "warp.log.in_session.abc"); // not a number
    touch(tmp.path(), "warp.log.in_session."); // empty suffix
    touch(tmp.path(), "warp.log.old.xyz"); // not a number
    touch(tmp.path(), "other.log"); // unrelated
    touch(tmp.path(), "warp.log.old.temp"); // matches old. prefix but non-numeric

    let paths = collect_log_paths_in(tmp.path(), "warp.log").unwrap();

    assert_eq!(paths, vec![active]);
}

#[test]
fn errors_when_directory_is_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let err = collect_log_paths_in(tmp.path(), "warp.log").unwrap_err();
    assert!(err.to_string().contains("No warp logs were found"));
}

#[test]
fn respects_channel_specific_logfile_name() {
    // Beta/preview channels use a different base name; make sure scanning
    // is gated on that name and doesn't pick up the wrong channel's files.
    let tmp = tempfile::tempdir().unwrap();
    let active = touch(tmp.path(), "warp_preview.log");
    let in_session_0 = touch(tmp.path(), "warp_preview.log.in_session.0");
    touch(tmp.path(), "warp.log"); // different channel — must be ignored
    touch(tmp.path(), "warp.log.in_session.0");

    let paths = collect_log_paths_in(tmp.path(), "warp_preview.log").unwrap();

    assert_eq!(paths, vec![active, in_session_0]);
}

#[test]
fn interleaves_old_logs_with_their_nested_in_session_chunks() {
    // Previous sessions' mid-rotation chunks are nested under their
    // parent .old.K slot; collection should output each .old.K
    // immediately followed by its .old.K.in_session.M chunks before
    // moving on to .old.{K+1}.
    let tmp = tempfile::tempdir().unwrap();
    let active = touch(tmp.path(), "warp.log");
    let cur_in_session_0 = touch(tmp.path(), "warp.log.in_session.0");
    let old_0 = touch(tmp.path(), "warp.log.old.0");
    let old_0_chunk_0 = touch(tmp.path(), "warp.log.old.0.in_session.0");
    let old_0_chunk_1 = touch(tmp.path(), "warp.log.old.0.in_session.1");
    let old_1 = touch(tmp.path(), "warp.log.old.1");
    let old_1_chunk_0 = touch(tmp.path(), "warp.log.old.1.in_session.0");

    let paths = collect_log_paths_in(tmp.path(), "warp.log").unwrap();

    assert_eq!(
        paths,
        vec![
            active,
            cur_in_session_0,
            old_0,
            old_0_chunk_0,
            old_0_chunk_1,
            old_1,
            old_1_chunk_0,
        ]
    );
}

#[test]
fn nested_chunks_surface_even_when_parent_old_slot_is_missing() {
    // If a .old.K slot is missing from disk but its nested chunks
    // remain (e.g. truncated by manual cleanup), the chunks should
    // still be bundled rather than silently dropped.
    let tmp = tempfile::tempdir().unwrap();
    let active = touch(tmp.path(), "warp.log");
    let orphan_chunk = touch(tmp.path(), "warp.log.old.3.in_session.0");

    let paths = collect_log_paths_in(tmp.path(), "warp.log").unwrap();

    assert_eq!(paths, vec![active, orphan_chunk]);
}

#[test]
fn migrate_previous_session_renames_in_session_to_old_0_in_session() {
    // Mid-session chunks from the previous session belong with the
    // .old.0 slot that holds that session's final-state log.
    let tmp = tempfile::tempdir().unwrap();
    touch(tmp.path(), "warp.log.in_session.0");
    touch(tmp.path(), "warp.log.in_session.1");
    touch(tmp.path(), "warp.log.in_session.2");

    migrate_previous_session_in_session_chunks(tmp.path(), "warp.log");

    assert!(tmp.path().join("warp.log.old.0.in_session.0").is_file());
    assert!(tmp.path().join("warp.log.old.0.in_session.1").is_file());
    assert!(tmp.path().join("warp.log.old.0.in_session.2").is_file());
    // Bare .in_session.* slots are free for the new session.
    assert!(!tmp.path().join("warp.log.in_session.0").exists());
    assert!(!tmp.path().join("warp.log.in_session.1").exists());
    assert!(!tmp.path().join("warp.log.in_session.2").exists());
}

#[test]
fn migrate_previous_session_is_a_noop_when_no_in_session_chunks_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let unrelated = touch(tmp.path(), "warp.log");

    migrate_previous_session_in_session_chunks(tmp.path(), "warp.log");

    // Active log untouched; no spurious .old.0.in_session.* files.
    assert!(unrelated.is_file());
    let any_nested = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("warp.log.old.0.in_session.")
        });
    assert!(!any_nested);
}

#[test]
fn migrate_previous_session_ignores_malformed_in_session_filenames() {
    let tmp = tempfile::tempdir().unwrap();
    let real = touch(tmp.path(), "warp.log.in_session.0");
    let bogus_a = touch(tmp.path(), "warp.log.in_session.abc");
    let bogus_b = touch(tmp.path(), "warp.log.in_session.");
    let unrelated = touch(tmp.path(), "warp.log.in_session.0.weird"); // not a usize

    migrate_previous_session_in_session_chunks(tmp.path(), "warp.log");

    assert!(!real.exists()); // moved
    assert!(tmp.path().join("warp.log.old.0.in_session.0").is_file());
    // Malformed entries are left where they are.
    assert!(bogus_a.is_file());
    assert!(bogus_b.is_file());
    assert!(unrelated.is_file());
}

#[test]
fn shift_nested_chunks_renames_old_n_in_session_to_old_n_plus_1() {
    let tmp = tempfile::tempdir().unwrap();
    touch(tmp.path(), "warp.log.old.0.in_session.0");
    touch(tmp.path(), "warp.log.old.0.in_session.1");

    shift_old_session_in_session_chunks(tmp.path(), "warp.log", 0);

    assert!(tmp.path().join("warp.log.old.1.in_session.0").is_file());
    assert!(tmp.path().join("warp.log.old.1.in_session.1").is_file());
    assert!(!tmp.path().join("warp.log.old.0.in_session.0").exists());
    assert!(!tmp.path().join("warp.log.old.0.in_session.1").exists());
}

#[test]
fn shift_nested_chunks_only_touches_the_requested_slot() {
    // Shifting slot 0 must not disturb slot 1's nested chunks.
    let tmp = tempfile::tempdir().unwrap();
    touch(tmp.path(), "warp.log.old.0.in_session.0");
    let slot1_chunk = touch(tmp.path(), "warp.log.old.1.in_session.0");

    shift_old_session_in_session_chunks(tmp.path(), "warp.log", 0);

    assert!(tmp.path().join("warp.log.old.1.in_session.0").is_file());
    assert_eq!(tmp.path().join("warp.log.old.1.in_session.0"), slot1_chunk);
}

#[test]
fn remove_nested_chunks_deletes_every_chunk_of_the_target_slot() {
    let tmp = tempfile::tempdir().unwrap();
    touch(tmp.path(), "warp.log.old.4.in_session.0");
    touch(tmp.path(), "warp.log.old.4.in_session.1");
    let survivor = touch(tmp.path(), "warp.log.old.3.in_session.0");

    remove_old_session_in_session_chunks(tmp.path(), "warp.log", 4);

    assert!(!tmp.path().join("warp.log.old.4.in_session.0").exists());
    assert!(!tmp.path().join("warp.log.old.4.in_session.1").exists());
    // Other slots' chunks are untouched.
    assert!(survivor.is_file());
}

#[test]
fn resolved_active_paths_use_frontend_directory_and_channel_name() {
    let base = PathBuf::from("/tmp/warp-logs");
    let gui = log_state(&base, LogFrontend::Gui, "warp_dev.log");
    let tui = log_state(&base, LogFrontend::Tui, "warp_local.log");
    let cli = log_state(&base, LogFrontend::Cli, "warp_preview.log");

    assert_eq!(gui.log_file_path(), base.join("warp_dev.log"));
    assert_eq!(
        tui.log_file_path(),
        base.join("warp-cli").join("warp_local.log")
    );
    assert_eq!(
        cli.log_file_path(),
        base.join("oz").join("warp_preview.log")
    );
}

#[test]
fn crash_recovery_paths_use_channel_name_in_tui_directory() {
    let tui_dir = PathBuf::from("/tmp/warp-logs/warp-cli");
    assert_eq!(
        temp_log_file_path(&tui_dir, "warp_dev.log"),
        tui_dir.join("warp_dev.log.old.temp")
    );
    assert_eq!(
        crash_recovery_process_log_file_path(&tui_dir, "warp_dev.log"),
        tui_dir.join("warp_dev.log.recovery")
    );
}
