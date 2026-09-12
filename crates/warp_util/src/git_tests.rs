use std::io;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_lite::future;
use futures_lite::io::AsyncRead;

use super::{
    BOUNDED_READ_CHUNK_SIZE, BoundedGitOutput, BoundedReadOutcome, WslGitCommand, build_wslenv,
    read_bounded, read_two_bounded, run_git_command, run_git_command_bounded,
    translate_for_wsl_unc_cwd,
};

/// Initializes a git repo at `repo_path` with `file_name` staged (but not
/// committed) containing `contents`, so `git show :<file_name>` returns the
/// exact staged blob content with no diff formatting overhead.
fn init_repo_with_staged_file(repo_path: &Path, file_name: &str, contents: &[u8]) {
    future::block_on(async {
        run_git_command(repo_path, &["init", "-q"])
            .await
            .expect("git init");
        std::fs::write(repo_path.join(file_name), contents).expect("write staged file");
        run_git_command(repo_path, &["add", file_name])
            .await
            .expect("git add");
    });
}

/// Translates a git command in `cwd`, asserting that the working directory qualified for the WSL
/// rewrite.
fn translate(args: &[&str], cwd: &str, env: &[(&str, &str)]) -> WslGitCommand {
    translate_for_wsl_unc_cwd(args, Path::new(cwd), env).expect("expected translation")
}

/// Regression guard mirroring `run_git_command_capped`'s own future-size
/// test (see APP-5462): a large stack buffer crossing an `.await` point
/// makes this future's stack footprint scale with the buffer size, which
/// can overflow a real worker thread's stack even though it's invisible
/// in a shallow unit test. `read_bounded` already heap-allocates its chunk
/// buffer (mirroring `read_capped`'s fix); this asserts the future stays
/// small rather than silently regressing back to a stack array.
#[test]
fn run_git_command_bounded_future_stays_small() {
    let repo = Path::new("/tmp");
    let fut = run_git_command_bounded(repo, &["status"], 10);
    let size = std::mem::size_of_val(&fut);
    assert!(
        size < 4096,
        "run_git_command_bounded's future grew to {size} bytes; a large stack buffer \
         crossing an .await point here can overflow a real worker thread's stack \
         even though it's invisible in unit tests (see APP-5462)"
    );
}

#[test]
fn bounded_command_returns_complete_output_under_budget() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    let contents = b"hello world\n".repeat(10);
    init_repo_with_staged_file(repo_dir.path(), "file.txt", &contents);

    let output = future::block_on(run_git_command_bounded(
        repo_dir.path(),
        &["show", ":file.txt"],
        contents.len() + 1,
    ))
    .expect("run_git_command_bounded should succeed under budget");

    match output {
        BoundedGitOutput::Complete(text) => assert_eq!(text.as_bytes(), contents.as_slice()),
        BoundedGitOutput::Exceeded(_) => panic!("expected Complete output under budget"),
    }
}

#[test]
fn bounded_command_reports_exceeded_with_partial_prefix_over_budget() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    // NUL-delimited records, like `git status ... -z`, so the test can
    // assert the partial text still contains only whole records.
    let record = b"?deleted-record\0";
    let contents: Vec<u8> = record.repeat(1_000);
    init_repo_with_staged_file(repo_dir.path(), "big.txt", &contents);

    let budget = record.len() * 10;
    let output = future::block_on(run_git_command_bounded(
        repo_dir.path(),
        &["show", ":big.txt"],
        budget,
    ))
    .expect("run_git_command_bounded should succeed even when the budget is exceeded");

    match output {
        BoundedGitOutput::Exceeded(text) => {
            assert!(
                text.len() <= budget + BOUNDED_READ_CHUNK_SIZE,
                "partial text of {} bytes exceeds the budget-plus-one-chunk bound",
                text.len()
            );
            // Every complete record up to the cut is still intact and
            // parseable — the read stopped on a byte cap, not by corrupting
            // the payload.
            let complete_records = text.matches("?deleted-record\0").count();
            assert!(complete_records > 0);
            assert_eq!(
                complete_records * record.len(),
                text.rfind('\0').map(|i| i + 1).unwrap_or(0)
            );
        }
        BoundedGitOutput::Complete(_) => panic!("expected Exceeded output over budget"),
    }
}

#[test]
fn bounded_command_preserves_git_error_semantics() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    future::block_on(run_git_command(repo_dir.path(), &["init", "-q"])).expect("git init");

    // No such path has ever been staged, so `git show` exits non-zero with
    // no stdout — the bounded path must classify this as an error exactly
    // like the unbounded `run_git_command`, not as a successful empty
    // capture.
    let result = future::block_on(run_git_command_bounded(
        repo_dir.path(),
        &["show", ":missing.txt"],
        1_000,
    ));

    assert!(result.is_err());
}

/// An `AsyncRead` that never reaches EOF and errors if driven past
/// `fail_after` total bytes — the point at which the reader under test must
/// have already stopped. Proves a cap is enforced incrementally during the
/// read itself, rather than by checking a fully-buffered length afterward
/// (the bug in APP-5462): a buggy implementation that read to EOF before
/// comparing lengths would drive this reader past the boundary and fail.
struct BoundedProbeReader {
    total_read: usize,
    fail_after: usize,
}

impl AsyncRead for BoundedProbeReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        if self.total_read >= self.fail_after {
            return Poll::Ready(Err(io::Error::other(
                "BoundedProbeReader read past the expected cap boundary",
            )));
        }
        let n = buf.len();
        buf.fill(b'x');
        self.total_read += n;
        Poll::Ready(Ok(n))
    }
}

#[test]
fn read_bounded_never_reads_past_budget_plus_one_chunk() {
    // A 3-chunk budget needs a 4th read to detect the overflow (3 chunks
    // lands exactly at the budget, which is not yet "exceeded"). The probe
    // reader fails the test if `read_bounded` reads even one byte beyond
    // that 4th chunk, proving the stop is incremental rather than post-hoc.
    let max_bytes = BOUNDED_READ_CHUNK_SIZE * 3;
    let expected_total_read = BOUNDED_READ_CHUNK_SIZE * 4;
    let mut reader = BoundedProbeReader {
        total_read: 0,
        fail_after: expected_total_read + 1,
    };

    let outcome = future::block_on(read_bounded(&mut reader, max_bytes))
        .expect("read_bounded must stop before the probe reader's failure boundary");

    assert!(matches!(outcome, BoundedReadOutcome::Exceeded(_)));
    assert_eq!(reader.total_read, expected_total_read);
}

/// An `AsyncRead` that never produces data and never reaches EOF — models a
/// live child process whose stdout has nothing left to write yet.
struct PendingForever;

impl AsyncRead for PendingForever {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Pending
    }
}

/// An `AsyncRead` that produces data forever and never reaches EOF — models
/// a misbehaving descendant that keeps a pipe open and keeps writing to it
/// indefinitely.
struct RepeatingReader;

impl AsyncRead for RepeatingReader {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        buf.fill(b'e');
        Poll::Ready(Ok(buf.len()))
    }
}

#[test]
fn read_two_bounded_stops_as_soon_as_either_pipe_exceeds_its_budget() {
    // `first` never produces data and never closes, like a still-running
    // child's stdout with nothing left to say yet; `second` produces data
    // forever without ever reaching EOF, like an unbounded writer. Before
    // bounding both pipes, waiting for both to finish meant a still-open
    // pipe like `first` could hang the whole read forever even after
    // `second` blew its budget — this proves that no longer happens, and
    // that it resolves without ever needing `first` to make progress.
    let outcome = future::block_on(read_two_bounded(
        PendingForever,
        10,
        RepeatingReader,
        BOUNDED_READ_CHUNK_SIZE * 3,
    ))
    .expect("read_two_bounded should not error");

    let (first_outcome, second_outcome) = outcome;
    assert!(first_outcome.is_none());
    assert!(matches!(
        second_outcome,
        Some(BoundedReadOutcome::Exceeded(_))
    ));
}

#[test]
fn translates_git_in_unc_cwd() {
    let translated = translate(&["status", "--short"], r"\\wsl$\Ubuntu\home\user\repo", &[]);

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "status",
            "--short",
        ]
    );
    assert_eq!(translated.wslenv, "");
}

#[test]
fn does_not_translate_non_unc_cwd() {
    assert_eq!(
        translate_for_wsl_unc_cwd(&["status"], Path::new(r"C:\Users\user\repo"), &[]),
        None
    );
    assert_eq!(
        translate_for_wsl_unc_cwd(&["status"], Path::new("/home/user/repo"), &[]),
        None
    );
}

#[test]
fn rewrites_same_distro_unc_argument_to_linux_path() {
    let translated = translate(
        &["-C", r"\\wsl$\Ubuntu\home\user\other"],
        r"\\wsl$\Ubuntu\home\user\repo",
        &[],
    );

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "-C",
            "/home/user/other",
        ]
    );
}

#[test]
fn rewrites_argument_with_case_insensitive_distro_match() {
    let translated = translate(
        &["-C", r"\\wsl$\ubuntu\home\user\other"],
        r"\\wsl$\Ubuntu\home\user\repo",
        &[],
    );

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "-C",
            "/home/user/other",
        ]
    );
}

#[test]
fn leaves_other_distro_unc_argument_unchanged() {
    let other = r"\\wsl$\Debian\home\user\other";
    let translated = translate(&["-C", other], r"\\wsl$\Ubuntu\home\user\repo", &[]);

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/home/user/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "-C",
            other,
        ]
    );
}

#[test]
fn build_wslenv_excludes_path_case_insensitively() {
    assert_eq!(
        build_wslenv(&[("PATH", "/usr/bin"), ("GIT_OPTIONAL_LOCKS", "0")]),
        "GIT_OPTIONAL_LOCKS/u"
    );
    assert_eq!(
        build_wslenv(&[("Path", "/usr/bin"), ("GIT_AUTHOR_NAME", "Ada")]),
        "GIT_AUTHOR_NAME/u"
    );
    assert_eq!(build_wslenv(&[("path", "/usr/bin")]), "");
    assert_eq!(build_wslenv(&[]), "");
}

#[test]
fn builds_wslenv_from_env_keys() {
    let translated = translate(
        &["commit"],
        r"\\wsl$\Ubuntu\repo",
        &[("GIT_AUTHOR_NAME", "Ada"), ("GIT_OPTIONAL_LOCKS", "0")],
    );

    assert_eq!(translated.wslenv, "GIT_AUTHOR_NAME/u:GIT_OPTIONAL_LOCKS/u");
}

#[test]
fn omits_wslenv_when_no_env_keys() {
    let translated = translate(&["status"], r"\\wsl$\Ubuntu\repo", &[]);

    assert_eq!(translated.wslenv, "");
}

#[test]
fn carries_explicit_path_through_argv() {
    let translated = translate(
        &["commit"],
        r"\\wsl$\Ubuntu\repo",
        &[("PATH", "/usr/local/bin:/usr/bin")],
    );

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/repo",
            "--exec",
            "/usr/bin/env",
            "PATH=/usr/local/bin:/usr/bin",
            "git",
            "commit",
        ]
    );
    assert_eq!(translated.wslenv, "");
}

#[test]
fn carries_case_insensitive_path_through_argv() {
    let translated = translate(&["status"], r"\\wsl$\Ubuntu\repo", &[("Path", "/opt/bin")]);

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/repo",
            "--exec",
            "/usr/bin/env",
            "PATH=/opt/bin",
            "git",
            "status",
        ]
    );
}

#[test]
fn routes_through_login_shell_when_no_path() {
    let translated = translate(
        &["status"],
        r"\\wsl$\Ubuntu\repo",
        &[("GIT_OPTIONAL_LOCKS", "0")],
    );

    assert_eq!(
        translated.args,
        [
            "--distribution",
            "Ubuntu",
            "--cd",
            "/repo",
            "--exec",
            "/bin/sh",
            "-lc",
            r#"exec git "$@""#,
            "git",
            "status",
        ]
    );
    assert_eq!(translated.wslenv, "GIT_OPTIONAL_LOCKS/u");
}
