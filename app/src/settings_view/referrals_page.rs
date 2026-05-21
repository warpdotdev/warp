use lazy_static::lazy_static;
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;
use std::{ops::Deref, sync::Arc};
use thiserror::Error;
use validator::ValidateEmail;

use super::{
    settings_page::{
        MatchData, PageType, SettingsPageMeta, SettingsPageViewHandle, SettingsWidget, PAGE_PADDING,
    },
    SettingsSection,
};
use crate::{
    appearance::Appearance,
    auth::AuthStateProvider,
    editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions},
    i18n::I18nKey,
    safe_info, send_telemetry_from_ctx,
    server::{
        server_api::referral::{ReferralInfo, ReferralsClient},
        telemetry::TelemetryEvent,
    },
    ui_components::blended_colors,
    view_components::ToastFlavor,
};
use warpui::{
    clipboard::ClipboardContent,
    elements::{
        Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Element, Fill,
        Flex, FormattedTextElement, HighlightedHyperlink, Icon, MainAxisSize, MouseStateHandle,
        ParentElement, Radius, Rect, Shrinkable,
    },
    fonts::Weight,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Entity, EventContext, FocusContext, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

const HEADER_FONT_SIZE: f32 = 18.;
const HEADER_MARGIN_BOTTOM: f32 = 32.;
const ANONYMOUS_USER_HEADER_TEXT: &str = "Sign up to participate in Warp's referral program";

const INVITE_FIELD_LABEL_BOTTOM_MARGIN: f32 = 8.;

const LINK_BOTTOM_MARGIN: f32 = 12.;
const LINK_TEXT_PADDING: f32 = 10.;
const LINK_CORNER_RADIUS: Radius = Radius::Pixels(4.);
const LINK_ERROR_TEXT: &str = "Failed to load referral code.";

const BUTTON_WIDTH: f32 = 98.;
const BUTTON_HEIGHT: f32 = 36.;
const BUTTON_LEFT_MARGIN: f32 = 8.;
const BUTTON_FONT_SIZE: f32 = 12.;
const LINK_BUTTON_TEXT: &str = "Copy link";
const EMAIL_BUTTON_TEXT: &str = "Send";
const EMAIL_BUTTON_SENDING_TEXT: &str = "Sending...";
const LOADING_TEXT: &str = "Loading...";

const LINK_COPIED_TOAST: &str = "Link copied.";
const EMAIL_SUCCESS_TOAST: &str = "Successfully sent emails.";
const EMAIL_FAILURE_TOAST: &str = "Failed to send emails. Please try again.";

const REWARD_INTRO: &str = "Get exclusive Warp goodies when you refer someone*";
const REWARD_INTRO_FONT_SIZE: f32 = 14.;
const REWARD_SECTION_VERTICAL_SPACING: f32 = 24.;

const REFERRAL_ICON_BOX_VERTICAL_SPACING: f32 = 8.;
const REWARD_ICON_BOX_HEIGHT: f32 = 60.;
const REWARD_ICON_BOX_WIDTH: f32 = 80.;
const REWARD_ICON_BORDER_CORNER_RADIUS: Radius = Radius::Pixels(8.);
const REWARD_ICON_BOX_DESCRIPTION_HORIZONTAL_SPACING: f32 = 12.;
const REWARD_ICON_BOX_BORDER_WIDTH: f32 = 1.;

const METER_LEVEL_BORDER_WIDTH: f32 = 2.;
const METER_LEVEL_CIRCLE_HEIGHT: f32 = 28.;
const METER_LEVEL_FONT_SIZE: f32 = 11.;
const METER_LINE_WIDTH: f32 = 2.;
const METER_LINE_HEIGHT: f32 = 26.;
const METER_ICON_SEPARATOR_VERTICAL_MARGIN: f32 = 7.;
const METER_DOT_SPACING: f32 = 2.;
const METER_TOP_MARGIN: f32 = 16.;
const METER_RIGHT_MARGIN: f32 = 12.;

const CLAIMED_REFERRALS_LABEL_HORIZONTAL_SPACING: f32 = 4.;
const CLAIMED_REFERRALS_COUNT_LABEL_SINGULAR: &str = "Current referral";
const CLAIMED_REFERRALS_COUNT_LABEL_PLURAL: &str = "Current referrals";
const CLAIMED_REFERRALS_LABEL_WIDTH: f32 = 52.;
const CLAIMED_REFERRALS_LABEL_FONT_SIZE: f32 = 14.;
const CLAIMED_REFERRALS_COUNT_FONT_SIZE: f32 = 48.;
const CLAIMED_REFERRAL_COUNT_LEFT_MARGIN: f32 = 40.;

const CLAIMED_REFERRAL_CLIP: usize = 999;

const TERMS_LINK_TEXT: &str = "Certain restrictions apply.";
const TERMS_URL: &str =
    "https://docs.warp.dev/support-and-community/community/refer-a-friend#referral-program-terms-and-conditions";
const TERMS_CONTACT_TEXT: &str =
    " If you have any questions about the referral program, please contact referrals@warp.dev.";

enum ApiState {
    Loading,
    Ready {
        referral_info: ReferralInfo,
        email_state: SendEmailState,
    },
    Failed,
}

#[derive(Debug)]
pub enum ReferralsPageAction {
    CopyLink,
    SendEmailInvite,
    SignupAnonymousUser,
}

pub enum ReferralsPageEvent {
    SignupAnonymousUser,
    FocusModal,
    ShowToast {
        message: String,
        flavor: ToastFlavor,
    },
}

enum SendEmailState {
    Idle,
    Sending,
}

pub struct ReferralsPageView {
    page: PageType<Self>,
    email_editor: ViewHandle<EditorView>,
    referrals_client: Arc<dyn ReferralsClient>,
    api_state: ApiState,
}

#[derive(Clone)]
struct Reward {
    required_referral_count: usize,
    icon_path: &'static str,
    icon_height: f32,
    icon_width: f32,
    label: String,
}

lazy_static! {
    static ref REWARDS: Vec<Reward> = vec![
        Reward {
            required_referral_count: 1,
            icon_path: "bundled/svg/referral-theme.svg",
            icon_width: 64.,
            icon_height: 64.,
            label: "Exclusive theme".to_owned(),
        },
        Reward {
            required_referral_count: 5,
            icon_path: "bundled/svg/referral-keycaps.svg",
            icon_width: 56.,
            icon_height: 56.,
            label: "Keycaps + stickers".to_owned(),
        },
        Reward {
            required_referral_count: 10,
            icon_path: "bundled/svg/referral-tshirt.svg",
            icon_width: 64.,
            icon_height: 64.,
            label: "T-shirt".to_owned(),
        },
        Reward {
            required_referral_count: 20,
            icon_path: "bundled/svg/referral-notebook.svg",
            icon_width: 64.,
            icon_height: 64.,
            label: "Notebook".to_owned(),
        },
        Reward {
            required_referral_count: 35,
            icon_path: "bundled/svg/referral-hat.svg",
            icon_width: 64.,
            icon_height: 64.,
            label: "Baseball cap".to_owned(),
        },
        Reward {
            required_referral_count: 50,
            icon_path: "bundled/svg/referral-hoodie.svg",
            icon_width: 64.,
            icon_height: 64.,
            label: "Hoodie".to_owned(),
        },
        Reward {
            required_referral_count: 75,
            icon_path: "bundled/svg/referral-hydroflask.svg",
            icon_width: 48.,
            icon_height: 48.,
            label: "Premium Hydro Flask".to_owned(),
        },
        Reward {
            required_referral_count: 100,
            icon_path: "bundled/svg/referral-backpack.svg",
            icon_width: 50.,
            icon_height: 50.,
            label: "Backpack".to_owned(),
        },
    ];
}

impl ReferralsPageView {
    pub fn new(referrals_client: Arc<dyn ReferralsClient>, ctx: &mut ViewContext<Self>) -> Self {
        let email_editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions::ui_font_size(Appearance::as_ref(ctx)),
                ..Default::default()
            };
            EditorView::single_line(options, ctx)
        });

        ctx.subscribe_to_view(&email_editor, |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let page = PageType::new_monolith_i18n_title(
            ReferralsWidget::default(),
            Some(I18nKey::SettingsPageTitleInviteFriend),
            true,
        );
        Self {
            page,
            referrals_client,
            api_state: ApiState::Loading,
            email_editor,
        }
    }

    /// Make a request to get the referral status
    ///
    /// If the status has already been fetched, the information will be kept while the request
    /// is in flight.
    fn fetch_referral_status(&mut self, ctx: &mut ViewContext<Self>) {
        // If we already have data, we fire another request to make sure it is up-to-date,
        // however, we don't want to update the state and lose the existing data until the
        // request completes.
        if matches!(self.api_state, ApiState::Failed) {
            self.api_state = ApiState::Loading;
        }

        let referrals_client = self.referrals_client.clone();
        let _ = ctx.spawn(
            async move { referrals_client.get_referral_info().await },
            Self::handle_referral_status_response,
        );
    }

    fn handle_referral_status_response(
        &mut self,
        response: anyhow::Result<ReferralInfo>,
        ctx: &mut ViewContext<Self>,
    ) {
        match response {
            Ok(info) => match &mut self.api_state {
                state @ ApiState::Loading | state @ ApiState::Failed => {
                    *state = ApiState::Ready {
                        referral_info: info,
                        email_state: SendEmailState::Idle,
                    };
                }
                ApiState::Ready { referral_info, .. } => {
                    *referral_info = info;
                }
            },
            Err(err) => {
                self.api_state = ApiState::Failed;
                log::warn!("Error loading referral info from server: {err}");
            }
        }
        ctx.notify();
    }

    fn copy_link(&mut self, ctx: &mut ViewContext<Self>) {
        match &self.api_state {
            ApiState::Loading | ApiState::Failed => {
                // Shouldn't happen as the buttons will be disabled
                log::warn!("Attempting to copy link before API request is complete");
            }
            ApiState::Ready { referral_info, .. } => {
                send_telemetry_from_ctx!(TelemetryEvent::CopyInviteLink, ctx);
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(referral_info.url.to_string()));
                ctx.emit(ReferralsPageEvent::ShowToast {
                    message: LINK_COPIED_TOAST.to_owned(),
                    flavor: ToastFlavor::Default,
                });
            }
        }
    }

    fn send_email_invite(&mut self, ctx: &mut ViewContext<Self>) {
        let emails = self.recipient_emails_from_editor(ctx);
        match &mut self.api_state {
            ApiState::Ready {
                email_state: state @ SendEmailState::Idle,
                ..
            } => match emails.iter().map(Deref::deref).try_for_each(validate_email) {
                Ok(_) => {
                    *state = SendEmailState::Sending;
                    let referrals_client = self.referrals_client.clone();
                    let _ = ctx.spawn(
                        async move { referrals_client.send_invite(emails).await },
                        Self::handle_send_email_invite_response,
                    );
                }
                Err(error) => {
                    ctx.emit(ReferralsPageEvent::ShowToast {
                        message: error.ui_message(),
                        flavor: ToastFlavor::Error,
                    });
                    log::warn!("Emails entered are invalid: {error}");
                }
            },
            _ => {
                // Shouldn't happen as the buttons will be disabled
                log::warn!("Attempting to send email referrals before API is available");
            }
        }
    }

    fn handle_send_email_invite_response(
        &mut self,
        response: anyhow::Result<Vec<String>>,
        ctx: &mut ViewContext<Self>,
    ) {
        match response {
            Ok(successful) => {
                self.email_editor.update(ctx, |view, ctx| {
                    view.clear_buffer_and_reset_undo_stack(ctx);
                    ctx.notify();
                });
                safe_info!(
                    safe: ("Successfully sent {} invites", successful.len()),
                    full: ("Successfully sent invites to: {:?}", successful)
                );
                ctx.emit(ReferralsPageEvent::ShowToast {
                    message: EMAIL_SUCCESS_TOAST.to_owned(),
                    flavor: ToastFlavor::Success,
                });
            }
            Err(err) => {
                log::error!("Error sending referral emails: {err}");
                ctx.emit(ReferralsPageEvent::ShowToast {
                    message: EMAIL_FAILURE_TOAST.to_owned(),
                    flavor: ToastFlavor::Error,
                });
            }
        }

        if let ApiState::Ready { email_state, .. } = &mut self.api_state {
            *email_state = SendEmailState::Idle;
        }
        ctx.notify();
    }

    fn recipient_emails_from_editor(&self, ctx: &mut ViewContext<Self>) -> Vec<String> {
        let editor_text = self.email_editor.as_ref(ctx).buffer_text(ctx);
        editor_text
            .split(',')
            .map(|email| email.trim().to_string())
            .collect()
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => {
                self.send_email_invite(ctx);
            }
            EditorEvent::Escape => ctx.emit(ReferralsPageEvent::FocusModal),
            _ => (),
        }
    }

    fn referral_claimed_count(&self) -> Option<usize> {
        match &self.api_state {
            ApiState::Ready { referral_info, .. } => Some(referral_info.number_claimed),
            _ => None,
        }
    }
}

impl Entity for ReferralsPageView {
    type Event = ReferralsPageEvent;
}

impl View for ReferralsPageView {
    fn ui_name() -> &'static str {
        "ReferralsPageView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.fetch_referral_status(ctx);
        }
    }
}

impl SettingsPageMeta for ReferralsPageView {
    fn section() -> SettingsSection {
        SettingsSection::Referrals
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        true
    }

    fn on_page_selected(&mut self, _: bool, ctx: &mut ViewContext<Self>) {
        self.fetch_referral_status(ctx);
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl TypedActionView for ReferralsPageView {
    type Action = ReferralsPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ReferralsPageAction::CopyLink => self.copy_link(ctx),
            ReferralsPageAction::SendEmailInvite => self.send_email_invite(ctx),
            ReferralsPageAction::SignupAnonymousUser => {
                ctx.emit(ReferralsPageEvent::SignupAnonymousUser)
            }
        }
    }
}

impl From<ViewHandle<ReferralsPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<ReferralsPageView>) -> Self {
        SettingsPageViewHandle::Referrals(view_handle)
    }
}
#[derive(Error, Debug)]
enum EmailValidationError {
    #[error("Email is empty")]
    Empty,
    #[error("Email is invalid: {0}")]
    Invalid(String),
}

impl EmailValidationError {
    /// The user-readable error descriptions.
    fn ui_message(&self) -> String {
        match self {
            EmailValidationError::Empty => "Please enter an email.".to_owned(),
            EmailValidationError::Invalid(invalid_email) => {
                format!("Please ensure the following email is valid: {invalid_email}")
            }
        }
    }
}

fn validate_email(email: &str) -> anyhow::Result<(), EmailValidationError> {
    if email.is_empty() {
        Err(EmailValidationError::Empty)
    } else if !email.validate_email() {
        Err(EmailValidationError::Invalid(email.to_owned()))
    } else {
        Ok(())
    }
}

#[derive(Default)]
struct ReferralsWidget {
    copy_link_mouse_state: MouseStateHandle,
    send_email_mouse_state: MouseStateHandle,
    sign_up_button_mouse_state: MouseStateHandle,
    term_docs_highlighted_hyperlink: HighlightedHyperlink,
}

impl ReferralsWidget {
    fn render_page_body(
        &self,
        view: &ReferralsPageView,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let is_anonymous = AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out();

        let invite_or_signup_section = if is_anonymous {
            self.render_signup_section(appearance, app)
        } else {
            self.render_send_invite_section(view, appearance)
        };

        Flex::column()
            .with_child(
                Container::new(invite_or_signup_section)
                    .with_padding_bottom(PAGE_PADDING)
                    .finish(),
            )
            .with_child(
                Container::new(self.render_rewards_section(is_anonymous, view, appearance))
                    .with_padding_bottom(PAGE_PADDING)
                    .finish(),
            )
            .finish()
    }

    fn render_link_row(
        &self,
        view: &ReferralsPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let (link_text, button_enabled) = match &view.api_state {
            ApiState::Ready { referral_info, .. } => (referral_info.url.clone(), true),
            ApiState::Loading => (LOADING_TEXT.into(), false),
            ApiState::Failed => (LINK_ERROR_TEXT.into(), false),
        };
        let theme = appearance.theme();

        Container::new(
            Flex::row()
                .with_child(
                    Shrinkable::new(
                        1.0,
                        Container::new(
                            Align::new(
                                appearance
                                    .ui_builder()
                                    .span(link_text)
                                    .with_style(UiComponentStyles {
                                        font_color: Some(
                                            theme.main_text_color(theme.background()).into_solid(),
                                        ),
                                        ..Default::default()
                                    })
                                    .build()
                                    .finish(),
                            )
                            .left()
                            .finish(),
                        )
                        .with_background(theme.background())
                        .with_uniform_padding(LINK_TEXT_PADDING)
                        .with_corner_radius(CornerRadius::with_all(LINK_CORNER_RADIUS))
                        .with_border(Border::all(1.).with_border_fill(theme.outline()))
                        .finish(),
                    )
                    .finish(),
                )
                .with_child(self.render_button(
                    button_enabled,
                    LINK_BUTTON_TEXT,
                    self.copy_link_mouse_state.clone(),
                    |ctx, _, _| ctx.dispatch_typed_action(ReferralsPageAction::CopyLink),
                    appearance,
                ))
                .with_main_axis_size(MainAxisSize::Max)
                .finish(),
        )
        .with_margin_bottom(LINK_BOTTOM_MARGIN)
        .finish()
    }

    fn render_email_row(
        &self,
        view: &ReferralsPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let (button_text, button_enabled) = match &view.api_state {
            ApiState::Ready {
                email_state: SendEmailState::Idle,
                ..
            } => (EMAIL_BUTTON_TEXT, true),
            ApiState::Ready {
                email_state: SendEmailState::Sending,
                ..
            } => (EMAIL_BUTTON_SENDING_TEXT, false),
            _ => (EMAIL_BUTTON_TEXT, false),
        };

        Flex::row()
            .with_child(
                Shrinkable::new(
                    1.0,
                    Align::new(
                        appearance
                            .ui_builder()
                            .text_input(view.email_editor.clone())
                            .with_style(UiComponentStyles::default())
                            .build()
                            .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            )
            .with_child(self.render_button(
                button_enabled,
                button_text,
                self.send_email_mouse_state.clone(),
                |ctx, _, _| ctx.dispatch_typed_action(ReferralsPageAction::SendEmailInvite),
                appearance,
            ))
            .finish()
    }

    fn render_send_invite_section(
        &self,
        view: &ReferralsPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Flex::column()
            .with_child(
                Container::new(self.render_label("Link", appearance))
                    .with_padding_top(PAGE_PADDING)
                    .finish(),
            )
            .with_child(self.render_link_row(view, appearance))
            .with_child(self.render_label("Email", appearance))
            .with_child(self.render_email_row(view, appearance))
            .finish()
    }

    fn render_signup_section(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let button_styles = UiComponentStyles {
            font_size: Some(14.),
            font_weight: Some(Weight::Semibold),
            border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.))),
            padding: Some(Coords {
                top: 12.,
                bottom: 12.,
                left: 40.,
                right: 40.,
            }),
            ..Default::default()
        };

        let sign_up_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.sign_up_button_mouse_state.clone(),
            )
            .with_style(button_styles)
            .with_text_label(crate::i18n::tr_static(app, "Sign up").to_owned())
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ReferralsPageAction::SignupAnonymousUser);
            })
            .finish();

        Flex::column()
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .span(ANONYMOUS_USER_HEADER_TEXT)
                        .with_style(UiComponentStyles {
                            font_size: Some(HEADER_FONT_SIZE),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_margin_bottom(HEADER_MARGIN_BOTTOM)
                .finish(),
            )
            .with_child(Flex::row().with_child(sign_up_button).finish())
            .finish()
    }

    /// Render submit buttons for the email and link fields.
    fn render_button<F>(
        &self,
        button_enabled: bool,
        button_text: &str,
        mouse_state_handle: MouseStateHandle,
        on_click: F,
        appearance: &Appearance,
    ) -> Box<dyn Element>
    where
        F: 'static + FnMut(&mut EventContext, &AppContext, Vector2F),
    {
        let button = appearance
            .ui_builder()
            .button(ButtonVariant::Accent, mouse_state_handle)
            .with_centered_text_label(button_text.to_owned())
            .with_style(UiComponentStyles {
                font_size: Some(BUTTON_FONT_SIZE),
                font_weight: Some(Weight::Semibold),
                width: Some(BUTTON_WIDTH),
                height: Some(BUTTON_HEIGHT),
                ..Default::default()
            });

        Container::new({
            if button_enabled {
                button.build().on_click(on_click).finish()
            } else {
                button.disabled().build().finish()
            }
        })
        .with_margin_left(BUTTON_LEFT_MARGIN)
        .finish()
    }

    /// Render text labels for the email and link fields.
    fn render_label<S>(&self, text: S, appearance: &Appearance) -> Box<dyn Element>
    where
        S: Into<String>,
    {
        Container::new(appearance.ui_builder().span(text.into()).build().finish())
            .with_margin_bottom(INVITE_FIELD_LABEL_BOTTOM_MARGIN)
            .finish()
    }

    fn render_rewards_section(
        &self,
        is_anonymous: bool,
        view: &ReferralsPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mut rewards_section = Flex::column();

        rewards_section.add_child(
            Container::new(
                appearance
                    .ui_builder()
                    .span(REWARD_INTRO)
                    .with_style(UiComponentStyles {
                        font_size: Some(REWARD_INTRO_FONT_SIZE),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_margin_bottom(REWARD_SECTION_VERTICAL_SPACING)
            .finish(),
        );

        let mut reward_status_row = Flex::row()
            .with_child(
                Container::new(self.render_meter(view, appearance))
                    .with_margin_top(METER_TOP_MARGIN)
                    .with_margin_bottom(METER_TOP_MARGIN)
                    .with_margin_right(METER_RIGHT_MARGIN)
                    .finish(),
            )
            .with_child(self.render_rewards_list(view, appearance));

        if !is_anonymous {
            if let Some(count) = self.render_claimed_referrals_count(view, appearance) {
                reward_status_row.add_child(
                    Container::new(count)
                        .with_margin_left(CLAIMED_REFERRAL_COUNT_LEFT_MARGIN)
                        .finish(),
                );
            }
        };

        rewards_section.add_child(reward_status_row.finish());

        rewards_section.add_child(
            Container::new(
                Align::new(
                    FormattedTextElement::new(
                        FormattedText::new([FormattedTextLine::Line(vec![
                            FormattedTextFragment::plain_text("*"),
                            FormattedTextFragment::hyperlink(TERMS_LINK_TEXT, TERMS_URL),
                            FormattedTextFragment::plain_text(TERMS_CONTACT_TEXT),
                        ])]),
                        12.,
                        appearance.ui_font_family(),
                        appearance.ui_font_family(),
                        blended_colors::text_sub(
                            appearance.theme(),
                            appearance.theme().surface_1(),
                        ),
                        self.term_docs_highlighted_hyperlink.clone(),
                    )
                    .with_hyperlink_font_color(appearance.theme().accent().into_solid())
                    .register_default_click_handlers(|url, _, ctx| {
                        ctx.open_url(&url.url);
                    })
                    .finish(),
                )
                .left()
                .finish(),
            )
            .with_margin_top(REWARD_SECTION_VERTICAL_SPACING)
            .finish(),
        );

        rewards_section.finish()
    }

    fn render_rewards_list(
        &self,
        view: &ReferralsPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            Flex::column()
                .with_children(REWARDS.iter().map(|reward| {
                    Container::new(self.render_reward(reward, view, appearance))
                        .with_margin_bottom(REFERRAL_ICON_BOX_VERTICAL_SPACING)
                        .finish()
                }))
                .finish(),
        )
        .finish()
    }

    fn render_reward(
        &self,
        reward: &Reward,
        view: &ReferralsPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let (icon_color, label_color, label_font_weight): (ColorU, ColorU, Option<Weight>) =
            match view.referral_claimed_count() {
                Some(claimed_referrals) if claimed_referrals >= reward.required_referral_count => (
                    blended_colors::accent(appearance.theme()).into(),
                    blended_colors::text_main(appearance.theme(), appearance.theme().background()),
                    Some(Weight::Bold),
                ),

                _ => (
                    blended_colors::text_sub(appearance.theme(), appearance.theme().background()),
                    blended_colors::text_sub(appearance.theme(), appearance.theme().background()),
                    None,
                ),
            };

        Flex::row()
            .with_child(
                ConstrainedBox::new(
                    Container::new(
                        Align::new(
                            ConstrainedBox::new(Icon::new(reward.icon_path, icon_color).finish())
                                .with_height(reward.icon_height)
                                .with_width(reward.icon_width)
                                .finish(),
                        )
                        .finish(),
                    )
                    .with_corner_radius(CornerRadius::with_all(REWARD_ICON_BORDER_CORNER_RADIUS))
                    .with_border(
                        Border::all(REWARD_ICON_BOX_BORDER_WIDTH)
                            .with_border_color(appearance.theme().surface_3().into()),
                    )
                    .finish(),
                )
                .with_width(REWARD_ICON_BOX_WIDTH)
                .with_height(REWARD_ICON_BOX_HEIGHT)
                .finish(),
            )
            .with_child(
                Container::new(
                    appearance
                        .ui_builder()
                        .span(reward.label.clone())
                        .with_style(UiComponentStyles {
                            font_color: Some(label_color),
                            font_weight: label_font_weight,
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_margin_left(REWARD_ICON_BOX_DESCRIPTION_HORIZONTAL_SPACING)
                .finish(),
            )
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish()
    }

    /// Render the meter tracking how many claimed referrals the user has sent.
    fn render_meter(&self, view: &ReferralsPageView, appearance: &Appearance) -> Box<dyn Element> {
        let referral_count = view.referral_claimed_count().unwrap_or_default();

        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Center);

        for (index, reward) in REWARDS.iter().enumerate() {
            let lower_threshold = reward.required_referral_count;

            let count_indicator =
                self.render_referral_meter_count(lower_threshold, referral_count, appearance);

            if index < (REWARDS.len() - 1) {
                column.add_child(
                    Container::new(count_indicator)
                        .with_margin_bottom(METER_ICON_SEPARATOR_VERTICAL_MARGIN)
                        .finish(),
                );

                let higher_threshold = REWARDS[index + 1].required_referral_count;

                column.add_child(
                    Container::new(self.render_meter_separator(
                        lower_threshold,
                        higher_threshold,
                        referral_count,
                        appearance,
                    ))
                    .with_margin_bottom(METER_ICON_SEPARATOR_VERTICAL_MARGIN)
                    .finish(),
                )
            } else {
                column.add_child(Container::new(count_indicator).finish());
            }
        }

        column.finish()
    }

    /// For the reward meter, render the count needed for a reward or an
    /// indicator that the user has met the required count.
    fn render_referral_meter_count(
        &self,
        required_referral_count: usize,
        referral_count: usize,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        if referral_count >= required_referral_count {
            ConstrainedBox::new(
                Icon::new(
                    "bundled/svg/check-circle-broken.svg",
                    appearance.theme().accent(),
                )
                .finish(),
            )
            .with_height(METER_LEVEL_CIRCLE_HEIGHT)
            .with_width(METER_LEVEL_CIRCLE_HEIGHT)
            .finish()
        } else {
            let gray: ColorU =
                blended_colors::text_sub(appearance.theme(), appearance.theme().background());

            Container::new(
                ConstrainedBox::new(
                    Align::new(
                        appearance
                            .ui_builder()
                            .span(required_referral_count.to_string())
                            .with_style(UiComponentStyles {
                                font_size: Some(METER_LEVEL_FONT_SIZE),
                                font_color: Some(gray),
                                font_weight: Some(Weight::Bold),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .finish(),
                )
                .with_height(METER_LEVEL_CIRCLE_HEIGHT)
                .with_width(METER_LEVEL_CIRCLE_HEIGHT)
                .finish(),
            )
            .with_border(Border::all(METER_LEVEL_BORDER_WIDTH).with_border_color(gray))
            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
            .finish()
        }
    }

    /// Render the solid or dotted lines that indicate completed or partial progress towards a reward's referral requirements.
    fn render_meter_separator(
        &self,
        lower_count: usize,
        higher_count: usize,
        current_count: usize,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let completed_color = blended_colors::accent(appearance.theme());

        let dot_color = if current_count > lower_count {
            completed_color
        } else {
            blended_colors::text_sub(appearance.theme(), appearance.theme().background()).into()
        };

        let line = ConstrainedBox::new(
            Rect::new()
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                .with_background(completed_color)
                .finish(),
        )
        .with_width(METER_LINE_WIDTH)
        .with_height(METER_LINE_HEIGHT)
        .finish();

        if current_count > higher_count {
            line
        } else {
            self.render_meter_dotted_line(dot_color)
        }
    }

    fn render_meter_dotted_line<F>(&self, color: F) -> Box<dyn Element>
    where
        F: Into<Fill> + Clone,
    {
        let mut dot_column = Flex::column();

        for _ in 0..5 {
            dot_column.add_child(
                Container::new(
                    ConstrainedBox::new(
                        Rect::new()
                            .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                            .with_background(color.clone())
                            .finish(),
                    )
                    .with_width(METER_LINE_WIDTH)
                    .with_height(METER_LINE_WIDTH)
                    .finish(),
                )
                .with_margin_bottom(METER_DOT_SPACING)
                .finish(),
            );
        }
        dot_column.add_child(
            ConstrainedBox::new(
                Rect::new()
                    .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                    .with_background(color)
                    .finish(),
            )
            .with_width(METER_LINE_WIDTH)
            .with_height(METER_LINE_WIDTH)
            .finish(),
        );

        dot_column.finish()
    }

    fn render_claimed_referrals_count(
        &self,
        view: &ReferralsPageView,
        appearance: &Appearance,
    ) -> Option<Box<dyn Element>> {
        let claimed_count = view.referral_claimed_count()?;

        let claimed_count_text = if claimed_count <= CLAIMED_REFERRAL_CLIP {
            claimed_count.to_string()
        } else {
            format!("{claimed_count}+")
        };

        let current_referrals_label = match claimed_count {
            1 => CLAIMED_REFERRALS_COUNT_LABEL_SINGULAR,
            _ => CLAIMED_REFERRALS_COUNT_LABEL_PLURAL,
        };

        Some(
            Flex::row()
                .with_child(
                    Container::new(
                        appearance
                            .ui_builder()
                            .span(claimed_count_text)
                            .with_style(UiComponentStyles {
                                font_size: Some(CLAIMED_REFERRALS_COUNT_FONT_SIZE),
                                font_color: Some(blended_colors::text_sub(
                                    appearance.theme(),
                                    appearance.theme().background(),
                                )),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .with_margin_right(CLAIMED_REFERRALS_LABEL_HORIZONTAL_SPACING)
                    .finish(),
                )
                .with_child(
                    ConstrainedBox::new(
                        appearance
                            .ui_builder()
                            .wrappable_text(current_referrals_label.to_string(), true)
                            .with_style(UiComponentStyles {
                                font_size: Some(CLAIMED_REFERRALS_LABEL_FONT_SIZE),
                                font_color: Some(blended_colors::text_sub(
                                    appearance.theme(),
                                    appearance.theme().background(),
                                )),
                                ..Default::default()
                            })
                            .build()
                            .finish(),
                    )
                    .with_width(CLAIMED_REFERRALS_LABEL_WIDTH)
                    .finish(),
                )
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
    }
}

impl SettingsWidget for ReferralsWidget {
    type View = ReferralsPageView;

    fn search_terms(&self) -> &str {
        "referrals invites"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        self.render_page_body(view, appearance, app)
    }
}
