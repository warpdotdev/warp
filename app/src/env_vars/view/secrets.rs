use pathfinder_geometry::vector::vec2f;
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, Empty, Fill, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement,
    ParentOffsetBounds, Shrinkable, Stack,
};
use warpui::fonts::Weight;
use warpui::ui_components::button::{ButtonVariant, TextAndIcon, TextAndIconAlignment};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
#[cfg(not(target_family = "wasm"))]
use warpui::SingletonEntity;
use warpui::{Element, ViewContext};

use super::env_var_collection::{
    EnvVarCollectionAction, EnvVarCollectionView, VariableRowIndex, CORE_MAX_WIDTH, ROW_SPACING,
};
use crate::drive::sharing::ContentEditability;
use crate::env_vars::active_env_var_collection_data::SavingStatus;
use crate::env_vars::EnvVarValue;
use crate::external_secrets::{ExternalSecretManager, SecretManager};
use crate::search::external_secrets::searcher::ExternalSecretSearchItemAction;
use crate::search::external_secrets::view::ExternalSecretsMenuEvent;
use crate::ui_components::icons::Icon;
#[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
use crate::{
    terminal::local_shell::LocalShellState,
    view_components::{DismissibleToast, ToastLink},
    workspace::{ToastStack, WorkspaceAction},
};

impl EnvVarCollectionView {
    pub(super) fn handle_external_secrets_dialog_event(
        &mut self,
        event: &ExternalSecretsMenuEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            ExternalSecretsMenuEvent::ItemSelected { payload } => {
                let ExternalSecretSearchItemAction::AcceptSecret(secret) = payload.as_ref();
                let row_index = self.pending_variable_row_index.take();
                if let Some(VariableRowIndex(index)) = row_index {
                    self.variable_rows[index].value = EnvVarValue::Secret(secret.clone());
                }

                self.set_saving_status(SavingStatus::Unsaved, ctx)
            }
            ExternalSecretsMenuEvent::Close => {
                self.dialog_open_states.secrets_dialog_open = false;
                self.update_open_modal_state(ctx);
                ctx.focus_self();
                ctx.notify();
            }
            ExternalSecretsMenuEvent::Open => {
                self.dialog_open_states.secrets_dialog_open = true;
                ctx.focus(&self.secrets_dialog);
                self.update_open_modal_state(ctx);
                ctx.notify();
            }
        }
    }

    pub(super) fn clear_secret(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(VariableRowIndex(index)) = self.pending_variable_row_index.take() {
            self.variable_rows[index].value = EnvVarValue::Constant(String::new());
        }

        ctx.notify();
    }

    #[cfg_attr(target_family = "wasm", allow(unused_variables))]
    pub(super) fn fetch_secret(
        &mut self,
        secret_manager: SecretManager,
        ctx: &mut ViewContext<Self>,
    ) {
        #[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
        {
            let window_id = ctx.window_id();
            let local_shell = LocalShellState::as_ref(ctx);
            let secret_manager_clone = secret_manager.clone();

            let Some(local_shell_state) = local_shell.local_shell_info() else {
                return;
            };

            let shell_type = local_shell_state.get_shell_type();
            let shell_path = local_shell_state.get_shell_path().clone();
            let path_env_var = local_shell_state.get_path_env_var().clone();

            self.secrets_dialog.update(ctx, |_, dialog_ctx| {
                let _ = dialog_ctx.spawn(
                    async move {
                        secret_manager
                            .verify_installed_and_fetch_secrets(
                                shell_type,
                                shell_path,
                                path_env_var,
                            )
                            .await
                    },
                    move |view, result, dialog_ctx| match result {
                        Ok(secrets) => {
                            view.setup(secrets, dialog_ctx);
                        }
                        Err(e) => {
                            let error_message_and_command =
                                secret_manager_clone.get_toast_message_and_link(e);

                            let mut toast =
                                DismissibleToast::error(error_message_and_command.message);

                            if let (Some(link), Some(link_message)) = (
                                error_message_and_command.link,
                                error_message_and_command.link_message,
                            ) {
                                toast = toast.with_link(
                                    ToastLink::new(link_message)
                                        .with_onclick_action(WorkspaceAction::OpenLink(link)),
                                );
                            }

                            ToastStack::handle(dialog_ctx).update(dialog_ctx, |toast_stack, ctx| {
                                toast_stack.add_persistent_toast(toast, window_id, ctx);
                            })
                        }
                    },
                );
            });
        }
    }

    pub(super) fn render_secret_or_command_button(
        &self,
        appearance: &Appearance,
        secret: &EnvVarValue,
        menu_button_mouse_state: MouseStateHandle,
        row_index: usize,
        is_focused: bool,
        editability: ContentEditability,
    ) -> Box<dyn Element> {
        let (display_name, action, menu, icon) = match secret {
            EnvVarValue::Secret(sec) => (
                sec.get_display_name(),
                EnvVarCollectionAction::DisplayRenderedSecretMenu(VariableRowIndex(row_index)),
                &self.menus.rendered_secret_menu,
                sec.icon(),
            ),
            EnvVarValue::Command(cmd) => (
                if !cmd.name.is_empty() {
                    cmd.name.clone()
                } else {
                    cmd.command.clone()
                },
                EnvVarCollectionAction::DisplayRenderedCommandMenu(VariableRowIndex(row_index)),
                &self.menus.rendered_command_menu,
                Icon::Terminal,
            ),
            _ => {
                log::warn!("Secret type not supported for button rendering");
                return Empty::new().finish();
            }
        };

        let text_and_icon = TextAndIcon::new(
            TextAndIconAlignment::IconFirst,
            display_name,
            icon.to_warpui_icon(appearance.theme().active_ui_text_color()),
            MainAxisSize::Max,
            MainAxisAlignment::Center,
            vec2f(16., 16.),
        )
        .with_inner_padding(4.);

        let default_button_styles = UiComponentStyles {
            font_size: Some(appearance.ui_font_size()),
            font_family_id: Some(appearance.ui_font_family()),
            font_color: Some(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().background())
                    .into(),
            ),
            border_width: Some(1.),
            border_color: Some(appearance.theme().surface_2().into()),
            font_weight: Some(Weight::Bold),
            background: Some(Fill::None),
            ..Default::default()
        };

        let hovered_styles = UiComponentStyles {
            border_width: Some(1.),
            border_color: Some(appearance.theme().accent_button_color().into()),
            ..default_button_styles
        };

        let mut button = appearance
            .ui_builder()
            .button(ButtonVariant::Outlined, menu_button_mouse_state)
            .with_style(default_button_styles)
            .with_hovered_styles(hovered_styles)
            .with_text_and_icon_label(text_and_icon);

        if FeatureFlag::SharedWithMe.is_enabled() && !editability.can_edit() {
            button = button.disabled();
        }

        let button = button
            .build()
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()));

        Shrinkable::new(
            1.,
            Container::new(
                ConstrainedBox::new({
                    let mut stack = Stack::new().with_child(Clipped::new(button.finish()).finish());
                    if is_focused {
                        stack.add_positioned_overlay_child(
                            ChildView::new(menu).finish(),
                            OffsetPositioning::offset_from_parent(
                                vec2f(0., 0.),
                                ParentOffsetBounds::WindowByPosition,
                                ParentAnchor::TopRight,
                                ChildAnchor::TopLeft,
                            ),
                        );
                    }
                    stack.finish()
                })
                .with_width(CORE_MAX_WIDTH)
                .finish(),
            )
            .with_margin_right(ROW_SPACING)
            .finish(),
        )
        .finish()
    }
}
