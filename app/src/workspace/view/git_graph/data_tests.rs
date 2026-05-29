//! [`super`]（数据层）解析纯函数的单元测试。

use super::*;

/// 按 [`LOG_FORMAT`] 的字段顺序拼出一条提交记录（含尾部记录分隔符）。
fn rec(
    hash: &str,
    parents: &str,
    author_name: &str,
    author_email: &str,
    author_time: &str,
    decorate: &str,
    subject: &str,
) -> String {
    format!(
        "{hash}{US}{parents}{US}{author_name}{US}{author_email}{US}{author_time}{US}{decorate}{US}{subject}{RS}",
        US = UNIT_SEP,
        RS = RECORD_SEP,
    )
}

#[test]
fn parse_commit_log_parses_single_linear_commit() {
    let input = rec(
        "abcdef1234567890",
        "0987654321fedcba",
        "Ada Lovelace",
        "ada@example.com",
        "1700000000",
        "",
        "Initial work",
    );
    let commits = parse_commit_log(&input);

    assert_eq!(commits.len(), 1);
    let c = &commits[0];
    assert_eq!(c.hash, "abcdef1234567890");
    assert_eq!(c.short_hash, "abcdef1");
    assert_eq!(c.parents, vec!["0987654321fedcba".to_string()]);
    assert_eq!(c.author_name, "Ada Lovelace");
    assert_eq!(c.author_email, "ada@example.com");
    assert_eq!(c.author_time, 1_700_000_000);
    assert_eq!(c.subject, "Initial work");
    assert!(c.refs.is_empty());
}

#[test]
fn parse_commit_log_handles_multiple_records_joined_by_newline() {
    // git 在记录之间用换行连接；解析需容忍前导换行。
    let input = format!(
        "{}\n{}",
        rec("h1", "h2", "A", "a@x", "100", "", "second"),
        rec("h2", "", "A", "a@x", "90", "", "first"),
    );
    let commits = parse_commit_log(&input);

    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].hash, "h1");
    assert_eq!(commits[1].hash, "h2");
    // 根提交无父。
    assert!(commits[1].parents.is_empty());
}

#[test]
fn parse_commit_log_parses_merge_parents() {
    let input = rec("m1", "p1 p2 p3", "A", "a@x", "100", "", "octopus merge");
    let commits = parse_commit_log(&input);

    assert_eq!(commits.len(), 1);
    assert_eq!(
        commits[0].parents,
        vec!["p1".to_string(), "p2".to_string(), "p3".to_string()]
    );
}

#[test]
fn parse_commit_log_keeps_subject_with_commas_and_spaces() {
    let input = rec(
        "h1",
        "h2",
        "A",
        "a@x",
        "100",
        "",
        "fix: handle a, b, and c edge cases",
    );
    let commits = parse_commit_log(&input);

    assert_eq!(commits[0].subject, "fix: handle a, b, and c edge cases");
}

#[test]
fn parse_commit_log_skips_blank_input() {
    assert!(parse_commit_log("").is_empty());
    assert!(parse_commit_log("\n\n").is_empty());
}

#[test]
fn parse_decorate_empty_yields_no_refs() {
    assert!(parse_decorate("").is_empty());
    assert!(parse_decorate("   ").is_empty());
}

#[test]
fn parse_decorate_head_pointing_to_branch() {
    let refs = parse_decorate("HEAD -> refs/heads/main");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].kind, RefKind::Head);
    assert_eq!(refs[0].name, "main");
}

#[test]
fn parse_decorate_detached_head() {
    let refs = parse_decorate("HEAD, refs/remotes/origin/main");
    assert_eq!(refs[0].kind, RefKind::Head);
    assert_eq!(refs[0].name, "HEAD");
    assert_eq!(refs[1].kind, RefKind::RemoteBranch);
    assert_eq!(refs[1].name, "origin/main");
}

#[test]
fn parse_decorate_mixed_kinds() {
    let refs = parse_decorate(
        "HEAD -> refs/heads/main, refs/remotes/origin/main, refs/tags/v1.0.0",
    );
    assert_eq!(refs.len(), 3);
    assert_eq!((refs[0].kind, refs[0].name.as_str()), (RefKind::Head, "main"));
    assert_eq!(
        (refs[1].kind, refs[1].name.as_str()),
        (RefKind::RemoteBranch, "origin/main")
    );
    assert_eq!((refs[2].kind, refs[2].name.as_str()), (RefKind::Tag, "v1.0.0"));
}

#[test]
fn parse_decorate_local_branch_with_slash() {
    let refs = parse_decorate("refs/heads/feature/login");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].kind, RefKind::LocalBranch);
    assert_eq!(refs[0].name, "feature/login");
}

#[test]
fn parse_decorate_hides_remote_symbolic_head() {
    // origin/HEAD 这类符号引用对历史浏览无意义，应被过滤。
    let refs = parse_decorate("refs/remotes/origin/HEAD, refs/remotes/origin/main");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].name, "origin/main");
}

#[test]
fn parse_decorate_ignores_unknown_tokens() {
    let refs = parse_decorate("grafted, HEAD -> refs/heads/main");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].name, "main");
}

#[test]
fn parse_commit_detail_extracts_header_and_files() {
    let input = format!(
        "Bob Committer{US}1700000050{US}Subject line\n\nBody paragraph.{RS}\n\
         3\t1\tsrc/main.rs\n0\t0\tREADME.md\n-\t-\tlogo.png\n",
        US = UNIT_SEP,
        RS = RECORD_SEP,
    );
    let detail = parse_commit_detail(&input);

    assert_eq!(detail.committer_name, "Bob Committer");
    assert_eq!(detail.committer_time, 1_700_000_050);
    // 完整信息保留标题 + 正文。
    assert_eq!(detail.message, "Subject line\n\nBody paragraph.");
    assert_eq!(detail.files.len(), 3);
    assert_eq!(
        detail.files[0],
        ChangedFile {
            path: "src/main.rs".to_string(),
            additions: 3,
            deletions: 1,
        }
    );
    // 二进制文件（"-"）按 0 增删处理。
    assert_eq!(
        detail.files[2],
        ChangedFile {
            path: "logo.png".to_string(),
            additions: 0,
            deletions: 0,
        }
    );
}

#[test]
fn parse_commit_detail_handles_empty_numstat() {
    let input = format!("Ann{US}100{US}msg{RS}", US = UNIT_SEP, RS = RECORD_SEP);
    let detail = parse_commit_detail(&input);

    assert_eq!(detail.message, "msg");
    assert!(detail.files.is_empty());
}
