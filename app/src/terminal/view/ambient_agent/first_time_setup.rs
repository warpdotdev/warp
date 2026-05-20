//! First-time cloud agent setup view.
//!
//! This view is displayed as an overlay when users first try to use cloud agent mode
//! and need to create an environment.

use crate::{
    ai::{
        ambient_agents::github_auth_url::{AuthSource, GithubAuthRedirectTarget},
        cloud_environments,
        request_usage_model::AMBIENT_AGENT_TRIAL_CREDIT_THRESHOLD,
        AIRequestUsageModel,
    },
    appearance::Appearance,
    server::{cloud_objects::update_manager::UpdateManager, ids::ClientId},
    settings_view::update_environment_form::{
        EnvironmentFormInitArgs, UpdateEnvironmentForm, UpdateEnvironmentFormEvent,
    },
    ui_components::blended_colors,
};
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use warp_core::ui::theme::{AnsiColorIdentifier, Fill};
use warpui::{
    elements::{
        new_scrollable::SingleAxisConfig, Align, Border, ChildView, ClippedScrollStateHandle,
        ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Expanded, Flex,
        FormattedTextElement, HighlightedHyperlink, NewScrollable, ParentElement, Radius, Text,
    },
    fonts::{Properties, Weight},
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

/// Max width for the content area (matches Figma: 592px)
const CONTENT_MAX_WIDTH: f32 = 592.;
const FORM_PADDING: f32 = 24.;
const SECTION_SPACING: f32 = 16.;
const HEADER_SPACING: f32 = 4.;

/// Events emitted by FirstTimeCloudAgentSetupView.
#[derive(Debug, Clone)]
pub enum FirstTimeCloudAgentSetupViewEvent {
    /// The user cancelled the setup (should pop from pane stack).
    Cancelled,
    /// The user created an environment and we should navigate to cloud agent mode.
    EnvironmentCreated,
}

/// A full-screen overlay view for first-time cloud agent environment setup.
pub struct FirstTimeCloudAgentSetupView {
    /// The embedded environment form (configured with hidden header).
    environment_form: ViewHandle<UpdateEnvironmentForm>,
    scroll_state: ClippedScrollStateHandle,
}

impl FirstTimeCloudAgentSetupView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        // Create the environment form in Create mode with header hidden and escape handling enabled
        let environment_form = ctx.add_typed_action_view(|ctx| {
            let mut form = UpdateEnvironmentForm::new(EnvironmentFormInitArgs::Create, ctx);
            form.set_github_auth_redirect_target(GithubAuthRedirectTarget::FocusCloudMode);
            form.set_show_header(false, ctx);
            form.set_should_handle_escape_from_editor(true);
            // Set auth source so GitHub auth redirects back here instead of opening settings
            form.set_auth_source(AuthSource::CloudSetup);
            form
        });

        // Subscribe to form events
        ctx.subscribe_to_view(&environment_form, |me, _, event, ctx| {
            me.handle_environment_form_event(event, ctx);
        });

        Self {
            environment_form,
            scroll_state: ClippedScrollStateHandle::default(),
        }
    }

    /// Resets the form to a fresh Create state.
    pub fn reset_form(&mut self, ctx: &mut ViewContext<Self>) {
        self.environment_form.update(ctx, |form, ctx| {
            form.set_mode(EnvironmentFormInitArgs::Create, ctx);
            form.focus(ctx);
        });
    }

    fn handle_environment_form_event(
        &mut self,
        event: &UpdateEnvironmentFormEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            UpdateEnvironmentFormEvent::Created {
                environment,
                share_with_team,
            } => {
                let owner = if *share_with_team {
                    cloud_environments::owner_for_new_environment(ctx)
                } else {
                    cloud_environments::owner_for_new_personal_environment(ctx)
                };

                let Some(owner) = owner else {
                    log::error!("Unable to create environment: not logged in");
                    // Reset form before emitting cancelled event
                    self.reset_form(ctx);
                    ctx.emit(FirstTimeCloudAgentSetupViewEvent::Cancelled);
                    return;
                };

                // Generate a client ID for tracking the environment
                let client_id = ClientId::default();

                // Create via UpdateManager
                UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
                    update_manager.create_ambient_agent_environment(
                        environment.clone(),
                        client_id,
                        owner,
                        ctx,
                    );
                });

                // Reset form after successful creation
                self.reset_form(ctx);

                // Emit event with the ClientId - the environment now exists in CloudModel
                ctx.emit(FirstTimeCloudAgentSetupViewEvent::EnvironmentCreated);
            }
            UpdateEnvironmentFormEvent::Cancelled => {
                // Reset form before emitting cancelled event
                self.reset_form(ctx);
                ctx.emit(FirstTimeCloudAgentSetupViewEvent::Cancelled);
            }
            UpdateEnvironmentFormEvent::Updated { .. }
            | UpdateEnvironmentFormEvent::DeleteRequested { .. } => {
                // These shouldn't happen in Create mode
            }
        }
    }

    /// Renders the header section (title + description) - displayed OUTSIDE the form card.
    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(HEADER_SPACING);

        // Title - 20px medium weight
        column.add_child(
            Text::new(
                "Start a new Oz cloud agent",
                appearance.ui_font_family(),
                20.,
            )
            .with_style(Properties::default().weight(Weight::Medium))
            .with_color(theme.foreground().into())
            .finish(),
        );

        // Description with "Visit docs" link
        let description_fragments = vec![
            FormattedTextFragment::plain_text(
                "Use Oz cloud agents to run parallel agents, build agents that run autonomously, and check in on your agents from anywhere. ",
            ),
            FormattedTextFragment::hyperlink(
                "Visit docs",
                "https://docs.warp.dev/agent-platform/cloud-agents/overview",
            ),
        ];
        column.add_child(
            FormattedTextElement::new(
                FormattedText::new([FormattedTextLine::Line(description_fragments)]),
                appearance.ui_font_size(),
                appearance.ui_font_family(),
                appearance.ui_font_family(),
                blended_colors::text_sub(theme, theme.surface_1()),
                HighlightedHyperlink::default(),
            )
            .with_hyperlink_font_color(theme.accent().into_solid())
            .register_default_click_handlers(|url, ctx, _| {
                ctx.dispatch_typed_action(FirstTimeCloudAgentSetupAction::OpenUrl(url.url));
            })
            .finish(),
        );

        column.finish()
    }

    /// Renders the subheading text in accent color - displayed OUTSIDE the form card.
    fn render_subheading(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        // Bold/semibold text in foreground color (per Figma: font-semibold text-[#e3e2df])
        Text::new(
            "Cloud agents require an environment that they'll run in to get their task done. Create your first environment below. You'll be able to edit the environment later, or add new environments when you need them.",
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_style(Properties::default().weight(Weight::Semibold))
        .with_color(theme.foreground().into())
        .soft_wrap(true)
        .finish()
    }

    /// Renders the free credits banner - displayed INSIDE the form card at the top.
    fn render_free_credits_banner(
        &self,
        credits: i32,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();

        // Badge with blue border
        let badge = Container::new(
            Text::new("Free credits", appearance.ui_font_family(), 12.)
                .with_style(Properties::default().weight(Weight::Semibold))
                .with_color(theme.accent().into())
                .finish(),
        )
        .with_horizontal_padding(6.)
        .with_vertical_padding(4.)
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
        .with_border(Border::all(1.).with_border_fill(theme.accent()))
        .finish();

        // Banner text - dynamic based on credits
        let credits_text = if credits == 1 {
            "You have 1 free credit to use on Oz cloud agents.".to_string()
        } else {
            format!(
                "You have {} free credits to use on Oz cloud agents.",
                credits
            )
        };
        let text = Text::new(credits_text, appearance.ui_font_family(), 12.)
            .with_color(blended_colors::text_sub(theme, theme.surface_1()))
            .finish();

        // Blue-tinted background: rgba(55,128,233,0.1)
        let blue_overlay = Fill::Solid(
            AnsiColorIdentifier::Blue
                .to_ansi_color(&theme.terminal_colors().normal)
                .into(),
        )
        .with_opacity(5);

        Container::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(4.)
                .with_child(badge)
                .with_child(text)
                .finish(),
        )
        .with_horizontal_padding(FORM_PADDING)
        .with_vertical_padding(12.)
        .with_corner_radius(CornerRadius::with_top(Radius::Pixels(8.)))
        .with_background(blue_overlay)
        .finish()
    }

    /// Renders the form card container with subtle background.
    fn render_form_card(&self, credits: Option<i32>, appearance: &Appearance) -> Box<dyn Element> {
        let card_bg = blended_colors::fg_overlay_1(appearance.theme()).into();

        let mut card_content =
            Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // Free credits banner at the top of the card - only show if credits are present
        if let Some(credits) = credits {
            card_content.add_child(self.render_free_credits_banner(credits, appearance));
        }

        // Embedded form with padding
        card_content.add_child(
            Container::new(ChildView::new(&self.environment_form).finish())
                .with_horizontal_padding(FORM_PADDING)
                .with_vertical_padding(FORM_PADDING)
                .finish(),
        );

        Container::new(card_content.finish())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
            .with_background_color(card_bg)
            .with_border(Border::all(1.).with_border_color(card_bg))
            .finish()
    }
}

impl Entity for FirstTimeCloudAgentSetupView {
    type Event = FirstTimeCloudAgentSetupViewEvent;
}

/// Action type for FirstTimeCloudAgentSetupView.
#[derive(Clone, Debug, PartialEq)]
pub enum FirstTimeCloudAgentSetupAction {
    OpenUrl(String),
}

impl TypedActionView for FirstTimeCloudAgentSetupView {
    type Action = FirstTimeCloudAgentSetupAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FirstTimeCloudAgentSetupAction::OpenUrl(url) => {
                ctx.open_url(url.as_str());
            }
        }
    }
}

impl View for FirstTimeCloudAgentSetupView {
    fn ui_name() -> &'static str {
        "FirstTimeCloudAgentSetupView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // Retrieve ambient credits and apply threshold filter
        // Only show banner if user has ambient credits >= threshold
        let credits_to_display = AIRequestUsageModel::as_ref(app)
            .ambient_only_credits_remaining()
            .filter(|&credits| credits >= AMBIENT_AGENT_TRIAL_CREDIT_THRESHOLD);

        // Build main content column:
        // 1. Header (title + description) - OUTSIDE the card
        // 2. Subheading - OUTSIDE the card
        // 3. Form card (contains free credits banner + form fields)
        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(SECTION_SPACING);

        // Header section (outside card)
        content.add_child(self.render_header(appearance));

        // Subheading (outside card)
        content.add_child(self.render_subheading(appearance));

        // Form card (contains banner + form)
        content.add_child(self.render_form_card(credits_to_display, appearance));

        // Constrain width and center the content
        let centered_content = Align::new(
            ConstrainedBox::new(content.finish())
                .with_max_width(CONTENT_MAX_WIDTH)
                .finish(),
        )
        .finish();

        let scrollable = NewScrollable::vertical(
            SingleAxisConfig::Clipped {
                handle: self.scroll_state.clone(),
                child: centered_content,
            },
            appearance.theme().nonactive_ui_detail().into(),
            appearance.theme().active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .finish();

        // Solid dark background for the overlay
        let overlay_bg = appearance.theme().background();

        // Wrap in Flex::column with Expanded to ensure it fills the terminal space
        Flex::column()
            .with_child(
                Expanded::new(
                    1.,
                    Container::new(scrollable)
                        .with_background(overlay_bg)
                        .with_uniform_padding(20.)
                        .finish(),
                )
                .finish(),
            )
            .finish()
    }
}
