use std::sync::Arc;

use ai::api_keys::{ApiKeyManager, ApiKeyManagerEvent};
use indexmap::IndexMap;
use instant::{Duration, Instant};
use parking_lot::FairMutex;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    DropShadow, Empty, Expanded, Flex, Hoverable, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement as _, ParentOffsetBounds,
    Percentage, PositionedElementAnchor, PositionedElementOffsetBounds, Radius, Rect, SavePosition,
    Stack, Text, DEFAULT_UI_LINE_HEIGHT_RATIO,
};
use warpui::platform::Cursor;
use warpui::text_layout::ClipConfig;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, EntityId, ModelHandle, SingletonEntity as _, TypedActionView,
    View, ViewContext, ViewHandle,
};

const SIDECAR_HORIZONTAL_GAP: f32 = 8.;
const SIDECAR_POSITION_ID: &str = "model_sidecar_panel";

use warp_cli::agent::Harness;
use warp_core::features::FeatureFlag;
use warp_core::ui::color::{coloru_with_opacity, Opacity};
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;

use crate::ai::blocklist::prompt::PromptIconButtonTheme;
use crate::ai::blocklist::{
    BlocklistAIController, BlocklistAIControllerEvent, BlocklistAIInputEvent, BlocklistAIInputModel,
};
use crate::ai::cloud_agent_settings::CloudAgentSettings;
use crate::ai::custom_model_routers::is_custom_router_id;
use crate::ai::execution_profiles::model_menu_items::{
    available_model_menu_items, has_reasoning_variants, is_auto,
};
use crate::ai::execution_profiles::profiles::{
    AIExecutionProfilesModel, AIExecutionProfilesModelEvent, ClientProfileId,
};
use crate::ai::harness_availability::{
    HarnessAvailabilityEvent, HarnessAvailabilityModel, HarnessModelInfo,
};
use crate::ai::llms::{
    dedupe_model_display_names, is_using_api_key_for_provider, LLMId, LLMInfo, LLMPreferences,
    LLMPreferencesEvent, LLMSpec,
};
use crate::appearance::Appearance;
use crate::cloud_object::model::generic_string_model::StringModel;
use crate::context_chips::display_chip::{udi_font_size, udi_icon_size};
use crate::context_chips::spacing;
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields};
use crate::settings_view::SettingsSection;
use crate::terminal::input::{MenuPositioning, MenuPositioningProvider};
use crate::terminal::view::ambient_agent::AmbientAgentViewModel;
use crate::terminal::TerminalModel;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, ButtonSize, SecondaryTheme,
};
use crate::view_components::{FeaturePopup, NewFeaturePopupEvent, NewFeaturePopupLabel};
use crate::workspace::WorkspaceAction;

const MENU_WIDTH: f32 = 280.;
const NEW_MODEL_CHOICES_POPUP_DELAY: Duration = Duration::from_millis(500);
const BLURRED_OPACITY: Opacity = 50;
const SEPARATOR_WIDTH: f32 = 1.0;
const CORNER_RADIUS: f32 = 4.0;
const BORDER_WIDTH: f32 = 1.0;
/// Inner rounded corners are 1px smaller than the outer border radius
const INNER_CORNER_RADIUS: f32 = CORNER_RADIUS - BORDER_WIDTH;
const BASE_FONT_SIZE: f32 = 10.0;
const HORIZONTAL_PADDING_SCALE: f32 = 0.35;
const VERTICAL_PADDING: f32 = 2.5;
const MIN_HORIZONTAL_PADDING: f32 = 3.5;
const ICON_SPACING: f32 = 8.0;
const MAX_PROFILE_NAME_WIDTH_SCALE_FACTOR: f32 = 10.0;

const PROFILE_SELECTOR_POSITION_ID: &str = "profile_selector";

const PROFILE_PICKER_TOOLTIP: &str = "Choose an AI execution profile";
const MODEL_PICKER_TOOLTIP: &str = "Choose an agent model";
const MODEL_LOCKED_FOR_FOLLOWUP_TOOLTIP: &str = "Follow-ups use the original run's model";
const MODEL_REQUIRES_EDIT_ACCESS_TOOLTIP: &str = "Request edit access to change model";
const HARNESS_DEFAULT_MODEL_LABEL: &str = "default";

pub fn calculate_scaled_font_size(appearance: &warp_core::ui::appearance::Appearance) -> f32 {
    if FeatureFlag::AgentView.is_enabled() {
        udi_font_size(appearance)
    } else {
        BASE_FONT_SIZE * appearance.monospace_ui_scalar()
    }
}

/// Calculate the maximum width for profile name text (we will clip to this width)
pub fn calculate_max_profile_name_width(appearance: &warp_core::ui::appearance::Appearance) -> f32 {
    let scaled_font_size = calculate_scaled_font_size(appearance);
    scaled_font_size * MAX_PROFILE_NAME_WIDTH_SCALE_FACTOR
}

#[derive(Clone, Debug)]
enum ButtonTextColor {
    Fill(Fill),
}

impl ButtonTextColor {
    fn to_color_u(&self, _appearance: &Appearance) -> pathfinder_color::ColorU {
        match self {
            ButtonTextColor::Fill(fill) => fill.into_solid(),
        }
    }
}

/// Unified theme for profile and model selector buttons
#[derive(Clone)]
struct SelectorChipTheme {
    text_color: ButtonTextColor,
    is_blurred: bool,
}

impl ActionButtonTheme for SelectorChipTheme {
    fn background(&self, hovered: bool, appearance: &Appearance) -> Option<Fill> {
        let theme = appearance.theme();
        Some(if hovered {
            theme.surface_2()
        } else {
            theme.surface_1()
        })
    }

    fn text_color(
        &self,
        _hovered: bool,
        _background: Option<Fill>,
        appearance: &Appearance,
    ) -> pathfinder_color::ColorU {
        let color = self.text_color.to_color_u(appearance);
        if self.is_blurred {
            coloru_with_opacity(color, BLURRED_OPACITY)
        } else {
            color
        }
    }

    fn font_properties(&self) -> Option<warpui::fonts::Properties> {
        if FeatureFlag::CloudModeInputV2.is_enabled() {
            Some(warpui::fonts::Properties {
                weight: warpui::fonts::Weight::Semibold,
                ..Default::default()
            })
        } else {
            None
        }
    }
}

/// A unified profile and model selector component that combines both selectors
/// into a single component.
pub struct ProfileModelSelector {
    profile_button: ViewHandle<ActionButton>,
    model_button: ViewHandle<ActionButton>,
    profile_compact_button: ViewHandle<ActionButton>,
    model_compact_button: ViewHandle<ActionButton>,
    profile_dropdown: ViewHandle<Menu<ProfileModelSelectorAction>>,
    model_dropdown: ViewHandle<Menu<ProfileModelSelectorAction>>,
    model_spec_sidecar: ModelSpecSidecar,
    is_profile_menu_open: bool,
    is_model_menu_open: bool,
    terminal_view_id: EntityId,
    profile_mouse_state: MouseStateHandle,
    model_mouse_state: MouseStateHandle,
    menu_positioning_provider: Arc<dyn MenuPositioningProvider>,
    is_blurred: bool,
    new_model_popup: ViewHandle<FeaturePopup>,
    input_model: ModelHandle<BlocklistAIInputModel>,
    ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
    render_compact: bool,
    hovered_llm_info: Option<LLMInfo>,
    manage_api_key_button: ViewHandle<ActionButton>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    all_model_choices: Vec<LLMInfo>,
}

pub enum ProfileModelSelectorEvent {
    OpenSettings(SettingsSection),
    MenuVisibilityChanged { open: bool },
    ToggleInlineModelSelector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileModelSelectorAction {
    SelectProfile(ClientProfileId),
    SelectModel(LLMId),
    SelectAutoModel,
    SelectReasoningModel(String),
    SelectHarnessModel {
        model_id: String,
        reasoning_level: Option<String>,
    },
    ManageProfiles,
    ToggleProfileMenu,
    ToggleModelMenu,
}

/// Menu type for get_selected_llm_info lookups
enum MenuType {
    Main,
    Sidecar,
}

/// Identifies which sidecar panel we're working with
#[derive(Clone)]
enum ModelSpecSidecarKind {
    Auto,
    Reasoning,
}

/// Encapsulates state for a sidecar panel (auto models or reasoning levels)
struct ModelSpecSidecar {
    dropdown: ViewHandle<Menu<ProfileModelSelectorAction>>,
    hovered_info: Option<LLMInfo>,
    active_kind: Option<ModelSpecSidecarKind>,
}

impl ProfileModelSelectorAction {
    pub fn selected_model_id(&self) -> Option<LLMId> {
        match self {
            ProfileModelSelectorAction::SelectModel(id) => Some(id.clone()),
            _ => None,
        }
    }
}

impl ProfileModelSelector {
    pub fn new(
        menu_positioning_provider: Arc<dyn crate::terminal::input::MenuPositioningProvider>,
        terminal_view_id: EntityId,
        input_model: ModelHandle<BlocklistAIInputModel>,
        ambient_agent_view_model: Option<ModelHandle<AmbientAgentViewModel>>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        controller: Option<ModelHandle<BlocklistAIController>>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let profile_button = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            ActionButton::new(
                "",
                SelectorChipTheme {
                    text_color: ButtonTextColor::Fill(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_1()),
                    ),
                    is_blurred: false,
                },
            )
            .with_disabled_theme(SelectorChipTheme {
                text_color: ButtonTextColor::Fill(
                    internal_colors::text_disabled(
                        appearance.theme(),
                        appearance.theme().surface_1(),
                    )
                    .into(),
                ),
                is_blurred: false,
            })
            .with_tooltip(PROFILE_PICKER_TOOLTIP)
            .with_size(ButtonSize::UDIButton)
            .with_icon(Icon::Psychology)
        });

        let model_button = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            ActionButton::new(
                "",
                SelectorChipTheme {
                    text_color: ButtonTextColor::Fill(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_1()),
                    ),
                    is_blurred: false,
                },
            )
            .with_disabled_theme(SelectorChipTheme {
                text_color: ButtonTextColor::Fill(
                    internal_colors::text_disabled(
                        appearance.theme(),
                        appearance.theme().surface_1(),
                    )
                    .into(),
                ),
                is_blurred: false,
            })
            .with_tooltip(MODEL_PICKER_TOOLTIP)
            .with_size(ButtonSize::UDIButton)
        });

        let profile_compact_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("", PromptIconButtonTheme::new(false))
                .with_icon(Icon::Psychology)
                .with_tooltip(PROFILE_PICKER_TOOLTIP)
                .with_size(ButtonSize::UDIButton)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(ProfileModelSelectorAction::ToggleProfileMenu);
                })
        });

        let model_compact_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("", PromptIconButtonTheme::new(false))
                .with_icon(Icon::Neurology)
                .with_tooltip(MODEL_PICKER_TOOLTIP)
                .with_size(ButtonSize::UDIButton)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(ProfileModelSelectorAction::ToggleModelMenu);
                })
        });

        let profile_dropdown = ctx.add_typed_action_view(|_ctx| {
            Menu::new()
                .prevent_interaction_with_other_elements()
                .with_drop_shadow()
        });

        let model_dropdown = ctx.add_typed_action_view(|_ctx| {
            Menu::new()
                .with_ignore_hover_when_covered()
                .with_safe_triangle()
                .prevent_interaction_with_other_elements()
                .with_drop_shadow()
        });

        let sidecar_dropdown = ctx.add_typed_action_view(|_ctx| Menu::new());

        let new_model_popup = ctx.add_typed_action_view(|_ctx| {
            FeaturePopup::new_feature(NewFeaturePopupLabel::FromCallable(Box::new(|ctx| {
                let llm_preferences = LLMPreferences::as_ref(ctx);
                let new_choices = llm_preferences.new_choices_since_last_update();
                if let Some(new_choices) = new_choices {
                    let deduped_names = dedupe_model_display_names(new_choices.iter());
                    let max_display = 5;
                    let has_overflow = deduped_names.len() > max_display;
                    let display_names = &deduped_names[..deduped_names.len().min(max_display)];

                    let mut label = display_names
                        .iter()
                        .map(|name| {
                            if *name == "auto" {
                                "auto-select the best model for the task"
                            } else {
                                name
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    if has_overflow {
                        label += ", ...";
                    }
                    label
                } else {
                    "New models available".to_string()
                }
            })))
        });

        ctx.subscribe_to_view(&profile_dropdown, |me, _, event, ctx| {
            if let MenuEvent::Close { .. } = event {
                me.set_profile_menu_visibility(false, ctx);
            }
        });

        ctx.subscribe_to_view(&model_dropdown, |me, _, event, ctx| {
            match event {
                MenuEvent::Close { .. } => {
                    me.set_model_menu_visibility(false, ctx);
                    // Reset hovered llm info to the selected model
                    let selected_index =
                        me.model_dropdown.read(ctx, |menu, _| menu.selected_index());
                    me.set_hovered_llm_info(selected_index, ctx);
                }
                MenuEvent::ItemSelected => {
                    let selected_index =
                        me.model_dropdown.read(ctx, |menu, _| menu.selected_index());
                    me.set_hovered_llm_info(selected_index, ctx);
                    ctx.notify();
                }
                MenuEvent::ItemHovered => {
                    if me.is_model_menu_open {
                        let hovered_index =
                            me.model_dropdown.read(ctx, |menu, _| menu.hovered_index());
                        if hovered_index.is_some() {
                            me.set_hovered_llm_info(hovered_index, ctx);
                            ctx.notify();
                        }
                    }
                }
            }
        });

        ctx.subscribe_to_view(&sidecar_dropdown, |me, _, event, ctx| match event {
            MenuEvent::Close { .. } => {}
            MenuEvent::ItemSelected => {
                let selected_index = me
                    .model_spec_sidecar
                    .dropdown
                    .read(ctx, |menu, _| menu.selected_index());
                me.set_sidecar_hovered_info(selected_index, ctx);
                ctx.notify();
            }
            MenuEvent::ItemHovered => {
                if me.is_model_menu_open {
                    let hovered_index = me
                        .model_spec_sidecar
                        .dropdown
                        .read(ctx, |menu, _| menu.hovered_index());
                    if hovered_index.is_some() {
                        me.set_sidecar_hovered_info(hovered_index, ctx);
                        ctx.notify();
                    }
                }
            }
        });

        ctx.subscribe_to_view(&new_model_popup, move |_me, _, event, ctx| {
            if matches!(event, NewFeaturePopupEvent::Dismissed) {
                LLMPreferences::handle(ctx).update(ctx, |preferences, _| {
                    preferences.hide_llm_popup(terminal_view_id)
                });
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(&input_model, move |_me, _, event, ctx| match event {
            BlocklistAIInputEvent::InputTypeChanged { config }
            | BlocklistAIInputEvent::LockChanged { config } => {
                if config.is_locked && !config.input_type.is_ai() {
                    let llm_preferences = LLMPreferences::as_ref(ctx);
                    llm_preferences.hide_llm_popup(terminal_view_id);
                } else if config.input_type.is_ai() {
                    ctx.spawn(
                        warpui::r#async::Timer::after(NEW_MODEL_CHOICES_POPUP_DELAY),
                        |_, _, ctx| {
                            ctx.notify();
                        },
                    );
                }
                ctx.notify();
            }
        });

        ctx.subscribe_to_model(
            &LLMPreferences::handle(ctx),
            |me, _, event, ctx| match event {
                LLMPreferencesEvent::UpdatedAvailableLLMs => {
                    me.refresh_state(ctx);
                    me.new_model_popup.update(ctx, |_popup, ctx| {
                        ctx.notify();
                    });
                    ctx.notify();
                }
                LLMPreferencesEvent::UpdatedActiveAgentModeLLM => {
                    me.refresh_state(ctx);
                    me.new_model_popup.update(ctx, |_popup, ctx| {
                        ctx.notify();
                    });
                    ctx.notify();
                }
                _ => (),
            },
        );

        if let Some(controller) = &controller {
            ctx.subscribe_to_model(controller, |me, _, event, ctx| {
                if let BlocklistAIControllerEvent::SentRequest { .. } = event {
                    let llm_preferences = LLMPreferences::as_ref(ctx);
                    llm_preferences.hide_llm_popup(me.terminal_view_id);
                    ctx.notify();
                }
            });
        }
        ctx.subscribe_to_model(&Appearance::handle(ctx), |me, _, _, ctx| {
            me.handle_appearance_change(ctx);
        });

        // Refresh model menu when BYO API keys update so the key icons reflect the latest state.
        ctx.subscribe_to_model(
            &ApiKeyManager::handle(ctx),
            |me, _model, _event: &ApiKeyManagerEvent, ctx| {
                me.refresh_model_menu(ctx);
                ctx.notify();
            },
        );

        ctx.subscribe_to_model(
            &AIExecutionProfilesModel::handle(ctx),
            |me, _, event, ctx| {
                match event {
                    AIExecutionProfilesModelEvent::ProfileCreated
                    | AIExecutionProfilesModelEvent::ProfileDeleted
                    | AIExecutionProfilesModelEvent::ProfileUpdated(_) => {
                        // Re-render when profiles are added or deleted to show/hide profile selector
                        me.refresh_state(ctx);
                    }
                    AIExecutionProfilesModelEvent::UpdatedActiveProfile { terminal_view_id }
                        if *terminal_view_id == me.terminal_view_id =>
                    {
                        me.refresh_state(ctx);
                    }
                    _ => (),
                }
            },
        );

        if let Some(ref ambient_model) = ambient_agent_view_model {
            ctx.subscribe_to_model(ambient_model, |me, _, event, ctx| {
                use crate::terminal::view::ambient_agent::AmbientAgentViewModelEvent;
                if matches!(
                    event,
                    AmbientAgentViewModelEvent::HarnessSelected
                        | AmbientAgentViewModelEvent::HarnessModelSelected
                        | AmbientAgentViewModelEvent::RunLifecycleChanged
                        | AmbientAgentViewModelEvent::SessionReady { .. }
                        | AmbientAgentViewModelEvent::FollowupDispatched
                ) {
                    me.refresh_state(ctx);
                }
            });
        }

        ctx.subscribe_to_model(
            &HarnessAvailabilityModel::handle(ctx),
            |me, _, event, ctx| {
                if let HarnessAvailabilityEvent::Changed = event {
                    me.refresh_state(ctx);
                }
            },
        );

        let manage_api_key_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Manage", SecondaryTheme)
                .with_tooltip("Manage API keys")
                .with_size(ButtonSize::XSmall)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(WorkspaceAction::ShowSettingsPageWithSearch {
                        search_query: "api".to_string(),
                        section: Some(SettingsSection::WarpAgent),
                    });
                })
        });

        let mut me = Self {
            profile_button,
            model_button,
            profile_compact_button,
            model_compact_button,
            profile_dropdown,
            model_dropdown,
            model_spec_sidecar: ModelSpecSidecar {
                dropdown: sidecar_dropdown,
                hovered_info: None,
                active_kind: None,
            },
            is_profile_menu_open: false,
            is_model_menu_open: false,
            terminal_view_id,
            profile_mouse_state: Default::default(),
            model_mouse_state: Default::default(),
            menu_positioning_provider,
            is_blurred: false,
            new_model_popup,
            input_model,
            ambient_agent_view_model,
            render_compact: false,
            hovered_llm_info: None,
            manage_api_key_button,
            terminal_model,
            all_model_choices: Vec::new(),
        };
        me.refresh_state(ctx);
        me
    }

    pub fn set_profile_menu_visibility(&mut self, is_open: bool, ctx: &mut ViewContext<Self>) {
        if self.is_profile_menu_open == is_open {
            return;
        }

        self.is_profile_menu_open = is_open;
        self.is_model_menu_open = false;
        if is_open {
            ctx.focus(&self.profile_dropdown);
        }
        ctx.emit(ProfileModelSelectorEvent::MenuVisibilityChanged { open: is_open });
        ctx.notify();
    }

    pub fn set_model_menu_visibility(&mut self, is_open: bool, ctx: &mut ViewContext<Self>) {
        if self.is_model_menu_open == is_open {
            return;
        }
        self.is_model_menu_open = is_open;
        self.is_profile_menu_open = false;
        if is_open {
            LLMPreferences::handle(ctx).update(ctx, |preferences, _| {
                preferences.hide_llm_popup(self.terminal_view_id)
            });

            // Initialize hovered_llm_info to the currently selected model
            let selected_index = self
                .model_dropdown
                .read(ctx, |menu, _| menu.selected_index());
            self.set_hovered_llm_info(selected_index, ctx);

            log::info!("Focusing model menu");
            ctx.focus(&self.model_dropdown);
        }
        ctx.emit(ProfileModelSelectorEvent::MenuVisibilityChanged { open: is_open });
        ctx.notify();
    }

    pub fn is_open(&self) -> bool {
        self.is_profile_menu_open || self.is_model_menu_open
    }

    pub fn model_menu_item_position_id(&self, llm_id: &LLMId) -> String {
        format!("{PROFILE_SELECTOR_POSITION_ID}_{llm_id}")
    }

    /// Locked because the user is composing a follow-up to a Cloud Mode run
    /// that has ended. The server inherits the original task's model config
    /// when accepting the follow-up, so changing the model locally is
    /// meaningless.
    fn is_locked_for_cloud_followup(&self, app: &AppContext) -> bool {
        self.ambient_agent_view_model
            .as_ref()
            .is_some_and(|m| m.as_ref(app).is_ready_for_cloud_followup_prompt())
    }

    /// Locked because a non-Oz cloud run (e.g. Claude Code, Codex) has been
    /// spawned. The harness owns model selection, so changing the model
    /// locally has no effect on the run. We lock silently in this case
    /// because the harness selection itself communicates the lock.
    fn is_locked_for_non_oz_run(&self, app: &AppContext) -> bool {
        self.ambient_agent_view_model.as_ref().is_some_and(|m| {
            let model = m.as_ref(app);
            model.task_id().is_some()
                && !matches!(model.selected_harness(), Harness::Oz | Harness::Unknown)
        })
    }

    fn is_model_locked(&self, app: &AppContext) -> bool {
        self.is_locked_for_cloud_followup(app) || self.is_locked_for_non_oz_run(app)
    }

    /// True when a non-Oz harness is selected.
    fn is_third_party_harness(&self, app: &AppContext) -> bool {
        self.ambient_agent_view_model.as_ref().is_some_and(|m| {
            let model = m.as_ref(app);
            !matches!(model.selected_harness(), Harness::Oz | Harness::Unknown)
        })
    }

    fn refresh_state(&mut self, ctx: &mut ViewContext<Self>) {
        self.refresh_profile_menu(ctx);
        self.refresh_model_menu(ctx);

        let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
        if profiles_model.has_multiple_profiles() {
            let profile_name = {
                let active_profile =
                    profiles_model.active_profile(Some(self.terminal_view_id), ctx);
                active_profile.data().display_name()
            };

            self.profile_button.update(ctx, |button, ctx| {
                button.set_label(profile_name, ctx);
            });
        }

        let model_name = if self.is_third_party_harness(ctx) {
            self.harness_model_display_name(ctx)
        } else {
            let llm_preferences = LLMPreferences::as_ref(ctx);
            let active_llm = if FeatureFlag::InlineMenuHeaders.is_enabled()
                && self
                    .terminal_model
                    .lock()
                    .block_list()
                    .active_block()
                    .is_agent_in_control_or_tagged_in()
            {
                llm_preferences.get_active_cli_agent_model(ctx, Some(self.terminal_view_id))
            } else {
                llm_preferences.get_active_base_model(ctx, Some(self.terminal_view_id))
            };

            // Don't append description for custom model routers — it would add a
            // redundant "(Custom auto · Local)" suffix to the button label.
            if !is_custom_router_id(active_llm.id.as_str()) {
                if let Some(description) = &active_llm.description {
                    format!("{} ({})", active_llm.display_name, description)
                } else {
                    active_llm.display_name.clone()
                }
            } else {
                active_llm.display_name.clone()
            }
        };

        // Non-Oz runs lock silently: the harness owns model selection, and the
        // user already knows that, so no tooltip is shown.
        let model_tooltip: Option<&str> = if self.is_locked_for_cloud_followup(ctx) {
            Some(MODEL_LOCKED_FOR_FOLLOWUP_TOOLTIP)
        } else if self.is_locked_for_non_oz_run(ctx) {
            None
        } else {
            Some(MODEL_PICKER_TOOLTIP)
        };
        let locked = self.is_model_locked(ctx);
        self.model_button.update(ctx, |button, ctx| {
            button.set_label(model_name, ctx);
            button.set_disabled(locked, ctx);
            match model_tooltip {
                Some(t) => button.set_tooltip(Some(t), ctx),
                None => button.clear_tooltip(ctx),
            }
        });
        self.model_compact_button.update(ctx, |button, ctx| {
            button.set_disabled(locked, ctx);
            match model_tooltip {
                Some(t) => button.set_tooltip(Some(t), ctx),
                None => button.clear_tooltip(ctx),
            }
        });
        ctx.notify();
    }

    pub fn set_blurred(&mut self, is_blurred: bool, ctx: &mut ViewContext<Self>) {
        self.is_blurred = is_blurred;
        self.update_chip_themes(ctx);
        self.update_compact_button_themes(ctx);
    }

    pub fn set_render_compact(&mut self, render_compact: bool, ctx: &mut ViewContext<Self>) {
        if self.render_compact != render_compact {
            self.render_compact = render_compact;
            ctx.notify();
        }
    }

    fn handle_appearance_change(&mut self, ctx: &mut ViewContext<Self>) {
        self.update_chip_themes(ctx);
        self.update_compact_button_themes(ctx);
    }

    fn update_chip_themes(&self, ctx: &mut ViewContext<Self>) {
        let appearance = Appearance::as_ref(ctx);
        let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
        let has_multiple_profiles = profiles_model.has_multiple_profiles();

        let new_theme = SelectorChipTheme {
            text_color: ButtonTextColor::Fill(
                appearance
                    .theme()
                    .sub_text_color(appearance.theme().surface_1()),
            ),
            is_blurred: self.is_blurred,
        };
        let new_disabled_theme = SelectorChipTheme {
            text_color: ButtonTextColor::Fill(
                internal_colors::text_disabled(appearance.theme(), appearance.theme().surface_1())
                    .into(),
            ),
            is_blurred: self.is_blurred,
        };

        // Only update profile button if there are multiple profiles
        if has_multiple_profiles {
            self.profile_button.update(ctx, |button, ctx| {
                button.set_theme(new_theme.clone(), ctx);
                button.set_disabled_theme(new_disabled_theme.clone(), ctx);
                button.set_disabled(self.is_blurred, ctx);
            });
        }

        self.model_button.update(ctx, |button, ctx| {
            button.set_theme(new_theme.clone(), ctx);
            button.set_disabled_theme(new_disabled_theme.clone(), ctx);
            button.set_disabled(self.is_blurred, ctx);
        });
        ctx.notify();
    }

    fn update_compact_button_themes(&self, ctx: &mut ViewContext<Self>) {
        let theme = PromptIconButtonTheme::new(self.is_blurred);
        let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
        let has_multiple_profiles = profiles_model.has_multiple_profiles();

        if has_multiple_profiles {
            self.profile_compact_button.update(ctx, |button, ctx| {
                button.set_theme(theme.clone(), ctx);
            });
        }

        self.model_compact_button.update(ctx, |button, ctx| {
            button.set_theme(theme.clone(), ctx);
        });
        ctx.notify();
    }

    fn refresh_profile_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let profiles_model = AIExecutionProfilesModel::as_ref(ctx);
        let all_profile_ids = profiles_model.get_all_profile_ids();
        let active_profile = profiles_model.active_profile(Some(self.terminal_view_id), ctx);

        let appearance = Appearance::as_ref(ctx);
        let mut menu_items = vec![
            MenuItem::Header {
                fields: MenuItemFields::new("Profiles").with_override_text_color(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().background())
                        .into_solid(),
                ),
                clickable: false,
                right_side_fields: None,
            },
            MenuItem::Separator,
        ];

        for profile_id in all_profile_ids {
            if let Some(profile_info) = profiles_model.get_profile_by_id(profile_id, ctx) {
                let profile = profile_info.data();
                let is_active = *active_profile.id() == profile_id;

                let mut fields = MenuItemFields::new(profile.display_name());
                if is_active {
                    fields = fields.with_icon(Icon::Check);
                } else {
                    fields = fields.with_indent();
                }
                menu_items.push(MenuItem::Item(fields.with_on_select_action(
                    ProfileModelSelectorAction::SelectProfile(profile_id),
                )));
            }
        }

        menu_items.push(MenuItem::Separator);
        menu_items.push(MenuItem::Item(
            MenuItemFields::new("Manage profiles")
                .with_icon(Icon::Gear)
                .with_on_select_action(ProfileModelSelectorAction::ManageProfiles),
        ));

        self.profile_dropdown.update(ctx, |menu, ctx| {
            menu.set_items(menu_items, ctx);
            let active_action = ProfileModelSelectorAction::SelectProfile(*active_profile.id());
            menu.set_selected_by_action(&active_action, ctx);
        });
    }

    // Checks that we have a harness in the `AmbientAgentViewModel` and returns model options from
    // the `HarnessAvailabilityModel` for that harness.
    fn active_harness_model_info<'a>(&self, app: &'a AppContext) -> Option<&'a HarnessModelInfo> {
        let ambient_model = self.ambient_agent_view_model.as_ref()?.as_ref(app);
        let harness = ambient_model.selected_harness();
        let model_id = ambient_model.selected_harness_model_id()?;
        let reasoning_level = ambient_model.selected_harness_reasoning_level();
        HarnessAvailabilityModel::as_ref(app)
            .models_for(harness)?
            .iter()
            .find(|m| m.id == model_id && m.reasoning_level.as_deref() == reasoning_level)
    }

    fn harness_model_display_name(&self, app: &AppContext) -> String {
        self.active_harness_model_info(app)
            .map(|info| info.display_name.clone())
            .unwrap_or_else(|| HARNESS_DEFAULT_MODEL_LABEL.to_string())
    }

    fn refresh_harness_model_menu(&mut self, ctx: &mut ViewContext<Self>) {
        let ambient_model = match self.ambient_agent_view_model.as_ref() {
            Some(m) => m,
            None => return,
        };
        let harness = ambient_model.as_ref(ctx).selected_harness();
        let selected_model_id = ambient_model
            .as_ref(ctx)
            .selected_harness_model_id()
            .map(str::to_owned);
        let selected_reasoning = ambient_model
            .as_ref(ctx)
            .selected_harness_reasoning_level()
            .map(str::to_owned);

        let models = HarnessAvailabilityModel::as_ref(ctx).models_for(harness);

        let mut items: Vec<MenuItem<ProfileModelSelectorAction>> = Vec::new();

        let default_selected = selected_model_id.is_none();
        let default_action = ProfileModelSelectorAction::SelectHarnessModel {
            model_id: String::new(),
            reasoning_level: None,
        };
        let mut default_fields =
            MenuItemFields::new(HARNESS_DEFAULT_MODEL_LABEL).with_on_select_action(default_action);
        if default_selected {
            default_fields = default_fields.with_icon(Icon::Check);
        } else {
            default_fields = default_fields.with_indent();
        }
        items.push(MenuItem::Item(default_fields));

        if let Some(models) = models {
            for model in models {
                let is_selected = selected_model_id.as_deref() == Some(&model.id)
                    && selected_reasoning.as_deref() == model.reasoning_level.as_deref();
                let mut fields = MenuItemFields::new(model.display_name.clone())
                    .with_on_select_action(ProfileModelSelectorAction::SelectHarnessModel {
                        model_id: model.id.clone(),
                        reasoning_level: model.reasoning_level.clone(),
                    });
                if is_selected {
                    fields = fields.with_icon(Icon::Check);
                } else {
                    fields = fields.with_indent();
                }
                items.push(MenuItem::Item(fields));
            }
        }

        let selected_index = items
            .iter()
            .position(|item| {
                matches!(
                    item.item_on_select_action(),
                    Some(ProfileModelSelectorAction::SelectHarnessModel { model_id, reasoning_level })
                        if (model_id.is_empty() && default_selected)
                            || (selected_model_id.as_deref() == Some(model_id.as_str())
                                && selected_reasoning.as_deref() == reasoning_level.as_deref())
                )
            })
            .unwrap_or(0);

        self.model_dropdown.update(ctx, |menu, ctx| {
            menu.set_width(MENU_WIDTH);
            menu.set_items(items, ctx);
            menu.set_selected_by_index(selected_index, ctx);
            ctx.notify();
        });
    }

    fn refresh_model_menu(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_third_party_harness(ctx) {
            self.refresh_harness_model_menu(ctx);
            return;
        }

        let llm_preferences = LLMPreferences::as_ref(ctx);

        let active_llm = llm_preferences.get_active_base_model(ctx, Some(self.terminal_view_id));

        let active_profile =
            AIExecutionProfilesModel::as_ref(ctx).active_profile(Some(self.terminal_view_id), ctx);

        let profile_base_model_id = active_profile
            .data()
            .base_model
            .clone()
            .and_then(|id| {
                llm_preferences
                    .get_llm_info(&id)
                    .map(|info| info.id.clone())
            })
            .unwrap_or_else(|| llm_preferences.get_default_base_model().id.clone());

        let model_id_to_add_profile_default_label_to = Some(&profile_base_model_id);

        // Store all model choices for reasoning variant lookups
        self.all_model_choices = llm_preferences
            .get_base_llm_choices_for_agent_mode(ctx)
            .cloned()
            .collect();

        // Partition into server-provided choices (subject to auto/reasoning collapsing) and
        // custom-endpoint choices (rendered separately under a `Custom models` sub-header so
        // the server-curated list stays visually distinct).
        let custom_ids: std::collections::HashSet<LLMId> = llm_preferences
            .custom_llm_choices(ctx)
            .map(|info| info.id.clone())
            .collect();
        let server_choices: Vec<&LLMInfo> = self
            .all_model_choices
            .iter()
            .filter(|llm| !custom_ids.contains(&llm.id))
            .collect();
        let custom_choices: Vec<&LLMInfo> = self
            .all_model_choices
            .iter()
            .filter(|llm| custom_ids.contains(&llm.id))
            .collect();

        // Group models by base_model_name to collapse reasoning variants.
        // Use "auto" as the key for all auto models so they collapse together.
        // Only group models that have reasoning levels - others stay separate.
        let mut groups: IndexMap<String, Vec<&LLMInfo>> = IndexMap::new();
        for llm in &server_choices {
            let key = if is_auto(llm) {
                "auto".to_string()
            } else if llm.has_reasoning_level() {
                llm.base_model_name().to_string()
            } else {
                llm.id.to_string()
            };
            groups.entry(key).or_default().push(*llm);
        }

        // Split collapsed choices so custom models can be placed right after auto models.
        let mut auto_choices: Vec<&LLMInfo> = Vec::new();
        let mut other_choices: Vec<&LLMInfo> = Vec::new();
        for (_, variants) in groups {
            if let Some(first) = variants.into_iter().next() {
                if is_auto(first) {
                    auto_choices.push(first);
                } else {
                    other_choices.push(first);
                }
            }
        }

        let mut items = available_model_menu_items(
            auto_choices,
            |llm| {
                let all_refs: Vec<_> = self.all_model_choices.iter().collect();
                if is_auto(llm) {
                    ProfileModelSelectorAction::SelectAutoModel
                } else if has_reasoning_variants(llm, &all_refs) {
                    ProfileModelSelectorAction::SelectReasoningModel(
                        llm.base_model_name().to_string(),
                    )
                } else {
                    ProfileModelSelectorAction::SelectModel(llm.id.clone())
                }
            },
            model_id_to_add_profile_default_label_to,
            Some(&|llm_id| self.model_menu_item_position_id(llm_id)),
            true,
            true,
            ctx,
        );

        // Append the "Custom models" section when the user has any custom endpoints configured.
        // Each row gets its own atomic `SelectModel(config_key)` action; no auto/reasoning
        // collapsing applies.
        if !custom_choices.is_empty() {
            let appearance = Appearance::as_ref(ctx);
            if !items.is_empty() {
                items.push(MenuItem::Separator);
            }
            items.push(MenuItem::Header {
                fields: MenuItemFields::new("Custom models").with_override_text_color(
                    appearance
                        .theme()
                        .sub_text_color(appearance.theme().background())
                        .into_solid(),
                ),
                clickable: false,
                right_side_fields: None,
            });
            for llm in &custom_choices {
                let fields = MenuItemFields::new(llm.menu_display_name())
                    .with_right_side_icon(Icon::Key)
                    .with_on_select_action(ProfileModelSelectorAction::SelectModel(llm.id.clone()));
                items.push(MenuItem::Item(fields));
            }
        }

        if !other_choices.is_empty() {
            if !items.is_empty() {
                items.push(MenuItem::Separator);
            }
            items.extend(available_model_menu_items(
                other_choices,
                |llm| {
                    let all_refs: Vec<_> = self.all_model_choices.iter().collect();
                    if is_auto(llm) {
                        ProfileModelSelectorAction::SelectAutoModel
                    } else if has_reasoning_variants(llm, &all_refs) {
                        ProfileModelSelectorAction::SelectReasoningModel(
                            llm.base_model_name().to_string(),
                        )
                    } else {
                        ProfileModelSelectorAction::SelectModel(llm.id.clone())
                    }
                },
                model_id_to_add_profile_default_label_to,
                Some(&|llm_id| self.model_menu_item_position_id(llm_id)),
                true,
                true,
                ctx,
            ));
        }

        let selected_index = Self::find_selected_index(&items, active_llm);
        self.model_dropdown.update(ctx, |menu, ctx| {
            menu.set_width(MENU_WIDTH);
            menu.set_items(items, ctx);
            menu.set_selected_by_index(selected_index, ctx);
            ctx.notify();
        });
        self.set_hovered_llm_info(Some(selected_index), ctx);
    }

    fn refresh_model_spec_sidecar(
        &mut self,
        kind: &ModelSpecSidecarKind,
        ctx: &mut ViewContext<Self>,
    ) {
        let llm_preferences = LLMPreferences::as_ref(ctx);
        let active_llm = llm_preferences.get_active_base_model(ctx, Some(self.terminal_view_id));
        let active_llm_id = active_llm.id.clone();

        let items: Vec<MenuItem<ProfileModelSelectorAction>> = match kind {
            ModelSpecSidecarKind::Auto => llm_preferences
                .get_base_llm_choices_for_agent_mode(ctx)
                .filter(|llm| is_auto(llm))
                .map(|llm| {
                    let is_selected = llm.id == active_llm_id;

                    let label = if llm.display_name.starts_with("auto (") {
                        // Auto display names are formatted like "auto (<sub-variant>)"
                        // We extract the sub-variant and capitalize it for use in the sidecar menu.
                        let trimmed = llm
                            .display_name
                            .trim_start_matches("auto (")
                            .trim_end_matches(")");

                        // Capitalize the first letter of the auto sub-variant.
                        let mut chars = trimmed.chars();
                        chars
                            .next()
                            .map(|first| first.to_uppercase().chain(chars).collect())
                            .unwrap_or_default()
                    } else {
                        llm.display_name.clone()
                    };

                    Self::make_sidecar_item(label, &llm.id, is_selected)
                })
                .collect(),
            ModelSpecSidecarKind::Reasoning => {
                // For Reasoning without a base_name, return empty (use refresh_model_spec_sidecar_for_model instead)
                Vec::new()
            }
        };

        let selected_index = Self::find_sidecar_selected_index(&items, &active_llm_id);
        self.model_spec_sidecar.active_kind = Some(kind.clone());
        self.model_spec_sidecar.dropdown.update(ctx, |menu, ctx| {
            menu.set_width(MENU_WIDTH);
            menu.set_items(items, ctx);
            menu.set_selected_by_index(selected_index, ctx);
            ctx.notify();
        });
    }

    fn refresh_model_spec_sidecar_for_model(
        &mut self,
        base_name: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        let llm_preferences = LLMPreferences::as_ref(ctx);
        let active_llm = llm_preferences.get_active_base_model(ctx, Some(self.terminal_view_id));
        let active_llm_id = active_llm.id.clone();

        let items: Vec<MenuItem<ProfileModelSelectorAction>> = self
            .all_model_choices
            .iter()
            .filter(|llm| llm.base_model_name() == base_name && llm.has_reasoning_level())
            .map(|llm| {
                let is_selected = llm.id == active_llm_id;
                let label = llm.reasoning_level().unwrap_or_default();
                Self::make_sidecar_item(label, &llm.id, is_selected)
            })
            .collect();

        let selected_index = Self::find_sidecar_selected_index(&items, &active_llm_id);
        self.model_spec_sidecar.active_kind = Some(ModelSpecSidecarKind::Reasoning);
        self.model_spec_sidecar.dropdown.update(ctx, |menu, ctx| {
            menu.set_width(MENU_WIDTH);
            menu.set_items(items, ctx);
            menu.set_selected_by_index(selected_index, ctx);
            ctx.notify();
        });

        self.set_sidecar_hovered_info(Some(selected_index), ctx);
    }

    fn make_sidecar_item(
        label: String,
        llm_id: &LLMId,
        is_selected: bool,
    ) -> MenuItem<ProfileModelSelectorAction> {
        let mut fields = MenuItemFields::new(label)
            .with_font_size_override(14.)
            .with_on_select_action(ProfileModelSelectorAction::SelectModel(llm_id.clone()));

        if is_selected {
            fields = fields.with_icon(Icon::Check);
        } else {
            fields = fields.with_indent();
        }

        fields.into_item()
    }

    fn find_sidecar_selected_index(
        items: &[MenuItem<ProfileModelSelectorAction>],
        active_llm_id: &LLMId,
    ) -> usize {
        items
            .iter()
            .position(|item| {
                if let MenuItem::Item(fields) = item {
                    let item_model_id = item
                        .item_on_select_action()
                        .and_then(|action| action.selected_model_id());
                    !fields.is_disabled() && item_model_id.as_ref() == Some(active_llm_id)
                } else {
                    false
                }
            })
            .unwrap_or(0)
    }

    fn handle_sidecar_selection(&mut self, ctx: &mut ViewContext<Self>) {
        let index = self
            .model_spec_sidecar
            .dropdown
            .read(ctx, |menu, _| menu.selected_index())
            .unwrap_or(0);
        if let Some(llm) = self.get_selected_llm_info(MenuType::Sidecar, index, ctx) {
            log::info!(
                "Selecting base agent model {} (from model selector)",
                &llm.id
            );
            LLMPreferences::handle(ctx).update(ctx, |preferences, ctx| {
                preferences.update_preferred_agent_mode_llm(&llm.id, self.terminal_view_id, ctx);
            });
        }
        self.set_model_menu_visibility(false, ctx);
    }

    fn find_selected_index(
        items: &[MenuItem<ProfileModelSelectorAction>],
        active_llm: &LLMInfo,
    ) -> usize {
        items
            .iter()
            .position(|item| {
                if let MenuItem::Item(fields) = item {
                    let is_disabled = fields.is_disabled();
                    let is_active = if is_auto(active_llm) {
                        matches!(
                            item.item_on_select_action(),
                            Some(ProfileModelSelectorAction::SelectAutoModel)
                        )
                    } else if active_llm.has_reasoning_level() {
                        // For models with reasoning levels, match by base_model_name
                        matches!(
                            item.item_on_select_action(),
                            Some(ProfileModelSelectorAction::SelectReasoningModel(name)) if *name == active_llm.base_model_name()
                        )
                    } else {
                        let item_model_id = item
                            .item_on_select_action()
                            .and_then(|action| action.selected_model_id());
                        item_model_id.map(|id| id == active_llm.id).unwrap_or(false)
                    };

                    !is_disabled && is_active
                } else {
                    false
                }
            })
            .or_else(|| {
                items.iter().position(|item| {
                    if let MenuItem::Item(fields) = item {
                        !fields.is_disabled()
                    } else {
                        false
                    }
                })
            })
            .unwrap_or(0)
    }

    // Gets the LLMInfo of the selected model in the given menu at the given index.
    fn get_selected_llm_info(
        &self,
        menu_type: MenuType,
        index: usize,
        ctx: &mut ViewContext<Self>,
    ) -> Option<LLMInfo> {
        let model_dropdown = match &menu_type {
            MenuType::Main => &self.model_dropdown,
            MenuType::Sidecar => &self.model_spec_sidecar.dropdown,
        };
        model_dropdown.read(ctx, |menu, _| {
            menu.items()
                .get(index)
                .and_then(|item| item.item_on_select_action())
                .and_then(|action| {
                    match action {
                        ProfileModelSelectorAction::SelectModel(llm_id) => {
                            LLMPreferences::as_ref(ctx).get_llm_info(llm_id).cloned()
                        }
                        ProfileModelSelectorAction::SelectAutoModel => {
                            // Get the first "auto" variant as the generic auto model
                            let llm_prefs = LLMPreferences::as_ref(ctx);
                            llm_prefs
                                .get_base_llm_choices_for_agent_mode(ctx)
                                .find(|llm| is_auto(llm))
                                .cloned()
                        }
                        ProfileModelSelectorAction::SelectReasoningModel(base_name) => {
                            // Get the first reasoning variant for this base model
                            self.all_model_choices
                                .iter()
                                .find(|llm| {
                                    llm.base_model_name() == base_name && llm.has_reasoning_level()
                                })
                                .cloned()
                        }
                        _ => None,
                    }
                })
        })
    }

    fn set_hovered_llm_info(&mut self, index: Option<usize>, ctx: &mut ViewContext<Self>) {
        let Some(index) = index else {
            return;
        };
        let llm_info = self.get_selected_llm_info(MenuType::Main, index, ctx);
        self.hovered_llm_info = llm_info.clone();

        let shows_sidecar = llm_info
            .as_ref()
            .is_some_and(|info| is_auto(info) || self.has_multiple_reasoning_variants(info));
        let shows_side_panel =
            shows_sidecar || llm_info.as_ref().is_some_and(|info| info.spec.is_some());

        if shows_sidecar {
            // Read the sidecar rect from last frame to update the safe zone target
            let window_id = self.model_dropdown.window_id(ctx);
            let sidecar_rect =
                ctx.element_position_by_id_at_last_frame(window_id, SIDECAR_POSITION_ID);
            self.model_dropdown.update(ctx, |menu, ctx| {
                menu.set_safe_zone_target(sidecar_rect);
                menu.set_submenu_being_shown_for_item_index(Some(index));
                ctx.notify();
            });
        } else {
            self.model_dropdown.update(ctx, |menu, ctx| {
                menu.set_safe_zone_target(None);
                menu.set_submenu_being_shown_for_item_index(if shows_side_panel {
                    Some(index)
                } else {
                    None
                });
                ctx.notify();
            });
        }

        if let Some(info) = &llm_info {
            if is_auto(info) {
                // If hovering auto, refresh sidecar with auto variants and set hovered_info
                self.refresh_model_spec_sidecar(&ModelSpecSidecarKind::Auto, ctx);
                let auto_index = self
                    .model_spec_sidecar
                    .dropdown
                    .read(ctx, |menu, _| menu.selected_index());
                self.set_sidecar_hovered_info(auto_index, ctx);
            } else if self.has_multiple_reasoning_variants(info) {
                // If hovering a model with multiple reasoning variants, refresh reasoning menu
                self.refresh_model_spec_sidecar_for_model(info.base_model_name(), ctx);
            }
        }
    }

    fn set_sidecar_hovered_info(&mut self, index: Option<usize>, ctx: &mut ViewContext<Self>) {
        let index = index.unwrap_or(0);
        self.model_spec_sidecar.hovered_info =
            self.get_selected_llm_info(MenuType::Sidecar, index, ctx);
    }

    fn has_multiple_reasoning_variants(&self, llm: &LLMInfo) -> bool {
        let all_refs: Vec<_> = self.all_model_choices.iter().collect();
        has_reasoning_variants(llm, &all_refs)
    }

    fn get_padding_values(&self, scaled_font_size: f32) -> (f32, f32) {
        if FeatureFlag::AgentView.is_enabled() {
            (
                spacing::UDI_CHIP_VERTICAL_PADDING,
                spacing::UDI_CHIP_HORIZONTAL_PADDING,
            )
        } else {
            let horizontal_padding =
                (scaled_font_size * HORIZONTAL_PADDING_SCALE).max(MIN_HORIZONTAL_PADDING);
            (VERTICAL_PADDING, horizontal_padding)
        }
    }

    fn get_menu_positioning(&self, app: &AppContext, is_profile: bool) -> OffsetPositioning {
        match self.menu_positioning_provider.menu_position(app) {
            MenuPositioning::BelowInputBox => {
                if self.render_compact {
                    if is_profile {
                        OffsetPositioning::offset_from_save_position_element(
                            "profile_model_selector_profile_compact_button",
                            vec2f(0., 4.),
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::BottomLeft,
                            ChildAnchor::TopLeft,
                        )
                    } else {
                        OffsetPositioning::offset_from_save_position_element(
                            "profile_model_selector_model_compact_button",
                            vec2f(0., 4.),
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::BottomLeft,
                            ChildAnchor::TopLeft,
                        )
                    }
                } else {
                    // In full mode, use the original positioning logic
                    if is_profile {
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., 4.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::BottomLeft,
                            ChildAnchor::TopLeft,
                        )
                    } else {
                        OffsetPositioning::offset_from_save_position_element(
                            "profile_model_selector_model_button",
                            vec2f(0., 4.),
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::BottomLeft,
                            ChildAnchor::TopLeft,
                        )
                    }
                }
            }
            MenuPositioning::AboveInputBox => {
                if self.render_compact {
                    if is_profile {
                        OffsetPositioning::offset_from_save_position_element(
                            "profile_model_selector_profile_compact_button",
                            vec2f(0., -4.),
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::TopLeft,
                            ChildAnchor::BottomLeft,
                        )
                    } else {
                        OffsetPositioning::offset_from_save_position_element(
                            "profile_model_selector_model_compact_button",
                            vec2f(0., -4.),
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::TopLeft,
                            ChildAnchor::BottomLeft,
                        )
                    }
                } else if is_profile {
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -4.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    )
                } else {
                    OffsetPositioning::offset_from_save_position_element(
                        "profile_model_selector_model_button",
                        vec2f(0., -4.),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    )
                }
            }
        }
    }

    fn should_render_model_sidecar_left(&self, position_id: &str, app: &AppContext) -> bool {
        // When AgentView is enabled, the model picker is right-aligned, so we default to
        // showing the sidecar on the left side to avoid overlap.
        let default_to_left = FeatureFlag::AgentView.is_enabled();

        let window_id = self.model_dropdown.window_id(app);
        let Some(window) = app.windows().platform_window(window_id) else {
            return default_to_left;
        };

        // If we don't have the anchor position cached yet, use the default based on AgentView.
        let Some(anchor_rect) = app.element_position_by_id_at_last_frame(window_id, position_id)
        else {
            return default_to_left;
        };

        // Sidecar is positioned center-to-center off the hovered menu item's position.
        // Check both sides for overflow and pick the side that fits.
        let anchor_center_x = (anchor_rect.min_x() + anchor_rect.max_x()) / 2.0;
        let sidecar_half_width = MENU_WIDTH / 2.0;

        // Calculate where the sidecar edges would be on each side
        let sidecar_left_edge_if_on_left =
            anchor_center_x - (MENU_WIDTH + SIDECAR_HORIZONTAL_GAP) - sidecar_half_width;
        let sidecar_right_edge_if_on_right =
            anchor_center_x + (MENU_WIDTH + SIDECAR_HORIZONTAL_GAP) + sidecar_half_width;

        let would_overflow_left = sidecar_left_edge_if_on_left < 0.0;
        let would_overflow_right = sidecar_right_edge_if_on_right >= window.size().x();

        // If both sides overflow, prefer the default based on AgentView state.
        // If only one side fits, use that side.
        // If neither overflows, use the default based on AgentView state.
        match (would_overflow_left, would_overflow_right) {
            (true, false) => false, // Only right fits, show on right
            (false, true) => true,  // Only left fits, show on left
            _ => default_to_left,   // Both fit or both overflow, use default
        }
    }

    fn render_profile_section(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let profiles_model = AIExecutionProfilesModel::as_ref(app);
        let active_profile = profiles_model.active_profile(Some(self.terminal_view_id), app);

        let text_color = if self.is_blurred {
            theme.disabled_text_color(theme.surface_1()).into()
        } else {
            theme.sub_text_color(theme.surface_1()).into()
        };

        let scaled_font_size = calculate_scaled_font_size(appearance);
        // Use the same icon size as the compact UDI button to ensure consistent height
        let icon_size = if FeatureFlag::AgentView.is_enabled() {
            udi_icon_size(appearance, app)
        } else {
            appearance.monospace_font_size() - 1.0
        };
        let (vertical_padding, horizontal_padding) = self.get_padding_values(scaled_font_size);

        let profile_icon = Icon::Psychology
            .to_warpui_icon(Fill::Solid(text_color))
            .finish();

        let max_label_width = calculate_max_profile_name_width(appearance);
        let profile_text = ConstrainedBox::new(
            Text::new_inline(
                active_profile.data().display_name(),
                appearance.ui_font_family(),
                scaled_font_size,
            )
            .with_color(text_color)
            .with_line_height_ratio(appearance.line_height_ratio())
            .with_clip(ClipConfig::end())
            .finish(),
        )
        .with_max_width(max_label_width)
        .finish();

        let content = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    ConstrainedBox::new(profile_icon)
                        .with_height(icon_size)
                        .with_width(icon_size)
                        .finish(),
                )
                .with_margin_right(ICON_SPACING)
                .finish(),
            )
            .with_child(profile_text)
            .finish();

        let button = Container::new(content)
            .with_vertical_padding(vertical_padding)
            .with_horizontal_padding(horizontal_padding)
            .finish();

        Hoverable::new(self.profile_mouse_state.clone(), move |state| {
            if state.is_hovered() {
                let button_with_hover = Container::new(button)
                    .with_background(theme.surface_2())
                    .with_corner_radius(CornerRadius::with_left(Radius::Pixels(
                        INNER_CORNER_RADIUS,
                    )))
                    .finish();

                let tooltip = appearance
                    .ui_builder()
                    .tool_tip(PROFILE_PICKER_TOOLTIP.to_owned());
                let mut stack = Stack::new();
                stack.add_child(button_with_hover);
                stack.add_positioned_overlay_child(
                    tooltip.build().finish(),
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -10.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    ),
                );
                stack.finish()
            } else {
                button
            }
        })
        .on_click(|ctx, _app, _position| {
            ctx.dispatch_typed_action(ProfileModelSelectorAction::ToggleProfileMenu);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    fn render_model_section(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let llm_preferences = LLMPreferences::as_ref(app);

        // Allow editing if composing an ambient agent query, or if the user has edit access
        // in a shared session (i.e., not a viewer, or is an executor).
        let is_composing_ambient_agent =
            self.ambient_agent_view_model
                .as_ref()
                .is_some_and(|ambient_agent_model| {
                    ambient_agent_model
                        .as_ref(app)
                        .is_configuring_ambient_agent()
                });
        let terminal_model = self.terminal_model.lock();
        let has_edit_access = is_composing_ambient_agent
            || !terminal_model.shared_session_status().is_viewer()
            || terminal_model.shared_session_status().is_executor();
        let is_lrc = FeatureFlag::InlineMenuHeaders.is_enabled()
            && terminal_model
                .block_list()
                .active_block()
                .is_agent_in_control_or_tagged_in();
        drop(terminal_model);

        let model_display_name = if self.is_third_party_harness(app) {
            self.harness_model_display_name(app)
        } else if is_lrc {
            llm_preferences
                .get_active_cli_agent_model(app, Some(self.terminal_view_id))
                .menu_display_name()
        } else {
            llm_preferences
                .get_active_base_model(app, Some(self.terminal_view_id))
                .menu_display_name()
        };

        let text_color = if self.is_blurred {
            theme.disabled_text_color(theme.surface_1()).into()
        } else {
            theme.sub_text_color(theme.surface_1()).into()
        };

        let scaled_font_size = calculate_scaled_font_size(appearance);
        let icon_size = if FeatureFlag::AgentView.is_enabled() {
            udi_icon_size(appearance, app)
        } else {
            appearance.monospace_font_size() - 1.0
        };
        let (vertical_padding, horizontal_padding) = self.get_padding_values(scaled_font_size);

        let model_text = Text::new_inline(
            model_display_name,
            appearance.ui_font_family(),
            scaled_font_size,
        )
        .with_color(text_color)
        .with_line_height_ratio(appearance.line_height_ratio())
        .finish();

        let mut content = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        if is_lrc {
            let terminal_icon = Icon::Terminal
                .to_warpui_icon(Fill::Solid(text_color))
                .finish();
            content = content.with_child(
                Container::new(
                    ConstrainedBox::new(terminal_icon)
                        .with_height(icon_size)
                        .with_width(icon_size)
                        .finish(),
                )
                .with_margin_right(ICON_SPACING)
                .finish(),
            );
        }

        content = content.with_child(model_text);

        // Only show chevron icon if the user can click to open the menu (i.e. has edit access)
        // and the InlineMenuHeaders feature flag is not enabled
        // (when enabled, clicking opens the inline model selector instead of a dropdown).
        if has_edit_access && !FeatureFlag::InlineMenuHeaders.is_enabled() {
            let chevron_icon = Icon::ChevronDown
                .to_warpui_icon(Fill::Solid(text_color))
                .finish();

            content = content.with_child(
                Container::new(
                    ConstrainedBox::new(chevron_icon)
                        .with_height(icon_size)
                        .with_width(icon_size)
                        .finish(),
                )
                .with_margin_left(ICON_SPACING)
                .finish(),
            );
        }

        let button = Container::new(content.finish())
            .with_vertical_padding(vertical_padding)
            .with_horizontal_padding(horizontal_padding)
            .finish();

        let button_with_save_position =
            SavePosition::new(button, "profile_model_selector_model_button").finish();

        let is_locked_for_followup = self.is_locked_for_cloud_followup(app);
        let is_locked_for_non_oz = self.is_locked_for_non_oz_run(app);
        let is_locked = is_locked_for_followup || is_locked_for_non_oz;
        let can_interact = has_edit_access && !is_locked;

        let hoverable = Hoverable::new(self.model_mouse_state.clone(), move |state| {
            if state.is_hovered() && can_interact {
                let button_with_hover = Container::new(button_with_save_position)
                    .with_background(theme.surface_2())
                    .with_corner_radius(CornerRadius::with_right(Radius::Pixels(
                        INNER_CORNER_RADIUS,
                    )))
                    .finish();

                let tooltip = appearance
                    .ui_builder()
                    .tool_tip(MODEL_PICKER_TOOLTIP.to_owned());
                let mut stack = Stack::new();
                stack.add_child(button_with_hover);
                stack.add_positioned_overlay_child(
                    tooltip.build().finish(),
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -10.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopLeft,
                        ChildAnchor::BottomLeft,
                    ),
                );
                stack.finish()
            } else if state.is_hovered() {
                // Non-Oz runs lock silently — skip the tooltip entirely.
                let tooltip_text: Option<&str> = if is_locked_for_followup {
                    Some(MODEL_LOCKED_FOR_FOLLOWUP_TOOLTIP)
                } else if is_locked_for_non_oz {
                    None
                } else {
                    Some(MODEL_REQUIRES_EDIT_ACCESS_TOOLTIP)
                };

                if let Some(text) = tooltip_text {
                    let tooltip = appearance.ui_builder().tool_tip(text.to_owned());
                    let mut stack = Stack::new();
                    stack.add_child(button_with_save_position);
                    stack.add_positioned_overlay_child(
                        tooltip.build().finish(),
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., -10.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::TopLeft,
                            ChildAnchor::BottomLeft,
                        ),
                    );
                    stack.finish()
                } else {
                    button_with_save_position
                }
            } else {
                button_with_save_position
            }
        });

        if can_interact {
            hoverable
                .on_click(|ctx, _app, _position| {
                    ctx.dispatch_typed_action(ProfileModelSelectorAction::ToggleModelMenu);
                })
                .with_cursor(Cursor::PointingHand)
                .finish()
        } else {
            hoverable.finish()
        }
    }

    fn render_separator(&self, app: &AppContext, visible: bool) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let separator_height = app.font_cache().line_height(
            appearance.monospace_font_size(),
            DEFAULT_UI_LINE_HEIGHT_RATIO / 1.4,
        );

        let container = Container::new(
            ConstrainedBox::new(Empty::new().finish())
                .with_width(SEPARATOR_WIDTH)
                .with_height(separator_height)
                .finish(),
        );

        if visible {
            container
                .with_background(Fill::Solid(internal_colors::neutral_3(theme)))
                .finish()
        } else {
            // Invisible separator that maintains width to prevent flickering
            container.finish()
        }
    }

    fn render_model_spec_header(
        &self,
        title: String,
        description: String,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let title = Text::new(title, appearance.ui_font_family(), 14.)
            .with_color(internal_colors::neutral_7(theme))
            .finish();

        let description = Text::new(
            description,
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(theme.disabled_ui_text_color().into())
        .finish();

        Container::new(
            Flex::column()
                .with_child(title)
                .with_child(Container::new(description).with_margin_top(4.).finish())
                .finish(),
        )
        .with_horizontal_margin(16.)
        .with_vertical_padding(16.)
        .with_border(
            Border::bottom(BORDER_WIDTH).with_border_color(internal_colors::neutral_3(theme)),
        )
        .finish()
    }

    fn render_model_spec_value_label(&self, name: String, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        Container::new(
            ConstrainedBox::new(
                Text::new(name, appearance.ui_font_family(), 14.)
                    .with_color(internal_colors::neutral_7(theme))
                    .finish(),
            )
            .with_width(72.)
            .finish(),
        )
        .with_margin_right(8.)
        .finish()
    }

    // Renders a single model spec value, including the label and the progress bar
    fn render_model_spec_value(
        &self,
        name: String,
        value: f32,
        bg_bar_color: ColorU,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let background_bar = Rect::new()
            .with_background_color(bg_bar_color)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)))
            .finish();
        let filled_bar = Rect::new()
            .with_background_color(internal_colors::neutral_6(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)))
            .finish();

        Container::new(
            Flex::row()
                .with_child(self.render_model_spec_value_label(name, app))
                .with_child(
                    Expanded::new(
                        1.,
                        ConstrainedBox::new(
                            Stack::new()
                                .with_child(background_bar)
                                .with_child(Percentage::width(value, filled_bar).finish())
                                .finish(),
                        )
                        .with_height(16.)
                        .finish(),
                    )
                    .finish(),
                )
                .finish(),
        )
        .with_margin_top(12.)
        .finish()
    }

    fn render_model_spec_api_key(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(self.render_model_spec_value_label("Cost".to_string(), app))
                .with_child(
                    Expanded::new(
                        1.,
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(
                                Container::new(
                                    Text::new(
                                        "Billed to API".to_string(),
                                        appearance.ui_font_family(),
                                        14.,
                                    )
                                    .with_color(theme.disabled_ui_text_color().into())
                                    .finish(),
                                )
                                .finish(),
                            )
                            .with_child(ChildView::new(&self.manage_api_key_button).finish())
                            .finish(),
                    )
                    .finish(),
                )
                .finish(),
        )
        .with_margin_top(12.)
        .finish()
    }

    // Renders all model spec values for a given model spec
    fn render_all_model_spec_values(
        &self,
        spec: &LLMSpec,
        is_using_api_key: bool,
        bg_bar_color: ColorU,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut spec_values = vec![
            self.render_model_spec_value(
                "Intelligence".to_string(),
                spec.quality,
                bg_bar_color,
                app,
            ),
            self.render_model_spec_value("Speed".to_string(), spec.speed, bg_bar_color, app),
        ];
        if is_using_api_key {
            spec_values.push(self.render_model_spec_api_key(app));
        } else {
            spec_values.push(self.render_model_spec_value(
                "Cost".to_string(),
                spec.cost,
                bg_bar_color,
                app,
            ));
        }
        Flex::column().with_children(spec_values).finish()
    }

    // Renders entire modal for a given model spec
    fn render_model_spec(
        &self,
        spec: &LLMSpec,
        is_using_api_key: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let header = self.render_model_spec_header(
            "Model Specs".to_string(),
            "Warp’s benchmarks for how well a model performs in our harness, the rate at which it consumes credits, and task speed.".to_string(),
            app,
        );
        let spec = self.render_all_model_spec_values(
            spec,
            is_using_api_key,
            internal_colors::neutral_3(theme),
            app,
        );

        ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_child(header)
                    .with_child(Container::new(spec).with_horizontal_padding(16.).finish())
                    .finish(),
            )
            .with_padding_bottom(12.)
            .with_vertical_margin(8.)
            .with_background_color(internal_colors::neutral_2(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)))
            .with_drop_shadow(DropShadow::default())
            .finish(),
        )
        .with_width(MENU_WIDTH)
        .finish()
    }

    fn render_sidecar_spec_panel(
        &self,
        kind: &ModelSpecSidecarKind,
        spec: &Option<LLMSpec>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let (title, description) = match kind {
            ModelSpecSidecarKind::Auto => (
                "Auto mode",
                "Auto will select the best model for the task. Cost-efficiency optimizes for cost, Responsiveness optimizes for response speed.",
            ),
            ModelSpecSidecarKind::Reasoning => (
                "Reasoning level",
                "Increased reasoning levels consume more credits and have higher latency, but higher performance for complicated tasks.",
            ),
        };

        let header = self.render_model_spec_header(title.to_string(), description.to_string(), app);
        let sidecar_menu = ChildView::new(&self.model_spec_sidecar.dropdown).finish();
        let spec_values = self.render_all_model_spec_values(
            &spec.clone().unwrap_or_default(),
            false,
            internal_colors::neutral_5(theme),
            app,
        );

        ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(header)
                    .with_child(sidecar_menu)
                    .with_child(
                        Container::new(spec_values)
                            .with_horizontal_margin(8.)
                            .with_horizontal_padding(16.)
                            .with_padding_top(4.)
                            .with_padding_bottom(16.)
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                CORNER_RADIUS,
                            )))
                            .with_background_color(internal_colors::neutral_3(theme))
                            .finish(),
                    )
                    .finish(),
            )
            .with_padding_bottom(8.)
            .with_background_color(internal_colors::neutral_2(theme))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)))
            .with_drop_shadow(DropShadow::default())
            .finish(),
        )
        .with_width(MENU_WIDTH)
        .finish()
    }
}

impl TypedActionView for ProfileModelSelector {
    type Action = ProfileModelSelectorAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ProfileModelSelectorAction::SelectProfile(profile_id) => {
                AIExecutionProfilesModel::handle(ctx).update(ctx, |profiles_model, ctx| {
                    profiles_model.set_active_profile(self.terminal_view_id, *profile_id, ctx);
                });

                // Remove any LLM override when switching profiles
                LLMPreferences::handle(ctx).update(ctx, |llm_prefs, ctx| {
                    llm_prefs.remove_llm_override(self.terminal_view_id, ctx);
                });

                self.set_profile_menu_visibility(false, ctx);
            }
            ProfileModelSelectorAction::SelectModel(llm_id) => {
                LLMPreferences::handle(ctx).update(ctx, |preferences, ctx| {
                    log::info!("Selecting base agent model {llm_id} (from model selector)");
                    preferences.update_preferred_agent_mode_llm(llm_id, self.terminal_view_id, ctx);
                });
                self.set_model_menu_visibility(false, ctx);
            }
            ProfileModelSelectorAction::SelectAutoModel
            | ProfileModelSelectorAction::SelectReasoningModel(_) => {
                self.handle_sidecar_selection(ctx);
            }
            ProfileModelSelectorAction::SelectHarnessModel {
                model_id,
                reasoning_level,
            } => {
                let is_default = model_id.is_empty();
                if let Some(ambient_agent_model) = self.ambient_agent_view_model.clone() {
                    ambient_agent_model.update(ctx, |model, ctx| {
                        model.set_harness_model_selection(
                            (!is_default).then(|| model_id.clone()),
                            if is_default {
                                None
                            } else {
                                reasoning_level.clone()
                            },
                            ctx,
                        );
                    });
                    let harness = ambient_agent_model.as_ref(ctx).selected_harness();
                    CloudAgentSettings::handle(ctx).update(ctx, |settings, ctx| {
                        settings.persist_harness_model_selection(
                            harness,
                            model_id,
                            reasoning_level.clone(),
                            ctx,
                        );
                    });
                }
                self.set_model_menu_visibility(false, ctx);
            }
            ProfileModelSelectorAction::ManageProfiles => {
                self.set_profile_menu_visibility(false, ctx);
                ctx.emit(ProfileModelSelectorEvent::OpenSettings(
                    SettingsSection::AgentProfiles,
                ));
            }
            ProfileModelSelectorAction::ToggleProfileMenu => {
                self.set_profile_menu_visibility(!self.is_profile_menu_open, ctx);
            }
            ProfileModelSelectorAction::ToggleModelMenu => {
                if self.is_model_locked(ctx) {
                    return;
                }
                if self.is_third_party_harness(ctx) {
                    self.set_model_menu_visibility(!self.is_model_menu_open, ctx);
                } else if FeatureFlag::InlineMenuHeaders.is_enabled() {
                    ctx.emit(ProfileModelSelectorEvent::ToggleInlineModelSelector);
                } else {
                    self.set_model_menu_visibility(!self.is_model_menu_open, ctx);
                }
            }
        }
    }
}

impl View for ProfileModelSelector {
    fn ui_name() -> &'static str {
        "ProfileModelSelector"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let profiles_model = AIExecutionProfilesModel::as_ref(app);
        let has_multiple_profiles = profiles_model.has_multiple_profiles();

        // Check if user is a viewer in a shared session
        let is_viewer = self
            .terminal_model
            .lock()
            .shared_session_status()
            .is_viewer();

        let mut compact_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        // Only add profile button to compact layout if there are multiple profiles
        // and the user is not a viewer (we currently don't support profiles in shared sessions).
        let is_ambient_agent = self.ambient_agent_view_model.is_some();
        let should_show_profile_section = has_multiple_profiles && !is_viewer && !is_ambient_agent;
        if should_show_profile_section {
            let profile_button_with_save_position = SavePosition::new(
                ChildView::new(&self.profile_compact_button).finish(),
                "profile_model_selector_profile_compact_button",
            )
            .finish();
            compact_row.add_child(profile_button_with_save_position);
        }

        let model_button_with_save_position = SavePosition::new(
            ChildView::new(&self.model_compact_button).finish(),
            "profile_model_selector_model_compact_button",
        )
        .finish();
        compact_row.add_child(model_button_with_save_position);

        let compact_layout = compact_row.finish();

        let mut chip_content = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        // Only add profile section and separator if there are multiple profiles
        // and the user is not a viewer
        if should_show_profile_section {
            // Don't show separator if either selector is hovered
            let profile_hovered = self.profile_mouse_state.lock().unwrap().is_hovered();
            let model_hovered = self.model_mouse_state.lock().unwrap().is_hovered();

            let show_separator = !(profile_hovered || model_hovered);

            chip_content.add_child(self.render_profile_section(app));
            chip_content.add_child(self.render_separator(app, show_separator));
        }
        chip_content.add_child(self.render_model_section(app));

        let unified_chip = Container::new(
            Container::new(chip_content.finish())
                .with_background(theme.surface_1())
                .with_border(
                    Border::all(BORDER_WIDTH).with_border_color(internal_colors::neutral_3(theme)),
                )
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)))
                .finish(),
        )
        .finish();

        let content = if self.render_compact {
            compact_layout
        } else {
            unified_chip
        };

        let mut stack = Stack::new();
        stack.add_child(content);

        if self.is_profile_menu_open && should_show_profile_section {
            let profile_menu = ChildView::new(&self.profile_dropdown).finish();
            let positioning = self.get_menu_positioning(app, true);
            stack.add_positioned_overlay_child(profile_menu, positioning);
        }

        if self.is_model_menu_open {
            let model_menu = ChildView::new(&self.model_dropdown).finish();
            let positioning = self.get_menu_positioning(app, false);
            stack.add_positioned_overlay_child(model_menu, positioning);

            if let Some(info) = self.hovered_llm_info.as_ref() {
                // Decide whether to show a sidecar purely from the hovered item
                let sidecar_kind = if is_auto(info) {
                    Some(ModelSpecSidecarKind::Auto)
                } else if self.has_multiple_reasoning_variants(info) {
                    Some(ModelSpecSidecarKind::Reasoning)
                } else {
                    None
                };

                let model_spec_sidecar = if let Some(kind) = sidecar_kind {
                    // Show sidecar panel with default spec if none exists
                    let sidecar_spec = self
                        .model_spec_sidecar
                        .hovered_info
                        .as_ref()
                        .and_then(|i| i.spec.as_ref())
                        .cloned();
                    Some(self.render_sidecar_spec_panel(&kind, &sidecar_spec, app))
                } else if let Some(spec) = info.spec.as_ref() {
                    let is_using_api_key = is_using_api_key_for_provider(&info.provider, app);
                    Some(self.render_model_spec(spec, is_using_api_key, app))
                } else {
                    None
                };

                if let Some(model_spec_sidecar) = model_spec_sidecar {
                    let position_id = self.model_menu_item_position_id(&info.id);
                    let flip_left = self.should_render_model_sidecar_left(&position_id, app);
                    let offset_x = if flip_left {
                        -(MENU_WIDTH + SIDECAR_HORIZONTAL_GAP)
                    } else {
                        MENU_WIDTH + SIDECAR_HORIZONTAL_GAP
                    };

                    let sidecar_with_position =
                        SavePosition::new(model_spec_sidecar, SIDECAR_POSITION_ID).finish();

                    stack.add_positioned_overlay_child(
                        sidecar_with_position,
                        OffsetPositioning::offset_from_save_position_element(
                            position_id,
                            vec2f(offset_x, 0.),
                            PositionedElementOffsetBounds::WindowByPosition,
                            PositionedElementAnchor::Center,
                            ChildAnchor::Center,
                        ),
                    );
                }
            }
        }

        let is_udi_enabled =
            crate::settings::InputSettings::as_ref(app).is_universal_developer_input_enabled(app);

        // The popup overflows the viewport on wasm mobile.
        let is_wasm_mobile = warpui::platform::is_mobile_device();

        if !is_wasm_mobile
            && (is_udi_enabled
                || self
                    .input_model
                    .as_ref(app)
                    .last_ai_autodetection_ts()
                    .is_none_or(|ts| {
                        Instant::now().duration_since(ts) > NEW_MODEL_CHOICES_POPUP_DELAY
                    }))
        {
            let llm_preferences = LLMPreferences::as_ref(app);
            match (
                llm_preferences.should_show_new_choices_popup(self.terminal_view_id),
                llm_preferences.new_choices_since_last_update(),
            ) {
                (true, Some(new_choices)) if !new_choices.is_empty() => {
                    llm_preferences.mark_new_choices_popup_as_shown(self.terminal_view_id);
                    stack.add_positioned_overlay_child(
                        ChildView::new(&self.new_model_popup).finish(),
                        // Render the popup above the chip, centered horizontally.
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., -6.),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::TopMiddle,
                            ChildAnchor::BottomMiddle,
                        ),
                    );
                }
                _ => (),
            }
        }

        stack.finish()
    }
}

impl Entity for ProfileModelSelector {
    type Event = ProfileModelSelectorEvent;
}
