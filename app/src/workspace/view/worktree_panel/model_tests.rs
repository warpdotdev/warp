use std::path::{Path, PathBuf};

use super::*;

const PORCELAIN: &str = "worktree /code/app
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main

worktree /code/app/.worktrees/feature-x
HEAD 2222222222222222222222222222222222222222
branch refs/heads/feature/x

worktree /trees/app/detached
HEAD 3333333333333333333333333333333333333333
detached
";

fn repository() -> Repository {
    Repository::from_worktrees(parse_worktree_porcelain(PORCELAIN)).expect("has a main worktree")
}

fn agent(name: &str, state: AgentState) -> AgentActivity {
    AgentActivity {
        name: name.to_owned(),
        state,
    }
}

#[test]
fn parses_main_linked_and_detached_worktrees() {
    let worktrees = parse_worktree_porcelain(PORCELAIN);
    assert_eq!(
        worktrees,
        vec![
            Worktree {
                path: PathBuf::from("/code/app"),
                branch: Some("main".to_owned()),
                is_main: true,
            },
            Worktree {
                path: PathBuf::from("/code/app/.worktrees/feature-x"),
                branch: Some("feature/x".to_owned()),
                is_main: false,
            },
            Worktree {
                path: PathBuf::from("/trees/app/detached"),
                branch: None,
                is_main: false,
            },
        ]
    );
}

#[test]
fn skips_bare_main_repository() {
    let output = "worktree /code/app.git\nbare\n\nworktree /trees/app/main\nHEAD 1\nbranch refs/heads/main\n";
    let worktrees = parse_worktree_porcelain(output);
    assert_eq!(worktrees.len(), 1);
    assert!(!worktrees[0].is_main);
    assert_eq!(Repository::from_worktrees(worktrees), None);
}

#[test]
fn repository_is_named_after_main_worktree() {
    let repository = repository();
    assert_eq!(repository.root, PathBuf::from("/code/app"));
    assert_eq!(repository.name, "app");
}

#[test]
fn detached_worktree_displays_directory_name() {
    let repository = repository();
    assert_eq!(repository.worktrees[2].display_name(), "detached");
    assert_eq!(repository.worktrees[1].display_name(), "feature/x");
}

#[test]
fn nested_worktree_wins_over_main_worktree() {
    let repositories = vec![repository()];
    assert_eq!(
        worktree_containing(
            Path::new("/code/app/.worktrees/feature-x/src"),
            &repositories
        ),
        Some(Path::new("/code/app/.worktrees/feature-x"))
    );
    assert_eq!(
        worktree_containing(Path::new("/code/app/src"), &repositories),
        Some(Path::new("/code/app"))
    );
    assert_eq!(
        worktree_containing(Path::new("/elsewhere"), &repositories),
        None
    );
}

#[test]
fn aggregates_agents_and_tabs_per_worktree() {
    let repositories = vec![repository()];
    let terminals = vec![
        TerminalActivity {
            tab_index: 0,
            cwd: PathBuf::from("/code/app"),
            agent: Some(agent("Claude Code", AgentState::Idle)),
        },
        TerminalActivity {
            tab_index: 0,
            cwd: PathBuf::from("/code/app/crates"),
            agent: Some(agent("Codex", AgentState::Working)),
        },
        TerminalActivity {
            tab_index: 2,
            cwd: PathBuf::from("/trees/app/detached"),
            agent: None,
        },
    ];
    let activity = aggregate_activity(&terminals, &repositories);

    let main = &activity[Path::new("/code/app")];
    assert_eq!(main.tab_indices, vec![0]);
    assert_eq!(main.state(), Some(AgentState::Working));
    assert_eq!(main.summary().as_deref(), Some("2 agents · Working"));

    let detached = &activity[Path::new("/trees/app/detached")];
    assert_eq!(detached.tab_indices, vec![2]);
    assert_eq!(detached.state(), None);
    assert_eq!(detached.summary(), None);
}

#[test]
fn needs_input_outranks_working() {
    let activity = WorktreeActivity {
        agents: vec![
            agent("Codex", AgentState::Working),
            agent("Claude Code", AgentState::NeedsInput),
        ],
        tab_indices: vec![1],
    };
    assert_eq!(activity.state(), Some(AgentState::NeedsInput));
}

#[test]
fn single_agent_summary_uses_agent_name() {
    let activity = WorktreeActivity {
        agents: vec![agent("Claude Code", AgentState::Idle)],
        tab_indices: vec![0],
    };
    assert_eq!(activity.summary().as_deref(), Some("Claude Code · Idle"));
}
