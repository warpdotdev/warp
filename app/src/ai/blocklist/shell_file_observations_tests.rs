use std::io::Write;

use tempfile::NamedTempFile;
use warpui::App;

use super::*;

fn parse(command: &str) -> Option<ShellFileObservation> {
    parse_shell_file_observation(command)
}

fn read(path: &str) -> Option<ShellFileObservation> {
    Some(ShellFileObservation::WholeFileRead {
        path: path.to_owned(),
    })
}

fn write_target(path: &str) -> Option<ShellFileObservation> {
    Some(ShellFileObservation::Write {
        path: path.to_owned(),
    })
}

#[test]
fn parses_plain_cat_as_whole_file_read() {
    assert_eq!(parse("cat foo.txt"), read("foo.txt"));
    assert_eq!(parse("cat -n src/main.rs"), read("src/main.rs"));
    assert_eq!(parse("cat 'my file.txt'"), read("my file.txt"));
    assert_eq!(parse("cat /app/run.py"), read("/app/run.py"));
}

#[test]
fn rejects_reads_that_are_not_a_single_whole_file_dump() {
    assert_eq!(parse("cat a.txt b.txt"), None);
    assert_eq!(parse("cat"), None);
    assert_eq!(parse("cat $FILE"), None);
    assert_eq!(parse("cat *.txt"), None);
    assert_eq!(parse("ls foo.txt"), None);
    assert_eq!(parse("cat foo.txt | head -5"), None);
    assert_eq!(parse("cat foo.txt\nls"), None);
    // A piped `cat` is not a read; `cat a > b`'s output goes to `b`, so it
    // parses as a write of `b` (and only confirms if content invariants hold).
    assert_eq!(parse("cat a.txt > b.txt"), write_target("b.txt"));
}

#[test]
fn parses_single_target_writes() {
    assert_eq!(
        parse("cat > run.py << 'EOF'\nprint(1)\nEOF"),
        write_target("run.py")
    );
    assert_eq!(
        parse("printf 'hello\\n' > out.txt"),
        write_target("out.txt")
    );
    assert_eq!(parse("echo hi | tee out.txt"), write_target("out.txt"));
    assert_eq!(
        parse("rm -f x.py && cat > x.py << 'PYEOF'\nprint(1)\nPYEOF"),
        write_target("x.py")
    );
    // Stderr redirects are ignored; the stdout target still counts.
    assert_eq!(parse("cmd > f.txt 2>/dev/null"), write_target("f.txt"));
}

#[test]
fn rejects_writes_with_unknowable_final_content() {
    // Appends: the final content includes bytes this command never saw.
    assert_eq!(parse("echo hi >> log.txt"), None);
    assert_eq!(parse("echo hi | tee -a log.txt"), None);
    // Multiple targets.
    assert_eq!(parse("echo hi | tee a.txt b.txt"), None);
    assert_eq!(parse("cmd > a.txt > b.txt"), None);
    // Combined stdout+stderr redirect.
    assert_eq!(parse("cmd &> f.txt"), None);
    // Expansions in the target path.
    assert_eq!(parse("echo hi > $OUT"), None);
    // Multi-line command without a heredoc.
    assert_eq!(parse("echo hi > f.txt\nls"), None);
    // No stdout target at all.
    assert_eq!(parse("cmd 2> err.log"), None);
}

#[test]
fn read_confirmation_requires_output_to_match_disk() {
    let observation = ShellFileObservation::WholeFileRead {
        path: "f".to_owned(),
    };
    assert!(confirms_disk_content(
        &observation,
        "cat f",
        "alpha\nbeta",
        "alpha\nbeta\n"
    ));
    assert!(confirms_disk_content(
        &observation,
        "cat f",
        "alpha\r\nbeta\r\n",
        "alpha\nbeta\n"
    ));
    // Truncated or transformed output is not a whole-file observation.
    assert!(!confirms_disk_content(
        &observation,
        "cat f",
        "alpha",
        "alpha\nbeta\n"
    ));
    // Empty files are never credited.
    assert!(!confirms_disk_content(&observation, "cat f", "", ""));
}

#[test]
fn write_confirmation_requires_model_authored_content() {
    let observation = ShellFileObservation::Write {
        path: "f".to_owned(),
    };
    // Heredoc: the disk content appears verbatim in the body.
    assert!(confirms_disk_content(
        &observation,
        "cat > f << 'EOF'\nprint(1)\nEOF",
        "",
        "print(1)\n"
    ));
    // Tee: the disk content came back out as command output.
    assert!(confirms_disk_content(
        &observation,
        "echo hi | tee f",
        "hi",
        "hi\n"
    ));
    // Computed content (`ls > f`) matches neither and is not credited.
    assert!(!confirms_disk_content(
        &observation,
        "ls > f",
        "",
        "a.txt\nb.txt\n"
    ));
    // Nor when the computed content coincides with the redirect target, which
    // searching the whole command text rather than the heredoc body would miss.
    assert!(!confirms_disk_content(
        &observation,
        "ls > out.txt",
        "",
        "out.txt\n"
    ));
    // Redirected literals are not credited either: only a heredoc body counts
    // as spelled-out content, and the bytes never came back as output.
    assert!(!confirms_disk_content(
        &observation,
        "echo hi > f",
        "",
        "hi\n"
    ));
}

#[test]
fn credits_whole_file_cat_read() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| ObservedFileContents::default());
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        writeln!(&mut temp_file, "alpha\nbeta").unwrap();
        let path = temp_file.path().to_string_lossy().to_string();
        let conversation_id = AIConversationId::new();

        app.update(|ctx| {
            credit_command_file_observations(
                conversation_id,
                &format!("cat {path}"),
                "alpha\nbeta",
                &None,
                &None,
                ctx,
            )
        });

        let observed =
            app.read(|ctx| ObservedFileContents::as_ref(ctx).snapshot(Some(conversation_id)));
        assert!(observed.contains(&path, ContentFingerprint::of("alpha\nbeta\n")));
    });
}

#[test]
fn does_not_credit_mismatched_read_output() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| ObservedFileContents::default());
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        writeln!(&mut temp_file, "alpha\nbeta").unwrap();
        let path = temp_file.path().to_string_lossy().to_string();
        let conversation_id = AIConversationId::new();

        app.update(|ctx| {
            credit_command_file_observations(
                conversation_id,
                &format!("cat {path}"),
                "alpha",
                &None,
                &None,
                ctx,
            )
        });

        let observed =
            app.read(|ctx| ObservedFileContents::as_ref(ctx).snapshot(Some(conversation_id)));
        assert!(!observed.contains(&path, ContentFingerprint::of("alpha\nbeta\n")));
    });
}

#[test]
fn credits_heredoc_write() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| ObservedFileContents::default());
        let mut temp_file = NamedTempFile::new().expect("Failed to create temporary file");
        writeln!(&mut temp_file, "print(1)").unwrap();
        let path = temp_file.path().to_string_lossy().to_string();
        let conversation_id = AIConversationId::new();

        app.update(|ctx| {
            credit_command_file_observations(
                conversation_id,
                &format!("cat > {path} << 'EOF'\nprint(1)\nEOF"),
                "",
                &None,
                &None,
                ctx,
            )
        });

        let observed =
            app.read(|ctx| ObservedFileContents::as_ref(ctx).snapshot(Some(conversation_id)));
        assert!(observed.contains(&path, ContentFingerprint::of("print(1)\n")));
    });
}
