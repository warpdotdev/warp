//! Tests for [`inherit_share_for_local_child`] and [`recorded_resume_flags`]. These verify the
//! pure branching independent of the PaneGroup dispatch code.

use uuid::Uuid;

use super::*;

fn new_task_id() -> AmbientAgentTaskId {
    Uuid::new_v4().to_string().parse().unwrap()
}

fn user_source(task_id: Option<&str>) -> SharedSessionSource {
    SharedSessionSource::user(task_id.map(str::to_owned))
}

fn ambient_source(task_id: Option<&str>) -> SharedSessionSource {
    SharedSessionSource::ambient_agent(task_id.map(str::to_owned))
}

/// The alias map a shell session reports, in the form detection reads it.
fn shell_aliases(pairs: &[(&str, &str)]) -> HashMap<SmolStr, String> {
    pairs
        .iter()
        .map(|(name, value)| (SmolStr::from(*name), (*value).to_owned()))
        .collect()
}

fn flag(name: &str, value: Option<&str>) -> RecordedFlag {
    RecordedFlag {
        name: name.to_owned(),
        value: value.map(str::to_owned),
    }
}

#[test]
fn recorded_flags_keep_the_allowlisted_flags_of_the_invocation() {
    assert_eq!(
        recorded_resume_flags(
            CLIAgent::Claude,
            "claude --model opus --dangerously-skip-permissions",
            Some(EscapeChar::Backslash),
            None,
        ),
        vec![
            flag("--model", Some("opus")),
            flag("--dangerously-skip-permissions", None),
        ]
    );
}

// KTD5: what gets recorded is the flag set the user actually ran, and an alias is part of that
// invocation — resolvable only now, while the shell session that defines it is alive.
#[test]
fn recorded_flags_include_the_flags_an_alias_carries() {
    let aliases = shell_aliases(&[("c", "claude --permission-mode plan")]);

    assert_eq!(
        recorded_resume_flags(
            CLIAgent::Claude,
            "c --model opus",
            Some(EscapeChar::Backslash),
            Some(&aliases),
        ),
        vec![
            flag("--permission-mode", Some("plan")),
            flag("--model", Some("opus")),
        ],
        "a flag the user only ever typed as an alias is still a flag their session was running \
         with"
    );
}

// The identifier can be reported by a plugin running inside something that is not the agent's own
// command line — a wrapper, or a pane whose foreground command has already moved on. Those
// arguments were never the agent's, so none of them are recorded.
#[test]
fn recorded_flags_are_empty_when_the_command_is_not_the_agent() {
    assert_eq!(
        recorded_resume_flags(
            CLIAgent::Claude,
            "git commit --model opus",
            Some(EscapeChar::Backslash),
            None,
        ),
        Vec::new()
    );
}

// R19/KTD5: the capture reads the obfuscated command text, so a secret in the invocation is
// recorded as its placeholder. That is the intended degradation — the placeholder fails the
// declared value shape when the resume command is built, which drops the flag rather than
// passing a wrong value to the agent.
#[test]
fn recorded_flags_carry_the_obfuscated_placeholder_rather_than_a_secret() {
    let recorded = recorded_resume_flags(
        CLIAgent::Claude,
        "claude --settings ********",
        Some(EscapeChar::Backslash),
        None,
    );

    assert_eq!(recorded, vec![flag("--settings", Some("********"))]);
    assert!(
        ResumeDeclarations::embedded()
            .build_resume_command(
                CLIAgent::Claude,
                "session-1",
                &recorded,
                crate::terminal::cli_agent_resume::PermissionPosture::Carry,
            )
            .is_some_and(|command| !command.contains('*')),
        "an obfuscated value must be dropped when the invocation is built, not replayed"
    );
}

// An agent Warp knows but has not declared resume support for records no flags: there is no
// allowlist to read them against, and a flag carried into an invocation nobody validated is
// exactly what KTD5 rules out.
#[test]
fn recorded_flags_are_empty_for_an_agent_without_resume_declarations() {
    assert!(
        !ResumeDeclarations::embedded().supports(CLIAgent::Gemini),
        "precondition: Gemini declares no resume support"
    );
    assert_eq!(
        recorded_resume_flags(
            CLIAgent::Gemini,
            "gemini --model pro",
            Some(EscapeChar::Backslash),
            None,
        ),
        Vec::new()
    );
}

#[test]
fn inherit_share_returns_no_when_host_is_not_sharing() {
    let result = inherit_share_for_local_child(None, new_task_id());
    assert!(matches!(result, IsSharedSessionCreator::No));
}

#[test]
fn inherit_share_returns_no_when_host_user_share_has_no_task_id() {
    let host = user_source(None);
    let result = inherit_share_for_local_child(Some(&host), new_task_id());
    assert!(
        matches!(result, IsSharedSessionCreator::No),
        "hosts without a stamped task_id must NOT cascade; the viewer cannot enumerate \
         children via REST without a task_id"
    );
}

#[test]
fn inherit_share_returns_no_when_host_ambient_share_has_no_task_id() {
    let host = ambient_source(None);
    let result = inherit_share_for_local_child(Some(&host), new_task_id());
    assert!(matches!(result, IsSharedSessionCreator::No));
}

#[test]
fn inherit_share_cascades_user_source_for_manually_shared_local_orchestrator() {
    let host = user_source(Some("parent-task-id"));
    let child_task_id = new_task_id();
    let expected_child_str = child_task_id.to_string();
    match inherit_share_for_local_child(Some(&host), child_task_id) {
        IsSharedSessionCreator::Yes {
            source:
                SharedSessionSource {
                    source_type: SessionSourceType::User,
                    source_task_id: Some(task_id),
                },
        } => {
            assert_eq!(
                task_id, expected_child_str,
                "the cascaded child must carry its own task_id in the sidecar, not the host's"
            );
        }
        other => panic!(
            "expected IsSharedSessionCreator::Yes with unit User variant carrying child task_id in \
             the sidecar, got {other:?}"
        ),
    }
}

#[test]
fn inherit_share_cascades_ambient_source_for_cloud_orchestrator() {
    let host = ambient_source(Some("parent-task-id"));
    let child_task_id = new_task_id();
    let expected_child_str = child_task_id.to_string();
    match inherit_share_for_local_child(Some(&host), child_task_id) {
        IsSharedSessionCreator::Yes {
            source:
                SharedSessionSource {
                    source_type:
                        SessionSourceType::AmbientAgent {
                            task_id: Some(task_id),
                        },
                    source_task_id,
                },
        } => {
            assert_eq!(task_id, expected_child_str);
            assert_eq!(
                source_task_id.as_deref(),
                Some(expected_child_str.as_str()),
                "the sidecar must mirror the cascaded child's task_id so viewers can read one \
                 field for both `User` and `AmbientAgent` shares"
            );
        }
        other => panic!(
            "expected IsSharedSessionCreator::Yes with AmbientAgent variant carrying child \
             task_id, got {other:?}"
        ),
    }
}
