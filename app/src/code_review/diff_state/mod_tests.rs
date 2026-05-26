use super::{DiffMode, DiffState, DiffStateModel};

#[test]
fn new_for_test_creates_local_variant() {
    warpui::App::test((), |mut app| async move {
        let handle = app.add_model(DiffStateModel::new_for_test);
        handle.read(&app, |model, _ctx| {
            assert!(matches!(model, DiffStateModel::Local(_)));
        });
    });
}

#[test]
fn get_returns_not_in_repository_for_test_model() {
    warpui::App::test((), |mut app| async move {
        let handle = app.add_model(DiffStateModel::new_for_test);
        let state = handle.read(&app, |model, ctx| model.get(ctx));
        assert!(matches!(state, DiffState::NotInRepository));
    });
}

#[test]
fn diff_mode_defaults_to_head() {
    warpui::App::test((), |mut app| async move {
        let handle = app.add_model(DiffStateModel::new_for_test);
        let mode = handle.read(&app, |model, ctx| model.diff_mode(ctx));
        assert!(matches!(mode, DiffMode::Head));
    });
}

#[test]
fn has_head_false_for_test_model() {
    warpui::App::test((), |mut app| async move {
        let handle = app.add_model(DiffStateModel::new_for_test);
        let has_head = handle.read(&app, |model, ctx| model.has_head(ctx));
        assert!(!has_head);
    });
}

#[test]
fn branch_info_none_for_test_model() {
    warpui::App::test((), |mut app| async move {
        let handle = app.add_model(DiffStateModel::new_for_test);
        handle.read(&app, |model, ctx| {
            assert_eq!(model.get_main_branch_name(ctx), None);
            assert_eq!(model.get_current_branch_name(ctx), None);
            assert!(!model.is_on_main_branch(ctx));
            assert!(model.unpushed_commits(ctx).is_empty());
            assert_eq!(model.upstream_ref(ctx), None);
            assert!(!model.upstream_differs_from_main(ctx));
            assert!(model.pr_info(ctx).is_none());
            assert!(!model.is_pr_info_refreshing(ctx));
            assert!(!model.is_git_operation_blocked(ctx));
        });
    });
}

#[test]
fn uncommitted_stats_none_for_test_model() {
    warpui::App::test((), |mut app| async move {
        let handle = app.add_model(DiffStateModel::new_for_test);
        let stats = handle.read(&app, |model, ctx| model.get_uncommitted_stats(ctx));
        assert!(stats.is_none());
    });
}
