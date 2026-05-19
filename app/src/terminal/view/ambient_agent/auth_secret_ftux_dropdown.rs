use warp_cli::agent::Harness;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Empty, Expanded, Flex, Hoverable, MainAxisSize, MouseStateHandle, OffsetPositioning,
    ParentAnchor, ParentElement as _, ParentOffsetBounds, Radius, Stack, Text,
};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::auth_secret_types::auth_secret_types_for_harness;
use crate::ai::harness_availability::{
    AuthSecretFetchState, HarnessAvailabilityEvent, HarnessAvailabilityModel,
};
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpEscapeKey, PropagateAndNoOpNavigationKeys,
    SingleLineEditorOptions, TextOptions,
};
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields, MenuVariant};
use crate::ui_components::icons::Icon;
use warp_editor::editor::NavigationKey;

const MENU_WIDTH: f32 = 720.;

const FONT_SIZE: f32 = 14.;

const HELPER_FONT_SIZE: f32 = 12.;

const SELECT_HORIZONTAL_PADDING: f32 = 12.;

const SELECT_VERTICAL_PADDING: f32 = 8.;

const SELECT_GAP: f32 = 6.;

const SELECT_ICON_SIZE: f32 = 16.;

const SELECT_CORNER_RADIUS: f32 = 4.;

const MENU_HORIZONTAL_PADDING: f32 = 16.;

const MENU_ITEM_VERTICAL_PADDING: f32 = 8.;

const LABEL_TO_SELECT_SPACING: f32 = 4.;

const MENU_MAX_HEIGHT: f32 = 300.;

#[derive(Clone, Debug, PartialEq)]
pub enum FtuxDropdownAction {
    SelectSecret(String),
    SelectNewType(usize),
    ClearDisplayLabel,
    Skip,
}

pub enum FtuxDropdownEvent {
    SecretSelected(String),
    NewTypeSelected { harness: Harness, type_index: usize },
    Opened,
    Closed,
    DisplayLabelCleared,
    SkipRequested,
}

pub struct AuthSecretFtuxDropdown {
    search_editor: ViewHandle<EditorView>,
    search_query: String,
    menu: ViewHandle<Menu<FtuxDropdownAction>>,
    is_menu_open: bool,
    harness: Harness,
    display_label: Option<String>,
    label_mouse_state: MouseStateHandle,
    /// Compact mode (orchestration modal): no auto-open, suppresses the
    /// existing-secrets list and Skip, shows only "+ New …" entries.
    compact_mode: bool,
}

impl AuthSecretFtuxDropdown {
    pub fn new(harness: Harness, ctx: &mut ViewContext<Self>) -> Self {
        let search_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(FONT_SIZE), appearance),
                    select_all_on_focus: false,
                    clear_selections_on_blur: true,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    propagate_and_no_op_escape_key: PropagateAndNoOpEscapeKey::PropagateFirst,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text("Search secrets or create a new one", ctx);
            editor
        });

        ctx.subscribe_to_view(&search_editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let menu = ctx.add_typed_action_view(|_ctx| {
            let mut menu = Menu::new()
                .with_width(MENU_WIDTH)
                .with_drop_shadow()
                .with_menu_variant(MenuVariant::scrollable())
                .prevent_interaction_with_other_elements();
            menu.set_height(MENU_MAX_HEIGHT);
            menu
        });

        ctx.subscribe_to_view(&menu, |me, _, event, ctx| match event {
            MenuEvent::Close { .. } => {
                me.set_menu_visibility(false, ctx);
            }
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });

        ctx.subscribe_to_model(
            &HarnessAvailabilityModel::handle(ctx),
            |me, _, event, ctx| match event {
                HarnessAvailabilityEvent::AuthSecretsLoaded
                | HarnessAvailabilityEvent::AuthSecretCreated { .. }
                | HarnessAvailabilityEvent::AuthSecretDeleted { .. }
                | HarnessAvailabilityEvent::AuthSecretsFetchFailed => {
                    me.refresh_menu(ctx);
                    ctx.notify();
                }
                HarnessAvailabilityEvent::Changed
                | HarnessAvailabilityEvent::AuthSecretCreationFailed { .. }
                | HarnessAvailabilityEvent::AuthSecretDeletionFailed { .. } => {}
            },
        );

        ctx.subscribe_to_model(&Appearance::handle(ctx), |me, _, _, ctx| {
            me.refresh_menu(ctx);
        });

        let mut me = Self {
            search_editor,
            search_query: String::new(),
            menu,
            is_menu_open: false,
            harness,
            display_label: None,
            label_mouse_state: MouseStateHandle::default(),
            compact_mode: false,
        };
        me.refresh_menu(ctx);
        me.set_menu_visibility(true, ctx);
        me
    }

    /// Toggle compact mode. See the `compact_mode` field for what changes.
    /// Idempotent.
    pub fn set_compact_mode(&mut self, compact: bool, ctx: &mut ViewContext<Self>) {
        if self.compact_mode == compact {
            return;
        }
        self.compact_mode = compact;
        if compact {
            self.set_menu_visibility(false, ctx);
        }
        self.refresh_menu(ctx);
        ctx.notify();
    }

    fn set_menu_visibility(&mut self, is_open: bool, ctx: &mut ViewContext<Self>) {
        if self.is_menu_open == is_open {
            return;
        }
        self.is_menu_open = is_open;
        if is_open {
            ctx.focus(&self.search_editor);
            ctx.emit(FtuxDropdownEvent::Opened);
        } else {
            self.search_editor.update(ctx, |editor, ctx| {
                editor.system_clear_buffer(true, ctx);
            });
            if !self.search_query.is_empty() {
                self.search_query.clear();
                self.refresh_menu(ctx);
            }
            ctx.emit(FtuxDropdownEvent::Closed);
        }
        ctx.notify();
    }

    pub fn select_previous_if_open(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_menu_open {
            self.menu.update(ctx, |menu, ctx| menu.select_previous(ctx));
        }
    }

    pub fn focus_search_editor(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.search_editor);
    }

    pub fn clear_display_label_quietly(&mut self) {
        self.display_label = None;
    }

    pub fn set_display_label(&mut self, label: Option<String>, ctx: &mut ViewContext<Self>) {
        self.display_label = label;
        if self.display_label.is_some() {
            self.set_menu_visibility(false, ctx);
        } else {
            self.search_editor.update(ctx, |editor, ctx| {
                editor.system_clear_buffer(true, ctx);
            });
            self.search_query.clear();
            self.refresh_menu(ctx);
            self.set_menu_visibility(true, ctx);
        }
        ctx.notify();
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Focused => {
                if !self.is_menu_open {
                    self.set_menu_visibility(true, ctx);
                }
            }
            EditorEvent::Edited(_) => {
                let new_query = self.search_editor.as_ref(ctx).buffer_text(ctx);
                if new_query != self.search_query {
                    self.search_query = new_query;
                    self.refresh_menu(ctx);
                }
                if !self.is_menu_open {
                    self.set_menu_visibility(true, ctx);
                }
            }
            EditorEvent::Navigate(NavigationKey::Up) => {
                self.menu.update(ctx, |menu, ctx| menu.select_previous(ctx));
            }
            EditorEvent::Navigate(NavigationKey::Down) => {
                self.menu.update(ctx, |menu, ctx| menu.select_next(ctx));
            }
            EditorEvent::Enter => {
                let selected_action =
                    self.menu
                        .as_ref(ctx)
                        .selected_item()
                        .and_then(|item| match item {
                            MenuItem::Item(fields) => fields.on_select_action().cloned(),
                            _ => None,
                        });
                if let Some(action) = selected_action {
                    <Self as TypedActionView>::handle_action(self, &action, ctx);
                }
            }
            EditorEvent::Escape => {
                self.set_menu_visibility(false, ctx);
            }
            _ => {}
        }
    }

    /// Updates the harness this dropdown should query secrets for. Resets the
    /// search query / editor and refreshes the menu, then lazily fetches
    /// secrets for the new harness if the menu is currently open.
    pub fn set_harness(&mut self, harness: Harness, ctx: &mut ViewContext<Self>) {
        if self.harness == harness {
            return;
        }
        self.harness = harness;
        self.search_query.clear();
        self.search_editor.update(ctx, |editor, ctx| {
            editor.system_clear_buffer(true, ctx);
        });
        if self.is_menu_open {
            HarnessAvailabilityModel::handle(ctx).update(ctx, |model, ctx| {
                model.ensure_auth_secrets_fetched(harness, ctx);
            });
        }
        self.refresh_menu(ctx);
        ctx.notify();
    }

    fn matching_secret_count(&self, app: &AppContext) -> usize {
        let harness = self.harness;
        let availability = HarnessAvailabilityModel::as_ref(app);
        let query = self.search_query.trim().to_lowercase();
        match availability.auth_secrets_for(harness) {
            AuthSecretFetchState::Loaded(secrets) => {
                if query.is_empty() {
                    secrets.len()
                } else {
                    secrets
                        .iter()
                        .filter(|s| s.name.to_lowercase().contains(&query))
                        .count()
                }
            }
            _ => 0,
        }
    }

    fn refresh_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();
        let hover_background: Fill = internal_colors::fg_overlay_2(theme);
        let disabled_text_color = theme.disabled_text_color(theme.surface_2()).into_solid();
        let border = Border::all(1.).with_border_color(internal_colors::neutral_4(theme));

        let harness = self.harness;
        let availability = HarnessAvailabilityModel::as_ref(ctx);
        let query = self.search_query.trim().to_lowercase();
        let compact = self.compact_mode;

        let mut items: Vec<MenuItem<FtuxDropdownAction>> = Vec::new();

        let no_results_text_color = internal_colors::text_sub(theme, theme.surface_2());

        if compact {
            // Compact (modal) mode: only render "+ New …" entries.
            for (index, info) in auth_secret_types_for_harness(harness).iter().enumerate() {
                items.push(MenuItem::Item(
                    MenuItemFields::new(format!("New {}", info.display_name))
                        .with_font_size_override(FONT_SIZE)
                        .with_padding_override(MENU_ITEM_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
                        .with_override_hover_background_color(hover_background)
                        .with_icon(Icon::Plus)
                        .with_on_select_action(FtuxDropdownAction::SelectNewType(index)),
                ));
            }
            self.menu.update(ctx, |menu, ctx| {
                menu.set_border(Some(border));
                menu.set_items(items, ctx);
            });
            return;
        }

        match availability.auth_secrets_for(harness) {
            AuthSecretFetchState::Loaded(secrets) => {
                let mut matched = false;
                for secret in secrets {
                    if !query.is_empty() && !secret.name.to_lowercase().contains(&query) {
                        continue;
                    }
                    matched = true;
                    items.push(MenuItem::Item(
                        MenuItemFields::new(secret.name.clone())
                            .with_font_size_override(FONT_SIZE)
                            .with_padding_override(
                                MENU_ITEM_VERTICAL_PADDING,
                                MENU_HORIZONTAL_PADDING,
                            )
                            .with_override_hover_background_color(hover_background)
                            .with_on_select_action(FtuxDropdownAction::SelectSecret(
                                secret.name.clone(),
                            )),
                    ));
                }
                if !matched {
                    items.push(MenuItem::Item(
                        MenuItemFields::new("No secrets found")
                            .with_font_size_override(FONT_SIZE)
                            .with_padding_override(
                                MENU_ITEM_VERTICAL_PADDING,
                                MENU_HORIZONTAL_PADDING,
                            )
                            .with_override_text_color(no_results_text_color)
                            .with_no_interaction_on_hover(),
                    ));
                }
            }
            AuthSecretFetchState::NotFetched | AuthSecretFetchState::Loading => {
                items.push(MenuItem::Item(
                    MenuItemFields::new("Loading…")
                        .with_font_size_override(FONT_SIZE)
                        .with_padding_override(MENU_ITEM_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
                        .with_disabled(true)
                        .with_override_text_color(disabled_text_color),
                ));
            }
            AuthSecretFetchState::Failed(_) => {
                items.push(MenuItem::Item(
                    MenuItemFields::new("Unable to load secrets")
                        .with_font_size_override(FONT_SIZE)
                        .with_padding_override(MENU_ITEM_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
                        .with_disabled(true)
                        .with_override_text_color(disabled_text_color),
                ));
            }
        }

        items.push(MenuItem::Separator);

        for (index, info) in auth_secret_types_for_harness(harness).iter().enumerate() {
            items.push(MenuItem::Item(
                MenuItemFields::new(format!("New {}", info.display_name))
                    .with_font_size_override(FONT_SIZE)
                    .with_padding_override(MENU_ITEM_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
                    .with_override_hover_background_color(hover_background)
                    .with_icon(Icon::Plus)
                    .with_on_select_action(FtuxDropdownAction::SelectNewType(index)),
            ));
        }

        items.push(MenuItem::Separator);

        items.push(MenuItem::Item(
            MenuItemFields::new_with_label(
                "Skip (advanced)",
                "Only if your key is already set in the environment (e.g. injected as a Kubernetes secret)",
            )
            .with_font_size_override(FONT_SIZE)
            .with_padding_override(MENU_ITEM_VERTICAL_PADDING, MENU_HORIZONTAL_PADDING)
            .with_override_hover_background_color(hover_background)
            .with_on_select_action(FtuxDropdownAction::Skip),
        ));

        self.menu.update(ctx, |menu, ctx| {
            menu.set_border(Some(border));
            menu.set_items(items, ctx);
        });
    }

    fn render_select_container(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let border_color = if self.is_menu_open {
            theme.accent().into_solid()
        } else {
            internal_colors::neutral_3(theme)
        };

        let right_icon = if self.is_menu_open {
            Icon::ChevronDown
        } else {
            Icon::Key
        };

        let icon_color: Fill = internal_colors::text_sub(theme, theme.surface_1()).into();

        let search_icon = ConstrainedBox::new(Icon::Search.to_warpui_icon(icon_color).finish())
            .with_height(SELECT_ICON_SIZE)
            .with_width(SELECT_ICON_SIZE)
            .finish();

        let right_icon_element =
            ConstrainedBox::new(right_icon.to_warpui_icon(icon_color).finish())
                .with_height(SELECT_ICON_SIZE)
                .with_width(SELECT_ICON_SIZE)
                .finish();

        let center: Box<dyn Element> = if let Some(label) = &self.display_label {
            Expanded::new(
                1.,
                Text::new_inline(label.clone(), appearance.ui_font_family(), FONT_SIZE)
                    .with_color(theme.foreground().into())
                    .finish(),
            )
            .finish()
        } else {
            Expanded::new(1., ChildView::new(&self.search_editor).finish()).finish()
        };

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(SELECT_GAP)
            .with_child(search_icon)
            .with_child(center)
            .with_child(right_icon_element)
            .finish();

        let container = Container::new(row)
            .with_padding_left(SELECT_HORIZONTAL_PADDING)
            .with_padding_right(SELECT_HORIZONTAL_PADDING)
            .with_padding_top(SELECT_VERTICAL_PADDING)
            .with_padding_bottom(SELECT_VERTICAL_PADDING)
            .with_border(Border::all(1.).with_border_color(border_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(SELECT_CORNER_RADIUS)))
            .finish();

        if self.display_label.is_some() {
            Hoverable::new(self.label_mouse_state.clone(), move |_| container)
                .with_cursor(warpui::platform::Cursor::PointingHand)
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(FtuxDropdownAction::ClearDisplayLabel);
                })
                .finish()
        } else {
            container
        }
    }

    fn render_helper_text(&self, app: &AppContext) -> Box<dyn Element> {
        if self.search_query.trim().is_empty() || self.matching_secret_count(app) > 0 {
            return Empty::new().finish();
        }
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let color = internal_colors::text_sub(theme, theme.surface_1());
        Text::new_inline(
            "No secrets found. Save to use this value directly or click the key to add a secret."
                .to_string(),
            appearance.ui_font_family(),
            HELPER_FONT_SIZE,
        )
        .with_color(color)
        .soft_wrap(true)
        .finish()
    }

    fn menu_positioning(&self) -> OffsetPositioning {
        OffsetPositioning::offset_from_parent(
            warpui::geometry::vector::vec2f(0., 0.),
            ParentOffsetBounds::WindowByPosition,
            ParentAnchor::BottomLeft,
            ChildAnchor::TopLeft,
        )
    }
}

impl Entity for AuthSecretFtuxDropdown {
    type Event = FtuxDropdownEvent;
}

impl TypedActionView for AuthSecretFtuxDropdown {
    type Action = FtuxDropdownAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FtuxDropdownAction::SelectSecret(name) => {
                ctx.emit(FtuxDropdownEvent::SecretSelected(name.clone()));
                self.set_menu_visibility(false, ctx);
            }
            FtuxDropdownAction::SelectNewType(type_index) => {
                ctx.emit(FtuxDropdownEvent::NewTypeSelected {
                    harness: self.harness,
                    type_index: *type_index,
                });
                self.set_menu_visibility(false, ctx);
            }
            FtuxDropdownAction::ClearDisplayLabel => {
                self.set_display_label(None, ctx);
                ctx.emit(FtuxDropdownEvent::DisplayLabelCleared);
            }
            FtuxDropdownAction::Skip => {
                self.set_menu_visibility(false, ctx);
                ctx.emit(FtuxDropdownEvent::SkipRequested);
            }
        }
    }
}

impl View for AuthSecretFtuxDropdown {
    fn ui_name() -> &'static str {
        "AuthSecretFtuxDropdown"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut stack = Stack::new();
        stack.add_child(self.render_select_container(app));

        if self.is_menu_open {
            stack.add_positioned_overlay_child(
                ChildView::new(&self.menu).finish(),
                self.menu_positioning(),
            );
        }

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(LABEL_TO_SELECT_SPACING);
        column.add_child(stack.finish());

        let helper = self.render_helper_text(app);
        column.add_child(helper);

        column.finish()
    }
}
