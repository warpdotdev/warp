use uuid::Uuid;
use warp::appearance::Appearance;
use warp::editor::CodeEditorModel;
use warp::tui_export::{
    TuiMcpInstallRequest, TuiMcpServerId, TuiMcpServerSource, TuiMcpTemplateVariable,
};
use warpui_core::App;

use super::{TuiMcpInstallFlowAction, TuiMcpInstallFlowModel, TuiMcpInstallStep};
use crate::input_suggestions_mode::TuiInputSuggestionsModeModel;

fn input_editor(ctx: &mut warpui_core::AppContext) -> warpui_core::ModelHandle<CodeEditorModel> {
    ctx.add_singleton_model(|_| Appearance::mock());
    ctx.add_model(|ctx| CodeEditorModel::new_tui(80, ctx))
}

fn request(variables: Vec<TuiMcpTemplateVariable>) -> TuiMcpInstallRequest {
    TuiMcpInstallRequest {
        id: TuiMcpServerId::Gallery(Uuid::from_u128(1)),
        name: "Example".to_owned(),
        description: Some("Example MCP".to_owned()),
        source: TuiMcpServerSource::Gallery,
        instructions: Some("# Setup\nFollow the instructions".to_owned()),
        variables,
    }
}

#[test]
fn zero_variable_flow_still_requires_confirmation() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let editor = input_editor(ctx);
            let mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
            let flow = ctx.add_model(|_| TuiMcpInstallFlowModel::new(editor, mode));

            assert!(flow.update(ctx, |flow, ctx| flow.start(request(Vec::new()), ctx)));
            assert!(matches!(
                &flow.as_ref(ctx).step,
                TuiMcpInstallStep::Confirmation
            ));
            assert!(flow.as_ref(ctx).confirmation().is_some());
            assert_eq!(
                flow.as_ref(ctx)
                    .snapshot(ctx)
                    .expect("flow is visible")
                    .rows[0]
                    .description
                    .as_deref(),
                Some("Example MCP · Setup")
            );
        });
    });
}

#[test]
fn collected_value_actions_are_redacted_from_debug_output() {
    let action = TuiMcpInstallFlowAction::ProvideValue {
        key: "TOKEN".to_owned(),
        value: "do-not-log-this".to_owned(),
    };

    let debug = format!("{action:?}");

    assert!(debug.contains("TOKEN"));
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("do-not-log-this"));
}

#[test]
fn allowed_values_are_presented_as_selectable_rows() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let editor = input_editor(ctx);
            let mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
            let flow = ctx.add_model(|_| TuiMcpInstallFlowModel::new(editor, mode));
            let variable = TuiMcpTemplateVariable {
                key: "REGION".to_owned(),
                allowed_values: Some(vec!["us".to_owned(), "eu".to_owned()]),
            };

            flow.update(ctx, |flow, ctx| {
                assert!(flow.start(request(vec![variable]), ctx));
            });
            let snapshot = flow.as_ref(ctx).snapshot(ctx).expect("flow is visible");
            assert_eq!(
                snapshot
                    .rows
                    .iter()
                    .map(|row| row.title.as_str())
                    .collect::<Vec<_>>(),
                vec!["us", "eu"]
            );
            assert_eq!(snapshot.selected_index, Some(0));
        });
    });
}

#[test]
fn cancellation_discards_collected_values() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let editor = input_editor(ctx);
            let mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
            let flow = ctx.add_model(|_| TuiMcpInstallFlowModel::new(editor, mode));
            let variable = TuiMcpTemplateVariable {
                key: "TOKEN".to_owned(),
                allowed_values: None,
            };

            flow.update(ctx, |flow, ctx| {
                assert!(flow.start(request(vec![variable]), ctx));
                flow.apply_value("TOKEN".to_owned(), "secret".to_owned(), ctx)
                    .expect("value is accepted");
                flow.dismiss(ctx);
            });

            assert!(flow.as_ref(ctx).confirmation().is_none());
            assert!(flow.as_ref(ctx).values.is_empty());
        });
    });
}
