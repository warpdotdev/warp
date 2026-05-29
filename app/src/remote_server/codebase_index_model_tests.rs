use super::*;

fn host() -> HostId {
    HostId::new("host".to_string())
}
fn host_with_name(name: &str) -> HostId {
    HostId::new(name.to_string())
}

fn host_label(label: &str) -> HostLabel {
    HostLabel {
        label: label.to_string(),
    }
}

fn remote_path(repo_path: &str) -> RemotePath {
    remote_path_from_repo_path(&host(), repo_path).unwrap()
}

fn remote_path_for_host(host: &HostId, repo_path: &str) -> RemotePath {
    remote_path_from_repo_path(host, repo_path).unwrap()
}

fn session(id: u64) -> SessionId {
    SessionId::from(id)
}

fn ready_status(repo_path: &str) -> RemoteCodebaseIndexStatus {
    RemoteCodebaseIndexStatus {
        repo_path: repo_path.to_string(),
        state: RemoteCodebaseIndexState::Ready,
        last_updated_epoch_millis: Some(1),
        progress_completed: None,
        progress_total: None,
        failure_message: None,
        root_hash: Some(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        ),
    }
}

fn status_with_state(
    repo_path: &str,
    state: RemoteCodebaseIndexState,
) -> RemoteCodebaseIndexStatus {
    let mut status = ready_status(repo_path);
    status.state = state;
    status
}
fn status_with_path(repo_path: &str) -> RemoteCodebaseIndexStatusWithPath {
    RemoteCodebaseIndexStatusWithPath {
        remote_path: remote_path(repo_path),
        status: ready_status(repo_path),
    }
}

#[test]
fn snapshot_replaces_statuses_for_host() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    model.apply_status_update(remote_path("/old"), ready_status("/old"));
    assert!(model.apply_statuses_snapshot(&host, &[status_with_path("/new")]));

    assert!(model.status_for_repo(&remote_path("/old")).is_none());
    assert!(model.status_for_repo(&remote_path("/new")).is_some());
}

#[test]
fn snapshot_replaces_statuses_for_same_host_label() {
    let mut model = RemoteCodebaseIndexModel::default();
    let old_host = host_with_name("old-daemon");
    let new_host = host_with_name("new-daemon");
    model
        .host_labels
        .insert(old_host.clone(), host_label("user@remote-host"));
    model
        .host_labels
        .insert(new_host.clone(), host_label("user@remote-host"));
    assert!(model.apply_statuses_snapshot(
        &old_host,
        &[RemoteCodebaseIndexStatusWithPath {
            remote_path: remote_path_for_host(&old_host, "/old"),
            status: ready_status("/old"),
        }],
    ));

    assert!(model.apply_statuses_snapshot(
        &new_host,
        &[RemoteCodebaseIndexStatusWithPath {
            remote_path: remote_path_for_host(&new_host, "/new"),
            status: ready_status("/new"),
        }],
    ));

    assert!(model
        .status_for_repo(&remote_path_for_host(&new_host, "/old"))
        .is_none());
    assert!(model
        .status_for_repo(&remote_path_for_host(&new_host, "/new"))
        .is_some());
    assert_eq!(model.entries_for_settings().len(), 1);
}
#[test]
fn status_update_reports_only_actual_changes() {
    let mut model = RemoteCodebaseIndexModel::default();
    let remote_path = remote_path("/repo");
    let status = ready_status("/repo");

    assert!(model.apply_status_update(remote_path.clone(), status.clone()));
    assert!(!model.apply_status_update(remote_path.clone(), status));
    assert!(model.apply_status_update(
        remote_path,
        status_with_state("/repo", RemoteCodebaseIndexState::Stale),
    ));
}

#[test]
fn snapshot_reports_only_actual_changes_for_host() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    let snapshot = [status_with_path("/repo")];

    assert!(model.apply_statuses_snapshot(&host, &snapshot));
    assert!(!model.apply_statuses_snapshot(&host, &snapshot));
    assert!(model.apply_statuses_snapshot(
        &host,
        &[RemoteCodebaseIndexStatusWithPath {
            remote_path: remote_path("/repo"),
            status: status_with_state("/repo", RemoteCodebaseIndexState::Stale),
        }],
    ));
}

#[test]
fn entries_for_settings_are_sorted_by_host_then_path() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host_b = host_with_name("host-b");
    let host_a = host_with_name("host-a");
    model.apply_status_update(
        remote_path_for_host(&host_b, "/z-repo"),
        ready_status("/z-repo"),
    );
    model.apply_status_update(
        remote_path_for_host(&host_a, "/b-repo"),
        ready_status("/b-repo"),
    );
    model.apply_status_update(
        remote_path_for_host(&host_a, "/a-repo"),
        ready_status("/a-repo"),
    );

    let entries = model.entries_for_settings();
    let labels_and_paths = entries
        .iter()
        .map(|entry| (entry.host_label.as_str(), entry.remote_path.path.as_str()))
        .collect::<Vec<_>>();

    assert_eq!(
        labels_and_paths,
        vec![
            ("host-a", "/a-repo"),
            ("host-a", "/b-repo"),
            ("host-b", "/z-repo")
        ]
    );
}

#[test]
fn entries_for_settings_use_host_label_when_available() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    model
        .host_labels
        .insert(host.clone(), host_label("user@ssh-testing"));
    model.apply_status_update(remote_path("/repo"), ready_status("/repo"));

    let entries = model.entries_for_settings();
    assert_eq!(entries[0].host_label, "user@ssh-testing");
}

#[test]
fn entries_for_settings_fall_back_to_host_id_without_label() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    model.apply_status_update(remote_path("/repo"), ready_status("/repo"));

    let entries = model.entries_for_settings();

    assert_eq!(entries[0].host_label, host.to_string());
}

#[test]
fn entries_for_settings_dedupe_same_ssh_host_and_path() {
    let mut model = RemoteCodebaseIndexModel::default();
    let old_host = host_with_name("old-daemon");
    let new_host = host_with_name("new-daemon");
    model
        .host_labels
        .insert(old_host.clone(), host_label("user@remote-host"));
    model
        .host_labels
        .insert(new_host.clone(), host_label("user@remote-host"));
    model.apply_status_update(
        remote_path_for_host(&old_host, "/repo"),
        ready_status("/repo"),
    );
    model.apply_status_update(
        remote_path_for_host(&new_host, "/repo"),
        ready_status("/repo"),
    );

    let entries = model.entries_for_settings();

    assert_eq!(entries.len(), 1);
    assert_eq!(model.statuses.len(), 1);
    assert_eq!(entries[0].host_label, "user@remote-host");
    assert_eq!(entries[0].remote_path.path.as_str(), "/repo");
}

#[test]
fn entries_for_settings_do_not_dedupe_different_ssh_hosts_or_paths() {
    let mut model = RemoteCodebaseIndexModel::default();
    let first_host = host_with_name("first-daemon");
    let second_host = host_with_name("second-daemon");
    model
        .host_labels
        .insert(first_host.clone(), host_label("user@first-host"));
    model
        .host_labels
        .insert(second_host.clone(), host_label("user@second-host"));
    model.apply_status_update(
        remote_path_for_host(&first_host, "/repo"),
        ready_status("/repo"),
    );
    model.apply_status_update(
        remote_path_for_host(&second_host, "/repo"),
        ready_status("/repo"),
    );
    model.apply_status_update(
        remote_path_for_host(&first_host, "/other-repo"),
        ready_status("/other-repo"),
    );

    let entries = model.entries_for_settings();

    assert_eq!(entries.len(), 3);
}

#[test]
fn host_disconnect_marks_settings_entries_unavailable_without_removing_them() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    model.apply_status_update(remote_path("/repo"), ready_status("/repo"));
    model.record_navigated_directory(session(1), &remote_path("/repo"), true);
    assert!(model.mark_host_unavailable(&host));
    assert!(!model.mark_host_unavailable(&host));

    let status = model.status_for_repo(&remote_path("/repo")).unwrap();
    assert_eq!(status.state, RemoteCodebaseIndexState::Unavailable);
    assert_eq!(
        status.failure_message.as_deref(),
        Some("The remote host is currently disconnected.")
    );
    assert_eq!(model.entries_for_settings().len(), 1);
    assert!(matches!(
        model.availability_for_remote(&host, Some("/repo"), None),
        RemoteCodebaseSearchAvailability::Unavailable { .. }
    ));
}

#[test]
fn availability_uses_active_navigated_repo() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    model.record_navigated_directory(session(1), &remote_path("/repo"), true);
    model.apply_status_update(remote_path("/repo"), ready_status("/repo"));

    let availability = model.availability_for_remote(&host, Some("/repo/src"), None);

    assert!(availability.is_ready());
    assert_eq!(availability.repo_path(), Some("/repo"));
}

#[test]
fn availability_uses_active_navigated_non_git_directory() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    model.record_navigated_directory(session(1), &remote_path("/directory"), false);
    model.apply_status_update(remote_path("/directory"), ready_status("/directory"));

    let availability = model.availability_for_remote(&host, Some("/repo/src"), None);

    assert!(availability.is_ready());
    assert_eq!(availability.repo_path(), Some("/directory"));
}

#[test]
fn availability_falls_back_to_longest_status_prefix() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    model.apply_status_update(remote_path("/repo"), ready_status("/repo"));
    model.apply_status_update(remote_path("/repo/nested"), ready_status("/repo/nested"));

    let availability = model.availability_for_remote(&host, Some("/repo/nested/src"), None);

    assert!(availability.is_ready());
    assert_eq!(availability.repo_path(), Some("/repo/nested"));
}

#[test]
fn availability_uses_unmatched_explicit_path_as_not_indexed() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    model.record_navigated_directory(session(1), &remote_path("/workspaces/warp"), true);
    model.apply_status_update(
        remote_path("/workspaces/warp"),
        ready_status("/workspaces/warp"),
    );

    let availability = model.availability_for_remote(
        &host,
        Some("/workspaces/warp"),
        Some("/Users/user/code/warp"),
    );

    assert!(matches!(
        availability,
        RemoteCodebaseSearchAvailability::NotIndexed { .. }
    ));
    assert_eq!(availability.repo_path(), Some("/Users/user/code/warp"));
}

#[test]
fn availability_uses_unknown_explicit_remote_path_as_not_indexed() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    model.record_navigated_directory(session(1), &remote_path("/workspaces/active"), true);
    model.apply_status_update(
        remote_path("/workspaces/active"),
        ready_status("/workspaces/active"),
    );

    let availability =
        model.availability_for_remote(&host, Some("/workspaces/active"), Some("/workspaces/other"));

    assert!(matches!(
        availability,
        RemoteCodebaseSearchAvailability::NotIndexed { .. }
    ));
    assert_eq!(availability.repo_path(), Some("/workspaces/other"));
}

#[test]
fn availability_uses_requested_path_when_it_matches_known_remote_repo() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    model.record_navigated_directory(session(1), &remote_path("/workspaces/other"), true);
    model.apply_status_update(
        remote_path("/workspaces/other"),
        ready_status("/workspaces/other"),
    );
    model.apply_status_update(
        remote_path("/workspaces/warp"),
        ready_status("/workspaces/warp"),
    );

    let availability = model.availability_for_remote(
        &host,
        Some("/workspaces/other"),
        Some("/workspaces/warp/app"),
    );

    assert!(availability.is_ready());
    assert_eq!(availability.repo_path(), Some("/workspaces/warp"));
}

#[test]
fn codebases_for_agent_context_includes_searchable_remote_paths() {
    let mut model = RemoteCodebaseIndexModel::default();
    model.apply_status_update(
        remote_path("/workspaces/warp"),
        ready_status("/workspaces/warp"),
    );
    model.apply_status_update(
        remote_path("/workspaces/stale"),
        status_with_state("/workspaces/stale", RemoteCodebaseIndexState::Stale),
    );

    let entries = model.codebases_for_agent_context(&host());

    assert_eq!(
        entries,
        vec![
            RemoteCodebaseContextEntry {
                name: "stale".to_string(),
                path: "/workspaces/stale".to_string(),
            },
            RemoteCodebaseContextEntry {
                name: "warp".to_string(),
                path: "/workspaces/warp".to_string(),
            },
        ]
    );
}

#[test]
fn codebases_for_agent_context_skips_unsearchable_remote_paths() {
    let mut model = RemoteCodebaseIndexModel::default();
    let mut missing_root_hash = ready_status("/workspaces/missing-root-hash");
    missing_root_hash.root_hash = None;
    model.apply_status_update(
        remote_path("/workspaces/missing-root-hash"),
        missing_root_hash,
    );
    model.apply_status_update(
        remote_path("/workspaces/indexing"),
        status_with_state("/workspaces/indexing", RemoteCodebaseIndexState::Indexing),
    );
    model.apply_status_update(
        remote_path("/workspaces/failed"),
        status_with_state("/workspaces/failed", RemoteCodebaseIndexState::Failed),
    );

    assert!(model.codebases_for_agent_context(&host()).is_empty());
}

#[test]
fn codebases_for_agent_context_only_includes_active_host_paths() {
    let mut model = RemoteCodebaseIndexModel::default();
    let active_host = host_with_name("active-host");
    let other_host = host_with_name("other-host");
    model.apply_status_update(
        remote_path_for_host(&active_host, "/workspaces/active"),
        ready_status("/workspaces/active"),
    );
    model.apply_status_update(
        remote_path_for_host(&other_host, "/workspaces/other"),
        ready_status("/workspaces/other"),
    );

    let entries = model.codebases_for_agent_context(&active_host);

    assert_eq!(
        entries,
        vec![RemoteCodebaseContextEntry {
            name: "active".to_string(),
            path: "/workspaces/active".to_string(),
        }]
    );
}

#[test]
fn clear_remote_codebase_indexing_state_returns_paths_and_removes_client_state() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    model.apply_status_update(
        remote_path("/workspaces/warp"),
        ready_status("/workspaces/warp"),
    );
    model.record_navigated_directory(session(1), &remote_path("/workspaces/warp"), true);

    let remote_paths = model.clear_remote_codebase_indexing_state();

    assert_eq!(remote_paths, vec![remote_path("/workspaces/warp")]);
    assert!(model.entries_for_settings().is_empty());
    assert!(matches!(
        model.availability_for_remote(&host, Some("/workspaces/warp"), None),
        RemoteCodebaseSearchAvailability::NotIndexed { .. }
    ));
}

#[test]
fn active_git_repo_paths_needing_auto_index_skips_searchable_and_indexing_repos_across_hosts() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host_ready = host_with_name("ready-host");
    let host_new = host_with_name("new-host");
    let host_indexing = host_with_name("indexing-host");
    let ready_path = remote_path_for_host(&host_ready, "/ready");
    let new_path = remote_path_for_host(&host_new, "/new");
    let indexing_path = remote_path_for_host(&host_indexing, "/indexing");
    model.record_navigated_directory(session(1), &ready_path, true);
    model.record_navigated_directory(session(2), &new_path, true);
    model.record_navigated_directory(session(3), &indexing_path, true);
    model.apply_status_update(ready_path, ready_status("/ready"));
    model.apply_status_update(
        indexing_path,
        status_with_state("/indexing", RemoteCodebaseIndexState::Indexing),
    );

    let remote_paths = model.active_git_repo_paths_needing_auto_index();

    assert_eq!(remote_paths, vec![new_path]);
}

#[test]
fn resolve_remote_repo_path_falls_back_to_current_remote_cwd_when_no_repo_is_known() {
    let model = RemoteCodebaseIndexModel::default();
    let host = host();

    let remote_path = model.resolve_remote_repo_path(&host, Some("/workspaces/new"), None);

    assert_eq!(
        remote_path.map(|remote_path| remote_path.path.as_str().to_string()),
        Some("/workspaces/new".to_string())
    );
}

#[test]
fn indexing_state_is_not_ready() {
    let mut status = ready_status("/repo");
    status.state = RemoteCodebaseIndexState::Indexing;

    let availability = search_availability_for_status(&status, remote_path("/repo"));

    assert!(matches!(
        availability,
        RemoteCodebaseSearchAvailability::Indexing { .. }
    ));
}

#[test]
fn stale_state_is_ready_and_marked_stale() {
    let mut status = ready_status("/repo");
    status.state = RemoteCodebaseIndexState::Stale;

    let availability = search_availability_for_status(&status, remote_path("/repo"));

    let RemoteCodebaseSearchAvailability::Ready(context) = availability else {
        panic!("Expected stale index to remain searchable");
    };
    assert!(context.is_stale);
}

#[test]
fn known_remote_repo_path_does_not_fall_back_to_unknown_cwd() {
    let model = RemoteCodebaseIndexModel::default();
    let host = host();

    let remote_path = model.resolve_known_remote_repo_path(&host, Some("/workspaces/new"), None);

    assert!(remote_path.is_none());
}

#[test]
fn known_remote_repo_path_finds_indexed_parent_for_cwd() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    model.apply_status_update(
        remote_path("/workspaces/repo"),
        ready_status("/workspaces/repo"),
    );

    let remote_path =
        model.resolve_known_remote_repo_path(&host, Some("/workspaces/repo/src"), None);

    assert_eq!(
        remote_path.map(|remote_path| remote_path.path.as_str().to_string()),
        Some("/workspaces/repo".to_string())
    );
}
#[test]
fn known_remote_repo_path_uses_single_indexed_descendant_for_broad_cwd() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    model.record_navigated_directory(session(1), &remote_path("/workspaces"), false);
    model.apply_status_update(
        remote_path("/workspaces/warp"),
        ready_status("/workspaces/warp"),
    );

    let remote_path = model.resolve_known_remote_repo_path(&host, Some("/workspaces"), None);

    assert_eq!(
        remote_path.map(|remote_path| remote_path.path.as_str().to_string()),
        Some("/workspaces/warp".to_string())
    );
}

#[test]
fn missing_root_hash_is_unavailable() {
    let mut status = ready_status("/repo");
    status.root_hash = None;

    let availability = search_availability_for_status(&status, remote_path("/repo"));

    assert!(matches!(
        availability,
        RemoteCodebaseSearchAvailability::Unavailable { .. }
    ));
}

#[test]
fn auto_index_navigated_git_repo_when_status_is_missing() {
    let model = RemoteCodebaseIndexModel::default();

    assert!(model.should_request_auto_index_for_navigated_git_repo(&remote_path("/repo")));
}

#[test]
fn auto_index_navigated_git_repo_skips_existing_searchable_index() {
    let mut model = RemoteCodebaseIndexModel::default();
    model.apply_status_update(remote_path("/ready"), ready_status("/ready"));
    model.apply_status_update(
        remote_path("/stale"),
        status_with_state("/stale", RemoteCodebaseIndexState::Stale),
    );

    assert!(!model.should_request_auto_index_for_navigated_git_repo(&remote_path("/ready")));
    assert!(!model.should_request_auto_index_for_navigated_git_repo(&remote_path("/stale")));
}

#[test]
fn auto_index_navigated_git_repo_skips_index_already_in_progress() {
    let mut model = RemoteCodebaseIndexModel::default();
    model.apply_status_update(
        remote_path("/queued"),
        status_with_state("/queued", RemoteCodebaseIndexState::Queued),
    );
    model.apply_status_update(
        remote_path("/indexing"),
        status_with_state("/indexing", RemoteCodebaseIndexState::Indexing),
    );

    assert!(!model.should_request_auto_index_for_navigated_git_repo(&remote_path("/queued")));
    assert!(!model.should_request_auto_index_for_navigated_git_repo(&remote_path("/indexing")));
}

#[test]
fn auto_index_navigated_git_repo_when_existing_index_is_unusable() {
    let mut model = RemoteCodebaseIndexModel::default();
    let mut missing_root_hash = ready_status("/missing-root-hash");
    missing_root_hash.root_hash = None;
    model.apply_status_update(remote_path("/missing-root-hash"), missing_root_hash);
    model.apply_status_update(
        remote_path("/failed"),
        status_with_state("/failed", RemoteCodebaseIndexState::Failed),
    );

    assert!(
        model.should_request_auto_index_for_navigated_git_repo(&remote_path("/missing-root-hash"))
    );
    assert!(model.should_request_auto_index_for_navigated_git_repo(&remote_path("/failed")));
}

#[test]
fn active_git_repo_paths_needing_auto_index_includes_missing_active_git_repo() {
    let mut model = RemoteCodebaseIndexModel::default();
    model.record_navigated_directory(session(1), &remote_path("/repo"), true);

    assert_eq!(
        model.active_git_repo_paths_needing_auto_index(),
        vec![remote_path("/repo")]
    );
}

#[test]
fn active_git_repo_paths_needing_auto_index_skips_ready_and_indexing_repos() {
    let mut model = RemoteCodebaseIndexModel::default();
    model.record_navigated_directory(session(1), &remote_path("/ready"), true);
    model.apply_status_update(remote_path("/ready"), ready_status("/ready"));

    let other_host = HostId::new("other-host".to_string());
    let indexing_path = remote_path_from_repo_path(&other_host, "/indexing").unwrap();
    model.record_navigated_directory(session(2), &indexing_path, true);
    model.apply_status_update(
        indexing_path,
        status_with_state("/indexing", RemoteCodebaseIndexState::Indexing),
    );
    assert!(model.active_git_repo_paths_needing_auto_index().is_empty());
}

#[test]
fn active_git_repo_paths_needing_auto_index_includes_each_active_remote_session_repo() {
    let mut model = RemoteCodebaseIndexModel::default();
    model.record_navigated_directory(session(1), &remote_path("/repo-a"), true);
    model.record_navigated_directory(session(2), &remote_path("/repo-b"), true);

    let mut paths = model
        .active_git_repo_paths_needing_auto_index()
        .into_iter()
        .map(|remote_path| remote_path.path.as_str().to_string())
        .collect::<Vec<_>>();
    paths.sort();

    assert_eq!(paths, vec!["/repo-a", "/repo-b"]);
}

#[test]
fn active_git_repo_paths_needing_auto_index_dedupes_matching_session_repos() {
    let mut model = RemoteCodebaseIndexModel::default();
    model.record_navigated_directory(session(1), &remote_path("/repo"), true);
    model.record_navigated_directory(session(2), &remote_path("/repo"), true);
    assert_eq!(
        model.active_git_repo_paths_needing_auto_index(),
        vec![remote_path("/repo")]
    );
}

#[test]
fn non_git_navigation_clears_only_that_sessions_active_git_repo() {
    let mut model = RemoteCodebaseIndexModel::default();

    model.record_navigated_directory(session(1), &remote_path("/repo-a"), true);
    model.record_navigated_directory(session(2), &remote_path("/repo-b"), true);
    model.record_navigated_directory(session(1), &remote_path("/not-git"), false);

    assert_eq!(
        model.active_git_repo_paths_needing_auto_index(),
        vec![remote_path("/repo-b")]
    );
}

#[test]
fn clearing_session_clears_only_that_sessions_active_git_repo() {
    let mut model = RemoteCodebaseIndexModel::default();

    model.record_navigated_directory(session(1), &remote_path("/repo-a"), true);
    model.record_navigated_directory(session(2), &remote_path("/repo-b"), true);
    model.clear_active_git_repo_for_session(session(1));

    assert_eq!(
        model.active_git_repo_paths_needing_auto_index(),
        vec![remote_path("/repo-b")]
    );
}

#[test]
fn remove_host_clears_active_git_repo_for_host() {
    let mut model = RemoteCodebaseIndexModel::default();
    let host = host();
    let other_host = host_with_name("other-host");
    let other_path = remote_path_for_host(&other_host, "/other-repo");
    model.record_navigated_directory(session(1), &remote_path("/repo"), true);
    model.record_navigated_directory(session(2), &other_path, true);
    model.mark_host_unavailable(&host);

    assert_eq!(
        model.active_git_repo_paths_needing_auto_index(),
        vec![other_path]
    );
}
