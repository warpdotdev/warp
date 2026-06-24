//! Editor view for creating and editing local custom model routers.
//!
//! Opened as a side-pane from the Warp Agent settings page when the user clicks
//! "Add router" or "Edit" on an existing router card. Writes changes to
//! `~/.warp/custom_model_routers/` via [`WarpConfig::save_custom_model_router`].

use warpui::elements::{
    Border, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CrossAxisAlignment, Expanded, Flex, Hoverable, MainAxisSize, MouseStateHandle, ParentElement,
    ScrollbarWidth, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::custom_model_routers::{
    is_auto_target, ComplexityRouting, CustomModelRouter, CustomModelRouting, PromptRouting,
    PromptRule,
};
use crate::ai::llms::{LLMPreferences, LLMPreferencesEvent};
use crate::appearance::Appearance;
use crate::editor::{EditorView, SingleLineEditorOptions, TextOptions};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::ui_components::icons::Icon;
use crate::user_config::WarpConfig;
use crate::view_components::action_button::{
    ActionButton, ButtonSize, DangerSecondaryTheme, PrimaryTheme, SecondaryTheme,
};
use crate::view_components::{Dropdown, DropdownItem};

pub const HEADER_TEXT: &str = "Router Editor";

const EDITOR_CONTENT_WIDTH: f32 = 340.;

/// Sentinel used as the dropdown selection for "inherit default" in optional
/// complexity-bucket dropdowns.
const INHERIT_DEFAULT: &str = "__inherit_default__";

#[derive(Debug, Clone)]
pub enum CustomRouterEditorEvent {
    Pane(PaneEvent),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CustomRouterEditorAction {
    Close,
    Save,
    Delete,
    SetRouterType(RouterEditorType),
    SetComplexityDefault(String),
    SetComplexityEasy(String),
    SetComplexityMedium(String),
    SetComplexityHard(String),
    SetPromptDefault(String),
    SetPromptRuleModel { index: usize, model_id: String },
    AddPromptRule,
    RemovePromptRule(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterEditorType {
    Complexity,
    Prompt,
}

struct PromptRuleRow {
    description_editor: ViewHandle<EditorView>,
    model_dropdown: ViewHandle<Dropdown<CustomRouterEditorAction>>,
    remove_mouse_state: MouseStateHandle,
    current_model: String,
}

pub struct CustomRouterEditorView {
    existing: Option<CustomModelRouter>,
    pane_configuration: warpui::ModelHandle<PaneConfiguration>,
    focus_handle: Option<PaneFocusHandle>,
    scroll_state: ClippedScrollStateHandle,

    name_editor: ViewHandle<EditorView>,
    router_type: RouterEditorType,
    type_dropdown: ViewHandle<Dropdown<CustomRouterEditorAction>>,

    complexity_default_dropdown: ViewHandle<Dropdown<CustomRouterEditorAction>>,
    complexity_easy_dropdown: ViewHandle<Dropdown<CustomRouterEditorAction>>,
    complexity_medium_dropdown: ViewHandle<Dropdown<CustomRouterEditorAction>>,
    complexity_hard_dropdown: ViewHandle<Dropdown<CustomRouterEditorAction>>,

    complexity_default: String,
    complexity_easy: Option<String>,
    complexity_medium: Option<String>,
    complexity_hard: Option<String>,

    prompt_default_dropdown: ViewHandle<Dropdown<CustomRouterEditorAction>>,
    prompt_default_model: String,
    prompt_rules: Vec<PromptRuleRow>,

    save_button: ViewHandle<ActionButton>,
    cancel_button: ViewHandle<ActionButton>,
    delete_button: ViewHandle<ActionButton>,

    save_error: Option<String>,
}

impl CustomRouterEditorView {
    /// Create the editor.
    ///
    /// `existing = None` → creating a new router.
    /// `existing = Some(router)` → editing an existing router.
    pub fn new(existing: Option<CustomModelRouter>, ctx: &mut ViewContext<Self>) -> Self {
        let title = existing
            .as_ref()
            .map(|r| r.info.display_name.clone())
            .unwrap_or_else(|| "New Router".to_string());
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new(&title));

        let router_type = match existing.as_ref().map(|r| &r.routing) {
            Some(CustomModelRouting::Prompt(_)) => RouterEditorType::Prompt,
            _ => RouterEditorType::Complexity,
        };

        let (init_cdefault, init_ceasy, init_cmedium, init_chard) =
            match existing.as_ref().map(|r| &r.routing) {
                Some(CustomModelRouting::Complexity(c)) => (
                    c.default.clone(),
                    c.easy.clone(),
                    c.medium.clone(),
                    c.hard.clone(),
                ),
                _ => (String::new(), None, None, None),
            };

        let (init_pdefault, init_prules) = match existing.as_ref().map(|r| &r.routing) {
            Some(CustomModelRouting::Prompt(p)) => (p.default_model.clone(), p.rules.clone()),
            _ => (String::new(), Vec::new()),
        };

        // Name editor
        let initial_name = existing
            .as_ref()
            .map(|r| r.info.display_name.clone())
            .unwrap_or_default();
        let is_editing = existing.is_some();
        let name_editor = ctx.add_view(move |ctx| {
            let font_size = Appearance::as_ref(ctx).ui_font_size();
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions {
                        font_size_override: Some(font_size),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text("e.g. \"my-router\"", ctx);
            if !initial_name.is_empty() {
                editor.set_buffer_text(&initial_name, ctx);
            }
            if is_editing {
                // Make read-only: Selectable = can navigate/copy but not edit
                editor.set_interaction_state(crate::editor::InteractionState::Selectable, ctx);
            }
            editor
        });
        let font_family = Appearance::as_ref(ctx).ui_font_family();
        let font_size = Appearance::as_ref(ctx).ui_font_size();
        name_editor.update(ctx, |editor, ctx| {
            editor.set_font_size(font_size, ctx);
            editor.set_font_family(font_family, ctx);
        });

        // Type dropdown
        let init_type = router_type;
        let type_dropdown = ctx.add_typed_action_view(move |ctx| {
            let mut d = Dropdown::new(ctx);
            d.set_items(
                vec![
                    DropdownItem::new(
                        "Complexity",
                        CustomRouterEditorAction::SetRouterType(RouterEditorType::Complexity),
                    ),
                    DropdownItem::new(
                        "Prompt",
                        CustomRouterEditorAction::SetRouterType(RouterEditorType::Prompt),
                    ),
                ],
                ctx,
            );
            match init_type {
                RouterEditorType::Complexity => d.set_selected_by_name("Complexity", ctx),
                RouterEditorType::Prompt => d.set_selected_by_name("Prompt", ctx),
            }
            d
        });

        // Model choices (concrete, non-auto)
        let choices = concrete_model_choices(ctx);

        let complexity_default_dropdown = make_model_dropdown(
            &choices,
            false,
            &init_cdefault,
            CustomRouterEditorAction::SetComplexityDefault,
            ctx,
        );
        let complexity_easy_dropdown = make_model_dropdown(
            &choices,
            true,
            init_ceasy.as_deref().unwrap_or(INHERIT_DEFAULT),
            CustomRouterEditorAction::SetComplexityEasy,
            ctx,
        );
        let complexity_medium_dropdown = make_model_dropdown(
            &choices,
            true,
            init_cmedium.as_deref().unwrap_or(INHERIT_DEFAULT),
            CustomRouterEditorAction::SetComplexityMedium,
            ctx,
        );
        let complexity_hard_dropdown = make_model_dropdown(
            &choices,
            true,
            init_chard.as_deref().unwrap_or(INHERIT_DEFAULT),
            CustomRouterEditorAction::SetComplexityHard,
            ctx,
        );
        let prompt_default_dropdown = make_model_dropdown(
            &choices,
            false,
            &init_pdefault,
            CustomRouterEditorAction::SetPromptDefault,
            ctx,
        );

        let prompt_rules = init_prules
            .iter()
            .enumerate()
            .map(|(i, rule)| make_prompt_rule_row(i, &rule.description, &rule.model, &choices, ctx))
            .collect();

        let save_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Save", PrimaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(CustomRouterEditorAction::Save))
        });
        let cancel_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Cancel", SecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(CustomRouterEditorAction::Close))
        });
        let delete_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Delete router", DangerSecondaryTheme)
                .with_icon(Icon::Trash)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(CustomRouterEditorAction::Delete))
        });

        let view = Self {
            existing,
            pane_configuration,
            focus_handle: None,
            scroll_state: Default::default(),
            name_editor,
            router_type,
            type_dropdown,
            complexity_default_dropdown,
            complexity_easy_dropdown,
            complexity_medium_dropdown,
            complexity_hard_dropdown,
            complexity_default: init_cdefault,
            complexity_easy: init_ceasy,
            complexity_medium: init_cmedium,
            complexity_hard: init_chard,
            prompt_default_dropdown,
            prompt_default_model: init_pdefault,
            prompt_rules,
            save_button,
            cancel_button,
            delete_button,
            save_error: None,
        };

        ctx.subscribe_to_model(&LLMPreferences::handle(ctx), |me, _, event, ctx| {
            if matches!(event, LLMPreferencesEvent::UpdatedAvailableLLMs) {
                me.refresh_all_model_dropdowns(ctx);
            }
        });

        view
    }

    pub fn pane_configuration(&self) -> warpui::ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn focus(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.name_editor);
    }

    // ------------------------------------------------------------------

    fn refresh_all_model_dropdowns(&mut self, ctx: &mut ViewContext<Self>) {
        let choices = concrete_model_choices(ctx);
        repopulate(
            &self.complexity_default_dropdown,
            &choices,
            false,
            &self.complexity_default,
            CustomRouterEditorAction::SetComplexityDefault,
            ctx,
        );
        let easy = self
            .complexity_easy
            .as_deref()
            .unwrap_or(INHERIT_DEFAULT)
            .to_string();
        repopulate(
            &self.complexity_easy_dropdown,
            &choices,
            true,
            &easy,
            CustomRouterEditorAction::SetComplexityEasy,
            ctx,
        );
        let med = self
            .complexity_medium
            .as_deref()
            .unwrap_or(INHERIT_DEFAULT)
            .to_string();
        repopulate(
            &self.complexity_medium_dropdown,
            &choices,
            true,
            &med,
            CustomRouterEditorAction::SetComplexityMedium,
            ctx,
        );
        let hard = self
            .complexity_hard
            .as_deref()
            .unwrap_or(INHERIT_DEFAULT)
            .to_string();
        repopulate(
            &self.complexity_hard_dropdown,
            &choices,
            true,
            &hard,
            CustomRouterEditorAction::SetComplexityHard,
            ctx,
        );
        repopulate(
            &self.prompt_default_dropdown,
            &choices,
            false,
            &self.prompt_default_model,
            CustomRouterEditorAction::SetPromptDefault,
            ctx,
        );
        for (i, row) in self.prompt_rules.iter_mut().enumerate() {
            let sel = row.current_model.clone();
            repopulate(
                &row.model_dropdown,
                &choices,
                false,
                &sel,
                move |id| CustomRouterEditorAction::SetPromptRuleModel {
                    index: i,
                    model_id: id,
                },
                ctx,
            );
        }
        ctx.notify();
    }

    fn router_name(&self, ctx: &AppContext) -> String {
        self.name_editor
            .as_ref(ctx)
            .buffer_text(ctx)
            .trim()
            .to_string()
    }

    fn try_save(&mut self, ctx: &mut ViewContext<Self>) {
        let name = self.router_name(ctx);
        if name.is_empty() {
            self.save_error = Some("Router name is required.".to_string());
            ctx.notify();
            return;
        }

        let routing = match self.router_type {
            RouterEditorType::Complexity => {
                if self.complexity_default.is_empty() {
                    self.save_error = Some("A default model is required.".to_string());
                    ctx.notify();
                    return;
                }
                CustomModelRouting::Complexity(ComplexityRouting {
                    default: self.complexity_default.clone(),
                    easy: self.complexity_easy.clone(),
                    medium: self.complexity_medium.clone(),
                    hard: self.complexity_hard.clone(),
                })
            }
            RouterEditorType::Prompt => {
                if self.prompt_default_model.is_empty() {
                    self.save_error = Some("A default model is required.".to_string());
                    ctx.notify();
                    return;
                }
                let rules: Vec<PromptRule> = self
                    .prompt_rules
                    .iter()
                    .filter_map(|row| {
                        let desc = row
                            .description_editor
                            .as_ref(ctx)
                            .buffer_text(ctx)
                            .trim()
                            .to_string();
                        if desc.is_empty() || row.current_model.is_empty() {
                            return None;
                        }
                        Some(PromptRule {
                            description: desc,
                            model: row.current_model.clone(),
                        })
                    })
                    .collect();
                CustomModelRouting::Prompt(PromptRouting {
                    default_model: self.prompt_default_model.clone(),
                    rules,
                })
            }
        };

        let existing_path = self
            .existing
            .as_ref()
            .and_then(|r| r.source_path.as_deref());
        let router = CustomModelRouter::new_local(name.clone(), routing, existing_path);
        if let Err(e) = router.validate() {
            self.save_error = Some(format!("Validation: {e}"));
            ctx.notify();
            return;
        }

        let yaml = match router.to_yaml_string() {
            Ok(y) => y,
            Err(e) => {
                self.save_error = Some(format!("Serialization: {e}"));
                ctx.notify();
                return;
            }
        };

        #[cfg(feature = "local_fs")]
        {
            let ep = self.existing.as_ref().and_then(|r| r.source_path.clone());
            if let Err(e) = WarpConfig::save_custom_model_router(&name, &yaml, ep.as_deref()) {
                self.save_error = Some(format!("Write error: {e}"));
                ctx.notify();
                return;
            }
        }

        self.save_error = None;
        ctx.emit(CustomRouterEditorEvent::Pane(PaneEvent::Close));
    }

    fn try_delete(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(path) = self.existing.as_ref().and_then(|r| r.source_path.as_ref()) {
            #[cfg(feature = "local_fs")]
            if let Err(e) = WarpConfig::delete_custom_model_router(path) {
                self.save_error = Some(format!("Delete error: {e}"));
                ctx.notify();
                return;
            }
        }
        ctx.emit(CustomRouterEditorEvent::Pane(PaneEvent::Close));
    }

    fn add_prompt_rule(&mut self, ctx: &mut ViewContext<Self>) {
        let choices = concrete_model_choices(ctx);
        let index = self.prompt_rules.len();
        let row = make_prompt_rule_row(index, "", "", &choices, ctx);
        self.prompt_rules.push(row);
        ctx.notify();
    }

    fn remove_prompt_rule(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if index < self.prompt_rules.len() {
            self.prompt_rules.remove(index);
        }
        ctx.notify();
    }

    // ------------------------------------------------------------------
    // Rendering
    // ------------------------------------------------------------------

    fn section_label(label: impl Into<String>, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            Text::new(label.into(), appearance.ui_font_family(), 11.)
                .with_style(Properties::default().weight(Weight::Medium))
                .with_color(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().surface_1())
                        .into(),
                )
                .finish(),
        )
        .with_margin_bottom(4.)
        .finish()
    }

    fn render_complexity_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        Flex::column()
            .with_child(Self::section_label("MODELS", appearance))
            .with_child(labeled_dropdown(
                "Default (required)",
                &self.complexity_default_dropdown,
                appearance,
            ))
            .with_child(
                Container::new(labeled_dropdown(
                    "Easy (optional)",
                    &self.complexity_easy_dropdown,
                    appearance,
                ))
                .with_margin_top(8.)
                .finish(),
            )
            .with_child(
                Container::new(labeled_dropdown(
                    "Medium (optional)",
                    &self.complexity_medium_dropdown,
                    appearance,
                ))
                .with_margin_top(8.)
                .finish(),
            )
            .with_child(
                Container::new(labeled_dropdown(
                    "Hard (optional)",
                    &self.complexity_hard_dropdown,
                    appearance,
                ))
                .with_margin_top(8.)
                .finish(),
            )
            .finish()
    }

    fn render_prompt_section(
        &self,
        appearance: &Appearance,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        let _sub = appearance
            .theme()
            .sub_text_color(appearance.theme().surface_1());

        let mut column = Flex::column()
            .with_child(Self::section_label("DEFAULT MODEL", appearance))
            .with_child(
                ConstrainedBox::new(ChildView::new(&self.prompt_default_dropdown).finish())
                    .with_width(EDITOR_CONTENT_WIDTH)
                    .finish(),
            );

        if !self.prompt_rules.is_empty() {
            column.add_child(
                Container::new(Self::section_label("RULES".to_string(), appearance))
                    .with_margin_top(12.)
                    .finish(),
            );
            for (i, row) in self.prompt_rules.iter().enumerate() {
                column.add_child(
                    Container::new(render_rule_row(i, row, appearance))
                        .with_margin_bottom(8.)
                        .finish(),
                );
            }
        }

        // "+ Add rule" inline text button
        let accent = warp_core::ui::theme::color::internal_colors::accent_fg(appearance.theme());
        let add_rule = Hoverable::new(MouseStateHandle::default(), move |_| {
            Text::new("+ Add rule", appearance.ui_font_family(), 12.)
                .with_color(accent.into())
                .finish()
        })
        .on_click(|ctx, _app, _pos| {
            ctx.dispatch_typed_action(CustomRouterEditorAction::AddPromptRule);
        })
        .finish();

        column.add_child(Container::new(add_rule).with_margin_top(4.).finish());
        column.finish()
    }

    fn render_content(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let mut col = Flex::column();

        // Name
        let name_label = if self.existing.is_some() {
            "NAME (read-only)"
        } else {
            "NAME"
        };
        col.add_child(
            Container::new(
                Flex::column()
                    .with_child(Self::section_label(name_label, appearance))
                    .with_child(editor_row(&self.name_editor, appearance))
                    .finish(),
            )
            .with_margin_bottom(16.)
            .finish(),
        );

        // Type
        col.add_child(
            Container::new(
                Flex::column()
                    .with_child(Self::section_label("ROUTING TYPE", appearance))
                    .with_child(
                        ConstrainedBox::new(ChildView::new(&self.type_dropdown).finish())
                            .with_width(EDITOR_CONTENT_WIDTH)
                            .finish(),
                    )
                    .finish(),
            )
            .with_margin_bottom(16.)
            .finish(),
        );

        // Routing section
        match self.router_type {
            RouterEditorType::Complexity => {
                col.add_child(
                    Container::new(self.render_complexity_section(appearance))
                        .with_margin_bottom(16.)
                        .finish(),
                );
            }
            RouterEditorType::Prompt => {
                col.add_child(
                    Container::new(self.render_prompt_section(appearance, app))
                        .with_margin_bottom(16.)
                        .finish(),
                );
            }
        }

        // Buttons
        let mut btn_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(ChildView::new(&self.save_button).finish())
                    .with_margin_right(8.)
                    .finish(),
            )
            .with_child(ChildView::new(&self.cancel_button).finish());
        if self.existing.is_some() {
            btn_row.add_child(
                Container::new(ChildView::new(&self.delete_button).finish())
                    .with_margin_left(16.)
                    .finish(),
            );
        }
        col.add_child(btn_row.finish());

        // Error
        if let Some(msg) = &self.save_error {
            let err_color = warp_core::ui::theme::Fill::Solid(appearance.theme().ui_error_color());
            col.add_child(
                Container::new(
                    Text::new(msg.clone(), appearance.ui_font_family(), 12.)
                        .with_color(err_color.into())
                        .finish(),
                )
                .with_margin_top(8.)
                .finish(),
            );
        }

        col.finish()
    }
}

// ------------------------------------------------------------------
// Entity / View / TypedActionView / BackingView
// ------------------------------------------------------------------

impl Entity for CustomRouterEditorView {
    type Event = CustomRouterEditorEvent;
}

impl View for CustomRouterEditorView {
    fn ui_name() -> &'static str {
        "CustomRouterEditorView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let content = Container::new(self.render_content(appearance, app))
            .with_padding_top(24.)
            .with_padding_bottom(24.)
            .with_padding_left(24.)
            .with_padding_right(24.)
            .finish();
        ClippedScrollable::vertical(
            self.scroll_state.clone(),
            content,
            ScrollbarWidth::Auto,
            appearance.theme().nonactive_ui_detail().into(),
            appearance.theme().active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .finish()
    }
}

impl TypedActionView for CustomRouterEditorView {
    type Action = CustomRouterEditorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CustomRouterEditorAction::Close => {
                ctx.emit(CustomRouterEditorEvent::Pane(PaneEvent::Close));
            }
            CustomRouterEditorAction::Save => self.try_save(ctx),
            CustomRouterEditorAction::Delete => self.try_delete(ctx),
            CustomRouterEditorAction::SetRouterType(t) => {
                self.router_type = *t;
                ctx.notify();
            }
            CustomRouterEditorAction::SetComplexityDefault(id) => {
                self.complexity_default = id.clone();
            }
            CustomRouterEditorAction::SetComplexityEasy(id) => {
                self.complexity_easy = if id == INHERIT_DEFAULT {
                    None
                } else {
                    Some(id.clone())
                };
            }
            CustomRouterEditorAction::SetComplexityMedium(id) => {
                self.complexity_medium = if id == INHERIT_DEFAULT {
                    None
                } else {
                    Some(id.clone())
                };
            }
            CustomRouterEditorAction::SetComplexityHard(id) => {
                self.complexity_hard = if id == INHERIT_DEFAULT {
                    None
                } else {
                    Some(id.clone())
                };
            }
            CustomRouterEditorAction::SetPromptDefault(id) => {
                self.prompt_default_model = id.clone();
            }
            CustomRouterEditorAction::SetPromptRuleModel { index, model_id } => {
                if let Some(row) = self.prompt_rules.get_mut(*index) {
                    row.current_model = model_id.clone();
                }
            }
            CustomRouterEditorAction::AddPromptRule => self.add_prompt_rule(ctx),
            CustomRouterEditorAction::RemovePromptRule(i) => self.remove_prompt_rule(*i, ctx),
        }
    }
}

impl BackingView for CustomRouterEditorView {
    type PaneHeaderOverflowMenuAction = CustomRouterEditorAction;
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.handle_action(action, ctx);
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(CustomRouterEditorEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus(ctx);
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        view::HeaderContent::Standard(view::StandardHeader {
            title: HEADER_TEXT.into(),
            title_secondary: None,
            title_style: None,
            title_clip_config: warpui::text_layout::ClipConfig::start(),
            title_max_width: None,
            left_of_title: None,
            right_of_title: None,
            left_of_overflow: None,
            options: view::StandardHeaderOptions {
                always_show_icons: true,
                ..Default::default()
            },
        })
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

// ------------------------------------------------------------------
// Module-level helper functions
// ------------------------------------------------------------------

fn concrete_model_choices(ctx: &mut ViewContext<CustomRouterEditorView>) -> Vec<String> {
    LLMPreferences::as_ref(ctx)
        .get_base_llm_choices_for_agent_mode(ctx)
        .filter(|llm| !is_auto_target(llm.id.as_str()))
        .map(|llm| llm.id.to_string())
        .collect()
}

fn make_model_dropdown<F>(
    choices: &[String],
    optional: bool,
    selected: &str,
    make_action: F,
    ctx: &mut ViewContext<CustomRouterEditorView>,
) -> ViewHandle<Dropdown<CustomRouterEditorAction>>
where
    F: Fn(String) -> CustomRouterEditorAction + 'static + Clone,
{
    let choices_owned = choices.to_vec();
    let selected_owned = selected.to_string();
    ctx.add_typed_action_view(move |ctx| {
        let mut d = Dropdown::new(ctx);
        fill_dropdown_items(
            &mut d,
            &choices_owned,
            optional,
            &selected_owned,
            make_action.clone(),
            ctx,
        );
        d
    })
}

fn repopulate<F>(
    dropdown: &ViewHandle<Dropdown<CustomRouterEditorAction>>,
    choices: &[String],
    optional: bool,
    selected: &str,
    make_action: F,
    ctx: &mut ViewContext<CustomRouterEditorView>,
) where
    F: Fn(String) -> CustomRouterEditorAction + Clone,
{
    let choices_owned = choices.to_vec();
    let selected_owned = selected.to_string();
    dropdown.update(ctx, move |d, ctx| {
        fill_dropdown_items(
            d,
            &choices_owned,
            optional,
            &selected_owned,
            make_action.clone(),
            ctx,
        );
    });
}

fn fill_dropdown_items<F>(
    dropdown: &mut Dropdown<CustomRouterEditorAction>,
    choices: &[String],
    optional: bool,
    selected: &str,
    make_action: F,
    ctx: &mut warpui::ViewContext<Dropdown<CustomRouterEditorAction>>,
) where
    F: Fn(String) -> CustomRouterEditorAction,
{
    let mut items: Vec<DropdownItem<CustomRouterEditorAction>> = Vec::new();
    if optional {
        items.push(DropdownItem::new(
            "Inherit default",
            make_action(INHERIT_DEFAULT.to_string()),
        ));
    }
    for id in choices {
        items.push(DropdownItem::new(id.clone(), make_action(id.clone())));
    }
    dropdown.set_items(items, ctx);
    if selected.is_empty() || (optional && selected == INHERIT_DEFAULT) {
        dropdown.set_selected_by_name("Inherit default", ctx);
    } else {
        dropdown.set_selected_by_name(selected, ctx);
    }
}

fn make_prompt_rule_row(
    index: usize,
    description: &str,
    model: &str,
    choices: &[String],
    ctx: &mut ViewContext<CustomRouterEditorView>,
) -> PromptRuleRow {
    let desc_owned = description.to_string();
    let description_editor = ctx.add_view(move |ctx| {
        let font_size = Appearance::as_ref(ctx).ui_font_size();
        let mut editor = EditorView::single_line(
            SingleLineEditorOptions {
                text: TextOptions {
                    font_size_override: Some(font_size),
                    ..Default::default()
                },
                ..Default::default()
            },
            ctx,
        );
        editor.set_placeholder_text("Describe when to use this model…", ctx);
        if !desc_owned.is_empty() {
            editor.set_buffer_text(&desc_owned, ctx);
        }
        editor
    });

    let model_dropdown = make_model_dropdown(
        choices,
        false,
        model,
        move |id| CustomRouterEditorAction::SetPromptRuleModel {
            index,
            model_id: id,
        },
        ctx,
    );

    PromptRuleRow {
        description_editor,
        model_dropdown,
        remove_mouse_state: Default::default(),
        current_model: model.to_string(),
    }
}

fn editor_row(editor: &ViewHandle<EditorView>, appearance: &Appearance) -> Box<dyn Element> {
    Container::new(ChildView::new(editor).finish())
        .with_background(appearance.theme().surface_2())
        .with_border(Border::new(1.).with_border_fill(appearance.theme().outline()))
        .with_corner_radius(warpui::elements::CornerRadius::with_all(
            warpui::elements::Radius::Pixels(4.),
        ))
        .with_horizontal_padding(8.)
        .with_vertical_padding(6.)
        .finish()
}

fn labeled_dropdown(
    label: impl Into<String>,
    dropdown: &ViewHandle<Dropdown<CustomRouterEditorAction>>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let sub = appearance
        .theme()
        .sub_text_color(appearance.theme().surface_1());
    Flex::column()
        .with_child(
            Container::new(
                Text::new(label.into(), appearance.ui_font_family(), 11.)
                    .with_color(sub.into())
                    .finish(),
            )
            .with_margin_bottom(2.)
            .finish(),
        )
        .with_child(
            ConstrainedBox::new(ChildView::new(dropdown).finish())
                .with_width(EDITOR_CONTENT_WIDTH)
                .finish(),
        )
        .finish()
}

fn render_rule_row(index: usize, row: &PromptRuleRow, appearance: &Appearance) -> Box<dyn Element> {
    let sub = appearance
        .theme()
        .sub_text_color(appearance.theme().surface_2());
    let icon_color = sub;
    let idx = index;
    let remove_state = row.remove_mouse_state.clone();

    let remove_btn = Hoverable::new(remove_state, move |_| {
        ConstrainedBox::new(Icon::X.to_warpui_icon(icon_color).finish())
            .with_width(14.)
            .with_height(14.)
            .finish()
    })
    .on_click(move |ctx, _app, _pos| {
        ctx.dispatch_typed_action(CustomRouterEditorAction::RemovePromptRule(idx));
    })
    .finish();

    let desc_label_str = "Description".to_string();
    let model_label_str = "Model".to_string();

    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(
            Expanded::new(
                1.,
                Flex::column()
                    .with_child(
                        Container::new(
                            Flex::column()
                                .with_child(
                                    Container::new(
                                        Text::new(
                                            desc_label_str.clone(),
                                            appearance.ui_font_family(),
                                            11.,
                                        )
                                        .with_color(sub.into())
                                        .finish(),
                                    )
                                    .with_margin_bottom(2.)
                                    .finish(),
                                )
                                .with_child(editor_row(&row.description_editor, appearance))
                                .finish(),
                        )
                        .with_margin_bottom(4.)
                        .finish(),
                    )
                    .with_child(
                        Flex::column()
                            .with_child(
                                Container::new(
                                    Text::new(
                                        model_label_str.clone(),
                                        appearance.ui_font_family(),
                                        11.,
                                    )
                                    .with_color(sub.into())
                                    .finish(),
                                )
                                .with_margin_bottom(2.)
                                .finish(),
                            )
                            .with_child(
                                ConstrainedBox::new(ChildView::new(&row.model_dropdown).finish())
                                    .with_width(EDITOR_CONTENT_WIDTH)
                                    .finish(),
                            )
                            .finish(),
                    )
                    .finish(),
            )
            .finish(),
        )
        .with_child(
            Container::new(remove_btn)
                .with_margin_left(8.)
                .with_margin_top(20.)
                .finish(),
        )
        .finish()
}
