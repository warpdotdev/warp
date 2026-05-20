use crate::settings::init_and_register_user_preferences;

use super::*;
use settings::manager::SettingsManager;

#[test]
fn test_set_aliases() {
    let workflow_1: SyncId = SyncId::ServerId(1.into());
    let workflow_2: SyncId = SyncId::ServerId(2.into());
    let alias_1: WorkflowAlias = WorkflowAlias {
        alias: "alias1".to_string(),
        workflow_id: workflow_1,
        arguments: None,
        env_vars: None,
    };
    let alias_2: WorkflowAlias = WorkflowAlias {
        alias: "alias2".to_string(),
        workflow_id: workflow_1,
        arguments: None,
        env_vars: None,
    };
    let alias_3: WorkflowAlias = WorkflowAlias {
        alias: "alias3".to_string(),
        workflow_id: workflow_2,
        arguments: None,
        env_vars: None,
    };

    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_user_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        WorkflowAliases::register(&mut app);

        app.update(|app| {
            let aliases = WorkflowAliases::as_ref(app).get_all_aliases();
            assert_eq!(0, aliases.len());
        });

        app.update(|app| {
            WorkflowAliases::handle(app).update(app, |aliases, ctx| {
                let _ = aliases.set_aliases(vec![alias_1.clone(), alias_2.clone()], ctx);
            });
            let aliases = WorkflowAliases::as_ref(app).get_all_aliases();
            assert_eq!(2, aliases.len());
        });

        app.update(|app| {
            WorkflowAliases::handle(app).update(app, |aliases, ctx| {
                let _ = aliases.set_aliases(vec![alias_3.clone()], ctx);
            });
            assert_eq!(WorkflowAliases::as_ref(app).get_all_aliases().len(), 3);
            assert_eq!(
                WorkflowAliases::as_ref(app)
                    .get_aliases_for_workflow(workflow_1)
                    .len(),
                2
            );
            assert_eq!(
                WorkflowAliases::as_ref(app)
                    .get_aliases_for_workflow(workflow_2)
                    .len(),
                1
            );
        });
    });
}

#[test]
fn test_set_aliases_replacement() {
    // Test that replacing aliases works correctly.
    let workflow_1: SyncId = SyncId::ServerId(1.into());
    let alias_1: WorkflowAlias = WorkflowAlias {
        alias: "alias1".to_string(),
        workflow_id: workflow_1,
        arguments: None,
        env_vars: None,
    };

    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_user_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        WorkflowAliases::register(&mut app);

        app.update(|app| {
            let aliases = WorkflowAliases::as_ref(app).get_all_aliases();
            assert_eq!(0, aliases.len());
        });

        app.update(|app| {
            WorkflowAliases::handle(app).update(app, |aliases, ctx| {
                let _ = aliases.set_aliases(vec![alias_1.clone()], ctx);
            });
            let aliases = WorkflowAliases::as_ref(app).get_all_aliases();
            assert_eq!(1, aliases.len());
        });

        app.update(|app| {
            WorkflowAliases::handle(app).update(app, |aliases, ctx| {
                let _ = aliases.set_aliases(vec![alias_1.clone()], ctx);
            });
            assert_eq!(WorkflowAliases::as_ref(app).get_all_aliases().len(), 1);
        });
    });
}

#[test]
fn test_remove_aliases() {
    let workflow_1: SyncId = SyncId::ServerId(1.into());
    let workflow_2: SyncId = SyncId::ServerId(2.into());
    let alias_1: WorkflowAlias = WorkflowAlias {
        alias: "alias1".to_string(),
        workflow_id: workflow_1,
        arguments: None,
        env_vars: None,
    };
    let alias_2: WorkflowAlias = WorkflowAlias {
        alias: "alias2".to_string(),
        workflow_id: workflow_1,
        arguments: None,
        env_vars: None,
    };
    let alias_3: WorkflowAlias = WorkflowAlias {
        alias: "alias3".to_string(),
        workflow_id: workflow_2,
        arguments: None,
        env_vars: None,
    };

    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_user_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        WorkflowAliases::register(&mut app);

        app.update(|app| {
            WorkflowAliases::handle(app).update(app, |aliases, ctx| {
                let _ = aliases
                    .set_aliases(vec![alias_1.clone(), alias_2.clone(), alias_3.clone()], ctx);
            });
            let aliases = WorkflowAliases::as_ref(app).get_all_aliases();
            assert_eq!(3, aliases.len());
        });

        app.update(|app| {
            WorkflowAliases::handle(app).update(app, |aliases, ctx| {
                let _ = aliases.remove_aliases(vec![alias_3.alias.clone()], ctx);
            });
            assert_eq!(WorkflowAliases::as_ref(app).get_all_aliases().len(), 2);
            assert_eq!(
                WorkflowAliases::as_ref(app)
                    .get_aliases_for_workflow(workflow_1)
                    .len(),
                2
            );
            assert_eq!(
                WorkflowAliases::as_ref(app)
                    .get_aliases_for_workflow(workflow_2)
                    .len(),
                0
            );
        });
    });
}
