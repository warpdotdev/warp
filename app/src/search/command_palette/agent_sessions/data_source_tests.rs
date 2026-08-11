use warpui::App;

use super::*;
use crate::search::command_palette::agent_sessions::candidate::CandidateOrigin;
use crate::search::command_palette::agent_sessions::tiers::NAME_ROW_TIER;
use crate::terminal::CLIAgent;

fn candidate(task_name: &str) -> AgentSessionCandidate {
    AgentSessionCandidate {
        agent: CLIAgent::Claude,
        session_id: format!("session-{task_name}"),
        project_name: "warp".to_owned(),
        task_name: task_name.to_owned(),
        cwd: "/repos/warp".to_owned(),
        origin: CandidateOrigin::Handle,
        last_active: None,
    }
}

fn data_source(task_names: &[&str]) -> DataSource {
    let mut data_source = DataSource::new();
    data_source.set_candidates(task_names.iter().copied().map(candidate).collect());
    data_source
}

#[test]
fn a_query_that_matches_nothing_returns_no_results_at_all() {
    App::test((), |app| async move {
        let data_source = data_source(&["Rename the rail"]);
        let results = app.read(|app| {
            data_source
                .run_query(&Query::from("kubernetes"), app)
                .unwrap()
        });

        assert!(
            results.is_empty(),
            "a lone section header would caption an empty section and suppress \
             the palette's \"No results found\" placeholder"
        );
    })
}

#[test]
fn results_are_headed_by_the_names_separator() {
    App::test((), |app| async move {
        let data_source = data_source(&["Rename the rail", "Rewrite the mixer"]);
        let results = app.read(|app| data_source.run_query(&Query::from("re"), app).unwrap());

        let separators: Vec<_> = results
            .iter()
            .filter(|result| result.is_static_separator())
            .collect();
        assert_eq!(separators.len(), 1, "exactly one header per section");
        assert_eq!(separators[0].priority_tier(), NAME_SEPARATOR_TIER);
        assert!(
            results
                .iter()
                .filter(|result| !result.is_static_separator())
                .all(|result| result.priority_tier() == NAME_ROW_TIER),
            "every session row belongs to the name tier"
        );
    })
}

#[test]
fn the_empty_query_orders_rows_newest_first() {
    App::test((), |app| async move {
        // `merge` hands the source a newest-first list, so position is the
        // recency ranking and scores must descend with it.
        let data_source = data_source(&["newest", "middle", "oldest"]);
        let results = app.read(|app| data_source.run_query(&Query::from(""), app).unwrap());

        let scores: Vec<_> = results
            .iter()
            .filter(|result| !result.is_static_separator())
            .map(|result| result.score())
            .collect();
        assert_eq!(scores.len(), 3);
        assert!(
            scores[0] > scores[1] && scores[1] > scores[2],
            "expected strictly descending scores, got {scores:?}"
        );
    })
}
