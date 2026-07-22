use super::*;

fn valid_agent_toml() -> &'static str {
    r#"
name = "Morning repo brief"
schedule = "0 9 * * 1-5"
cwd = "~/code/warp"

[runner]
type = "warp_agent"
prompt = "Summarize commits on main from the last 24h."
"#
}

#[test]
fn parses_valid_warp_agent_automation() {
    let automation = LocalAutomation::parse(valid_agent_toml()).unwrap();
    assert_eq!(automation.name, "Morning repo brief");
    assert!(automation.enabled, "enabled should default to true");
    assert_eq!(automation.schedule, "0 9 * * 1-5");
    assert_eq!(automation.cwd.as_deref(), Some("~/code/warp"));
    assert_eq!(
        automation.runner,
        LocalAutomationRunner::WarpAgent {
            prompt: "Summarize commits on main from the last 24h.".to_string()
        }
    );
    assert_eq!(automation.timeout_seconds, None);
    assert!(automation.env.is_empty());
    assert_eq!(automation.source_path, None);
}

#[test]
fn parses_valid_shell_automation_with_optional_fields() {
    let toml = r#"
name = "PR sweep"
enabled = false
schedule = "@daily"
cwd = "/tmp"
timeout_seconds = 1800

[runner]
type = "shell"
command = "gh pr list --author @me"

[env]
FOO = "bar"
"#;
    let automation = LocalAutomation::parse(toml).unwrap();
    assert!(!automation.enabled);
    assert_eq!(automation.timeout_seconds, Some(1800));
    assert_eq!(automation.env.get("FOO").map(String::as_str), Some("bar"));
    assert_eq!(
        automation.runner,
        LocalAutomationRunner::Shell {
            command: "gh pr list --author @me".to_string()
        }
    );
}

#[test]
fn parses_worktree_automation() {
    let toml = r#"
name = "Nightly bug hunt"
schedule = "0 2 * * *"

[worktree]
repo = "~/code/warp"
name = "automation-bug-hunt"
base_branch = "main"

[runner]
type = "warp_agent"
prompt = "Find and fix one flaky test."
"#;
    let automation = LocalAutomation::parse(toml).unwrap();
    let worktree = automation.worktree.as_ref().unwrap();
    assert_eq!(worktree.repo, "~/code/warp");
    assert_eq!(worktree.name, "automation-bug-hunt");
    assert_eq!(worktree.base_branch.as_deref(), Some("main"));
}

#[test]
fn rejects_unknown_top_level_fields() {
    let toml = r#"
name = "Bad"
schedule = "@daily"
cwd = "/tmp"
mystery_field = true

[runner]
type = "shell"
command = "true"
"#;
    let error = LocalAutomation::parse(toml).unwrap_err();
    assert!(error.contains("mystery_field"), "unexpected error: {error}");
}

#[test]
fn rejects_missing_schedule() {
    let toml = r#"
name = "No schedule"
cwd = "/tmp"

[runner]
type = "shell"
command = "true"
"#;
    let error = LocalAutomation::parse(toml).unwrap_err();
    assert!(error.contains("schedule"), "unexpected error: {error}");
}

#[test]
fn rejects_missing_runner() {
    let toml = r#"
name = "No runner"
schedule = "@daily"
cwd = "/tmp"
"#;
    let error = LocalAutomation::parse(toml).unwrap_err();
    assert!(error.contains("runner"), "unexpected error: {error}");
}

#[test]
fn rejects_agent_runner_without_prompt() {
    let toml = r#"
name = "No prompt"
schedule = "@daily"
cwd = "/tmp"

[runner]
type = "warp_agent"
"#;
    let error = LocalAutomation::parse(toml).unwrap_err();
    assert!(error.contains("prompt"), "unexpected error: {error}");
}

#[test]
fn rejects_shell_runner_with_empty_command() {
    let toml = r#"
name = "Empty command"
schedule = "@daily"
cwd = "/tmp"

[runner]
type = "shell"
command = "  "
"#;
    let error = LocalAutomation::parse(toml).unwrap_err();
    assert!(
        error.contains("runner.command"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_both_cwd_and_worktree() {
    let toml = r#"
name = "Both dirs"
schedule = "@daily"
cwd = "/tmp"

[worktree]
repo = "~/code/warp"
name = "wt"

[runner]
type = "shell"
command = "true"
"#;
    let error = LocalAutomation::parse(toml).unwrap_err();
    assert!(error.contains("not both"), "unexpected error: {error}");
}

#[test]
fn rejects_neither_cwd_nor_worktree() {
    let toml = r#"
name = "No dir"
schedule = "@daily"

[runner]
type = "shell"
command = "true"
"#;
    let error = LocalAutomation::parse(toml).unwrap_err();
    assert!(
        error.contains("'cwd' or '[worktree]'"),
        "unexpected error: {error}"
    );
}

#[test]
fn bad_schedule_string_still_parses() {
    // Slice A treats the schedule as an opaque string; a bogus cron
    // expression must not fail loading.
    let toml = r#"
name = "Weird schedule"
schedule = "whenever I feel like it"
cwd = "/tmp"

[runner]
type = "shell"
command = "true"
"#;
    assert!(LocalAutomation::parse(toml).is_ok());
}

#[cfg(feature = "local_fs")]
mod resolution {
    use super::*;

    #[test]
    fn resolves_existing_cwd() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut automation = LocalAutomation::parse(valid_agent_toml()).unwrap();
        automation.cwd = Some(temp_dir.path().display().to_string());
        let resolved = automation.resolve_working_directory().unwrap();
        assert_eq!(resolved, temp_dir.path());
    }

    #[test]
    fn missing_cwd_fails_with_concrete_error() {
        let mut automation = LocalAutomation::parse(valid_agent_toml()).unwrap();
        automation.cwd = Some("/nonexistent/automation/dir".to_string());
        let error = automation.resolve_working_directory().unwrap_err();
        assert!(
            error.contains("does not exist"),
            "unexpected error: {error}"
        );
        assert!(
            error.contains("/nonexistent/automation/dir"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn worktree_with_non_git_repo_fails() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut automation = LocalAutomation::parse(valid_agent_toml()).unwrap();
        automation.cwd = None;
        automation.worktree = Some(LocalAutomationWorktree {
            repo: temp_dir.path().display().to_string(),
            name: "wt".to_string(),
            base_branch: None,
        });
        let error = automation.resolve_working_directory().unwrap_err();
        assert!(
            error.contains("not a git repository"),
            "unexpected error: {error}"
        );
    }
}
