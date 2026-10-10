use std::io;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_lite::future;
use futures_lite::io::AsyncRead;

use super::{
    CAPPED_READ_CHUNK_SIZE, CappedGitOutput, CappedReadOutcome, WslGitCommand, build_wslenv,
    read_capped, read_two_capped, run_git_command, run_git_command_capped,
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

/// Regression guard for the stack-overflow bug this fix addresses: a
/// `[u8; CAPPED_READ_CHUNK_SIZE]` stack array crossing an `.await` point
/// used to make this future ~129KB, all embedded inline in every caller's
/// future up through the real app's diff-loading task — large enough to
/// overflow a worker thread's stack deep in a real UI integration test,
/// even though the same code never overflowed a shallow unit test's stack.
/// Heap-allocating the chunk buffer keeps the future small; this asserts it
/// stays that way rather than silently regressing.
#[test]
fn run_git_command_capped_future_stays_small() {
    let repo = Path::new("/tmp");
    let fut = run_git_command_capped(repo, &["status"], 10);
    let size = std::mem::size_of_val(&fut);
    assert!(
        size < 4096,
        "run_git_command_capped's future grew to {size} bytes; a large stack buffer \
         crossing an .await point here can overflow a real worker thread's stack \
         even though it's invisible in unit tests (see APP-5462)"
    );
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

#[test]
fn capped_command_returns_complete_output_under_budget() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    let contents = b"hello world\n".repeat(10);
    init_repo_with_staged_file(repo_dir.path(), "file.txt", &contents);

    let output = future::block_on(run_git_command_capped(
        repo_dir.path(),
        &["show", ":file.txt"],
        contents.len() + 1,
    ))
    .expect("run_git_command_capped should succeed under budget");

    match output {
        CappedGitOutput::Complete(text) => assert_eq!(text.as_bytes(), contents.as_slice()),
        CappedGitOutput::Exceeded => panic!("expected Complete output under budget"),
    }
}

#[test]
fn capped_command_reports_exceeded_over_budget_without_full_payload() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    let contents = vec![b'a'; 10_000];
    init_repo_with_staged_file(repo_dir.path(), "big.txt", &contents);

    let output = future::block_on(run_git_command_capped(
        repo_dir.path(),
        &["show", ":big.txt"],
        1_000,
    ))
    .expect("run_git_command_capped should succeed even when the budget is exceeded");

    assert!(matches!(output, CappedGitOutput::Exceeded));
}

#[test]
fn capped_command_preserves_git_error_semantics() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    future::block_on(run_git_command(repo_dir.path(), &["init", "-q"])).expect("git init");

    // No such path has ever been staged, so `git show` exits non-zero with no
    // stdout — the capped path must classify this as an error exactly like
    // the unbounded `run_git_command`, not as a successful empty capture.
    let result = future::block_on(run_git_command_capped(
        repo_dir.path(),
        &["show", ":missing.txt"],
        1_000,
    ));

    assert!(result.is_err());
}

/// A misbehaving `textconv` diff driver runs even with `--no-ext-diff` (which
/// only disables `diff.<driver>.command`, not `.gitattributes`-based
/// `textconv`), and can write unbounded output to stderr — inherited
/// straight through from the driver subprocess to git's own stderr, which is
/// exactly what `run_git_command_capped` pipes and reads. Reproduces the
/// reviewer's finding against the exact diff invocation `get_file_diff` uses.
#[cfg(unix)]
#[test]
fn capped_command_terminates_on_stderr_overflow_from_a_diff_driver() {
    use std::os::unix::fs::PermissionsExt;

    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    let repo_path = repo_dir.path();

    future::block_on(async {
        run_git_command(repo_path, &["init", "-q"])
            .await
            .expect("git init");
        run_git_command(repo_path, &["config", "user.email", "test@test.com"])
            .await
            .expect("git config email");
        run_git_command(repo_path, &["config", "user.name", "Test"])
            .await
            .expect("git config name");
    });

    // A driver that floods stderr well past STDERR_CAPTURE_CAP before
    // printing the (irrelevant) converted content to stdout.
    let script_path = repo_path.join("noisy-textconv.sh");
    std::fs::write(
        &script_path,
        "#!/bin/sh\nhead -c 1100000 /dev/zero | tr '\\0' 'e' >&2\ncat \"$1\"\n",
    )
    .expect("write textconv script");
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
        .expect("make textconv script executable");
    std::fs::write(repo_path.join(".gitattributes"), "*.noisy diff=noisy\n")
        .expect("write gitattributes");
    std::fs::write(repo_path.join("file.noisy"), "before\n").expect("write initial file");
    future::block_on(async {
        run_git_command(
            repo_path,
            &[
                "config",
                "diff.noisy.textconv",
                &script_path.to_string_lossy(),
            ],
        )
        .await
        .expect("git config textconv");
        run_git_command(repo_path, &["add", "file.noisy", ".gitattributes"])
            .await
            .expect("git add");
        run_git_command(repo_path, &["commit", "-q", "-m", "initial"])
            .await
            .expect("git commit");
    });
    std::fs::write(repo_path.join("file.noisy"), "after\n").expect("modify file");

    let result = future::block_on(run_git_command_capped(
        repo_path,
        &[
            "diff",
            "--no-ext-diff",
            "--patch-with-raw",
            "-z",
            "--no-color",
            "HEAD",
            "--",
            "file.noisy",
        ],
        1_000_000,
    ));

    // Must terminate with an error rather than hang waiting for the
    // driver's stderr to close, or succeed with a silently truncated
    // capture that drops the overflow.
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
fn read_capped_never_reads_past_budget_plus_one_chunk() {
    // A 3-chunk budget needs a 4th read to detect the overflow (3 chunks
    // lands exactly at the budget, which is not yet "exceeded"). The probe
    // reader fails the test if `read_capped` reads even one byte beyond that
    // 4th chunk, proving the stop is incremental rather than post-hoc.
    let max_bytes = CAPPED_READ_CHUNK_SIZE * 3;
    let expected_total_read = CAPPED_READ_CHUNK_SIZE * 4;
    let mut reader = BoundedProbeReader {
        total_read: 0,
        fail_after: expected_total_read + 1,
    };

    let outcome = future::block_on(read_capped(&mut reader, max_bytes))
        .expect("read_capped must stop before the probe reader's failure boundary");

    assert!(matches!(outcome, CappedReadOutcome::Exceeded(_)));
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
/// a misbehaving descendant (e.g. a `textconv` helper) that keeps a pipe
/// open and keeps writing to it indefinitely.
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
fn read_two_capped_stops_as_soon_as_either_pipe_exceeds_its_budget() {
    // `first` never produces data and never closes, like a still-running
    // child's stdout with nothing left to say yet; `second` produces data
    // forever without ever reaching EOF, like an unbounded stderr writer.
    // Before bounding both pipes, waiting for both to finish meant a
    // still-open pipe like `first` could hang the whole read forever even
    // after `second` blew its budget — this proves that no longer happens,
    // and that it resolves without ever needing `first` to make progress.
    let outcome = future::block_on(read_two_capped(
        PendingForever,
        10,
        RepeatingReader,
        CAPPED_READ_CHUNK_SIZE * 3,
    ))
    .expect("read_two_capped should not error");

    let (first_outcome, second_outcome) = outcome;
    assert!(first_outcome.is_none());
    assert!(matches!(
        second_outcome,
        Some(CappedReadOutcome::Exceeded(_))
    ));
}
