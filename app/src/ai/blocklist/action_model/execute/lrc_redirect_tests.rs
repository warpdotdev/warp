use std::path::{Path, PathBuf};

use super::{MAX_TRACKED_FILES, parse_redirect_targets};

fn targets(command: &str) -> Vec<String> {
    parse_redirect_targets(command, None)
        .into_iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

fn targets_in(command: &str, cwd: &str) -> Vec<PathBuf> {
    parse_redirect_targets(command, Some(Path::new(cwd)))
}

#[test]
fn parses_stdout_redirect() {
    assert_eq!(targets("make > build.log"), vec!["build.log"]);
}

#[test]
fn parses_append_redirect() {
    assert_eq!(targets("make >> build.log"), vec!["build.log"]);
}

#[test]
fn parses_redirect_without_surrounding_whitespace() {
    assert_eq!(targets("make >build.log"), vec!["build.log"]);
    assert_eq!(targets("make>build.log"), vec!["build.log"]);
}

#[test]
fn parses_stderr_redirect() {
    assert_eq!(targets("cargo build 2> errors.log"), vec!["errors.log"]);
    assert_eq!(targets("cargo build 2>>errors.log"), vec!["errors.log"]);
}

#[test]
fn parses_explicit_stdout_fd_redirect() {
    assert_eq!(targets("cargo build 1> out.log"), vec!["out.log"]);
}

#[test]
fn parses_combined_output_redirect() {
    assert_eq!(targets("make &> all.log"), vec!["all.log"]);
    assert_eq!(targets("make &>> all.log"), vec!["all.log"]);
}

#[test]
fn parses_clobbering_redirect() {
    assert_eq!(targets("make >| build.log"), vec!["build.log"]);
}

#[test]
fn parses_bash_style_merge_redirect() {
    assert_eq!(targets("make >& all.log"), vec!["all.log"]);
}

#[test]
fn ignores_descriptor_duplication() {
    assert!(targets("make > /dev/null 2>&1").is_empty());
}

#[test]
fn ignores_null_and_tty_sinks() {
    assert!(targets("curl -s -o /dev/null https://example.com").is_empty());
    assert!(targets("make > /dev/stdout").is_empty());
    assert!(targets("make > /dev/tty").is_empty());
}

#[test]
fn parses_multiple_redirects_in_one_command() {
    assert_eq!(
        targets("cargo build > out.log 2> err.log"),
        vec!["out.log", "err.log"]
    );
}

#[test]
fn parses_redirects_across_pipeline_stages() {
    assert_eq!(
        targets("make 2> err.log | grep warning > warnings.txt"),
        vec!["err.log", "warnings.txt"]
    );
}

#[test]
fn parses_tee_targets() {
    assert_eq!(targets("make | tee build.log"), vec!["build.log"]);
    assert_eq!(targets("make | tee -a build.log"), vec!["build.log"]);
    assert_eq!(
        targets("make | tee build.log copy.log"),
        vec!["build.log", "copy.log"]
    );
    assert_eq!(targets("make | /usr/bin/tee build.log"), vec!["build.log"]);
}

#[test]
fn tee_targets_do_not_leak_past_a_separator() {
    assert_eq!(
        targets("make | tee build.log; cargo test something"),
        vec!["build.log"]
    );
}

#[test]
fn parses_output_flags() {
    assert_eq!(
        targets("curl -o archive.tar.gz https://example.com"),
        vec!["archive.tar.gz"]
    );
    assert_eq!(
        targets("wget -O archive.tar.gz https://example.com"),
        vec!["archive.tar.gz"]
    );
    assert_eq!(
        targets("curl --output archive.tar.gz https://example.com"),
        vec!["archive.tar.gz"]
    );
    assert_eq!(targets("rsync --log-file=sync.log a b"), vec!["sync.log"]);
    assert_eq!(targets("rsync --log-file sync.log a b"), vec!["sync.log"]);
}

#[test]
fn deduplicates_repeated_targets() {
    assert_eq!(targets("make > build.log 2>> build.log"), vec!["build.log"]);
}

#[test]
fn caps_the_number_of_tracked_files() {
    let command = "cmd > a.log > b.log > c.log > d.log > e.log > f.log";
    assert_eq!(targets(command).len(), MAX_TRACKED_FILES);
}

#[test]
fn keeps_quoted_paths_intact() {
    assert_eq!(targets("make > \"my build.log\""), vec!["my build.log"]);
    assert_eq!(targets("make > 'my build.log'"), vec!["my build.log"]);
    assert_eq!(targets("make > my\\ build.log"), vec!["my build.log"]);
}

#[test]
fn skips_targets_that_cannot_be_resolved_statically() {
    assert!(targets("make > $LOGFILE").is_empty());
    assert!(targets("make > logs/*.log").is_empty());
    assert!(targets("make > `date`.log").is_empty());
}

#[test]
fn ignores_input_redirects() {
    assert!(targets("sort < input.txt").is_empty());
    assert!(targets("cat <<EOF").is_empty());
}

#[test]
fn resolves_relative_paths_against_the_block_working_directory() {
    assert_eq!(
        targets_in("make > build.log", "/home/user/project"),
        vec![PathBuf::from("/home/user/project/build.log")]
    );
    assert_eq!(
        targets_in("make > logs/build.log", "/home/user/project"),
        vec![PathBuf::from("/home/user/project/logs/build.log")]
    );
}

#[test]
fn leaves_absolute_paths_untouched() {
    assert_eq!(
        targets_in("make > /tmp/build.log", "/home/user/project"),
        vec![PathBuf::from("/tmp/build.log")]
    );
}

#[test]
fn returns_nothing_for_commands_without_output_targets() {
    assert!(targets("cargo build").is_empty());
    assert!(targets("python3 -c 'import time; time.sleep(60)'").is_empty());
    assert!(targets("").is_empty());
}
