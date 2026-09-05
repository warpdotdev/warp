use warp_core::ui::appearance::Appearance;
use warpui::App;
use warpui::platform::WindowStyle;

use super::*;
use crate::test_util::settings::initialize_settings_for_tests;

fn initialize_test_app(app: &mut App) {
    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
}

fn row(pr_number: u64, head_ref: &str, is_selected: bool) -> StackMapRow {
    StackMapRow {
        pr_number,
        title: format!("PR {pr_number}"),
        head_ref: head_ref.to_string(),
        state: StackRowState::Open,
        is_current_branch: false,
        is_selected,
    }
}

#[test]
fn from_pr_state_prioritizes_merged_over_draft_and_closed() {
    // A merged PR reported as draft/open/closed by `state` is still merged.
    assert_eq!(
        StackRowState::from_pr_state("closed", true, true),
        StackRowState::Merged
    );
}

#[test]
fn from_pr_state_draft_before_closed() {
    assert_eq!(
        StackRowState::from_pr_state("open", true, false),
        StackRowState::Draft
    );
}

#[test]
fn from_pr_state_closed_when_not_draft_or_merged() {
    assert_eq!(
        StackRowState::from_pr_state("closed", false, false),
        StackRowState::Closed
    );
    assert_eq!(
        StackRowState::from_pr_state("CLOSED", false, false),
        StackRowState::Closed
    );
}

#[test]
fn from_pr_state_defaults_to_open() {
    assert_eq!(
        StackRowState::from_pr_state("open", false, false),
        StackRowState::Open
    );
}

#[test]
fn visual_rows_renders_top_of_stack_first() {
    // `rows` is bottom-to-top; the map renders top-to-bottom (trunk last),
    // per PRODUCT.md item 7.
    let presentation = StackMapPresentation {
        trunk_ref: "main".to_string(),
        rows: vec![
            row(1, "bottom", false),
            row(2, "middle", false),
            row(3, "top", false),
        ],
        current_position: None,
    };

    let visual: Vec<u64> = presentation.visual_rows().map(|r| r.pr_number).collect();
    assert_eq!(
        visual,
        vec![3, 2, 1],
        "visual order should be top layer first, bottom layer last (trunk renders after all rows)"
    );
}

#[test]
fn set_presentation_normalizes_single_layer_stack_to_none() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (_, control) = app.add_window(WindowStyle::NotStealFocus, StackControl::new);

        control.update(&mut app, |control, ctx| {
            let single_layer = StackMapPresentation {
                trunk_ref: "main".to_string(),
                rows: vec![row(1, "only", true)],
                current_position: Some(1),
            };
            control.set_presentation(Some(single_layer), ctx);
        });
        assert!(
            !control.read(&app, |control, _| control.is_visible()),
            "a stack with fewer than two layers should not show the control"
        );
    });
}

#[test]
fn set_presentation_shows_control_for_two_or_more_layers() {
    App::test((), |mut app| async move {
        initialize_test_app(&mut app);
        let (_, control) = app.add_window(WindowStyle::NotStealFocus, StackControl::new);

        control.update(&mut app, |control, ctx| {
            let two_layers = StackMapPresentation {
                trunk_ref: "main".to_string(),
                rows: vec![row(1, "bottom", false), row(2, "top", true)],
                current_position: Some(2),
            };
            control.set_presentation(Some(two_layers), ctx);
        });
        assert!(
            control.read(&app, |control, _| control.is_visible()),
            "a stack with two or more layers should show the control"
        );
    });
}
