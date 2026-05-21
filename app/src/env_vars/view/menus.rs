use pathfinder_geometry::vector::Vector2F;
use warp_core::context_flag::ContextFlag;
use warpui::{keymap::Trigger, SingletonEntity, ViewContext, ViewHandle};

use crate::{
    cloud_object::{CloudObject, GenericStringObjectFormat, Space},
    drive::{
        drive_helpers::has_feature_gated_anonymous_user_reached_env_var_limit,
        export::ExportManager, CloudObjectTypeAndId,
    },
    env_vars::active_env_var_collection_data::TrashStatus,
    external_secrets::SecretManager,
    i18n::{self, I18nKey},
    menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields},
    pane_group::PaneEvent,
    server::cloud_objects::update_manager::UpdateManager,
    ui_components::icons::Icon,
    util::bindings::{keybinding_name_to_display_string, trigger_to_keystroke, CustomAction},
    AppContext, CloudModel, FeatureFlag,
};

use super::env_var_collection::{EnvVarCollectionAction, EnvVarCollectionView, VariableRowIndex};

const PANE_MENU_WIDTH: f32 = 200.;

pub struct Menus {
    pub(super) secret_menu: ViewHandle<Menu<EnvVarCollectionAction>>,
    pub(super) rendered_secret_menu: ViewHandle<Menu<EnvVarCollectionAction>>,
    pub(super) rendered_command_menu: ViewHandle<Menu<EnvVarCollectionAction>>,
    pub(super) pane_context_menu: ViewHandle<Menu<EnvVarCollectionAction>>,
}

impl EnvVarCollectionView {
    pub(super) fn initialize_menus(ctx: &mut ViewContext<Self>) -> Menus {
        let command_item = Self::item(
            i18n::tr(ctx, I18nKey::EnvVarsCommand),
            EnvVarCollectionAction::DisplayCommandDialog,
            None,
            Some(Icon::Terminal),
        );

        let one_password_item = Self::item(
            "1Password",
            EnvVarCollectionAction::SelectSecretManager(SecretManager::OnePassword),
            None,
            Some(Icon::OnePassword),
        );

        let lastpass_item = Self::item(
            "LastPass",
            EnvVarCollectionAction::SelectSecretManager(SecretManager::LastPass),
            None,
            Some(Icon::LastPass),
        );

        let edit_item = Self::item(
            i18n::tr(ctx, I18nKey::CommonEdit),
            EnvVarCollectionAction::EditCommand,
            None,
            Some(Icon::Terminal),
        );

        let clear_secret_item = Self::item(
            i18n::tr(ctx, I18nKey::EnvVarsClearSecret),
            EnvVarCollectionAction::ClearSecret,
            None,
            Some(Icon::Trash),
        );

        let separator = MenuItem::Separator;

        let secret_menu = Self::menu(
            vec![
                command_item.clone(),
                one_password_item.clone(),
                lastpass_item.clone(),
            ],
            None,
            ctx,
        );

        let rendered_secret_menu = Self::menu(
            vec![
                command_item.clone(),
                one_password_item.clone(),
                lastpass_item.clone(),
                separator.clone(),
                clear_secret_item.clone(),
            ],
            None,
            ctx,
        );

        let rendered_command_menu = Self::menu(
            vec![
                edit_item,
                one_password_item,
                lastpass_item,
                separator,
                clear_secret_item,
            ],
            None,
            ctx,
        );

        ctx.subscribe_to_view(&secret_menu, |me, _, event, ctx| {
            me.handle_secret_menu_event(event, ctx);
        });

        ctx.subscribe_to_view(&rendered_secret_menu, |me, _, event, ctx| {
            me.handle_rendered_secret_menu_event(event, ctx);
        });

        ctx.subscribe_to_view(&rendered_command_menu, |me, _, event, ctx| {
            me.handle_rendered_command_menu_event(event, ctx);
        });

        let pane_context_menu = Self::menu(Vec::new(), Some(PANE_MENU_WIDTH), ctx);

        Menus {
            secret_menu,
            rendered_secret_menu,
            rendered_command_menu,
            pane_context_menu,
        }
    }

    pub(super) fn initialize_pane_context_menu(
        &self,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<Menu<EnvVarCollectionAction>> {
        let split_pane_right = Self::item(
            "Split pane right",
            EnvVarCollectionAction::EmitPaneEvent(PaneEvent::SplitRight(None)),
            keybinding_name_to_display_string("pane_group:add_right", ctx),
            None,
        );

        let split_pane_left = Self::item(
            "Split pane left",
            EnvVarCollectionAction::EmitPaneEvent(PaneEvent::SplitLeft(None)),
            keybinding_name_to_display_string("pane_group:add_left", ctx),
            None,
        );

        let split_pane_down = Self::item(
            "Split pane down",
            EnvVarCollectionAction::EmitPaneEvent(PaneEvent::SplitDown(None)),
            keybinding_name_to_display_string("pane_group:add_down", ctx),
            None,
        );

        let split_pane_up = Self::item(
            "Split pane up",
            EnvVarCollectionAction::EmitPaneEvent(PaneEvent::SplitUp(None)),
            keybinding_name_to_display_string("pane_group:add_up", ctx),
            None,
        );

        let is_maximized = self
            .focus_handle
            .as_ref()
            .is_some_and(|handle| handle.split_pane_state(ctx).is_maximized());
        let toggle_maximize_pane = Self::item(
            if is_maximized {
                "Minimize pane"
            } else {
                "Maximize pane"
            },
            EnvVarCollectionAction::EmitPaneEvent(PaneEvent::ToggleMaximized),
            keybinding_name_to_display_string("pane_group:toggle_maximize_pane", ctx),
            None,
        );

        let close_pane = Self::item(
            "Close pane",
            EnvVarCollectionAction::EmitPaneEvent(PaneEvent::Close),
            trigger_to_keystroke(&Trigger::Custom(CustomAction::CloseCurrentSession.into()))
                .map(|keystroke| keystroke.displayed()),
            None,
        );

        let mut items = Vec::new();

        if ContextFlag::CreateNewSession.is_enabled() {
            items.extend(vec![
                split_pane_right,
                split_pane_left,
                split_pane_down,
                split_pane_up,
            ]);
        }

        if self
            .focus_handle
            .as_ref()
            .is_some_and(|handle| handle.is_in_split_pane(ctx))
        {
            items.extend(vec![toggle_maximize_pane, close_pane]);
        }

        let pane_context_menu = Self::menu(items, Some(PANE_MENU_WIDTH), ctx);

        ctx.subscribe_to_view(&pane_context_menu, |me, _, event, ctx| {
            me.handle_pane_context_menu_event(event, ctx);
        });

        pane_context_menu
    }

    pub(super) fn display_secret_menu(&mut self, index: usize) {
        let row = &mut self.variable_rows[index];
        row.secret_menu_is_focused = true;

        self.pending_variable_row_index = Some(VariableRowIndex(index));
    }

    pub(super) fn display_rendered_secret_menu(&mut self, index: usize) {
        let row = &mut self.variable_rows[index];
        row.rendered_secret_menu_is_focused = true;

        self.pending_variable_row_index = Some(VariableRowIndex(index));
    }

    pub(super) fn display_rendered_command_menu(&mut self, index: usize) {
        let row = &mut self.variable_rows[index];
        row.rendered_command_menu_is_focused = true;

        self.pending_variable_row_index = Some(VariableRowIndex(index));
    }

    pub(super) fn display_pane_context_menu(
        &mut self,
        position: &Vector2F,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(parent_bounds) = ctx.element_position_by_id(self.view_position_id.clone()) {
            self.menus.pane_context_menu = self.initialize_pane_context_menu(ctx);
            let offset = *position - parent_bounds.origin();
            self.pane_context_menu_offset = Some(offset);
            ctx.notify();
        }
    }

    fn handle_secret_menu_event(&mut self, event: &MenuEvent, ctx: &mut ViewContext<Self>) {
        match event {
            MenuEvent::Close { via_select_item: _ } => self.reset_secret_menu(ctx),
            MenuEvent::ItemSelected => self.reset_secret_menu(ctx),
            MenuEvent::ItemHovered => {}
        }
    }

    fn handle_rendered_secret_menu_event(
        &mut self,
        event: &MenuEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            MenuEvent::Close { via_select_item: _ } => self.reset_rendered_secret_menu(ctx),
            MenuEvent::ItemSelected => self.reset_rendered_secret_menu(ctx),
            MenuEvent::ItemHovered => {}
        }
    }

    fn handle_rendered_command_menu_event(
        &mut self,
        event: &MenuEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            MenuEvent::Close { via_select_item: _ } => self.reset_rendered_command_menu(ctx),
            MenuEvent::ItemSelected => self.reset_rendered_command_menu(ctx),
            MenuEvent::ItemHovered => {}
        }
    }

    fn handle_pane_context_menu_event(&mut self, event: &MenuEvent, ctx: &mut ViewContext<Self>) {
        match event {
            MenuEvent::Close { via_select_item: _ } | MenuEvent::ItemSelected => {
                self.pane_context_menu_offset = None;
                self.menus
                    .pane_context_menu
                    .update(ctx, |menu, ctx| menu.reset_selection(ctx));
                ctx.notify()
            }
            MenuEvent::ItemHovered => {}
        }
    }

    fn reset_secret_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.variable_rows.iter_mut().for_each(|row| {
            row.secret_menu_is_focused = false;
        });
        self.menus
            .secret_menu
            .update(ctx, |menu, ctx| menu.reset_selection(ctx));
        ctx.notify()
    }

    fn reset_rendered_secret_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.variable_rows.iter_mut().for_each(|row| {
            row.rendered_secret_menu_is_focused = false;
        });
        self.menus
            .rendered_secret_menu
            .update(ctx, |menu, ctx| menu.reset_selection(ctx));
        ctx.notify()
    }

    fn reset_rendered_command_menu(&mut self, ctx: &mut ViewContext<Self>) {
        self.variable_rows.iter_mut().for_each(|row| {
            row.rendered_command_menu_is_focused = false;
        });
        self.menus
            .rendered_command_menu
            .update(ctx, |menu, ctx| menu.reset_selection(ctx));
        ctx.notify()
    }

    fn menu(
        items: Vec<MenuItem<EnvVarCollectionAction>>,
        width: Option<f32>,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<Menu<EnvVarCollectionAction>> {
        ctx.add_typed_action_view(|_| {
            let mut menu = Menu::new()
                .prevent_interaction_with_other_elements()
                .with_drop_shadow();

            if let Some(width) = width {
                menu = menu.with_width(width);
            }

            menu.add_items(items);

            menu
        })
    }

    fn item(
        name: &str,
        action: EnvVarCollectionAction,
        key_shortcut: Option<String>,
        icon: Option<Icon>,
    ) -> MenuItem<EnvVarCollectionAction> {
        let mut field = MenuItemFields::new(name)
            .with_on_select_action(action)
            .with_key_shortcut_label(key_shortcut);

        if let Some(icon) = icon {
            field = field.with_icon(icon);
        }

        field.into_item()
    }

    // Used for duplicate, copy link, trash etc
    pub(super) fn overflow_menu_items(
        &self,
        ctx: &AppContext,
    ) -> Vec<MenuItem<EnvVarCollectionAction>> {
        let mut menu_items = Vec::new();

        let active_collection_data = self.active_env_var_collection_data.as_ref(ctx);
        let access_level = active_collection_data.access_level(ctx);
        let space = active_collection_data.space(ctx);

        if !active_collection_data.is_on_server()
            || active_collection_data.trash_status(ctx) != TrashStatus::Active
        {
            return menu_items;
        }

        // Add "Copy Link" to menu
        if let Some(link) = self.env_var_collection_link(ctx) {
            menu_items.push(
                MenuItemFields::new(i18n::tr(ctx, I18nKey::CommonCopyLink))
                    .with_on_select_action(EnvVarCollectionAction::CopyLink(link))
                    .with_icon(Icon::Link)
                    .into_item(),
            );
        }

        // Add "Duplicate" to menu
        if space != Some(Space::Shared) {
            menu_items.push(
                MenuItemFields::new(i18n::tr(ctx, I18nKey::CommonDuplicate))
                    .with_on_select_action(EnvVarCollectionAction::Duplicate)
                    .with_icon(Icon::Duplicate)
                    .into_item(),
            );
        }

        // Add "Trash" to menu
        if self.is_online(ctx)
            && (!FeatureFlag::SharedWithMe.is_enabled() || access_level.can_trash())
        {
            menu_items.push(
                MenuItemFields::new(i18n::tr(ctx, I18nKey::CommonTrash))
                    .with_on_select_action(EnvVarCollectionAction::Trash)
                    .with_icon(Icon::Trash)
                    .into_item(),
            );
        }

        #[cfg(feature = "local_fs")]
        menu_items.push(
            MenuItemFields::new(i18n::tr(ctx, I18nKey::CommonExport))
                .with_on_select_action(EnvVarCollectionAction::Export)
                .with_icon(Icon::Download)
                .into_item(),
        );

        menu_items
    }

    pub(super) fn env_var_collection_link(&self, ctx: &AppContext) -> Option<String> {
        self.env_var_collection_id(ctx)
            .and_then(|id| CloudModel::as_ref(ctx).get_env_var_collection(&id))
            .map(|env_var_collection| env_var_collection.object_link())?
    }

    pub(super) fn untrash_env_var_collection(&self, ctx: &mut ViewContext<Self>) {
        if let Some(env_var_collection_id) = self.active_env_var_collection_data.as_ref(ctx).id() {
            if has_feature_gated_anonymous_user_reached_env_var_limit(ctx) {
                return;
            }

            UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                update_manager.untrash_object(
                    CloudObjectTypeAndId::GenericStringObject {
                        object_type: GenericStringObjectFormat::Json(
                            crate::cloud_object::JsonObjectType::EnvVarCollection,
                        ),
                        id: env_var_collection_id,
                    },
                    ctx,
                );
            });
        }
        ctx.notify();
    }

    pub(super) fn trash_env_var_collection(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(env_var_collection_id) = self.env_var_collection_id(ctx) {
            self.close_env_var_collection(ctx);

            UpdateManager::handle(ctx).update(ctx, move |update_manager, ctx| {
                update_manager.trash_object(
                    CloudObjectTypeAndId::from_generic_string_object(
                        GenericStringObjectFormat::Json(
                            crate::cloud_object::JsonObjectType::EnvVarCollection,
                        ),
                        env_var_collection_id,
                    ),
                    ctx,
                );
            });
            ctx.notify();
        }
    }

    pub(super) fn duplicate_env_var_collection(&self, ctx: &mut ViewContext<Self>) {
        if let Some(env_var_collection_id) = self.env_var_collection_id(ctx) {
            UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                update_manager.duplicate_object(
                    &CloudObjectTypeAndId::from_generic_string_object(
                        GenericStringObjectFormat::Json(
                            crate::cloud_object::JsonObjectType::EnvVarCollection,
                        ),
                        env_var_collection_id,
                    ),
                    ctx,
                );
            });
            ctx.notify();
        }
    }

    pub(super) fn export_env_var_collection(&self, ctx: &mut ViewContext<Self>) {
        if let Some(env_var_collection_id) = self.env_var_collection_id(ctx) {
            let window_id = ctx.window_id();
            ExportManager::handle(ctx).update(ctx, |export_manager, ctx| {
                export_manager.export(
                    window_id,
                    &[CloudObjectTypeAndId::from_generic_string_object(
                        GenericStringObjectFormat::Json(
                            crate::cloud_object::JsonObjectType::EnvVarCollection,
                        ),
                        env_var_collection_id,
                    )],
                    ctx,
                )
            });
        }
    }
}
