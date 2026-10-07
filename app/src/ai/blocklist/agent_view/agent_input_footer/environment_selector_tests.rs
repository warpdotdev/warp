use warpui::App;

use super::*;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

#[test]
fn factory_row_renders_without_environment_sidecar() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        terminal.update(&mut app, |_, ctx| {
            let environment_id = SyncId::ServerId(ServerId::from(2));
            let environment_item = EnvironmentMenuItem {
                choice: CloudSelectorChoice::Environment(environment_id),
                name: "Environment".to_owned(),
                is_selected: false,
            };
            assert_eq!(
                environment_item.environment_sidecar_id(),
                Some(environment_id)
            );
            let item = EnvironmentMenuItem {
                choice: CloudSelectorChoice::Factory {
                    uid: ServerId::from(1).to_string(),
                    environment_uid: environment_id,
                    foreman_agent_uid: ServerId::from(3).to_string(),
                },
                name: "Build · Factory".to_owned(),
                is_selected: false,
            };
            assert_eq!(item.action_data().len(), 30);

            let menu = ctx.add_typed_action_view(move |ctx| {
                DisplayChipMenu::new(vec![item], None, ChipMenuType::Environments, ctx)
            });
            let _ = menu.as_ref(ctx).render(ctx);
        });
    });
}
