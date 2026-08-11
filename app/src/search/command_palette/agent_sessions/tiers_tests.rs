use ordered_float::OrderedFloat;
use warpui::elements::Empty;
use warpui::{App, AppContext, Element};

use super::*;
use crate::appearance::Appearance;
use crate::search::SearchItem;
use crate::search::command_palette::agent_sessions::candidate::{
    AgentSessionCandidate, CandidateOrigin,
};
use crate::search::command_palette::agent_sessions::content_data_source::ContentDataSource;
use crate::search::command_palette::agent_sessions::data_source::DataSource;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::separator_search_item::SeparatorSearchItem;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::SyncDataSource;
use crate::search::result_renderer::ItemHighlightState;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::transcript_digest::ContentHit;

/// A stand-in row for whichever tier a test needs one in — including the
/// content tier, which Phase 1 has no producer for yet. Ordering has to be
/// assertable before the rows exist, because the failure it guards against
/// (a tier scheme that renders upside down) is invisible until they do.
#[derive(Debug)]
struct TieredRow {
    label: String,
    priority_tier: u8,
    score: f64,
}

impl SearchItem for TieredRow {
    type Action = CommandPaletteItemAction;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        _appearance: &Appearance,
    ) -> Box<dyn Element> {
        Empty::new().finish()
    }

    fn render_item(
        &self,
        _highlight_state: ItemHighlightState,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        Empty::new().finish()
    }

    fn priority_tier(&self) -> u8 {
        self.priority_tier
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.score)
    }

    fn accept_result(&self) -> Self::Action {
        CommandPaletteItemAction::NoOp
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        self.label.clone()
    }
}

fn row(label: &str, priority_tier: u8, score: f64) -> QueryResult<CommandPaletteItemAction> {
    QueryResult::from(TieredRow {
        label: label.to_owned(),
        priority_tier,
        score,
    })
}

fn separator(title: &str, priority_tier: u8) -> QueryResult<CommandPaletteItemAction> {
    QueryResult::from(SeparatorSearchItem::new(title.to_owned()).with_priority_tier(priority_tier))
}

/// Reproduces what the palette does to a result set: the mixer sorts ascending
/// on `(priority_tier, score)` and a `TopDown` search bar then reverses, so the
/// label order this returns is the on-screen order, top to bottom.
fn rendered_labels(mut results: Vec<QueryResult<CommandPaletteItemAction>>) -> Vec<String> {
    results.sort_by_key(|result| (result.priority_tier(), result.score()));
    results
        .into_iter()
        .rev()
        .map(|result| result.accessibility_label())
        .collect()
}

#[test]
fn sections_render_names_above_content_with_each_header_above_its_rows() {
    // Deliberately shuffled, and with the content rows scoring *higher* than
    // the name rows: tiers, not scores, decide which section comes first.
    let labels = rendered_labels(vec![
        row("content row b", CONTENT_ROW_TIER, 900.),
        separator(NAME_SEPARATOR_TITLE, NAME_SEPARATOR_TIER),
        row("name row b", NAME_ROW_TIER, 5.),
        row("content row a", CONTENT_ROW_TIER, 990.),
        separator(CONTENT_SEPARATOR_TITLE, CONTENT_SEPARATOR_TIER),
        row("name row a", NAME_ROW_TIER, 50.),
    ]);

    assert_eq!(
        labels,
        vec![
            format!("Section: {NAME_SEPARATOR_TITLE}"),
            "name row a".to_owned(),
            "name row b".to_owned(),
            format!("Section: {CONTENT_SEPARATOR_TITLE}"),
            "content row a".to_owned(),
            "content row b".to_owned(),
        ],
        "higher tier must render higher: swapping any two tier constants \
         inverts this silently, with no compile error"
    );
}

/// The name source, holding candidates whose task names match `query`.
fn name_source(task_names: &[&str]) -> DataSource {
    let mut source = DataSource::new();
    source.set_candidates(
        task_names
            .iter()
            .map(|task_name| AgentSessionCandidate {
                agent: CLIAgent::Claude,
                session_id: format!("session-{task_name}"),
                project_name: "warp".to_owned(),
                task_name: (*task_name).to_owned(),
                cwd: "/repos/warp".to_owned(),
                origin: CandidateOrigin::Handle,
                last_active: None,
            })
            .collect(),
    );
    source
}

/// The content source, holding hits the digest published for `query`.
fn content_source(query: &str, task_names: &[&str]) -> ContentDataSource {
    let mut source = ContentDataSource::new();
    source.set_results(
        query.to_owned(),
        task_names
            .iter()
            .map(|task_name| ContentHit {
                agent: CLIAgent::Claude,
                session_id: format!("content-{task_name}"),
                project_name: "warp".to_owned(),
                task_name: (*task_name).to_owned(),
                cwd: "/repos/warp".to_owned(),
                snippet: format!("…mentions {query} somewhere in {task_name}…"),
                snippet_match_indices: Vec::new(),
                partial: false,
            })
            .collect(),
    );
    source
}

#[test]
fn the_two_real_sources_render_as_names_then_content() {
    App::test((), |app| async move {
        // The same assertion as the stand-in test above, but on the rows the
        // popup actually emits: nothing about the ordering may depend on which
        // types produced the results.
        let query = "rail";
        let names = name_source(&["Rail search popup", "Rail triage rewrite"]);
        let content = content_source(query, &["Deadlock in the mixer", "Two-line tab titles"]);

        let (name_results, content_results) = app.read(|app| {
            (
                names.run_query(&Query::from(query), app).unwrap(),
                content.run_query(&Query::from(query), app).unwrap(),
            )
        });

        let names_only = rendered_labels(name_results.clone());
        let mixed = rendered_labels([name_results, content_results].concat());

        let agent = CLIAgent::Claude.display_name();
        assert_eq!(
            mixed,
            vec![
                format!("Section: {NAME_SEPARATOR_TITLE}"),
                format!("{agent} session: Rail search popup"),
                format!("{agent} session: Rail triage rewrite"),
                format!("Section: {CONTENT_SEPARATOR_TITLE}"),
                format!("{agent} session with matching text: Deadlock in the mixer"),
                format!("{agent} session with matching text: Two-line tab titles"),
            ],
            "content appends below the names under its own header, and never \
             interleaves with them"
        );
        assert_eq!(
            mixed[..names_only.len()],
            names_only[..],
            "the promise of the popup: appending content must not reorder, \
             rename or drop a single name row"
        );
    })
}

#[test]
fn the_first_selectable_row_is_at_rendered_index_one() {
    // The whole justification for `set_initial_selection_offset(1)`:
    // `SelectionUpdate::Top` does not skip non-interactable items the way
    // Up/Down do, so index 0 lands on the header and Enter silently does
    // nothing — while index 1 is the first real row. Both halves have to hold,
    // so both are asserted here rather than only described in a comment.
    let mut results = vec![
        row("name row", NAME_ROW_TIER, 1.),
        separator(NAME_SEPARATOR_TITLE, NAME_SEPARATOR_TIER),
    ];
    results.sort_by_key(|result| (result.priority_tier(), result.score()));
    let rendered: Vec<_> = results.into_iter().rev().collect();

    assert!(rendered.len() >= 2);
    assert!(
        rendered[0].is_static_separator(),
        "index 0 is the section header, which cannot be accepted"
    );
    assert!(
        !rendered[1].is_static_separator(),
        "index 1 — what the offset selects — must be a real row"
    );
    assert_eq!(rendered[1].accessibility_label(), "name row");
}
