use std::{cmp::Ordering, collections::HashMap};

use anyhow::Error;
use pathfinder_geometry::vector::vec2f;
use warp_core::{
    features::FeatureFlag,
    ui::{
        appearance::Appearance,
        theme::{color::internal_colors::neutral_4, Fill},
    },
};
use warpui::{
    elements::{
        ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss, Flex, Hoverable,
        MainAxisAlignment, MainAxisSize, MouseState, MouseStateHandle, ParentElement, Radius,
    },
    ui_components::{
        button::{ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::{
    cloud_object::{model::persistence::CloudModel, CloudObject},
    editor::{
        EditOrigin, EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys,
        SingleLineEditorOptions, TextOptions, ValidInputType,
    },
    send_telemetry_from_ctx,
    server::{ids::SyncId, telemetry::TelemetrySpace},
    ui_components::{buttons::icon_button, icons::Icon},
    workflows::aliases::{WorkflowAlias, WorkflowAliases},
    TelemetryEvent,
};

/// Width of the alias name editor.
const ALIAS_EDITOR_WIDTH: f32 = 100.;

/// Minimum size of an alias pill.
const ALIAS_PILL_MIN_WIDTH: f32 = 48.;

/// Padding within all alias pills.
const ALIAS_PILL_VERTICAL_PADDING: f32 = 4.;
const ALIAS_PILL_HORIZONTAL_PADDING: f32 = 8.;
const ALIAS_PILL_VERTICAL_MARGIN: f32 = 2.;

/// Dimensions for button icons.
const ICON_BUTTON_SIZE: f32 = 16.;

pub struct AliasBar {
    selected_alias: Option<usize>,
    renaming_alias: Option<usize>,
    aliases: Vec<AliasState>,
    name_editor: ViewHandle<EditorView>,
    template_button_mouse_state: MouseStateHandle,
    add_button_mouse_state: MouseStateHandle,

    /// True if any aliases have been modified. Since aliases are saved in bulk, we don't need to
    /// track this per-alias.
    is_dirty: bool,

    workflow_id: SyncId,
    deleted_aliases: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum AliasBarAction {
    Add,
    Select(usize),
    Deselect,
    Remove(usize),
    Rename(usize),
    StopRenaming,
}

#[derive(Debug, Clone)]
pub enum AliasBarEvent {
    SelectedAliasChanged,
    AliasesUpdated,
}

impl Entity for AliasBar {
    type Event = AliasBarEvent;
}

impl AliasBar {
    pub fn new(workflow_id: SyncId, ctx: &mut ViewContext<Self>) -> Self {
        let name_editor = ctx.add_typed_action_view(|ctx| {
            let mut view = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions {
                        font_size_override: Some(14.),
                        font_family_override: Some(Appearance::as_ref(ctx).ui_font_family()),
                        ..Default::default()
                    },
                    valid_input_type: ValidInputType::NoSpaces,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            );
            view.set_placeholder_text(crate::i18n::tr_static(ctx, "alias name"), ctx);

            view
        });
        ctx.subscribe_to_view(&name_editor, |me, _, event, ctx| {
            me.handle_name_editor_event(event, ctx);
        });

        let aliases = WorkflowAliases::as_ref(ctx)
            .get_aliases_for_workflow(workflow_id)
            .into_iter()
            .map(AliasState::from)
            .collect();

        Self {
            selected_alias: None,
            renaming_alias: None,
            aliases,
            name_editor,
            template_button_mouse_state: Default::default(),
            add_button_mouse_state: Default::default(),
            is_dirty: false,
            workflow_id,
            deleted_aliases: Default::default(),
        }
    }

    /// The current workflow's space for telemetry events.
    fn workflow_space(&self, app: &AppContext) -> Option<TelemetrySpace> {
        let workflow = CloudModel::as_ref(app).get_workflow(&self.workflow_id)?;
        Some(workflow.space(app).into())
    }

    fn mark_dirty(&mut self, is_dirty: bool, ctx: &mut ViewContext<Self>) {
        self.is_dirty = is_dirty;
        ctx.emit(AliasBarEvent::AliasesUpdated);
    }

    pub fn set_workflow_id(&mut self, workflow_id: SyncId, ctx: &mut ViewContext<Self>) {
        self.aliases = WorkflowAliases::as_ref(ctx)
            .get_aliases_for_workflow(workflow_id)
            .into_iter()
            .map(AliasState::from)
            .collect();
        self.selected_alias = None;
        self.renaming_alias = None;
        self.workflow_id = workflow_id;
        self.mark_dirty(false, ctx);
        ctx.notify();
    }

    pub fn has_selected_alias(&self) -> bool {
        self.selected_alias.is_some()
    }

    /// Prepopulated argument values for the current alias.
    pub fn current_argument_values(&self) -> Option<&HashMap<String, String>> {
        self.selected_alias
            .and_then(|index| self.aliases.get(index))
            .map(|alias| &alias.argument_values)
    }
    pub fn get_all_argument_values(&self) -> Vec<String> {
        self.aliases
            .iter()
            .flat_map(|alias| alias.argument_values.values())
            .cloned()
            .collect()
    }

    /// Update an argument value for the current alias.
    pub fn set_current_argument_value(
        &mut self,
        name: &str,
        value: String,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(alias) = self
            .selected_alias
            .and_then(|index| self.aliases.get_mut(index))
        {
            if value.is_empty() {
                alias.argument_values.remove(name);
            } else {
                alias.argument_values.insert(name.to_string(), value);
            }

            self.mark_dirty(true, ctx);

            send_telemetry_from_ctx!(
                TelemetryEvent::WorkflowAliasArgumentEdited {
                    workflow_id: self.workflow_id.into_server().map(Into::into),
                    workflow_space: self.workflow_space(ctx)
                },
                ctx
            );
        }
    }

    /// Updates the associated environment variables for the current alias.
    pub fn set_current_env_vars(&mut self, sync_id: Option<SyncId>, ctx: &mut ViewContext<Self>) {
        if let Some(alias) = self
            .selected_alias
            .and_then(|index| self.aliases.get_mut(index))
        {
            if alias.env_vars != sync_id {
                alias.env_vars = sync_id;
                self.mark_dirty(true, ctx);

                let env_vars_space = sync_id
                    .and_then(|id| CloudModel::as_ref(ctx).get_env_var_collection(&id))
                    .map(|env_vars| env_vars.space(ctx))
                    .map(Into::into);

                send_telemetry_from_ctx!(
                    TelemetryEvent::WorkflowAliasEnvVarsAttached {
                        workflow_id: self.workflow_id.into_server().map(Into::into),
                        workflow_space: self.workflow_space(ctx),
                        env_vars_id: sync_id.and_then(|id| id.into_server()).map(Into::into),
                        env_vars_space,
                    },
                    ctx
                );
            }
        }
    }

    /// Environment variables associated with the current alias.
    pub fn current_env_vars(&self) -> Option<SyncId> {
        self.selected_alias
            .and_then(|index| self.aliases.get(index))
            .and_then(|alias| alias.env_vars)
    }

    /// Whether or not there are unsaved changes to any aliases.
    pub fn has_unsaved_changes(&self) -> bool {
        self.is_dirty
    }

    pub fn save(&mut self, ctx: &mut ViewContext<Self>) -> Result<(), Error> {
        self.mark_dirty(false, ctx);
        WorkflowAliases::handle(ctx).update(ctx, |aliases, ctx| {
            // Reset the deleted aliases.
            let deleted_aliases = std::mem::take(&mut self.deleted_aliases);
            aliases.remove_aliases(deleted_aliases, ctx)?;

            let aliases_to_add = self
                .aliases
                .iter()
                .map(|alias| WorkflowAlias {
                    alias: alias.alias_name.clone(),
                    workflow_id: self.workflow_id,
                    arguments: Some(alias.argument_values.clone()),
                    env_vars: alias.env_vars,
                })
                .collect::<Vec<_>>();

            aliases.set_aliases(aliases_to_add, ctx)
        })
    }

    /// Internal helper that sets the selected alias and notifies observers.
    fn set_selected_alias(&mut self, index: Option<usize>, ctx: &mut ViewContext<Self>) {
        self.selected_alias = index;
        ctx.notify();
        ctx.emit(AliasBarEvent::SelectedAliasChanged);
    }

    fn select_alias(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::WorkflowAliases.is_enabled() {
            return;
        }

        if index >= self.aliases.len() {
            return;
        }

        self.set_selected_alias(Some(index), ctx);
    }

    fn deselect_alias(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::WorkflowAliases.is_enabled() {
            return;
        }

        self.set_selected_alias(None, ctx);
    }

    fn add_alias(&mut self, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::WorkflowAliases.is_enabled() {
            return;
        }

        self.renaming_alias = Some(self.aliases.len());
        self.set_selected_alias(self.renaming_alias, ctx);
        self.name_editor
            .update(ctx, |editor, ctx| editor.system_clear_buffer(true, ctx));
        ctx.focus(&self.name_editor);
        self.aliases.push(AliasState::new(String::new()));
        self.is_dirty = true;
        ctx.emit(AliasBarEvent::AliasesUpdated);
        ctx.notify();

        send_telemetry_from_ctx!(
            TelemetryEvent::WorkflowAliasAdded {
                workflow_id: self.workflow_id.into_server().map(Into::into),
                workflow_space: self.workflow_space(ctx),
            },
            ctx
        );
    }

    fn remove_alias(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::WorkflowAliases.is_enabled() {
            return;
        }

        let removed = self.aliases.remove(index);
        self.deleted_aliases.push(removed.alias_name.clone());
        if let Some(selected_index) = &mut self.selected_alias {
            match (*selected_index).cmp(&index) {
                Ordering::Less => (),
                Ordering::Equal => {
                    self.selected_alias = None;
                }
                Ordering::Greater => {
                    *selected_index -= 1;
                }
            }
        }
        self.is_dirty = true;
        ctx.emit(AliasBarEvent::AliasesUpdated);
        ctx.notify();

        send_telemetry_from_ctx!(
            TelemetryEvent::WorkflowAliasRemoved {
                workflow_id: self.workflow_id.into_server().map(Into::into),
                workflow_space: self.workflow_space(ctx),
            },
            ctx
        );
    }

    fn rename_alias(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if !FeatureFlag::WorkflowAliases.is_enabled() {
            return;
        }

        if let Some(alias) = self.aliases.get(index) {
            self.deleted_aliases.push(alias.alias_name.clone());
            self.name_editor.update(ctx, |editor, ctx| {
                editor.system_reset_buffer_text(&alias.alias_name, ctx);
            });
            self.renaming_alias = Some(index);
            ctx.focus(&self.name_editor);
            ctx.notify();
        }
    }

    fn stop_renaming_alias(&mut self, ctx: &mut ViewContext<Self>) {
        self.renaming_alias = None;
        ctx.notify();
    }

    fn render_name_editor(&self, appearance: &Appearance) -> Box<dyn Element> {
        Dismiss::new(
            Container::new(
                appearance
                    .ui_builder()
                    .text_input(self.name_editor.clone())
                    .with_style(UiComponentStyles {
                        padding: Some(Coords {
                            top: ALIAS_PILL_VERTICAL_PADDING,
                            bottom: ALIAS_PILL_VERTICAL_PADDING,
                            left: ALIAS_PILL_HORIZONTAL_PADDING,
                            right: ALIAS_PILL_HORIZONTAL_PADDING,
                        }),
                        ..Default::default()
                    })
                    .build()
                    .with_width(ALIAS_EDITOR_WIDTH)
                    .finish(),
            )
            .with_margin_right(8.)
            .finish(),
        )
        .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(AliasBarAction::StopRenaming))
        .finish()
    }

    fn handle_name_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter
            | EditorEvent::ShiftEnter
            | EditorEvent::Escape
            | EditorEvent::Blurred => {
                self.stop_renaming_alias(ctx);
            }
            EditorEvent::Edited(EditOrigin::UserTyped | EditOrigin::UserInitiated) => {
                if let Some(alias) = self
                    .renaming_alias
                    .and_then(|index| self.aliases.get_mut(index))
                {
                    alias.alias_name = self.name_editor.as_ref(ctx).buffer_text(ctx);
                    self.is_dirty = true;
                    ctx.emit(AliasBarEvent::AliasesUpdated);
                }
            }
            _ => (),
        }
    }
}

impl View for AliasBar {
    fn ui_name() -> &'static str {
        "AliasBar"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        if !self.aliases.is_empty() {
            // Only show the button to switch back to the workflow template if there are also
            // aliases.
            let template_button = Container::new(
                build_alias_pill(
                    self.template_button_mouse_state.clone(),
                    self.selected_alias.is_none(),
                    appearance,
                    |_state, background| {
                        appearance
                            .ui_builder()
                            .span(crate::i18n::tr_static(app, "Default"))
                            .with_style(UiComponentStyles {
                                font_color: Some(
                                    appearance.theme().main_text_color(background).into_solid(),
                                ),
                                font_size: Some(14.),
                                ..Default::default()
                            })
                            .build()
                            .finish()
                    },
                )
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(AliasBarAction::Deselect))
                .finish(),
            )
            .with_margin_right(8.)
            .finish();
            row.add_child(template_button);
        }

        // TODO: this should be horizontally scrollable or use a Wrap

        row.add_children(self.aliases.iter().enumerate().map(|(idx, alias)| {
            if Some(idx) == self.renaming_alias {
                self.render_name_editor(appearance)
            } else {
                alias.render(idx, self, appearance)
            }
        }));

        let add_button = appearance
            .ui_builder()
            .button(ButtonVariant::Text, self.add_button_mouse_state.clone())
            .with_text_and_icon_label(
                TextAndIcon::new(
                    TextAndIconAlignment::IconFirst,
                    "Add alias",
                    Icon::Plus.to_warpui_icon(
                        appearance
                            .theme()
                            .main_text_color(appearance.theme().background()),
                    ),
                    MainAxisSize::Min,
                    MainAxisAlignment::Start,
                    vec2f(ICON_BUTTON_SIZE, ICON_BUTTON_SIZE),
                )
                .with_inner_padding(4.),
            )
            .with_style(UiComponentStyles {
                padding: Some(Coords {
                    top: ALIAS_PILL_VERTICAL_PADDING,
                    bottom: ALIAS_PILL_VERTICAL_PADDING,
                    left: ALIAS_PILL_HORIZONTAL_PADDING,
                    right: ALIAS_PILL_HORIZONTAL_PADDING,
                }),
                ..Default::default()
            })
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(AliasBarAction::Add))
            .finish();
        row.add_child(add_button);

        Container::new(row.finish())
            .with_vertical_padding(8.)
            .finish()
    }
}

impl TypedActionView for AliasBar {
    type Action = AliasBarAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AliasBarAction::Add => self.add_alias(ctx),
            AliasBarAction::Select(index) => self.select_alias(*index, ctx),
            AliasBarAction::Deselect => self.deselect_alias(ctx),
            AliasBarAction::Remove(index) => self.remove_alias(*index, ctx),
            AliasBarAction::Rename(index) => self.rename_alias(*index, ctx),
            AliasBarAction::StopRenaming => {
                self.stop_renaming_alias(ctx);
            }
        }
    }
}

#[derive(Debug, Clone)]
struct AliasState {
    alias_name: String,
    argument_values: HashMap<String, String>,
    env_vars: Option<SyncId>,
    pill_mouse_state_handle: MouseStateHandle,
    delete_mouse_state_handle: MouseStateHandle,
}

impl From<&WorkflowAlias> for AliasState {
    fn from(alias: &WorkflowAlias) -> Self {
        Self {
            alias_name: alias.alias.clone(),
            argument_values: alias.arguments.clone().unwrap_or_default(),
            pill_mouse_state_handle: Default::default(),
            delete_mouse_state_handle: Default::default(),
            env_vars: alias.env_vars,
        }
    }
}

impl AliasState {
    fn new(name: String) -> Self {
        Self {
            alias_name: name,
            argument_values: Default::default(),
            env_vars: None,
            pill_mouse_state_handle: Default::default(),
            delete_mouse_state_handle: Default::default(),
        }
    }

    fn render(
        &self,
        index: usize,
        alias_bar: &AliasBar,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let name = self.alias_name.clone();
        let delete_handle = self.delete_mouse_state_handle.clone();
        let pill = build_alias_pill(
            self.pill_mouse_state_handle.clone(),
            alias_bar.selected_alias == Some(index),
            appearance,
            |_state, background| {
                let name = appearance
                    .ui_builder()
                    .span(name)
                    .with_style(UiComponentStyles {
                        font_color: Some(
                            appearance.theme().main_text_color(background).into_solid(),
                        ),
                        font_size: Some(14.),
                        // Remove the default span padding, since the entire pill is padded.
                        padding: Some(Coords::uniform(0.)),
                        ..Default::default()
                    })
                    .build()
                    .with_margin_right(4.)
                    .finish();

                let padded_name = ConstrainedBox::new(name)
                    .with_min_width(ALIAS_PILL_MIN_WIDTH - ICON_BUTTON_SIZE)
                    .finish();

                let delete_button = icon_button(appearance, Icon::X, false, delete_handle)
                    .with_style(UiComponentStyles {
                        padding: Some(Coords::uniform(0.)),
                        width: Some(ICON_BUTTON_SIZE),
                        height: Some(ICON_BUTTON_SIZE),
                        ..Default::default()
                    })
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(AliasBarAction::Remove(index))
                    })
                    .finish();
                Flex::row()
                    .with_children([padded_name, delete_button])
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish()
            },
        )
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(AliasBarAction::Select(index)))
        .on_double_click(move |ctx, _, _| ctx.dispatch_typed_action(AliasBarAction::Rename(index)))
        .finish();

        Container::new(pill).with_margin_right(8.).finish()
    }
}

fn build_alias_pill<F: FnOnce(&MouseState, Fill) -> Box<dyn Element>>(
    hover_state: MouseStateHandle,
    is_active: bool,
    appearance: &Appearance,
    build_content: F,
) -> Hoverable {
    Hoverable::new(hover_state, move |state| {
        let background = if state.is_hovered() {
            Fill::Solid(neutral_4(appearance.theme()))
        } else if is_active {
            appearance.theme().surface_2()
        } else {
            appearance.theme().background()
        };

        ConstrainedBox::new(
            Container::new(build_content(state, background))
                .with_vertical_padding(ALIAS_PILL_VERTICAL_PADDING)
                .with_horizontal_padding(ALIAS_PILL_HORIZONTAL_PADDING)
                .with_vertical_margin(ALIAS_PILL_VERTICAL_MARGIN)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_background(background)
                .finish(),
        )
        .with_min_width(ALIAS_PILL_MIN_WIDTH)
        .finish()
    })
}
