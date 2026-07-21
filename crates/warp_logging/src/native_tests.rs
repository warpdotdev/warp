use std::io::Write;

use warp_core::channel::{Channel, ChannelState};

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

#[test]
fn tui_frontend_maps_all_channels_to_dedicated_filenames() {
    assert_eq!(
        logfile_name_for_frontend(LogFrontend::Tui, Channel::Dev),
        "warp_tui_dev.log"
    );
    assert_eq!(
        logfile_name_for_frontend(LogFrontend::Tui, Channel::Local),
        "warp_tui_dev.log"
    );
    assert_eq!(
        logfile_name_for_frontend(LogFrontend::Tui, Channel::Preview),
        "warp_tui_preview.log"
    );
    for channel in [Channel::Stable, Channel::Oss, Channel::Integration] {
        assert_eq!(
            logfile_name_for_frontend(LogFrontend::Tui, channel),
            "warp_tui.log"
        );
    }
}

#[test]
fn frontend_directory_selection_keeps_gui_and_cli_paths_unchanged() {
    let base = PathBuf::from("/tmp/warp-logs");
    assert_eq!(
        log_directory_for_frontend(base.clone(), LogFrontend::Gui),
        base
    );
    assert_eq!(
        log_directory_for_frontend(base.clone(), LogFrontend::Cli),
        PathBuf::from("/tmp/warp-logs/oz")
    );
    assert_eq!(
        log_directory_for_frontend(base, LogFrontend::Tui),
        PathBuf::from("/tmp/warp-logs/tui")
    );
}

#[test]
fn tui_bundle_collection_ignores_legacy_oz_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let active = touch(tmp.path(), "warp_tui_preview.log");
    let rotated = touch(tmp.path(), "warp_tui_preview.log.old.0");
    let current_chunk = touch(tmp.path(), "warp_tui_preview.log.in_session.0");
    let legacy = tmp.path().join("oz");
    fs::create_dir(&legacy).unwrap();
    touch(&legacy, "warp_preview.log");

    let paths = collect_log_paths_in(tmp.path(), "warp_tui_preview.log").unwrap();

    assert_eq!(paths, vec![active, current_chunk, rotated]);
    assert!(!paths.iter().any(|path| path.starts_with(&legacy)));
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

// --- CODE-1902: LOG_STATE consumer regression (spec validation criteria #3–#6, #10–#11) ---
// The public LOG_STATE consumers (`log_file_path`, `rotate_log_files`,
// `on_parent_process_crash`, `on_crash_recovery_process_killed`, and
// `create_log_bundle_zip` — the API `/view-logs` calls) now delegate to the
// `_in` seams below. These tests drive those seams with an explicit
// (log_directory, logfile_name) so the resolved-frontend path/rotation/crash/
// bundle behavior is exercised deterministically without initializing the
// global logger (which is a OnceLock and can only be set once per process).

#[test]
fn resolved_active_path_uses_tui_directory_and_mapped_name() {
    // log_file_path() = main_process_log_file_path(state.log_directory, state.logfile_name).
    // A resolved TUI LOG_STATE (dev channel) must report <tui_dir>/warp_tui_dev.log,
    // not the GUI channel name or the legacy oz directory.
    let tui_dir = PathBuf::from("/tmp/warp-logs/tui");
    assert_eq!(
        main_process_log_file_path(&tui_dir, "warp_tui_dev.log"),
        tui_dir.join("warp_tui_dev.log")
    );
    assert_eq!(
        main_process_log_file_path(&tui_dir, "warp_tui.log"),
        tui_dir.join("warp_tui.log")
    );
}

#[test]
fn resolved_active_path_keeps_gui_name_and_base_directory() {
    // GUI LOG_STATE resolves to the base directory with ChannelState::logfile_name();
    // no tui/oz suffix is appended.
    let base = PathBuf::from("/tmp/warp-logs");
    assert_eq!(
        main_process_log_file_path(&base, "warp_dev.log"),
        base.join("warp_dev.log")
    );
}

#[test]
fn resolved_active_path_keeps_cli_oz_directory_and_channel_name() {
    let oz_dir = PathBuf::from("/tmp/warp-logs/oz");
    assert_eq!(
        main_process_log_file_path(&oz_dir, "warp_dev.log"),
        oz_dir.join("warp_dev.log")
    );
}

#[test]
fn gui_and_cli_frontend_names_still_delegate_to_channel_state_logfile_name() {
    // GUI and CLI must keep using ChannelState::logfile_name() — the frontend
    // seam must not hand them a TUI-style name. The TUI name must differ from
    // the GUI/CLI name, which is the whole point of CODE-1902.
    let channel_name = ChannelState::logfile_name().to_string();
    assert_eq!(
        logfile_name_for_frontend(LogFrontend::Gui, Channel::Dev),
        channel_name
    );
    assert_eq!(
        logfile_name_for_frontend(LogFrontend::Cli, Channel::Dev),
        channel_name.clone()
    );
    assert_ne!(
        logfile_name_for_frontend(LogFrontend::Tui, Channel::Dev),
        channel_name
    );
}

#[test]
fn rotate_files_in_uses_resolved_tui_name_for_startup_rotation() {
    let tmp = tempfile::tempdir().unwrap();
    // Previous session left a .old.temp; existing .old.0/.old.1 must shift up.
    touch(tmp.path(), "warp_tui_dev.log.old.temp");
    touch(tmp.path(), "warp_tui_dev.log.old.0");
    touch(tmp.path(), "warp_tui_dev.log.old.1");

    rotate_files_in(tmp.path(), "warp_tui_dev.log", 5).unwrap();

    assert!(tmp.path().join("warp_tui_dev.log.old.0").is_file()); // temp -> old.0
    assert!(tmp.path().join("warp_tui_dev.log.old.1").is_file()); // old.0 -> old.1
    assert!(tmp.path().join("warp_tui_dev.log.old.2").is_file()); // old.1 -> old.2
    assert!(!tmp.path().join("warp_tui_dev.log.old.temp").exists());
    // GUI/CLI names must not appear from a TUI rotation.
    assert!(!tmp.path().join("warp.log.old.0").exists());
    assert!(!tmp.path().join("warp_dev.log.old.0").exists());
}

#[test]
fn rotate_files_in_keeps_gui_channel_name_for_gui_rotation() {
    let tmp = tempfile::tempdir().unwrap();
    touch(tmp.path(), "warp_dev.log.old.temp");
    touch(tmp.path(), "warp_dev.log.old.0");

    rotate_files_in(tmp.path(), "warp_dev.log", 5).unwrap();

    assert!(tmp.path().join("warp_dev.log.old.0").is_file());
    assert!(tmp.path().join("warp_dev.log.old.1").is_file());
    assert!(!tmp.path().join("warp_tui_dev.log.old.0").exists());
}

#[test]
fn on_parent_process_crash_in_uses_resolved_tui_name() {
    let tmp = tempfile::tempdir().unwrap();
    touch(tmp.path(), "warp_tui_dev.log");
    touch(tmp.path(), "warp_tui_dev.log.recovery");

    on_parent_process_crash_in(tmp.path(), "warp_tui_dev.log");

    assert!(tmp.path().join("warp_tui_dev.log.old.temp").is_file()); // active -> temp
    assert!(tmp.path().join("warp_tui_dev.log").is_file()); // recovery -> active
    assert!(!tmp.path().join("warp_tui_dev.log.recovery").exists());
}

#[test]
fn on_crash_recovery_process_killed_in_removes_tui_recovery_sidecar() {
    let tmp = tempfile::tempdir().unwrap();
    touch(tmp.path(), "warp_tui_dev.log.recovery");

    on_crash_recovery_process_killed_in(tmp.path(), "warp_tui_dev.log");

    assert!(!tmp.path().join("warp_tui_dev.log.recovery").exists());
}

#[test]
fn crash_recovery_paths_use_resolved_frontend_name() {
    let tui_dir = PathBuf::from("/tmp/warp-logs/tui");
    assert_eq!(
        temp_log_file_path(&tui_dir, "warp_tui_dev.log"),
        tui_dir.join("warp_tui_dev.log.old.temp")
    );
    assert_eq!(
        crash_recovery_process_log_file_path(&tui_dir, "warp_tui_dev.log"),
        tui_dir.join("warp_tui_dev.log.recovery")
    );
    // GUI keeps its existing name in the same helpers.
    let gui_dir = PathBuf::from("/tmp/warp-logs");
    assert_eq!(
        temp_log_file_path(&gui_dir, "warp_dev.log"),
        gui_dir.join("warp_dev.log.old.temp")
    );
}

#[test]
fn create_log_bundle_zip_in_uses_resolved_tui_stem_and_excludes_legacy_oz_logs() {
    let tmp = tempfile::tempdir().unwrap();
    let active = touch(tmp.path(), "warp_tui_dev.log");
    write_bytes(&active, b"active tui session");
    let rotated = touch(tmp.path(), "warp_tui_dev.log.old.0");
    write_bytes(&rotated, b"previous tui session");
    let chunk = touch(tmp.path(), "warp_tui_dev.log.in_session.0");
    write_bytes(&chunk, b"mid-session tui chunk");
    // Legacy oz-directory GUI log must NOT be bundled with the TUI bundle.
    let legacy = tmp.path().join("oz");
    fs::create_dir(&legacy).unwrap();
    touch(&legacy, "warp_dev.log");

    let zip_path = create_log_bundle_zip_in(tmp.path(), "warp_tui_dev.log").unwrap();

    // Zip lives beside the TUI logs and uses the resolved TUI stem.
    assert_eq!(zip_path.parent(), Some(tmp.path()));
    let zip_name = zip_path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        zip_name.starts_with("warp_tui_dev-"),
        "zip name was {zip_name}"
    );
    assert!(zip_name.ends_with(".zip"));

    // Zip contains exactly the TUI fixtures in collection order — no legacy GUI log.
    let entries = zip_entry_names(&zip_path);
    assert_eq!(
        entries,
        vec![
            "warp_tui_dev.log",
            "warp_tui_dev.log.in_session.0",
            "warp_tui_dev.log.old.0",
        ]
    );
}

#[test]
fn create_log_bundle_zip_in_uses_gui_channel_stem_and_ignores_tui_files() {
    // GUI bundle must keep using ChannelState::logfile_name() (e.g. warp_dev.log),
    // not a TUI name, and ignore TUI files sitting in the same directory.
    let tmp = tempfile::tempdir().unwrap();
    touch(tmp.path(), "warp_dev.log");
    touch(tmp.path(), "warp_dev.log.old.0");
    touch(tmp.path(), "warp_tui_dev.log");

    let zip_path = create_log_bundle_zip_in(tmp.path(), "warp_dev.log").unwrap();
    let zip_name = zip_path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(zip_name.starts_with("warp_dev-"), "zip name was {zip_name}");

    let entries = zip_entry_names(&zip_path);
    assert_eq!(entries, vec!["warp_dev.log", "warp_dev.log.old.0"]);
}

#[test]
fn create_log_bundle_zip_in_uses_cli_oz_stem() {
    // CLI keeps the oz directory and channel filename; its bundle stem is the
    // channel name, never a TUI name.
    let tmp = tempfile::tempdir().unwrap();
    touch(tmp.path(), "warp_dev.log");
    touch(tmp.path(), "warp_dev.log.old.0");

    let zip_path = create_log_bundle_zip_in(tmp.path(), "warp_dev.log").unwrap();
    let zip_name = zip_path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(zip_name.starts_with("warp_dev-"), "zip name was {zip_name}");
    assert!(!zip_name.contains("warp_tui"));
}
