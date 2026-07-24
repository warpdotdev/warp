//! Editor view for creating and editing local custom model routers.
//!
//! Opened as a side-pane from the Warp Agent settings page when the user clicks
//! "Add router" or "Edit" on an existing router card. Writes changes to
//! `~/.warp/custom_model_routers/` via [`WarpConfig::save_custom_model_router`].

use itertools::Itertools;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use warpui::elements::{
    ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, Empty, Expanded, Fill as UiFill, Flex, FormattedTextElement,
    HighlightedHyperlink, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    ParentElement, Radius, ScrollbarWidth, Text,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::segmented_control::{
    LabelConfig, RenderableOptionConfig, SegmentedControl, SegmentedControlEvent,
};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::custom_model_routers::{
    ComplexityRouting, CustomModelRouter, CustomModelRouting, PromptRouting, PromptRule,
    is_auto_target,
};
use crate::ai::execution_profiles::model_menu_items::available_model_menu_items;
use crate::ai::llms::{LLMPreferences, LLMPreferencesEvent};
use crate::appearance::Appearance;
use crate::auth::AuthStateProvider;
use crate::editor::{EditorView, SingleLineEditorOptions, TextOptions};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view;
use crate::pane_group::{BackingView, PaneConfiguration, PaneEvent};
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
#[cfg(feature = "local_fs")]
use crate::user_config::WarpConfig;
use crate::view_components::FilterableDropdown;
use crate::view_components::action_button::{
    ActionButton, ButtonSize, PrimaryTheme, SecondaryTheme,
};
use crate::view_components::dropdown::DropdownAction;

pub const HEADER_TEXT: &str = "Router Editor";

const EDITOR_CONTENT_WIDTH: f32 = 340.;
const MODEL_MENU_WIDTH: f32 = 340.;

/// Empty placeholder shown in a model dropdown when no model has been chosen
/// yet, so new routers start blank instead of defaulting to the first available
/// model. The empty string prevents auto-selection of the first item while
/// preserving the blank appearance.
const MODEL_PLACEHOLDER: &str = "";

/// Height of the description input and model dropdown within a prompt rule row.
/// Both fields are forced to this exact height so they line up visually.
const RULE_FIELD_HEIGHT: f32 = 34.;

/// Size (width/height) of the reorder/remove icon buttons in a rule row.
const RULE_ICON_BUTTON_SIZE: f32 = 14.;

/// Horizontal gap between the reorder/remove icon buttons in a rule row.
const RULE_ICON_GAP: f32 = 6.;

/// Total width of the reorder/remove controls (three icon buttons separated by
/// two gaps). This slot is reserved even for a single rule so the rule input
/// and model dropdown keep a constant width regardless of rule count.
const RULE_CONTROLS_WIDTH: f32 = RULE_ICON_BUTTON_SIZE * 3. + RULE_ICON_GAP * 2.;

#[derive(Debug, Clone)]
pub enum CustomRouterEditorEvent {
    Pane(PaneEvent),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CustomRouterEditorAction {
    Close,
    Save,
    SetComplexityDefault(String),
    SetComplexityEasy(String),
    SetComplexityMedium(String),
    SetComplexityHard(String),
    SetPromptDefault(String),
    SetPromptRuleModel { index: usize, model_id: String },
    AddPromptRule,
    RemovePromptRule(usize),
    MovePromptRuleUp(usize),
    MovePromptRuleDown(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterEditorType {
    Complexity,
    Prompt,
}

struct PromptRuleRow {
    description_editor: ViewHandle<EditorView>,
    model_dropdown: ViewHandle<FilterableDropdown<CustomRouterEditorAction>>,
    move_up_mouse_state: MouseStateHandle,
    move_down_mouse_state: MouseStateHandle,
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
    type_control: ViewHandle<SegmentedControl<RouterEditorType>>,

    complexity_default_dropdown: ViewHandle<FilterableDropdown<CustomRouterEditorAction>>,
    complexity_easy_dropdown: ViewHandle<FilterableDropdown<CustomRouterEditorAction>>,
    complexity_medium_dropdown: ViewHandle<FilterableDropdown<CustomRouterEditorAction>>,
    complexity_hard_dropdown: ViewHandle<FilterableDropdown<CustomRouterEditorAction>>,

    complexity_default: String,
    complexity_easy: Option<String>,
    complexity_medium: Option<String>,
    complexity_hard: Option<String>,

    prompt_default_dropdown: ViewHandle<FilterableDropdown<CustomRouterEditorAction>>,
    prompt_default_model: String,
    prompt_rules: Vec<PromptRuleRow>,

    save_button: ViewHandle<ActionButton>,
    cancel_button: ViewHandle<ActionButton>,
    add_rule_button: ViewHandle<ActionButton>,

    upgrade_footer_mouse_state: MouseStateHandle,
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
        // Personalize the placeholder with the user's first name (derived from
        // their display name), falling back to a generic placeholder when no
        // name is available.
        let name_placeholder = AuthStateProvider::as_ref(ctx)
            .get()
            .display_name()
            .as_deref()
            .and_then(|name| name.split_whitespace().next())
            .map(|first_name| format!("{first_name}'s custom router"))
            .unwrap_or_else(|| "My custom router".to_string());
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
            editor.set_placeholder_text(&name_placeholder, ctx);
            if !initial_name.is_empty() {
                editor.set_buffer_text(&initial_name, ctx);
            }
            editor
        });
        let font_family = Appearance::as_ref(ctx).ui_font_family();
        let font_size = Appearance::as_ref(ctx).ui_font_size();
        name_editor.update(ctx, |editor, ctx| {
            editor.set_font_size(font_size, ctx);
            editor.set_font_family(font_family, ctx);
        });

        // Routing type selector (segmented control)
        let init_type = router_type;
        let type_control = ctx.add_typed_action_view(move |ctx| {
            SegmentedControl::new(
                vec![RouterEditorType::Complexity, RouterEditorType::Prompt],
                |router_type, is_selected, app| {
                    let appearance = Appearance::as_ref(app);
                    let theme = appearance.theme();
                    Some(RenderableOptionConfig {
                        icon_path: "",
                        icon_color: theme.main_text_color(theme.background()).into(),
                        label: Some(LabelConfig {
                            label: match router_type {
                                RouterEditorType::Complexity => "Complexity".into(),
                                RouterEditorType::Prompt => "Rules".into(),
                            },
                            width_override: Some(70.0),
                            color: if is_selected {
                                theme.accent().into()
                            } else {
                                theme.main_text_color(theme.background()).into()
                            },
                        }),
                        tooltip: None,
                        background: if is_selected {
                            UiFill::Solid(theme.surface_3().into())
                        } else {
                            UiFill::None
                        },
                    })
                },
                init_type,
                type_control_styles(ctx),
            )
        });
        ctx.subscribe_to_view(&type_control, |me, _, event, ctx| {
            let SegmentedControlEvent::OptionSelected(t) = event;
            me.set_router_type(*t, ctx);
        });
        ctx.subscribe_to_model(&Appearance::handle(ctx), |me, _, _, ctx| {
            me.type_control.update(ctx, |control, ctx| {
                control.set_styles(type_control_styles(ctx), ctx);
            });
            ctx.notify();
        });

        // Model dropdowns — searchable, with icons and display names.
        let upgrade_footer_mouse_state = MouseStateHandle::default();

        let complexity_default_dropdown = make_filterable_model_dropdown(
            &init_cdefault,
            CustomRouterEditorAction::SetComplexityDefault,
            &upgrade_footer_mouse_state,
            ctx,
        );
        let complexity_easy_dropdown = make_filterable_model_dropdown(
            init_ceasy.as_deref().unwrap_or_default(),
            CustomRouterEditorAction::SetComplexityEasy,
            &upgrade_footer_mouse_state,
            ctx,
        );
        let complexity_medium_dropdown = make_filterable_model_dropdown(
            init_cmedium.as_deref().unwrap_or_default(),
            CustomRouterEditorAction::SetComplexityMedium,
            &upgrade_footer_mouse_state,
            ctx,
        );
        let complexity_hard_dropdown = make_filterable_model_dropdown(
            init_chard.as_deref().unwrap_or_default(),
            CustomRouterEditorAction::SetComplexityHard,
            &upgrade_footer_mouse_state,
            ctx,
        );
        let prompt_default_dropdown = make_filterable_model_dropdown(
            &init_pdefault,
            CustomRouterEditorAction::SetPromptDefault,
            &upgrade_footer_mouse_state,
            ctx,
        );

        let mut prompt_rules: Vec<PromptRuleRow> = init_prules
            .iter()
            .enumerate()
            .map(|(i, rule)| {
                make_prompt_rule_row(
                    i,
                    &rule.description,
                    &rule.model,
                    &upgrade_footer_mouse_state,
                    ctx,
                )
            })
            .collect();
        // Prompt routing requires at least one rule, so seed empty rule rows
        // when none were loaded. New routers start with two rows to hint that
        // multiple rules are supported (saving still only requires one populated
        // rule); editing a router without prompt rules seeds a single row.
        if prompt_rules.is_empty() {
            let default_rule_rows: usize = if existing.is_none() { 2 } else { 1 };
            for i in 0..default_rule_rows {
                prompt_rules.push(make_prompt_rule_row(
                    i,
                    "",
                    "",
                    &upgrade_footer_mouse_state,
                    ctx,
                ));
            }
        }

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
        let add_rule_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("+ Add rule", SecondaryTheme)
                .with_size(ButtonSize::Small)
                .on_click(|ctx| ctx.dispatch_typed_action(CustomRouterEditorAction::AddPromptRule))
        });

        let view = Self {
            existing,
            pane_configuration,
            focus_handle: None,
            scroll_state: Default::default(),
            name_editor,
            router_type,
            type_control,
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
            add_rule_button,
            upgrade_footer_mouse_state,
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
        let ms = self.upgrade_footer_mouse_state.clone();
        repopulate_filterable(
            &self.complexity_default_dropdown,
            &self.complexity_default,
            CustomRouterEditorAction::SetComplexityDefault,
            &ms,
            ctx,
        );
        repopulate_filterable(
            &self.complexity_easy_dropdown,
            self.complexity_easy.as_deref().unwrap_or_default(),
            CustomRouterEditorAction::SetComplexityEasy,
            &ms,
            ctx,
        );
        repopulate_filterable(
            &self.complexity_medium_dropdown,
            self.complexity_medium.as_deref().unwrap_or_default(),
            CustomRouterEditorAction::SetComplexityMedium,
            &ms,
            ctx,
        );
        repopulate_filterable(
            &self.complexity_hard_dropdown,
            self.complexity_hard.as_deref().unwrap_or_default(),
            CustomRouterEditorAction::SetComplexityHard,
            &ms,
            ctx,
        );
        repopulate_filterable(
            &self.prompt_default_dropdown,
            &self.prompt_default_model,
            CustomRouterEditorAction::SetPromptDefault,
            &ms,
            ctx,
        );
        for (i, row) in self.prompt_rules.iter_mut().enumerate() {
            let sel = row.current_model.clone();
            repopulate_filterable(
                &row.model_dropdown,
                &sel,
                move |id| CustomRouterEditorAction::SetPromptRuleModel {
                    index: i,
                    model_id: id,
                },
                &ms,
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
                for (field, val) in [
                    ("Default", self.complexity_default.as_str()),
                    ("Easy", self.complexity_easy.as_deref().unwrap_or_default()),
                    (
                        "Medium",
                        self.complexity_medium.as_deref().unwrap_or_default(),
                    ),
                    ("Hard", self.complexity_hard.as_deref().unwrap_or_default()),
                ] {
                    if val.is_empty() {
                        self.save_error = Some(format!("{field} model is required."));
                        ctx.notify();
                        return;
                    }
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
                if rules.is_empty() {
                    self.save_error = Some(
                        "At least one rule with a description and model is required.".to_string(),
                    );
                    ctx.notify();
                    return;
                }
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

        #[cfg(feature = "local_fs")]
        {
            let yaml = match router.to_yaml_string() {
                Ok(y) => y,
                Err(e) => {
                    self.save_error = Some(format!("Serialization: {e}"));
                    ctx.notify();
                    return;
                }
            };
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

    fn add_prompt_rule(&mut self, ctx: &mut ViewContext<Self>) {
        let index = self.prompt_rules.len();
        let ms = self.upgrade_footer_mouse_state.clone();
        let row = make_prompt_rule_row(index, "", "", &ms, ctx);
        self.prompt_rules.push(row);
        ctx.notify();
    }

    fn set_router_type(&mut self, router_type: RouterEditorType, ctx: &mut ViewContext<Self>) {
        self.router_type = router_type;
        // Prompt routing requires at least one rule; ensure a row exists when
        // switching into prompt mode.
        if router_type == RouterEditorType::Prompt && self.prompt_rules.is_empty() {
            self.add_prompt_rule(ctx);
        }
        ctx.notify();
    }

    fn remove_prompt_rule(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        // Prompt routing requires at least one rule; never remove the last row.
        if self.prompt_rules.len() <= 1 {
            return;
        }
        if index < self.prompt_rules.len() {
            self.prompt_rules.remove(index);
        }
        ctx.notify();
    }

    /// Swaps the rule at `index` with the one at `target`, preserving each
    /// rule's description text and selected model.
    ///
    /// The rows are fully rebuilt (rather than swapped in place) because each
    /// model dropdown captures its row index in its on-select action; rebuilding
    /// keeps those captured indices in sync with the new ordering.
    fn move_prompt_rule(&mut self, index: usize, target: usize, ctx: &mut ViewContext<Self>) {
        let len = self.prompt_rules.len();
        if index >= len || target >= len {
            return;
        }
        // Snapshot each row's current content in order, then swap.
        let mut data: Vec<(String, String)> = self
            .prompt_rules
            .iter()
            .map(|row| {
                (
                    row.description_editor.as_ref(ctx).buffer_text(ctx),
                    row.current_model.clone(),
                )
            })
            .collect();
        data.swap(index, target);

        let ms = self.upgrade_footer_mouse_state.clone();
        self.prompt_rules = data
            .iter()
            .enumerate()
            .map(|(i, (desc, model))| make_prompt_rule_row(i, desc, model, &ms, ctx))
            .collect();
        ctx.notify();
    }

    // ------------------------------------------------------------------
    // Rendering
    // ------------------------------------------------------------------

    /// A section header label, styled to match the field headers in the
    /// execution profile editor (sentence case, active text color).
    fn section_label(label: impl Into<String>, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            Text::new(label.into(), appearance.ui_font_family(), 13.)
                .with_color(appearance.theme().active_ui_text_color().into())
                .finish(),
        )
        .with_margin_bottom(4.)
        .finish()
    }

    fn render_complexity_section(&self, appearance: &Appearance) -> Box<dyn Element> {
        Flex::column()
            .with_child(Self::section_label("Models", appearance))
            .with_child(labeled_dropdown(
                "Default (required)",
                &self.complexity_default_dropdown,
                appearance,
            ))
            .with_child(
                Container::new(labeled_dropdown(
                    "Easy (required)",
                    &self.complexity_easy_dropdown,
                    appearance,
                ))
                .with_margin_top(8.)
                .finish(),
            )
            .with_child(
                Container::new(labeled_dropdown(
                    "Medium (required)",
                    &self.complexity_medium_dropdown,
                    appearance,
                ))
                .with_margin_top(8.)
                .finish(),
            )
            .with_child(
                Container::new(labeled_dropdown(
                    "Hard (required)",
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
            .with_child(Self::section_label("Default model", appearance))
            .with_child(
                ConstrainedBox::new(ChildView::new(&self.prompt_default_dropdown).finish())
                    .with_width(EDITOR_CONTENT_WIDTH)
                    .finish(),
            );

        if !self.prompt_rules.is_empty() {
            column.add_child(
                Container::new(Self::section_label("Rules".to_string(), appearance))
                    .with_margin_top(12.)
                    .finish(),
            );
            let rules_copy = FormattedText::new([
                FormattedTextLine::Line(vec![FormattedTextFragment::plain_text(
                    "Rules are custom prompts that describe when to use a specific model. Warp intelligently matches your tasks against these rules.",
                )]),
                FormattedTextLine::Line(vec![FormattedTextFragment::plain_text(
                    "Rules are matched top to bottom — rules higher in the list take precedence over those below.",
                )]),
            ]);
            column.add_child(
                Container::new(
                    FormattedTextElement::new(
                        rules_copy,
                        11.,
                        appearance.ui_font_family(),
                        appearance.ui_font_family(),
                        blended_colors::text_sub(
                            appearance.theme(),
                            appearance.theme().surface_1(),
                        ),
                        HighlightedHyperlink::default(),
                    )
                    .with_line_height_ratio(1.5)
                    .finish(),
                )
                .with_margin_bottom(12.)
                .finish(),
            );
            let rule_count = self.prompt_rules.len();
            for (i, row) in self.prompt_rules.iter().enumerate() {
                column.add_child(
                    Container::new(render_rule_row(i, row, rule_count, appearance))
                        .with_margin_bottom(16.)
                        .finish(),
                );
            }
        }

        column.finish()
    }

    fn render_content(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let mut col = Flex::column();

        // Name
        col.add_child(
            Container::new(
                Flex::column()
                    .with_child(Self::section_label("Router name", appearance))
                    .with_child(
                        ConstrainedBox::new(editor_row(&self.name_editor, None, appearance))
                            .with_width(EDITOR_CONTENT_WIDTH)
                            .finish(),
                    )
                    .finish(),
            )
            .with_margin_bottom(16.)
            .finish(),
        );

        // Router type: a short, bold-prefixed explanation of each mode shown
        // above the segmented control.
        let routing_type_copy = FormattedText::new([
            FormattedTextLine::Line(vec![
                FormattedTextFragment::bold("Complexity-based"),
                FormattedTextFragment::plain_text(
                    " routing chooses a model based on Warp's classification of the task's difficulty.",
                ),
            ]),
            FormattedTextLine::Line(vec![
                FormattedTextFragment::bold("Rule-based"),
                FormattedTextFragment::plain_text(
                    " routing chooses a model based on custom prompts.",
                ),
            ]),
        ]);
        col.add_child(
            Container::new(
                Flex::column()
                    .with_child(Self::section_label("Router type", appearance))
                    .with_child(
                        Container::new(
                            FormattedTextElement::new(
                                routing_type_copy,
                                11.,
                                appearance.ui_font_family(),
                                appearance.ui_font_family(),
                                blended_colors::text_sub(
                                    appearance.theme(),
                                    appearance.theme().surface_1(),
                                ),
                                HighlightedHyperlink::default(),
                            )
                            .with_line_height_ratio(1.5)
                            .finish(),
                        )
                        .with_margin_bottom(8.)
                        .finish(),
                    )
                    .with_child(ChildView::new(&self.type_control).finish())
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
                        .with_margin_bottom(8.)
                        .finish(),
                );
            }
            RouterEditorType::Prompt => {
                col.add_child(
                    Container::new(self.render_prompt_section(appearance, app))
                        .with_margin_bottom(8.)
                        .finish(),
                );
            }
        }

        // Buttons row: "Add rule" (prompt routing only) sits left-aligned,
        // while Cancel and Save stay right-aligned. A flexible spacer pushes
        // the Cancel/Save group to the right edge in both routing modes.
        let mut btn_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);
        if self.router_type == RouterEditorType::Prompt {
            btn_row.add_child(ChildView::new(&self.add_rule_button).finish());
        }
        btn_row.add_child(Expanded::new(1., Empty::new().finish()).finish());
        btn_row.add_child(
            Container::new(ChildView::new(&self.cancel_button).finish())
                .with_margin_right(8.)
                .finish(),
        );
        btn_row.add_child(ChildView::new(&self.save_button).finish());
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
            CustomRouterEditorAction::SetComplexityDefault(id) => {
                self.complexity_default = id.clone();
            }
            CustomRouterEditorAction::SetComplexityEasy(id) => {
                self.complexity_easy = Some(id.clone());
            }
            CustomRouterEditorAction::SetComplexityMedium(id) => {
                self.complexity_medium = Some(id.clone());
            }
            CustomRouterEditorAction::SetComplexityHard(id) => {
                self.complexity_hard = Some(id.clone());
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
            CustomRouterEditorAction::MovePromptRuleUp(i) => {
                if *i > 0 {
                    self.move_prompt_rule(*i, *i - 1, ctx);
                }
            }
            CustomRouterEditorAction::MovePromptRuleDown(i) => {
                self.move_prompt_rule(*i, *i + 1, ctx);
            }
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

/// Styles for the routing-type [`SegmentedControl`], themed to match the
/// editor's other inputs.
fn type_control_styles(app: &AppContext) -> UiComponentStyles {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    UiComponentStyles {
        font_family_id: Some(appearance.ui_font_family()),
        font_size: Some(appearance.ui_font_size()),
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.0))),
        border_width: Some(1.0),
        border_color: Some(UiFill::Solid(theme.surface_3().into())),
        background: Some(UiFill::Solid(theme.background().into())),
        height: Some(24.0),
        padding: Some(Coords {
            top: 2.,
            bottom: 2.,
            left: 2.,
            right: 2.,
        }),
        ..Default::default()
    }
}

/// Creates and populates a [`FilterableDropdown`] for model selection.
///
/// Items are built via [`available_model_menu_items`] so they carry provider
/// icons and display names. The current selection is restored by action.
fn make_filterable_model_dropdown<F>(
    selected_id: &str,
    make_action: F,
    upgrade_mouse_state: &MouseStateHandle,
    ctx: &mut ViewContext<CustomRouterEditorView>,
) -> ViewHandle<FilterableDropdown<CustomRouterEditorAction>>
where
    F: Fn(String) -> CustomRouterEditorAction + 'static + Clone,
{
    let selected_owned = selected_id.to_string();
    let ms = upgrade_mouse_state.clone();
    ctx.add_typed_action_view(move |ctx| {
        let mut d = FilterableDropdown::new(ctx);
        d.set_menu_width(MODEL_MENU_WIDTH, ctx);
        fill_filterable_dropdown(ctx, &mut d, &selected_owned, make_action.clone(), &ms);
        d
    })
}

/// Repopulates an existing [`FilterableDropdown`] after model choices change.
fn repopulate_filterable<F>(
    dropdown: &ViewHandle<FilterableDropdown<CustomRouterEditorAction>>,
    selected_id: &str,
    make_action: F,
    upgrade_mouse_state: &MouseStateHandle,
    ctx: &mut ViewContext<CustomRouterEditorView>,
) where
    F: Fn(String) -> CustomRouterEditorAction + Clone,
{
    let selected_owned = selected_id.to_string();
    let ms = upgrade_mouse_state.clone();
    dropdown.update(ctx, move |d, ctx| {
        fill_filterable_dropdown(ctx, d, &selected_owned, make_action.clone(), &ms);
    });
}

/// Inner helper: fills a [`FilterableDropdown`] with rich model items.
fn fill_filterable_dropdown<F>(
    ctx: &mut warpui::ViewContext<FilterableDropdown<CustomRouterEditorAction>>,
    dropdown: &mut FilterableDropdown<CustomRouterEditorAction>,
    selected_id: &str,
    make_action: F,
    upgrade_mouse_state: &MouseStateHandle,
) where
    F: Fn(String) -> CustomRouterEditorAction + Clone,
{
    // Set the placeholder before populating items so the initial
    // `set_filtered_items` keeps an empty selection blank rather than
    // auto-selecting the first model.
    dropdown.set_placeholder(MODEL_PLACEHOLDER, ctx);
    let items = available_model_menu_items(
        LLMPreferences::as_ref(ctx)
            .get_base_llm_choices_for_agent_mode(ctx)
            .filter(|llm| !is_auto_target(llm.id.as_str()))
            .collect_vec(),
        |llm| DropdownAction::select_action_and_close(make_action(llm.id.to_string())),
        None,
        None,
        false,
        false,
        ctx,
    );
    dropdown.set_rich_items(items, ctx);
    dropdown.clear_footer(ctx);

    if !selected_id.is_empty() {
        dropdown.set_selected_by_action(make_action(selected_id.to_string()), ctx);
    }
    let _ = upgrade_mouse_state; // reserved for future upgrade footer
}

fn make_prompt_rule_row(
    index: usize,
    description: &str,
    model: &str,
    upgrade_mouse_state: &MouseStateHandle,
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
        editor.set_placeholder_text("Describe when to use this model\u{2026}", ctx);
        // Use the UI font (rather than the editor's default mono font) so the
        // input matches the rest of the editor's text inputs.
        let font_family = Appearance::as_ref(ctx).ui_font_family();
        editor.set_font_family(font_family, ctx);
        if !desc_owned.is_empty() {
            editor.set_buffer_text(&desc_owned, ctx);
        }
        editor
    });

    let model_dropdown = make_filterable_model_dropdown(
        model,
        move |id| CustomRouterEditorAction::SetPromptRuleModel {
            index,
            model_id: id,
        },
        upgrade_mouse_state,
        ctx,
    );
    // Match the dropdown's bar height to the description input and drop its
    // default vertical margin so the two fields align flush within the row.
    model_dropdown.update(ctx, |dropdown, ctx| {
        dropdown.set_vertical_margin(0., ctx);
        dropdown.set_top_bar_height(RULE_FIELD_HEIGHT, ctx);
    });

    PromptRuleRow {
        description_editor,
        model_dropdown,
        move_up_mouse_state: Default::default(),
        move_down_mouse_state: Default::default(),
        remove_mouse_state: Default::default(),
        current_model: model.to_string(),
    }
}

/// Renders an `EditorView` as a text input styled like the AI settings API-key
/// inputs. The returned element has no width constraint, so callers should wrap
/// it (e.g. in a `ConstrainedBox` or `Expanded`) to size it. Pass `height` to
/// force an exact box height (used so rule inputs match the model dropdown).
fn editor_row(
    editor: &ViewHandle<EditorView>,
    height: Option<f32>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    appearance
        .ui_builder()
        .text_input(editor.clone())
        .with_style(UiComponentStyles {
            padding: Some(Coords {
                top: 8.,
                bottom: 8.,
                left: 12.,
                right: 12.,
            }),
            background: Some(appearance.theme().surface_2().into()),
            height,
            ..Default::default()
        })
        .build()
        .finish()
}

fn labeled_dropdown(
    label: impl Into<String>,
    dropdown: &ViewHandle<FilterableDropdown<CustomRouterEditorAction>>,
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

/// A label stacked above a field (input/dropdown), matching the spacing used by
/// the AI settings API-key inputs.
fn labeled_field(
    label: impl Into<String>,
    field: Box<dyn Element>,
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
            .with_margin_bottom(4.)
            .finish(),
        )
        .with_child(field)
        .finish()
}

/// A small icon button for the per-rule reorder/remove controls. When `enabled`
/// is false it renders greyed out and is non-interactive; when enabled it shows
/// a pointing-hand cursor and dispatches `action` on click.
fn rule_icon_button(
    icon: Icon,
    enabled: bool,
    mouse_state: MouseStateHandle,
    action: CustomRouterEditorAction,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let color = if enabled {
        appearance
            .theme()
            .sub_text_color(appearance.theme().surface_1())
    } else {
        appearance.theme().disabled_ui_text_color()
    };
    // Inline the builder closure (rather than binding it to a `let`) so the
    // compiler infers the higher-ranked closure lifetime `Hoverable` requires.
    let hoverable = Hoverable::new(mouse_state, move |_| {
        ConstrainedBox::new(icon.to_warpui_icon(color).finish())
            .with_width(RULE_ICON_BUTTON_SIZE)
            .with_height(RULE_ICON_BUTTON_SIZE)
            .finish()
    });
    if enabled {
        hoverable
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _app, _pos| {
                ctx.dispatch_typed_action(action.clone());
            })
            .finish()
    } else {
        hoverable.finish()
    }
}

/// Renders the reorder (up/down) and remove controls for a rule, vertically
/// centered against the input/dropdown fields (not the labels above them).
fn render_rule_controls(
    index: usize,
    row: &PromptRuleRow,
    rule_count: usize,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let buttons = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(rule_icon_button(
            Icon::ChevronUp,
            index > 0,
            row.move_up_mouse_state.clone(),
            CustomRouterEditorAction::MovePromptRuleUp(index),
            appearance,
        ))
        .with_child(
            Container::new(rule_icon_button(
                Icon::ChevronDown,
                index + 1 < rule_count,
                row.move_down_mouse_state.clone(),
                CustomRouterEditorAction::MovePromptRuleDown(index),
                appearance,
            ))
            .with_margin_left(RULE_ICON_GAP)
            .finish(),
        )
        .with_child(
            Container::new(rule_icon_button(
                Icon::X,
                true,
                row.remove_mouse_state.clone(),
                CustomRouterEditorAction::RemovePromptRule(index),
                appearance,
            ))
            .with_margin_left(RULE_ICON_GAP)
            .finish(),
        )
        .finish();

    // Spacer matching the label height + gap so the controls line up with the
    // fields below the labels rather than the labels themselves.
    let label_spacer = Container::new(
        Text::new(" ", appearance.ui_font_family(), 11.)
            .with_color(
                appearance
                    .theme()
                    .sub_text_color(appearance.theme().surface_1())
                    .into(),
            )
            .finish(),
    )
    .with_margin_bottom(4.)
    .finish();

    Flex::column()
        .with_child(label_spacer)
        .with_child(
            ConstrainedBox::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_child(buttons)
                    .finish(),
            )
            .with_height(RULE_FIELD_HEIGHT)
            .finish(),
        )
        .finish()
}

fn render_rule_row(
    index: usize,
    row: &PromptRuleRow,
    rule_count: usize,
    appearance: &Appearance,
) -> Box<dyn Element> {
    const MODEL_WIDTH: f32 = 170.;

    let description_field = labeled_field(
        "Rule",
        editor_row(&row.description_editor, Some(RULE_FIELD_HEIGHT), appearance),
        appearance,
    );
    let model_field = labeled_field(
        "Model",
        ConstrainedBox::new(ChildView::new(&row.model_dropdown).finish())
            .with_width(MODEL_WIDTH)
            .finish(),
        appearance,
    );

    let mut rule_row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_child(Expanded::new(1., description_field).finish())
        .with_child(Container::new(model_field).with_margin_left(8.).finish());

    // Always reserve a fixed-width slot for the reorder/remove controls so the
    // rule input and model dropdown keep the same width regardless of rule
    // count. The controls themselves are only meaningful with multiple rules
    // (prompt routing always keeps at least one rule); otherwise the slot stays
    // empty.
    let controls: Box<dyn Element> = if rule_count > 1 {
        render_rule_controls(index, row, rule_count, appearance)
    } else {
        Empty::new().finish()
    };
    rule_row.add_child(
        Container::new(
            ConstrainedBox::new(controls)
                .with_width(RULE_CONTROLS_WIDTH)
                .finish(),
        )
        .with_margin_left(8.)
        .finish(),
    );
    rule_row.finish()
}
