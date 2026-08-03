use warp_util::standardized_path::StandardizedPath;
use warpui_core::App;

use super::*;
use crate::StandingQueryContent;

fn path(path: &str) -> StandardizedPath {
    StandardizedPath::try_new(path).unwrap()
}

#[test]
fn snapshot_and_incremental_update_maintain_remote_standing_results() {
    App::test((), |mut app| async move {
        let model = app.add_model(RemoteRepoMetadataModel::new);
        let host = HostId::new("remote-host".to_string());
        let repo_path = path("/repo");
        let skill = StandingQueryContent::file(path("/repo/.agents/skills/review/SKILL.md"));
        let rule = StandingQueryContent::file(path("/repo/WARP.md"));
        let id = RemoteRepositoryIdentifier::new(host.clone(), repo_path.clone());
        let snapshot = RepoMetadataUpdate {
            repo_path: repo_path.clone(),
            remove_entries: Vec::new(),
            update_entries: Vec::new(),
            standing_results_delta: StandingQueryResultsDelta {
                upserted_project_skills: vec![skill.clone()],
                ..Default::default()
            },
        };
        model.update(&mut app, |model, ctx| {
            model.insert_from_snapshot(host.clone(), &snapshot, ctx);
        });

        let incremental = RepoMetadataUpdate {
            repo_path,
            remove_entries: Vec::new(),
            update_entries: Vec::new(),
            standing_results_delta: StandingQueryResultsDelta {
                removed_project_skills: vec![skill],
                upserted_project_rules: vec![rule.clone()],
                ..Default::default()
            },
        };
        model.update(&mut app, |model, ctx| {
            model.apply_incremental_update(&host, &incremental, ctx);
        });

        model.read(&app, |model, _ctx| {
            let results = model.standing_query_results(&id).unwrap();
            assert!(results.project_skills().next().is_none());
            assert!(results.project_rules().any(|content| content == &rule));
        });
    });
}
