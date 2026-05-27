use warpui::integration::{AssertionCallback, TestStep};
use warpui::{async_assert, SingletonEntity};
use warpui::{App, WindowId};

use crate::app_menus;
use crate::integration_testing::step::new_step_with_default_assertions;
use crate::integration_testing::view_getters::workspace_view;
use crate::launch_configs::save_modal::LaunchConfigSaveModal;
use crate::localization;
use crate::view_components::{DismissibleToast, DismissibleToastStack};
use crate::workspace::{ToastStack, WorkspaceAction};

pub fn show_localized_launch_config_dialog() -> TestStep {
    new_step_with_default_assertions("Show localized launch config dialog")
        .with_action(|app, window_id, _| {
            let workspace = workspace_view(app, window_id);
            workspace.update(app, |workspace, ctx| {
                workspace.open_launch_config_save_modal(ctx);
            });
        })
        .add_named_assertion(
            "Launch config dialog is focused",
            assert_launch_config_dialog_focused(),
        )
}

pub fn show_localized_workspace_toast(message_key: &'static str) -> TestStep {
    new_step_with_default_assertions("Show localized workspace toast")
        .with_action(move |app, window_id, _| {
            ToastStack::handle(app).update(app, |toast_stack, ctx| {
                let message = localization::text_for_app(ctx, message_key);
                toast_stack.add_persistent_toast(
                    DismissibleToast::default(message),
                    window_id,
                    ctx,
                );
            });
        })
        .add_named_assertion(
            "Workspace toast is visible",
            assert_localized_workspace_toast_visible(),
        )
}

pub fn assert_localized_app_and_dock_menus() -> TestStep {
    new_step_with_default_assertions("Assert localized app and Dock menus").add_named_assertion(
        "App menu and Dock menu titles use zh-CN catalog text",
        Box::new(|app: &mut App, _window_id: WindowId| {
            app.update(|ctx: &mut warpui::AppContext| {
                let dock_menu = app_menus::dock_menu(ctx);
                let Some(first_dock_item_name) = dock_menu.menu_items.first().and_then(|item| {
                    if let warpui::platform::menu::MenuItem::Custom(item) = item {
                        Some(item.properties.name.as_str())
                    } else {
                        None
                    }
                }) else {
                    return async_assert!(false, "Dock menu should start with a custom menu item");
                };

                let menu_bar = app_menus::menu_bar(ctx);
                let menu_titles = menu_bar
                    .menus
                    .iter()
                    .map(|menu| menu.title.as_str())
                    .collect::<Vec<_>>();
                let expected_titles = [
                    "Warp", "文件", "编辑", "视图", "标签页", "块", "AI", "Drive", "窗口", "帮助",
                ];
                let has_expected_titles = expected_titles
                    .iter()
                    .all(|expected| menu_titles.contains(expected));

                async_assert!(
                    dock_menu.title == "新建窗口"
                        && first_dock_item_name == "新建窗口"
                        && has_expected_titles,
                    "Expected zh-CN menu titles, got dock={:?}, first_dock_item={:?}, menu_titles={:?}",
                    dock_menu.title,
                    first_dock_item_name,
                    menu_titles
                )
            })
        }),
    )
}

fn assert_localized_workspace_toast_visible() -> AssertionCallback {
    Box::new(|app, window_id| {
        let toast_stacks = app
            .views_of_type::<DismissibleToastStack<WorkspaceAction>>(window_id)
            .expect("workspace toast stack should exist");
        let has_toast = toast_stacks
            .iter()
            .any(|toast_stack| toast_stack.read(app, |toast_stack, _| toast_stack.has_toasts()));

        async_assert!(has_toast, "Expected a visible workspace toast")
    })
}

fn assert_launch_config_dialog_focused() -> AssertionCallback {
    Box::new(|app, window_id| {
        let dialog = app
            .views_of_type::<LaunchConfigSaveModal>(window_id)
            .expect("launch config dialog should exist")
            .first()
            .expect("launch config dialog should exist")
            .clone();
        let dialog_id = dialog.id();

        app.update(|ctx| {
            async_assert!(
                ctx.check_view_or_child_focused(window_id, &dialog_id),
                "Expected launch config dialog to be focused"
            )
        })
    })
}
