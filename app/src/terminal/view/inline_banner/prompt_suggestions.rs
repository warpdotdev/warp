use std::rc::Rc;

use pathfinder_geometry::vector::vec2f;
use serde::Serialize;
use warp_core::channel::ChannelState;
use warp_core::ui::theme::color::internal_colors::{neutral_2, neutral_3};
use warpui::elements::{
    ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty,
    Fill, Flex, HighlightedHyperlink, Hoverable, Icon, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius,
    Shrinkable, Stack, Text,
};
use warpui::keymap::Keystroke;
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, EventContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

use crate::ai::agent::api::ServerConversationToken;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::{PassiveSuggestionTrigger, StaticQueryType};
use crate::ai::blocklist::prompt::prompt_alert::{
    PromptAlertEvent, PromptAlertState, PromptAlertView,
};
use crate::ai::blocklist::BlocklistAIInputModel;
use crate::ai::predict::prompt_suggestions::ACCEPT_PROMPT_SUGGESTION_KEYBINDING;
use crate::appearance::Appearance;
use crate::server::ids::ServerId;
use crate::server::telemetry::InteractionSource;
use crate::settings::InputSettings;
use crate::terminal::view::passive_suggestions::PromptSuggestionResolution;
use crate::terminal::view::{ContextMenuAction, InputType, PromptSuggestion, TerminalAction};
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon as WarpUIIcon;
use crate::util::bindings::keybinding_name_to_keystroke;

const INLINE_BANNER_SPACING: f32 = 8.;
const INLINE_BANNER_BUTTON_PADDING: f32 = 8.;
const INLINE_BANNER_BUTTON_VERTICAL_PADDING: f32 = 4.;

const DELINQUENT_DUE_TO_PAYMENT_ISSUE_TOOLTIP_MESSAGE: &str = "Restricted due to payment issue";
const OUT_OF_REQUESTS_TOOLTIP_MESSAGE: &str = "Out of credits";

/// Types of zero-state prompt suggestions.
#[derive(Debug, Copy, Clone, Serialize)]
pub enum ZeroStatePromptSuggestionType {
    Explain,
    Fix,
    Install,
    Code,
    Deploy,
    SomethingElse,
}

/// Places zero-stage prompt suggestions are surfaced.
#[derive(Debug, Copy, Clone, Serialize)]
pub enum ZeroStatePromptSuggestionTriggeredFrom {
    InputBar,
    AgentModeHomepage,
    TryAgentModeBanner,
    AgentManagementPopup,
}

impl ZeroStatePromptSuggestionType {
    /// Constant for the number of zero-state prompt suggestion types.
    pub const COUNT: usize = 5;

    pub fn query(&self) -> &'static str {
        match self {
            Self::Explain => "Explain this to me.",
            Self::Fix => "Help me fix this.",
            Self::Install => {
                "Help me install a binary/dependency. What information do I need to provide to you to do this?"
            }
            Self::Code => {
                "Help me write some code. What information do I need to provide to you to do this?"
            }
            Self::Deploy => {
                "Help me deploy my project. What information do I need to provide to you to do this?"
            }
            Self::SomethingElse => "Something else?",
        }
    }

    pub fn static_query_type(&self) -> Option<StaticQueryType> {
        match self {
            Self::Explain | Self::Fix => None,
            Self::Install => Some(StaticQueryType::Install),
            Self::Code => Some(StaticQueryType::Code),
            Self::Deploy => Some(StaticQueryType::Deploy),
            Self::SomethingElse => Some(StaticQueryType::SomethingElse),
        }
    }
}

const KEYBOARD_SHORTCUT_MARGIN: f32 = 8.;

#[derive(Clone, Debug)]
pub struct PromptSuggestionBannerState {
    pub banner_id: usize,
    pub prompt_suggestion: PromptSuggestion,
    pub accept_button_mouse_state: MouseStateHandle,
    pub llm_warning_learn_more_hyperlink: HighlightedHyperlink,
    pub should_hide: bool,
    /// The trigger for this suggestion. `None` when the server indicated the
    /// trigger is not relevant to the suggestion (and should be omitted from
    /// the result sent back).
    pub trigger: Option<PassiveSuggestionTrigger>,

    /// The conversation that this suggestion should be associated with.
    /// Only populated when a prompt suggestion is generated in the agent view.
    pub conversation_id: Option<AIConversationId>,

    /// The server request token, used to construct a debug link (dogfood only).
    pub server_request_token: Option<String>,
}

/// Renders the Prompt Suggestions button, with appropriate hover and click effects.
#[allow(clippy::too_many_arguments)]
fn render_button(
    text: String,
    icon: WarpUIIcon,
    button_index: usize,
    keystroke: Option<Keystroke>,
    mouse_state: MouseStateHandle,
    on_click: Rc<impl Fn(&mut EventContext) + 'static>,
    debug_request_token: Option<ServerConversationToken>,
    prompt_alert_state: &PromptAlertState,
    should_shrink: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let is_button_disabled = matches!(
        prompt_alert_state,
        PromptAlertState::NoConnection
            | PromptAlertState::AnonymousUserRequestLimitHardGate
            | PromptAlertState::DelinquentDueToPaymentIssue
            | PromptAlertState::OveragesToggleableButNotEnabled
            | PromptAlertState::MonthlyOveragesSpendLimitReached
            | PromptAlertState::RequestLimitReached
    );
    let opacity: f32 = if is_button_disabled { 0.5 } else { 1.0 };
    let opacity_u8 = (opacity * 255.0).round() as u8;
    let hoverable = Hoverable::new(mouse_state.clone(), |mouse_state| {
        let mut background_color = if mouse_state.is_hovered() {
            neutral_3(theme)
        } else {
            neutral_2(theme)
        };
        background_color.a = opacity_u8;
        let background_fill = Fill::Solid(background_color);

        let mut text_color = blended_colors::text_main(theme, theme.surface_1());
        if is_button_disabled {
            text_color = blended_colors::text_disabled(theme, theme.surface_1());
        }

        let icon_size = appearance.monospace_font_size();
        let mut icon_color = blended_colors::text_main(theme, theme.surface_1());
        icon_color.a = opacity_u8;

        let should_expand_button = mouse_state.is_hovered();
        let text = {
            let base = Text::new(
                text,
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .soft_wrap(should_expand_button)
            .with_color(text_color)
            .finish();

            if should_shrink {
                Shrinkable::new(1.0, base).finish()
            } else {
                base
            }
        };

        let mut flex = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    ConstrainedBox::new(Icon::new(icon.into(), icon_color).finish())
                        .with_width(icon_size)
                        .with_height(icon_size)
                        .finish(),
                )
                .with_padding_left(INLINE_BANNER_BUTTON_PADDING)
                .with_padding_right(INLINE_BANNER_BUTTON_PADDING)
                .finish(),
            )
            .with_child(text);

        if let Some(keystroke) = keystroke {
            let style = UiComponentStyles {
                font_family_id: Some(appearance.ui_font_family()),
                font_size: Some(icon_size),
                height: Some(20.),

                border_radius: Some(CornerRadius::with_all(Radius::Pixels(3.))),
                padding: Some(Coords::uniform(3.)),
                margin: Some(Coords::default().left(KEYBOARD_SHORTCUT_MARGIN)),

                font_color: Some(text_color),
                background: Some(neutral_3(theme).into()),
                ..Default::default()
            };

            flex.add_child(
                appearance
                    .ui_builder()
                    .keyboard_shortcut(&keystroke)
                    .with_style(style)
                    .with_line_height_ratio(1.)
                    .build()
                    .finish(),
            );
        }

        let mut container = Container::new(flex.finish())
            .with_background(background_fill)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_padding_right(INLINE_BANNER_BUTTON_PADDING)
            .with_padding_top(INLINE_BANNER_BUTTON_VERTICAL_PADDING)
            .with_padding_bottom(INLINE_BANNER_BUTTON_VERTICAL_PADDING);

        if button_index != 0 {
            container = container.with_margin_left(INLINE_BANNER_SPACING);
        }

        let mut stack = Stack::new();
        stack.add_child(container.finish());

        if is_button_disabled && mouse_state.is_hovered() {
            if let Some(tooltip_text) = get_tooltip_text_for_alert_state(prompt_alert_state) {
                let tooltip = appearance
                    .ui_builder()
                    .tool_tip(tooltip_text)
                    .with_style(UiComponentStyles {
                        font_size: Some(appearance.monospace_font_size() - 4.),
                        padding: Some(Coords {
                            top: 4.,
                            bottom: 4.,
                            left: 8.,
                            right: 8.,
                        }),
                        background: Some(theme.tooltip_background().into()),
                        font_color: Some(theme.background().into_solid()),
                        ..Default::default()
                    })
                    .build()
                    .finish();
                let tooltip_offset = OffsetPositioning::offset_from_parent(
                    vec2f(0., 4.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::BottomMiddle,
                    ChildAnchor::TopMiddle,
                );
                stack.add_positioned_overlay_child(tooltip, tooltip_offset);
            }
        }

        stack.finish()
    })
    .with_cursor(Cursor::PointingHand);

    let hoverable = if let Some(token) = debug_request_token {
        hoverable.on_right_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(TerminalAction::ContextMenu(
                ContextMenuAction::CopyServerRequestId {
                    request_id: token.clone(),
                },
            ));
        })
    } else {
        hoverable
    };

    if is_button_disabled {
        hoverable.finish()
    } else {
        hoverable.on_click(move |ctx, _, _| on_click(ctx)).finish()
    }
}

fn get_tooltip_text_for_alert_state(alert_state: &PromptAlertState) -> Option<String> {
    // This is not an exhaustive list; the actual prompt alert component will have more information,
    // so we can keep the tooltip's text relatively minimal and just capture broad groups.
    match alert_state {
        PromptAlertState::DelinquentDueToPaymentIssue => {
            Some(DELINQUENT_DUE_TO_PAYMENT_ISSUE_TOOLTIP_MESSAGE.to_string())
        }
        PromptAlertState::RequestLimitReached
        | PromptAlertState::AnonymousUserRequestLimitHardGate
        | PromptAlertState::AnonymousUserRequestLimitSoftGate
        | PromptAlertState::OveragesToggleableButNotEnabled
        | PromptAlertState::MonthlyOveragesSpendLimitReached => {
            Some(OUT_OF_REQUESTS_TOOLTIP_MESSAGE.to_string())
        }
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptSuggestionsEvent {
    SignupAnonymousUser,
    OpenBillingAndUsagePage,
    OpenPrivacyPage,
    OpenBillingPortal { team_uid: ServerId },
}

pub struct PromptSuggestionsView {
    ai_input_model: ModelHandle<BlocklistAIInputModel>,
    prompt_alert: ViewHandle<PromptAlertView>,
    banner_state: Option<PromptSuggestionBannerState>,
}

impl PromptSuggestionsView {
    pub fn new(
        ai_input_model: ModelHandle<BlocklistAIInputModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let prompt_alert = ctx.add_typed_action_view(PromptAlertView::new);
        ctx.subscribe_to_view(&prompt_alert, |me, _, event, ctx| {
            me.handle_prompt_alert_event(event, ctx);
        });

        ctx.subscribe_to_model(&ai_input_model, |_, _, _, ctx| {
            ctx.notify();
        });

        Self {
            ai_input_model,
            prompt_alert,
            banner_state: None,
        }
    }

    pub fn set_banner_state(&mut self, banner_state: PromptSuggestionBannerState) {
        self.banner_state = Some(banner_state);
    }

    fn handle_prompt_alert_event(&mut self, event: &PromptAlertEvent, ctx: &mut ViewContext<Self>) {
        match event {
            PromptAlertEvent::SignupAnonymousUser => {
                ctx.emit(PromptSuggestionsEvent::SignupAnonymousUser);
            }
            PromptAlertEvent::OpenBillingAndUsagePage => {
                ctx.emit(PromptSuggestionsEvent::OpenBillingAndUsagePage);
            }
            PromptAlertEvent::OpenPrivacyPage => {
                ctx.emit(PromptSuggestionsEvent::OpenPrivacyPage);
            }
            PromptAlertEvent::OpenBillingPortal { team_uid } => {
                ctx.emit(PromptSuggestionsEvent::OpenBillingPortal {
                    team_uid: *team_uid,
                });
            }
        }
    }
}

impl Entity for PromptSuggestionsView {
    type Event = PromptSuggestionsEvent;
}

impl View for PromptSuggestionsView {
    fn ui_name() -> &'static str {
        "PromptSuggestionsView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut inner_banner_flex = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max);

        let prompt_alert_state = self.prompt_alert.as_ref(app).state();

        let Some(banner_state) = &self.banner_state else {
            return Empty::new().finish();
        };
        let prompt_suggestion = &banner_state.prompt_suggestion;

        let debug_request_token = if ChannelState::enable_debug_features() {
            banner_state
                .server_request_token
                .as_ref()
                .map(|t| ServerConversationToken::new(t.clone()))
        } else {
            None
        };

        inner_banner_flex.add_child(
            Shrinkable::new(
                1.0,
                render_button(
                    prompt_suggestion.label().clone(),
                    WarpUIIcon::Oz,
                    0,
                    keybinding_name_to_keystroke(ACCEPT_PROMPT_SUGGESTION_KEYBINDING, app),
                    banner_state.accept_button_mouse_state.clone(),
                    Rc::new(move |ctx: &mut warpui::EventContext<'_>| {
                        ctx.dispatch_typed_action(TerminalAction::ResolvePromptSuggestion(
                            PromptSuggestionResolution::Accept {
                                interaction_source: InteractionSource::Button,
                            },
                        ));
                    }),
                    debug_request_token,
                    prompt_alert_state,
                    true, // should_shrink
                    appearance,
                ),
            )
            .finish(),
        );

        // Render the right-side alert chip. This is also rendered by the AI prompt, so if we're in AI mode,
        // we skip over rendering it to avoid two of them showing up. Also skip if UDI is enabled since
        // UDI will render it in the bottom bar.
        if self.ai_input_model.as_ref(app).input_type() != InputType::AI
            && !InputSettings::as_ref(app).is_universal_developer_input_enabled(app)
        {
            // We don't want to render the chip if there's nothing that will show up;
            // that will cause it to take up space , which squishes the left-side prompt.
            if !self.prompt_alert.as_ref(app).is_no_alert() {
                inner_banner_flex.add_child(
                    Shrinkable::new(1., ChildView::new(&self.prompt_alert).finish()).finish(),
                );
            }
        }

        Container::new(inner_banner_flex.finish())
            // Add 1px top padding to balance out the 1px overdraw on the bottom
            // and keep everything vertically centered.
            .with_padding_top(1.)
            .with_overdraw_bottom(1.)
            .finish()
    }
}

impl TypedActionView for PromptSuggestionsView {
    type Action = PromptSuggestionsEvent;

    fn handle_action(&mut self, action: &PromptSuggestionsEvent, ctx: &mut ViewContext<Self>) {
        match action {
            PromptSuggestionsEvent::SignupAnonymousUser => {
                ctx.emit(PromptSuggestionsEvent::SignupAnonymousUser);
            }
            PromptSuggestionsEvent::OpenBillingAndUsagePage => {
                ctx.emit(PromptSuggestionsEvent::OpenBillingAndUsagePage);
            }
            PromptSuggestionsEvent::OpenPrivacyPage => {
                ctx.emit(PromptSuggestionsEvent::OpenPrivacyPage);
            }
            PromptSuggestionsEvent::OpenBillingPortal { team_uid } => {
                ctx.emit(PromptSuggestionsEvent::OpenBillingPortal {
                    team_uid: *team_uid,
                });
            }
        }
    }
}
