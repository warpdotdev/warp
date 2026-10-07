use uuid::Uuid;
use warp::appearance::Appearance;
use warp::editor::CodeEditorModel;
use warp::tui_export::{TuiMcpInstallRequest, TuiMcpServerId, TuiMcpTemplateVariable};
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
        variables,
    }
}

#[test]
fn zero_variable_request_skips_the_install_flow() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let editor = input_editor(ctx);
            let mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
            let flow = ctx.add_model(|_| TuiMcpInstallFlowModel::new(editor, mode));
            assert!(!flow.update(ctx, |flow, ctx| flow.start(request(Vec::new()), ctx)));
            assert!(matches!(&flow.as_ref(ctx).step, TuiMcpInstallStep::Closed));
            assert!(!flow.as_ref(ctx).is_open(ctx));
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
            assert_eq!(
                flow.as_ref(ctx).primary_action_hint(),
                Some("to install and enable")
            );
        });
    });
}

#[test]
fn final_value_completes_installation_without_confirmation() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let editor = input_editor(ctx);
            let mode = ctx.add_model(|_| TuiInputSuggestionsModeModel::new());
            let flow = ctx.add_model(|_| TuiMcpInstallFlowModel::new(editor, mode));
            let variable = TuiMcpTemplateVariable {
                key: "TOKEN".to_owned(),
                allowed_values: None,
            };
            let completion = flow.update(ctx, |flow, ctx| {
                assert!(flow.start(request(vec![variable]), ctx));
                flow.apply_value("TOKEN".to_owned(), "secret".to_owned(), ctx)
                    .expect("value is accepted")
                    .expect("the final value completes installation")
            });

            assert_eq!(completion.name, "Example");
            assert_eq!(completion.values.len(), 1);
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
            let variables = vec![
                TuiMcpTemplateVariable {
                    key: "TOKEN".to_owned(),
                    allowed_values: None,
                },
                TuiMcpTemplateVariable {
                    key: "REGION".to_owned(),
                    allowed_values: None,
                },
            ];

            flow.update(ctx, |flow, ctx| {
                assert!(flow.start(request(variables), ctx));
                assert!(
                    flow.apply_value("TOKEN".to_owned(), "secret".to_owned(), ctx)
                        .expect("value is accepted")
                        .is_none()
                );
                flow.dismiss(ctx);
            });

            assert!(matches!(&flow.as_ref(ctx).step, TuiMcpInstallStep::Closed));
            assert!(flow.as_ref(ctx).request.is_none());
            assert!(flow.as_ref(ctx).values.is_empty());
        });
    });
}
