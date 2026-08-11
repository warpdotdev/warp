use warpui::App;

use super::*;
use crate::search::command_palette::agent_sessions::tiers::CONTENT_ROW_TIER;
use crate::terminal::CLIAgent;

fn hit(task_name: &str) -> ContentHit {
    ContentHit {
        agent: CLIAgent::Claude,
        session_id: format!("session-{task_name}"),
        project_name: "warp".to_owned(),
        task_name: task_name.to_owned(),
        cwd: "/repos/warp".to_owned(),
        snippet: format!("…the {task_name} snippet…"),
        snippet_match_indices: vec![5, 6, 7],
        partial: false,
    }
}

fn data_source(query: &str, task_names: &[&str]) -> ContentDataSource {
    let mut data_source = ContentDataSource::new();
    data_source.set_results(
        query.to_owned(),
        task_names.iter().copied().map(hit).collect(),
    );
    data_source
}

#[test]
fn hits_for_another_query_are_not_served() {
    App::test((), |app| async move {
        // The user has typed past what the digest has answered. Anything served
        // here would be rows that do not contain what is in the search box.
        let data_source = data_source("deadlock", &["Fix the deadlock"]);
        let results = app.read(|app| {
            data_source
                .run_query(&Query::from("deadlocks"), app)
                .unwrap()
        });

        assert!(
            results.is_empty(),
            "a published hit only answers the exact query it was found for"
        );
    })
}

#[test]
fn hits_for_the_current_query_are_headed_by_the_content_separator() {
    App::test((), |app| async move {
        let data_source = data_source("deadlock", &["Fix the deadlock", "Terminal model lock"]);
        let results = app.read(|app| {
            data_source
                .run_query(&Query::from("deadlock"), app)
                .unwrap()
        });

        let separators: Vec<_> = results
            .iter()
            .filter(|result| result.is_static_separator())
            .collect();
        assert_eq!(separators.len(), 1, "exactly one header per section");
        assert_eq!(separators[0].priority_tier(), CONTENT_SEPARATOR_TIER);

        let rows: Vec<_> = results
            .iter()
            .filter(|result| !result.is_static_separator())
            .collect();
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|row| row.priority_tier() == CONTENT_ROW_TIER),
            "every content hit belongs to the content tier"
        );
        assert!(
            rows[0].score() > rows[1].score(),
            "corpus order is the ranking, so scores must descend with it"
        );
    })
}

#[test]
fn an_empty_query_serves_nothing_even_when_hits_are_published() {
    App::test((), |app| async move {
        // The zero state is "every session you have", by name. A content
        // section there would be answering a question nobody asked.
        let data_source = data_source("", &["Fix the deadlock"]);
        let results = app.read(|app| data_source.run_query(&Query::from(""), app).unwrap());

        assert!(results.is_empty());
    })
}
