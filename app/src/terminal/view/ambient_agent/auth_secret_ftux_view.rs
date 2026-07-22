use warp_cli::agent::Harness;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warp_core::ui::theme::color::internal_colors;
use warp_editor::editor::NavigationKey;
use warp_managed_secrets::client::SecretOwner;
use warpui::elements::{
    Border, ChildAnchor, ChildView, Clipped, ConstrainedBox, Container, CornerRadius,
    CrossAxisAlignment, Empty, Expanded, Flex, Hoverable, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement as _, ParentOffsetBounds,
    Radius, Stack, Text, Wrap,
};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::auth_secret_types::{
    AuthSecretTypeInfo, auth_secret_types_for_harness, build_managed_secret_value,
    learn_more_url_for_harness,
};
use crate::ai::harness_availability::{HarnessAvailabilityEvent, HarnessAvailabilityModel};
use crate::ai::harness_display;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpEscapeKey, PropagateAndNoOpNavigationKeys,
    SingleLineEditorOptions, TextOptions,
};
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields};
use crate::terminal::view::ambient_agent::auth_secret_ftux_dropdown::{
    AuthSecretFtuxDropdown, FtuxDropdownEvent,
};
use crate::ui_components::icons::Icon as UiIcon;
use crate::view_components::DismissibleToast;
use crate::workspace::ToastStack;
use crate::workspaces::user_workspaces::UserWorkspaces;

const DESCRIPTION_FONT_SIZE: f32 = 14.;

const FIELD_LABEL_FONT_SIZE: f32 = 10.;

const BUTTON_FONT_SIZE: f32 = 14.;

const EDITOR_FONT_SIZE: f32 = 14.;

const CONTENT_SECTION_SPACING: f32 = 12.;

const FORM_FIELD_SPACING: f32 = 8.;

const BUTTON_PADDING: f32 = 8.;

const FIELD_EDITOR_PADDING: f32 = 8.;

const CORNER_RADIUS: f32 = 4.;

const FIELD_EDITOR_MIN_HEIGHT: f32 = 32.;

const TYPE_DESCRIPTION_FONT_SIZE: f32 = 12.;

const CHECKBOX_SIZE: f32 = 14.;

#[derive(Clone, Debug, PartialEq)]
pub enum AuthSecretFtuxAction {
    Skip,
    Cancel,
    Continue,
    /// Cancel an in-progress creation form and return to the dropdown.
    Back,
    /// Open the harness-specific documentation in a browser.
    LearnMore(&'static str),
    /// Toggle the "Share with team" checkbox on the creation form.
    ToggleTeamScope,
    /// Toggle the compact-mode harness picker. Ignored when compact mode
    /// is disabled (cloud mode).
    ToggleHarnessMenu,
    /// Picks a harness in compact mode. The view forwards to
    /// `set_harness`, which clears in-progress creation state and
    /// re-enters creation for the new harness's first type.
    SelectHarness(Harness),
}

/// Lifecycle events the view emits to its host. Side effects (persist
/// selection, mark FTUX complete, etc.) are the host's responsibility so the
/// same view can be embedded in cloud mode or in a workspace-level modal.
#[derive(Debug, Clone)]
pub enum AuthSecretFtuxViewEvent {
    /// User picked an existing secret from the in-view dropdown.
    SecretSelected { harness: Harness, name: String },
    /// User created a new secret via the form.
    Created { harness: Harness, name: String },
    /// User dismissed the form via Cancel.
    Cancelled,
    /// User skipped via the in-dropdown "Skip" item.
    Skipped { harness: Harness },
    /// `create_auth_secret` failed. The view also shows a toast.
    Failed { error: String },
}

struct SecretCreationState {
    harness: Harness,
    secret_type_index: usize,
    is_saving: bool,
    /// The exact name we fired at `create_auth_secret`. Set when
    /// `handle_continue` dispatches; cleared on success / failure /
    /// cancel. Used to filter the global `AuthSecretCreated` event so
    /// another concurrent FTUX view's success can't close us.
    pending_name: Option<String>,
}

/// Validated, ready-to-submit snapshot of the creation form.
struct ValidatedForm {
    harness: Harness,
    info: &'static AuthSecretTypeInfo,
    /// Trimmed; non-empty.
    name: String,
    /// One per `info.fields`. Required fields are non-empty after trimming.
    field_values: Vec<String>,
}

pub struct AuthSecretFtuxView {
    harness: Harness,
    ftux_dropdown: ViewHandle<AuthSecretFtuxDropdown>,
    name_editor: ViewHandle<EditorView>,
    field_editors: Vec<ViewHandle<EditorView>>,
    creation_state: Option<SecretCreationState>,
    back_button_mouse_state: MouseStateHandle,
    share_with_team: bool,
    continue_mouse_state: MouseStateHandle,
    learn_more_mouse_state: MouseStateHandle,
    team_checkbox_mouse_state: MouseStateHandle,
    /// Suppresses the in-dropdown "Skip" item; default `false` (cloud mode
    /// keeps Skip, orchestration modal hides it).
    skip_hidden: bool,
    /// Compact (orchestration-modal) presentation: no description, dropdown
    /// hides existing secrets and Skip, auto-enters creation for the first
    /// secret type, and renders a harness picker above the form.
    compact_mode: bool,
    harness_picker_mouse_state: MouseStateHandle,
    is_harness_menu_open: bool,
    /// Lazily built; only in compact mode.
    harness_menu: Option<ViewHandle<Menu<AuthSecretFtuxAction>>>,
}

impl AuthSecretFtuxView {
    pub fn new(harness: Harness, ctx: &mut ViewContext<Self>) -> Self {
        let name_editor = make_single_line_editor(Some("e.g. My API Key"), false, ctx);

        ctx.subscribe_to_view(&name_editor, |me, _, event, ctx| {
            me.handle_form_editor_event(0, event, ctx);
        });

        let ftux_dropdown =
            ctx.add_typed_action_view(|ctx| AuthSecretFtuxDropdown::new(harness, ctx));

        ctx.subscribe_to_view(&ftux_dropdown, |me, _, event, ctx| {
            if matches!(event, FtuxDropdownEvent::Opened) {
                let harness = me.harness;
                HarnessAvailabilityModel::handle(ctx).update(ctx, |model, ctx| {
                    model.ensure_auth_secrets_fetched(harness, ctx);
                });
            }
        });

        ctx.subscribe_to_view(&ftux_dropdown, |me, _, event, ctx| {
            if let FtuxDropdownEvent::SecretSelected(name) = event {
                me.clear_all_editor_buffers(ctx);
                me.creation_state = None;
                me.field_editors.clear();
                ctx.emit(AuthSecretFtuxViewEvent::SecretSelected {
                    harness: me.harness,
                    name: name.clone(),
                });
                ctx.notify();
            }
        });

        ctx.subscribe_to_view(&ftux_dropdown, |me, _, event, ctx| match event {
            FtuxDropdownEvent::NewTypeSelected {
                harness,
                type_index,
            } => {
                me.enter_creation_state(*harness, *type_index, ctx);
            }
            FtuxDropdownEvent::DisplayLabelCleared => {
                ctx.notify();
            }
            FtuxDropdownEvent::SkipRequested => {
                me.handle_skip(ctx);
            }
            _ => {}
        });

        ctx.subscribe_to_model(
            &HarnessAvailabilityModel::handle(ctx),
            |me, _, event, ctx| match event {
                HarnessAvailabilityEvent::AuthSecretCreated { harness, name } => {
                    // Only consume the event when it matches the request
                    // *this* view actually fired. Without the harness/name
                    // match a concurrent FTUX view's success would close
                    // us (or worse, emit a Created event the host then
                    // routes to the wrong modal).
                    let is_ours = me.creation_state.as_ref().is_some_and(|state| {
                        state.is_saving
                            && state.harness == *harness
                            && state.pending_name.as_deref() == Some(name.as_str())
                    });
                    if is_ours {
                        me.handle_secret_created(*harness, name.clone(), ctx);
                    }
                }
                HarnessAvailabilityEvent::AuthSecretCreationFailed { error } => {
                    // Only react if *we* are mid-save; otherwise this
                    // failure belongs to another FTUX view's request.
                    if let Some(state) = me.creation_state.as_mut() {
                        if !state.is_saving {
                            return;
                        }
                        state.is_saving = false;
                        state.pending_name = None;
                        let window_id = ctx.window_id();
                        let message = format!("Failed to save API key: {error}");
                        ToastStack::handle(ctx).update(ctx, |ts, ctx| {
                            ts.add_ephemeral_toast(
                                DismissibleToast::error(message),
                                window_id,
                                ctx,
                            );
                        });
                        ctx.emit(AuthSecretFtuxViewEvent::Failed {
                            error: error.clone(),
                        });
                        ctx.notify();
                    }
                }
                HarnessAvailabilityEvent::Changed
                | HarnessAvailabilityEvent::AuthSecretsLoaded
                | HarnessAvailabilityEvent::AuthSecretsFetchFailed
                | HarnessAvailabilityEvent::AuthSecretDeleted { .. }
                | HarnessAvailabilityEvent::AuthSecretDeletionFailed { .. } => {}
            },
        );

        Self {
            harness,
            ftux_dropdown,
            name_editor,
            field_editors: Vec::new(),
            creation_state: None,
            back_button_mouse_state: MouseStateHandle::default(),
            share_with_team: false,
            continue_mouse_state: MouseStateHandle::default(),
            learn_more_mouse_state: MouseStateHandle::default(),
            team_checkbox_mouse_state: MouseStateHandle::default(),
            skip_hidden: false,
            compact_mode: false,
            harness_picker_mouse_state: MouseStateHandle::default(),
            is_harness_menu_open: false,
            harness_menu: None,
        }
    }

    /// Switch to the orchestration modal's compact presentation. See the
    /// `compact_mode` field for what changes.
    pub fn with_compact_mode(mut self, ctx: &mut ViewContext<Self>) -> Self {
        self.compact_mode = true;
        let dropdown = self.ftux_dropdown.clone();
        dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_compact_mode(true, ctx);
        });
        // Auto-enter creation state for the harness's first secret type.
        let harness = self.harness;
        self.enter_creation_state(harness, 0, ctx);
        // Build the harness picker menu lazily.
        let menu = ctx.add_typed_action_view(|_ctx| {
            Menu::new()
                .with_width(220.)
                .with_drop_shadow()
                .prevent_interaction_with_other_elements()
        });
        ctx.subscribe_to_view(&menu, |me, _, event, ctx| match event {
            MenuEvent::Close { .. } => {
                if me.is_harness_menu_open {
                    me.is_harness_menu_open = false;
                    ctx.notify();
                }
            }
            MenuEvent::ItemSelected | MenuEvent::ItemHovered => {}
        });
        self.harness_menu = Some(menu);
        self.refresh_harness_menu(ctx);
        self
    }

    /// Returns true when the view is rendering in the compact (modal)
    /// presentation.
    pub fn is_compact_mode(&self) -> bool {
        self.compact_mode
    }

    /// Hide the in-dropdown "Skip" affordance. Used by modal hosts.
    pub fn with_skip_hidden(mut self) -> Self {
        self.skip_hidden = true;
        self
    }

    /// Returns the harness this view is currently targeting.
    pub fn harness(&self) -> Harness {
        self.harness
    }

    /// Retarget at a new harness: clear in-progress creation state and
    /// propagate the harness change to the embedded dropdown.
    pub fn set_harness(&mut self, harness: Harness, ctx: &mut ViewContext<Self>) {
        if self.harness == harness {
            return;
        }
        self.harness = harness;
        self.clear_creation_state(ctx);
        let dropdown = self.ftux_dropdown.clone();
        dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_harness(harness, ctx);
        });
        if self.compact_mode {
            // Compact mode keeps the form always-visible, so re-enter
            // creation state for the new harness's first secret type.
            self.enter_creation_state(harness, 0, ctx);
            self.refresh_harness_menu(ctx);
        }
        ctx.notify();
    }

    /// Rebuilds the compact harness picker's menu items. Only invoked in
    /// compact mode; a no-op when the harness menu hasn't been built.
    fn refresh_harness_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(menu) = self.harness_menu.clone() else {
            return;
        };
        let current = self.harness;
        let availability = HarnessAvailabilityModel::handle(ctx);
        let entries: Vec<(Harness, String, bool)> = availability
            .as_ref(ctx)
            .available_harnesses()
            .iter()
            .filter(|entry| {
                // Only show harnesses that support managed auth secrets,
                // since this modal exists to create one.
                !auth_secret_types_for_harness(entry.harness).is_empty()
            })
            .map(|entry| (entry.harness, entry.display_name.clone(), entry.enabled))
            .collect();
        let current_display = entries
            .iter()
            .find(|(h, _, _)| *h == current)
            .map(|(_, name, _)| name.clone());
        menu.update(ctx, |menu, menu_ctx| {
            let mut items: Vec<MenuItem<AuthSecretFtuxAction>> = Vec::new();
            for (harness, display_name, enabled) in &entries {
                let mut fields = MenuItemFields::new(display_name)
                    .with_icon(harness_display::icon_for(*harness));
                if let Some(color) = harness_display::brand_color(*harness) {
                    fields = fields.with_override_icon_color(Fill::from(color));
                }
                if *enabled {
                    fields =
                        fields.with_on_select_action(AuthSecretFtuxAction::SelectHarness(*harness));
                } else {
                    fields = fields.with_disabled(true);
                }
                items.push(MenuItem::Item(fields));
            }
            menu.set_items(items, menu_ctx);
            if let Some(name) = &current_display {
                menu.set_selected_by_name(name, menu_ctx);
            }
        });
    }

    fn handle_toggle_harness_menu(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.compact_mode || self.harness_menu.is_none() {
            return;
        }
        self.is_harness_menu_open = !self.is_harness_menu_open;
        if self.is_harness_menu_open {
            // Refresh in case the available-harnesses list changed since
            // the last open.
            self.refresh_harness_menu(ctx);
            if let Some(menu) = &self.harness_menu {
                ctx.focus(menu);
            }
        }
        ctx.notify();
    }

    fn render_compact_harness_picker(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let harness = self.harness;
        let display_name = harness_display::display_name(harness).to_string();
        let icon_color: Fill = harness_display::brand_color(harness)
            .map(Fill::from)
            .unwrap_or_else(|| internal_colors::text_sub(theme, theme.surface_1()).into());

        let leading_icon = ConstrainedBox::new(
            harness_display::icon_for(harness)
                .to_warpui_icon(icon_color)
                .finish(),
        )
        .with_width(16.)
        .with_height(16.)
        .finish();

        let label = Text::new_inline(
            display_name,
            appearance.ui_font_family(),
            DESCRIPTION_FONT_SIZE,
        )
        .with_color(theme.foreground().into())
        .with_style(Properties::default().weight(Weight::Semibold))
        .finish();

        let chevron = ConstrainedBox::new(
            UiIcon::ChevronDown
                .to_warpui_icon(internal_colors::text_sub(theme, theme.surface_1()).into())
                .finish(),
        )
        .with_width(12.)
        .with_height(12.)
        .finish();

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(6.)
            .with_child(leading_icon)
            .with_child(label)
            .with_child(chevron)
            .finish();

        let trigger = Hoverable::new(self.harness_picker_mouse_state.clone(), move |hover| {
            let mut container = Container::new(row)
                .with_padding_left(8.)
                .with_padding_right(8.)
                .with_padding_top(4.)
                .with_padding_bottom(4.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)));
            if hover.is_hovered() {
                container = container.with_background(internal_colors::fg_overlay_1(theme));
            }
            container.finish()
        })
        .with_cursor(warpui::platform::Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(AuthSecretFtuxAction::ToggleHarnessMenu);
        })
        .finish();

        let mut stack = Stack::new();
        stack.add_child(trigger);
        if self.is_harness_menu_open
            && let Some(menu) = &self.harness_menu
        {
            stack.add_positioned_overlay_child(
                ChildView::new(menu).finish(),
                OffsetPositioning::offset_from_parent(
                    warpui::geometry::vector::vec2f(0., 4.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }
        stack.finish()
    }

    pub fn has_creation_state(&self) -> bool {
        self.creation_state.is_some()
    }

    pub fn select_previous_in_dropdown(&self, ctx: &mut ViewContext<Self>) {
        self.ftux_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.select_previous_if_open(ctx);
        });
    }

    pub fn focus_dropdown_editor(&self, ctx: &mut ViewContext<Self>) {
        self.ftux_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.focus_search_editor(ctx);
        });
    }

    pub fn enter_creation_state_public(
        &mut self,
        harness: Harness,
        type_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        self.enter_creation_state(harness, type_index, ctx);
    }

    fn all_form_editors(&self) -> Vec<&ViewHandle<EditorView>> {
        let mut editors = vec![&self.name_editor];
        editors.extend(self.field_editors.iter());
        editors
    }

    fn focus_form_editor(&self, index: usize, ctx: &mut ViewContext<Self>) {
        let editors = self.all_form_editors();
        if let Some(editor) = editors.get(index % editors.len()) {
            ctx.focus(editor);
        }
    }

    fn handle_form_editor_event(
        &self,
        editor_index: usize,
        event: &EditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        let count = self.all_form_editors().len();
        if count == 0 {
            return;
        }
        match event {
            EditorEvent::Navigate(key) => match key {
                NavigationKey::Tab => {
                    self.focus_form_editor((editor_index + 1) % count, ctx);
                }
                NavigationKey::ShiftTab => {
                    let prev = if editor_index == 0 {
                        count - 1
                    } else {
                        editor_index - 1
                    };
                    self.focus_form_editor(prev, ctx);
                }
                _ => {}
            },
            // Re-render so the Continue button restyles as the user types.
            EditorEvent::Edited(_) => ctx.notify(),
            _other => {}
        }
    }

    fn enter_creation_state(
        &mut self,
        harness: Harness,
        type_index: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        let info = match auth_secret_types_for_harness(harness).get(type_index) {
            Some(info) => info,
            None => return,
        };
        let mut editors = Vec::with_capacity(info.fields.len());
        for (field_idx, field) in info.fields.iter().enumerate() {
            let placeholder = field.placeholder.unwrap_or(field.label);
            let editor = make_single_line_editor(Some(placeholder), field.sensitive, ctx);
            let editor_index = field_idx + 1;
            ctx.subscribe_to_view(&editor, move |me, _, event, ctx| {
                me.handle_form_editor_event(editor_index, event, ctx);
            });
            editors.push(editor);
        }
        self.field_editors = editors;
        self.share_with_team = false;
        self.creation_state = Some(SecretCreationState {
            harness,
            secret_type_index: type_index,
            is_saving: false,
            pending_name: None,
        });
        self.ftux_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_display_label(Some(info.display_name.to_string()), ctx);
        });
        ctx.focus(&self.name_editor);
        ctx.notify();
    }

    fn current_type_info(&self) -> Option<&'static AuthSecretTypeInfo> {
        let state = self.creation_state.as_ref()?;
        auth_secret_types_for_harness(state.harness).get(state.secret_type_index)
    }

    /// Validated form snapshot; `None` if not ready to submit. Single
    /// source of truth for both `can_submit_creation_form` and
    /// `handle_continue`.
    fn validated_form_snapshot(&self, ctx: &AppContext) -> Option<ValidatedForm> {
        let state = self.creation_state.as_ref()?;
        if state.is_saving {
            return None;
        }
        let info = auth_secret_types_for_harness(state.harness).get(state.secret_type_index)?;
        if self.field_editors.len() != info.fields.len() {
            return None;
        }
        let name = self
            .name_editor
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_string();
        if name.is_empty() {
            return None;
        }
        let field_values: Vec<String> = self
            .field_editors
            .iter()
            .map(|e| e.as_ref(ctx).buffer_text(ctx))
            .collect();
        for (field, value) in info.fields.iter().zip(field_values.iter()) {
            if !field.optional && value.trim().is_empty() {
                return None;
            }
        }
        Some(ValidatedForm {
            harness: state.harness,
            info,
            name,
            field_values,
        })
    }

    /// Whether the Continue button should be enabled.
    fn can_submit_creation_form(&self, ctx: &AppContext) -> bool {
        self.validated_form_snapshot(ctx).is_some()
    }

    fn handle_skip(&mut self, ctx: &mut ViewContext<Self>) {
        if self.skip_hidden {
            return;
        }
        let harness = self.harness;
        self.clear_all_editor_buffers(ctx);
        self.creation_state = None;
        self.field_editors.clear();
        self.ftux_dropdown.update(ctx, |dropdown, _ctx| {
            dropdown.clear_display_label_quietly();
        });
        ctx.emit(AuthSecretFtuxViewEvent::Skipped { harness });
        ctx.notify();
    }

    fn handle_cancel(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(AuthSecretFtuxViewEvent::Cancelled);
    }

    fn handle_continue(&mut self, ctx: &mut ViewContext<Self>) {
        // The button is disabled when the form isn't ready; no-op if we
        // got dispatched anyway.
        let Some(form) = self.validated_form_snapshot(ctx) else {
            return;
        };
        let ValidatedForm {
            harness,
            info,
            name,
            field_values,
        } = form;

        let value = match build_managed_secret_value(info, &field_values) {
            Ok(v) => v,
            Err(err) => {
                // Defensive: `validated_form_snapshot` already enforces
                // the same required-field rules.
                let msg = err.to_string();
                HarnessAvailabilityModel::handle(ctx).update(ctx, |_model, ctx| {
                    ctx.emit(HarnessAvailabilityEvent::AuthSecretCreationFailed { error: msg });
                });
                return;
            }
        };

        if let Some(state) = self.creation_state.as_mut() {
            state.is_saving = true;
            state.pending_name = Some(name.clone());
        }
        ctx.notify();

        let owner = self.resolve_secret_owner(ctx);
        HarnessAvailabilityModel::handle(ctx).update(ctx, |model, ctx| {
            model.create_auth_secret(harness, name, value, owner, ctx);
        });
    }

    fn resolve_secret_owner(&self, ctx: &ViewContext<Self>) -> SecretOwner {
        if self.share_with_team
            && let Some(team) = UserWorkspaces::as_ref(ctx).current_team()
        {
            return SecretOwner::Team {
                team_uid: team.uid.uid(),
            };
        }
        SecretOwner::CurrentUser
    }

    fn clear_creation_state(&mut self, ctx: &mut ViewContext<Self>) {
        if self.creation_state.is_some() {
            self.creation_state = None;
            self.share_with_team = false;
            self.clear_all_editor_buffers(ctx);
            self.field_editors.clear();
            self.ftux_dropdown.update(ctx, |dropdown, ctx| {
                dropdown.set_display_label(None, ctx);
            });
        }
    }

    fn clear_all_editor_buffers(&self, ctx: &mut ViewContext<Self>) {
        self.name_editor.update(ctx, |editor, ctx| {
            editor.system_clear_buffer(true, ctx);
        });
        for editor in &self.field_editors {
            editor.update(ctx, |editor, ctx| {
                editor.system_clear_buffer(true, ctx);
            });
        }
    }

    fn handle_secret_created(
        &mut self,
        harness: Harness,
        name: String,
        ctx: &mut ViewContext<Self>,
    ) {
        let window_id = ctx.window_id();
        let message = format!("API key '{name}' saved.");
        ToastStack::handle(ctx).update(ctx, |ts, ctx| {
            ts.add_ephemeral_toast(DismissibleToast::default(message), window_id, ctx);
        });
        self.clear_creation_state(ctx);
        ctx.emit(AuthSecretFtuxViewEvent::Created { harness, name });
        ctx.notify();
    }

    fn render_description(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let font_family = appearance.ui_font_family();
        let sub_color = internal_colors::text_sub(theme, theme.surface_1());
        let accent_color = theme.accent().into_solid();

        let main_text = {
            let description = if self.current_type_info().is_some() {
                "Enter your credentials below.".to_string()
            } else {
                let display_name = harness_display::display_name(self.harness);
                format!("Select an API key type to use {display_name} in the cloud with Oz.")
            };
            Text::new_inline(description, font_family, DESCRIPTION_FONT_SIZE)
                .with_color(theme.foreground().into())
                .soft_wrap(true)
                .finish()
        };

        let privacy_text = Text::new_inline(
            "Your credentials are encrypted end-to-end. ".to_string(),
            font_family,
            TYPE_DESCRIPTION_FONT_SIZE,
        )
        .with_color(sub_color)
        .soft_wrap(true)
        .finish();

        let harness_name = harness_display::display_name(self.harness);
        let learn_more_url = self
            .current_type_info()
            .map(|info| info.learn_more_url)
            .unwrap_or_else(|| learn_more_url_for_harness(self.harness));
        let learn_more_label =
            format!("Learn more about authentication for {harness_name} in Warp.");
        let learn_more = Hoverable::new(self.learn_more_mouse_state.clone(), move |state| {
            let color = if state.is_hovered() {
                accent_color
            } else {
                sub_color
            };
            Text::new_inline(
                learn_more_label.clone(),
                font_family,
                TYPE_DESCRIPTION_FONT_SIZE,
            )
            .with_color(color)
            .with_style(Properties::default().weight(Weight::Semibold))
            .soft_wrap(true)
            .finish()
        })
        .with_cursor(warpui::platform::Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(AuthSecretFtuxAction::LearnMore(learn_more_url));
        })
        .finish();

        let privacy_row = Wrap::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(privacy_text)
            .with_child(learn_more)
            .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(4.)
            .with_child(main_text)
            .with_child(privacy_row)
            .finish()
    }

    fn render_field_label(&self, label: &str, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let color = internal_colors::text_sub(theme, theme.surface_1());
        Text::new_inline(
            label.to_string(),
            appearance.ui_font_family(),
            FIELD_LABEL_FONT_SIZE,
        )
        .with_color(color)
        .finish()
    }

    fn render_editor_container(
        &self,
        editor: &ViewHandle<EditorView>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let border_color = internal_colors::neutral_3(theme);
        let background = internal_colors::fg_overlay_1(theme);
        let editor_element = ChildView::new(editor).finish();
        // The editor's text layout includes descender space below the baseline,
        // which makes text appear top-heavy when padding is symmetric. Bias the
        // top padding slightly larger to visually center the text.
        Clipped::new(
            ConstrainedBox::new(
                Container::new(editor_element)
                    .with_padding_left(FIELD_EDITOR_PADDING)
                    .with_padding_right(FIELD_EDITOR_PADDING)
                    .with_padding_top(FIELD_EDITOR_PADDING)
                    .with_padding_bottom(FIELD_EDITOR_PADDING / 3.)
                    .with_background(background)
                    .with_border(Border::all(1.).with_border_color(border_color))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)))
                    .finish(),
            )
            .with_min_height(FIELD_EDITOR_MIN_HEIGHT)
            .finish(),
        )
        .finish()
    }

    fn render_scope_checkbox(&self, app: &AppContext) -> Box<dyn Element> {
        let has_team = UserWorkspaces::as_ref(app).current_team().is_some();
        if !has_team {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(app);
        let zero = Coords::uniform(0.);
        let checkbox = appearance
            .ui_builder()
            .checkbox(self.team_checkbox_mouse_state.clone(), Some(CHECKBOX_SIZE))
            .with_style(UiComponentStyles {
                margin: Some(zero),
                ..Default::default()
            })
            .check(self.share_with_team)
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(AuthSecretFtuxAction::ToggleTeamScope);
            })
            .with_cursor(warpui::platform::Cursor::PointingHand)
            .finish();

        let theme = appearance.theme();
        let label_color = internal_colors::text_sub(theme, theme.surface_1());
        let label = Text::new_inline(
            "Share with team".to_string(),
            appearance.ui_font_family(),
            TYPE_DESCRIPTION_FONT_SIZE,
        )
        .with_color(label_color)
        .finish();

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.)
            .with_child(checkbox)
            .with_child(label)
            .finish()
    }

    fn render_creation_form(&self, app: &AppContext) -> Box<dyn Element> {
        let Some(info) = self.current_type_info() else {
            return Empty::new().finish();
        };
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(FORM_FIELD_SPACING);

        column.add_child(
            Container::new(self.render_field_label("NAME", app))
                .with_padding_top(CONTENT_SECTION_SPACING)
                .finish(),
        );
        column.add_child(self.render_editor_container(&self.name_editor, app));

        for (idx, field) in info.fields.iter().enumerate() {
            let label = if field.optional {
                format!("{} (optional)", field.label)
            } else {
                field.label.to_string()
            };
            column.add_child(self.render_field_label(&label, app));
            if let Some(editor) = self.field_editors.get(idx) {
                column.add_child(self.render_editor_container(editor, app));
            }
        }

        column.add_child(self.render_scope_checkbox(app));

        column.finish()
    }

    fn render_button(
        &self,
        label: &'static str,
        mouse_state: MouseStateHandle,
        background: Option<Fill>,
        action: AuthSecretFtuxAction,
        disabled: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let font_family = appearance.ui_font_family();
        // Mirrors the design system's `DisabledTheme`.
        let text_color = if disabled {
            internal_colors::neutral_5(theme)
        } else {
            theme.foreground().into()
        };
        let effective_background = if disabled {
            background.map(|_| Fill::from(internal_colors::neutral_4(theme)))
        } else {
            background
        };
        Hoverable::new(mouse_state, move |_| {
            let inner = Container::new(
                Text::new_inline(label.to_string(), font_family, BUTTON_FONT_SIZE)
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .with_color(text_color)
                    .finish(),
            )
            .with_padding_left(BUTTON_PADDING * 2.)
            .with_padding_right(BUTTON_PADDING * 2.)
            .with_padding_top(BUTTON_PADDING)
            .with_padding_bottom(BUTTON_PADDING);
            let inner = if let Some(background) = effective_background {
                inner
                    .with_background(background)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)))
            } else {
                inner
            };
            inner.finish()
        })
        .with_cursor(if disabled {
            warpui::platform::Cursor::Arrow
        } else {
            warpui::platform::Cursor::PointingHand
        })
        .on_click(move |ctx, _, _| {
            if disabled {
                return;
            }
            ctx.dispatch_typed_action(action.clone());
        })
        .finish()
    }

    fn render_bottom_row(&self, app: &AppContext) -> Box<dyn Element> {
        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        row.add_child(Expanded::new(1., Empty::new().finish()).finish());

        let (label, action) = if self.creation_state.is_some() {
            ("Back", AuthSecretFtuxAction::Back)
        } else {
            ("Cancel", AuthSecretFtuxAction::Cancel)
        };
        row.add_child(self.render_button(
            label,
            self.back_button_mouse_state.clone(),
            None,
            action,
            false,
            app,
        ));

        let accent_fill = Appearance::as_ref(app).theme().accent();
        let continue_disabled = !self.can_submit_creation_form(app);
        row.add_child(self.render_button(
            "Continue",
            self.continue_mouse_state.clone(),
            Some(accent_fill),
            AuthSecretFtuxAction::Continue,
            continue_disabled,
            app,
        ));

        row.finish()
    }
}

fn make_single_line_editor(
    placeholder: Option<&str>,
    is_password: bool,
    ctx: &mut ViewContext<AuthSecretFtuxView>,
) -> ViewHandle<EditorView> {
    let placeholder = placeholder.map(str::to_owned);
    ctx.add_typed_action_view(move |ctx| {
        let appearance = Appearance::as_ref(ctx);
        let mut editor = EditorView::single_line(
            SingleLineEditorOptions {
                text: TextOptions::ui_text(Some(EDITOR_FONT_SIZE), appearance),
                select_all_on_focus: false,
                clear_selections_on_blur: true,
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                propagate_and_no_op_escape_key: PropagateAndNoOpEscapeKey::PropagateFirst,
                is_password,
                ..Default::default()
            },
            ctx,
        );
        if let Some(placeholder) = placeholder {
            editor.set_placeholder_text(&placeholder, ctx);
        }
        editor
    })
}

impl Entity for AuthSecretFtuxView {
    type Event = AuthSecretFtuxViewEvent;
}

impl TypedActionView for AuthSecretFtuxView {
    type Action = AuthSecretFtuxAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AuthSecretFtuxAction::Skip => self.handle_skip(ctx),
            AuthSecretFtuxAction::Cancel => self.handle_cancel(ctx),
            AuthSecretFtuxAction::Continue => self.handle_continue(ctx),
            AuthSecretFtuxAction::Back => self.clear_creation_state(ctx),
            AuthSecretFtuxAction::LearnMore(url) => {
                ctx.open_url(url);
            }
            AuthSecretFtuxAction::ToggleTeamScope => {
                self.share_with_team = !self.share_with_team;
                ctx.notify();
            }
            AuthSecretFtuxAction::ToggleHarnessMenu => self.handle_toggle_harness_menu(ctx),
            AuthSecretFtuxAction::SelectHarness(harness) => {
                self.is_harness_menu_open = false;
                self.set_harness(*harness, ctx);
            }
        }
    }
}

impl View for AuthSecretFtuxView {
    fn ui_name() -> &'static str {
        "AuthSecretFtuxView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);

        let mut content_section = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(CONTENT_SECTION_SPACING);
        if self.compact_mode {
            // Compact mode: render the harness picker at the top, skip the
            // descriptive header (the modal title supplies context), and
            // keep the existing-secrets dropdown trimmed to "+ New …"
            // entries so the user can still switch secret types.
            content_section.add_child(self.render_compact_harness_picker(app));
            content_section.add_child(ChildView::new(&self.ftux_dropdown).finish());
        } else {
            content_section.add_child(self.render_description(app));
            if self.creation_state.is_none() {
                content_section.add_child(ChildView::new(&self.ftux_dropdown).finish());
            }
        }
        column.add_child(content_section.finish());

        if self.creation_state.is_some() {
            column.add_child(self.render_creation_form(app));
        }

        column.add_child(
            Container::new(self.render_bottom_row(app))
                .with_padding_top(CONTENT_SECTION_SPACING)
                .finish(),
        );

        column.finish()
    }
}
