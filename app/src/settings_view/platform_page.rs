use super::{
    platform::{
        CreateApiKeyModal, CreateApiKeyModalEvent, CreateApiKeyModalViewState, ExpireApiKeyButton,
        ExpireApiKeyButtonEvent,
    },
    settings_page::{
        MatchData, PageType, SettingsPageMeta, SettingsPageViewHandle, SettingsWidget,
        CONTENT_FONT_SIZE, SUBHEADER_FONT_SIZE,
    },
    SettingsSection,
};
use crate::auth::AuthStateProvider;
use crate::server::{ids::ApiKeyUid, server_api::auth::AuthClient};
use crate::{
    appearance::Appearance,
    modal::{Modal, ModalEvent, ModalViewState},
    ui_components::icons::Icon,
    util::time_format::format_approx_duration_from_now_utc,
};
use chrono::{DateTime, Utc};
use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use std::collections::HashMap;
use warp_core::features::FeatureFlag;
use warpui::text_layout::ClipConfig;
use warpui::{
    elements::{
        resizable_state_handle, Align, Border, ChildView, ConstrainedBox, Container,
        CrossAxisAlignment, DragBarSide, Element, Empty, Expanded, Flex, FormattedTextElement,
        HighlightedHyperlink, MainAxisSize, MouseStateHandle, Padding, ParentElement, Resizable,
        ResizableStateHandle, Shrinkable, Text,
    },
    fonts::{Properties, Weight},
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

const MODAL_WIDTH: f32 = 460.;
const MODAL_HEIGHT: f32 = 320.;
const API_KEY_DOCS_URL: &str = "https://docs.warp.dev/reference/cli/api-keys";
const API_KEY_NAME_COLUMN_DEFAULT_WIDTH: f32 = 220.;
const API_KEY_NAME_COLUMN_MIN_WIDTH: f32 = 120.;
const API_KEY_KEY_COLUMN_WIDTH: f32 = 120.;
const API_KEY_METADATA_COLUMN_MIN_WIDTH: f32 = 80.;
const API_KEY_ACTION_COLUMN_MIN_WIDTH: f32 = 48.;
const API_KEY_TABLE_MIN_NON_RESIZABLE_COLUMNS_WIDTH: f32 = API_KEY_KEY_COLUMN_WIDTH
    + (API_KEY_METADATA_COLUMN_MIN_WIDTH * 3.)
    + API_KEY_ACTION_COLUMN_MIN_WIDTH;
const API_KEY_TABLE_MIN_SCOPE_COLUMN_WIDTH: f32 = API_KEY_METADATA_COLUMN_MIN_WIDTH;
const API_KEY_TABLE_LAYOUT_SAFETY_PADDING: f32 = 16.;
const SETTINGS_SIDEBAR_WIDTH_DEFAULT: f32 = 200.;
const SETTINGS_SIDEBAR_WIDTH_WITH_FOOTER: f32 = 248.;
const SETTINGS_SECTION_BORDER_WIDTH: f32 = 1.;
const SETTINGS_PAGE_HORIZONTAL_PADDING: f32 = 56.;
const SETTINGS_PAGE_MAX_CONTENT_WIDTH: f32 = 800.;
fn settings_sidebar_width_for_platform_page() -> f32 {
    if FeatureFlag::SettingsFile.is_enabled() {
        SETTINGS_SIDEBAR_WIDTH_WITH_FOOTER
    } else {
        SETTINGS_SIDEBAR_WIDTH_DEFAULT
    }
}

fn api_key_table_width_chrome() -> f32 {
    settings_sidebar_width_for_platform_page()
        + SETTINGS_SECTION_BORDER_WIDTH
        + SETTINGS_PAGE_HORIZONTAL_PADDING
        + API_KEY_TABLE_LAYOUT_SAFETY_PADDING
}

fn api_key_table_min_non_resizable_columns_width(show_scope_column: bool) -> f32 {
    if show_scope_column {
        API_KEY_TABLE_MIN_NON_RESIZABLE_COLUMNS_WIDTH + API_KEY_TABLE_MIN_SCOPE_COLUMN_WIDTH
    } else {
        API_KEY_TABLE_MIN_NON_RESIZABLE_COLUMNS_WIDTH
    }
}

fn compute_api_key_name_column_max_width(
    window_width: f32,
    min_width: f32,
    min_non_resizable_columns_width: f32,
    table_width_chrome: f32,
) -> f32 {
    let available_table_width =
        (window_width - table_width_chrome).clamp(0., SETTINGS_PAGE_MAX_CONTENT_WIDTH);
    (available_table_width - min_non_resizable_columns_width).max(min_width)
}

#[derive(Clone, Copy)]
pub enum PlatformPageViewEvent {
    ShowCreateApiKeyModal,
    HideCreateApiKeyModal,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlatformPageAction {
    ShowCreateApiKeyModal,
    HyperlinkClick(String),
}

pub struct PlatformPageView {
    page: PageType<Self>,
    create_api_key_modal_state: CreateApiKeyModalViewState,
    api_keys: Vec<APIKeyProperties>,
    api_key_table_column_widths: ApiKeyTableColumnWidths,
    expire_buttons: HashMap<ApiKeyUid, ViewHandle<ExpireApiKeyButton>>,
    is_loading: bool,
    documentation_link_highlight: HighlightedHyperlink,
}

impl PlatformPageView {
    fn fetch_api_keys(&mut self, ctx: &mut ViewContext<PlatformPageView>) {
        // Set loading state only if we don't have any keys yet
        if self.api_keys.is_empty() {
            self.is_loading = true;
            ctx.notify();
        }

        // Build and send the GraphQL query
        let server_api = crate::server::server_api::ServerApiProvider::as_ref(ctx).get();

        ctx.spawn(
            async move { server_api.list_api_keys().await },
            |me, res, ctx| {
                me.is_loading = false;
                match res {
                    Ok(keys) => {
                        me.api_keys = keys
                            .into_iter()
                            .map(|gql_key| {
                                // Ensure the per-key expire button exists
                                let uid = gql_key.uid.into_inner();
                                me.ensure_expire_button_for_key(ctx, uid.clone());
                                let scope = match gql_key.owner_type {
                                    warp_graphql::object_permissions::OwnerType::User => {
                                        ApiKeyScope::Personal
                                    }
                                    warp_graphql::object_permissions::OwnerType::Team => {
                                        ApiKeyScope::Team
                                    }
                                };
                                APIKeyProperties::new(
                                    uid,
                                    gql_key.name,
                                    gql_key.key_suffix,
                                    scope,
                                    gql_key.created_at.utc(),
                                    gql_key.last_used_at.map(|t| t.utc()),
                                    gql_key.expires_at.map(|t| t.utc()),
                                )
                            })
                            .collect();
                        ctx.notify();
                    }
                    Err(err) => {
                        let window_id = ctx.window_id();
                        crate::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                            let toast =
                                crate::view_components::DismissibleToast::error(format!("{err}"));
                            toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                        });
                        ctx.notify();
                    }
                }
            },
        );
    }
    pub fn new(ctx: &mut ViewContext<PlatformPageView>) -> Self {
        let create_api_key_body = ctx.add_typed_action_view(CreateApiKeyModal::new);
        ctx.subscribe_to_view(&create_api_key_body, |me, _, event, ctx| {
            me.handle_create_api_key_modal_event(event, ctx);
        });

        let create_api_key_modal_view = ctx.add_typed_action_view(|ctx| {
            Modal::new(Some("New API key".to_string()), create_api_key_body, ctx)
                .with_modal_style(UiComponentStyles {
                    width: Some(MODAL_WIDTH),
                    height: Some(MODAL_HEIGHT),
                    ..Default::default()
                })
                .with_header_style(UiComponentStyles {
                    padding: Some(Coords {
                        top: 24.,
                        bottom: 0.,
                        left: 24.,
                        right: 24.,
                    }),
                    font_size: Some(16.),
                    font_weight: Some(warpui::fonts::Weight::Bold),
                    ..Default::default()
                })
                .with_body_style(UiComponentStyles {
                    padding: Some(Coords {
                        top: 0.,
                        bottom: 24.,
                        left: 24.,
                        right: 24.,
                    }),
                    ..Default::default()
                })
                .with_background_opacity(100)
                .with_dismiss_on_click()
        });
        ctx.subscribe_to_view(&create_api_key_modal_view, |me, _, event, ctx| {
            me.handle_modal_event(event, ctx);
        });

        PlatformPageView {
            page: PageType::new_monolith(PlatformPageWidget::default(), None, true),
            create_api_key_modal_state: CreateApiKeyModalViewState::new(ModalViewState::new(
                create_api_key_modal_view,
            )),
            api_keys: vec![],
            api_key_table_column_widths: ApiKeyTableColumnWidths::default(),
            expire_buttons: HashMap::new(),
            is_loading: true,
            documentation_link_highlight: HighlightedHyperlink::default(),
        }
    }

    fn show_create_api_key_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.create_api_key_modal_state
            .set_title(Some("New API key".to_string()), ctx);
        self.create_api_key_modal_state.open(ctx);
        ctx.emit(PlatformPageViewEvent::ShowCreateApiKeyModal);
    }

    fn hide_create_api_key_modal(&mut self, ctx: &mut ViewContext<Self>) {
        self.create_api_key_modal_state.close(ctx);
        ctx.emit(PlatformPageViewEvent::HideCreateApiKeyModal);
    }

    fn handle_modal_event(&mut self, event: &ModalEvent, ctx: &mut ViewContext<Self>) {
        match event {
            ModalEvent::Close => {
                self.hide_create_api_key_modal(ctx);
            }
        }
    }

    fn handle_create_api_key_modal_event(
        &mut self,
        event: &CreateApiKeyModalEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CreateApiKeyModalEvent::Close => {
                self.hide_create_api_key_modal(ctx);
            }
            CreateApiKeyModalEvent::Created { api_key } => {
                self.create_api_key_modal_state
                    .set_title(Some("Save your key".to_string()), ctx);
                let uid = api_key.uid.clone().into_inner();
                self.ensure_expire_button_for_key(ctx, uid.clone());

                let scope = match api_key.owner_type {
                    warp_graphql::object_permissions::OwnerType::User => ApiKeyScope::Personal,
                    warp_graphql::object_permissions::OwnerType::Team => ApiKeyScope::Team,
                };
                let ui_key = APIKeyProperties::new(
                    uid,
                    api_key.name.clone(),
                    api_key.key_suffix.clone(),
                    scope,
                    api_key.created_at.utc(),
                    api_key.last_used_at.map(|t| t.utc()),
                    api_key.expires_at.map(|t| t.utc()),
                );
                self.api_keys.push(ui_key);
                ctx.notify();
            }
            CreateApiKeyModalEvent::Error { message } => {
                let window_id = ctx.window_id();
                crate::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = crate::view_components::DismissibleToast::error(message.clone());
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                ctx.notify();
            }
        }
    }

    pub fn get_modal_content(&self) -> Option<Box<dyn Element>> {
        if self.create_api_key_modal_state.is_open() {
            Some(self.create_api_key_modal_state.render())
        } else {
            None
        }
    }

    fn ensure_expire_button_for_key(&mut self, ctx: &mut ViewContext<Self>, uid: ApiKeyUid) {
        if self.expire_buttons.contains_key(&uid) {
            return;
        }
        let handle = ctx.add_typed_action_view(|_ctx| ExpireApiKeyButton::new(uid.clone()));
        ctx.subscribe_to_view(&handle, |me, _emitter, event, ctx| match event {
            ExpireApiKeyButtonEvent::ExpireApiKeySucceeded { uid } => {
                me.api_keys.retain(|k| k.uid != *uid);
                me.expire_buttons.remove(uid);
                let window_id = ctx.window_id();
                crate::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = crate::view_components::DismissibleToast::success(
                        "API key deleted".to_string(),
                    );
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                ctx.notify();
            }
            ExpireApiKeyButtonEvent::ExpireApiKeyFailed { message } => {
                let window_id = ctx.window_id();
                crate::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = crate::view_components::DismissibleToast::error(message.clone());
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                ctx.notify();
            }
        });
        self.expire_buttons.insert(uid, handle);
    }
}

impl Entity for PlatformPageView {
    type Event = PlatformPageViewEvent;
}

impl TypedActionView for PlatformPageView {
    type Action = PlatformPageAction;

    fn handle_action(&mut self, action: &PlatformPageAction, ctx: &mut ViewContext<Self>) {
        match action {
            PlatformPageAction::ShowCreateApiKeyModal => {
                self.show_create_api_key_modal(ctx);
            }
            PlatformPageAction::HyperlinkClick(url) => {
                ctx.open_url(url);
            }
        }
    }
}

impl View for PlatformPageView {
    fn ui_name() -> &'static str {
        "PlatformPage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

#[derive(Debug, Clone)]
struct APIKeyProperties {
    uid: ApiKeyUid,
    name: String,
    key_suffix: String,
    scope: ApiKeyScope,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy)]
enum ApiKeyScope {
    Personal,
    Team,
    /// Not yet constructed — the server doesn't distinguish agent-scoped keys
    /// from team keys yet, but the create modal already supports the Agent
    /// type and the render path needs this variant for display.
    #[allow(dead_code)]
    Agent,
}

impl APIKeyProperties {
    fn new(
        uid: ApiKeyUid,
        name: impl Into<String>,
        key_suffix: impl Into<String>,
        scope: ApiKeyScope,
        created_at: DateTime<Utc>,
        last_used_at: Option<DateTime<Utc>>,
        expires_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            uid,
            name: name.into(),
            key_suffix: key_suffix.into(),
            scope,
            created_at,
            last_used_at,
            expires_at,
        }
    }
}

struct ApiKeyTableColumnWidths {
    name: ResizableStateHandle,
}

impl Default for ApiKeyTableColumnWidths {
    fn default() -> Self {
        Self {
            name: resizable_state_handle(API_KEY_NAME_COLUMN_DEFAULT_WIDTH),
        }
    }
}

impl ApiKeyTableColumnWidths {
    fn width(state_handle: &ResizableStateHandle) -> f32 {
        state_handle
            .lock()
            .expect("API key table column width handle should lock")
            .size()
    }

    fn name_width(&self) -> f32 {
        Self::width(&self.name)
    }
}
#[derive(Default)]
struct PlatformPageWidget {
    create_api_key_button_mouse_state: MouseStateHandle,
}

impl SettingsWidget for PlatformPageWidget {
    type View = PlatformPageView;

    fn search_terms(&self) -> &str {
        "oz cloud platform api keys authentication"
    }

    fn render(
        &self,
        view: &PlatformPageView,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        // Main container
        Flex::column()
            .with_child(self.render_api_keys_section(appearance, view, app))
            .finish()
    }
}

impl PlatformPageWidget {
    fn render_description_with_link(
        &self,
        view: &PlatformPageView,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let text = vec![
            FormattedTextFragment::plain_text("Create and manage API keys to allow other Oz cloud agents to access your Warp account.\nFor more information, visit the "),
            FormattedTextFragment::hyperlink("Documentation.", API_KEY_DOCS_URL),
        ];

        let text_element = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(text)]),
            CONTENT_FONT_SIZE,
            appearance.ui_font_family(),
            appearance.ui_font_family(),
            appearance.theme().nonactive_ui_text_color().into(),
            view.documentation_link_highlight.clone(),
        )
        .with_hyperlink_font_color(appearance.theme().accent().into_solid());

        let text_element = text_element.register_default_click_handlers(|url, ctx, _| {
            ctx.dispatch_typed_action(PlatformPageAction::HyperlinkClick(url.url.clone()));
        });

        text_element.finish()
    }

    fn render_api_keys_section(
        &self,
        appearance: &Appearance,
        view: &PlatformPageView,
        _app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let api_keys = &view.api_keys;

        let mut col = Flex::column();
        col.add_child(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Text::new_inline("Oz Cloud API Keys", appearance.ui_font_family(), 16.)
                        .with_style(Properties::default().weight(Weight::Bold))
                        .with_color(appearance.theme().active_ui_text_color().into())
                        .with_clip(ClipConfig::end())
                        .finish(),
                )
                .with_child(Shrinkable::new(1.0, Empty::new().finish()).finish())
                .with_child(
                    ui_builder
                        .button(
                            ButtonVariant::Outlined,
                            self.create_api_key_button_mouse_state.clone(),
                        )
                        .with_text_label("+ Create API Key".to_string())
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(PlatformPageAction::ShowCreateApiKeyModal);
                        })
                        .finish(),
                )
                .finish(),
        );

        col.add_child(
            Container::new(self.render_description_with_link(view, appearance))
                .with_margin_top(8.)
                .finish(),
        );

        if api_keys.is_empty() {
            if view.is_loading {
                // Render nothing (just the description) while loading
            } else {
                col.add_child(self.render_zero_state(appearance));
            }
        } else {
            col.add_child(self.render_api_keys_header(appearance, view));
            col.add_child(self.render_api_keys_rows(appearance, view, api_keys));
        }

        col.finish()
    }
    fn render_api_keys_header(
        &self,
        appearance: &Appearance,
        view: &PlatformPageView,
    ) -> Box<dyn Element> {
        let table_width_chrome = api_key_table_width_chrome();
        let show_scope_column =
            FeatureFlag::TeamApiKeys.is_enabled() || FeatureFlag::NamedAgents.is_enabled();
        let min_non_resizable_columns_width =
            api_key_table_min_non_resizable_columns_width(show_scope_column);
        let mut header_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);
        header_row.add_child(self.render_resizable_header_cell(
            appearance,
            "Name",
            view.api_key_table_column_widths.name.clone(),
            API_KEY_NAME_COLUMN_MIN_WIDTH,
            min_non_resizable_columns_width,
            table_width_chrome,
        ));
        header_row.add_child(
            ConstrainedBox::new(self.render_header_cell(appearance, "Key"))
                .with_width(API_KEY_KEY_COLUMN_WIDTH)
                .finish(),
        );
        if show_scope_column {
            header_row.add_child(
                Expanded::new(1., self.render_header_cell(appearance, "Scope")).finish(),
            );
        }
        header_row
            .add_child(Expanded::new(1., self.render_header_cell(appearance, "Created")).finish());
        header_row.add_child(
            Expanded::new(1., self.render_header_cell(appearance, "Last used")).finish(),
        );
        header_row.add_child(
            Expanded::new(1., self.render_header_cell(appearance, "Expires at")).finish(),
        );
        header_row.add_child(Expanded::new(0.5, self.render_header_cell(appearance, "")).finish());

        Container::new(header_row.finish())
            .with_margin_top(16.)
            .with_padding_bottom(8.)
            .with_border(Border::bottom(1.).with_border_fill(appearance.theme().outline()))
            .finish()
    }

    fn render_resizable_header_cell(
        &self,
        appearance: &Appearance,
        label: &str,
        width_handle: ResizableStateHandle,
        min_width: f32,
        min_non_resizable_columns_width: f32,
        table_width_chrome: f32,
    ) -> Box<dyn Element> {
        let width = width_handle
            .lock()
            .expect("API key header width handle should lock")
            .size();
        let header_cell = ConstrainedBox::new(
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(Expanded::new(1., self.render_header_cell(appearance, label)).finish())
                .with_child(
                    Container::new(
                        Text::new_inline("⋮", appearance.ui_font_family(), CONTENT_FONT_SIZE)
                            .with_color(appearance.theme().nonactive_ui_detail().into())
                            .finish(),
                    )
                    .with_padding_right(3.)
                    .finish(),
                )
                .finish(),
        )
        .with_width(width)
        .finish();
        Resizable::new(width_handle, header_cell)
            .with_dragbar_side(DragBarSide::Right)
            .with_bounds_callback(Box::new(move |window_size| {
                let max_width = compute_api_key_name_column_max_width(
                    window_size.x(),
                    min_width,
                    min_non_resizable_columns_width,
                    table_width_chrome,
                );
                (min_width, max_width)
            }))
            .on_resize(|ctx, _| {
                ctx.notify();
            })
            .finish()
    }

    fn render_api_keys_rows(
        &self,
        appearance: &Appearance,
        view: &PlatformPageView,
        api_keys: &[APIKeyProperties],
    ) -> Box<dyn Element> {
        let mut col = Flex::column();
        for key in api_keys.iter() {
            col.add_child(self.render_api_key_row(appearance, view, key));
        }
        col.finish()
    }

    fn render_header_cell(&self, appearance: &Appearance, label: &str) -> Box<dyn Element> {
        Container::new(
            Text::new_inline(
                label.to_string(),
                appearance.ui_font_family(),
                CONTENT_FONT_SIZE,
            )
            .with_style(Properties::default().weight(Weight::Semibold))
            .with_color(appearance.theme().nonactive_ui_text_color().into())
            .with_clip(ClipConfig::end())
            .finish(),
        )
        .with_padding(Padding::uniform(8.))
        .finish()
    }
    fn render_api_key_row(
        &self,
        appearance: &Appearance,
        view: &PlatformPageView,
        key: &APIKeyProperties,
    ) -> Box<dyn Element> {
        let created = format_approx_duration_from_now_utc(key.created_at);
        let last_used = key
            .last_used_at
            .map(format_approx_duration_from_now_utc)
            .unwrap_or_else(|| "Never".to_owned());
        let expires_at = key
            .expires_at
            .map(|dt| format!("{}", dt.format("%b %-d, %Y")))
            .unwrap_or_else(|| "Never".to_owned());
        let name_column_width = view.api_key_table_column_widths.name_width();
        let key_column_width = API_KEY_KEY_COLUMN_WIDTH;
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);
        // TODO: use appearance.ui_font_size() instead of hardcoded 12
        row.add_child(
            ConstrainedBox::new(
                Container::new(
                    Text::new_inline(key.name.clone(), appearance.ui_font_family(), 13.)
                        .with_color(appearance.theme().active_ui_text_color().into())
                        .with_clip(ClipConfig::end())
                        .finish(),
                )
                .with_padding(Padding::uniform(8.))
                .finish(),
            )
            .with_width(name_column_width)
            .finish(),
        );
        row.add_child(
            ConstrainedBox::new(
                Container::new(
                    Text::new_inline(
                        format!("wk-**{}", key.key_suffix),
                        appearance.monospace_font_family(),
                        12.,
                    )
                    .with_color(appearance.theme().active_ui_text_color().into())
                    .with_clip(ClipConfig::end())
                    .finish(),
                )
                .with_padding(Padding::uniform(8.))
                .finish(),
            )
            .with_width(key_column_width)
            .finish(),
        );
        if FeatureFlag::TeamApiKeys.is_enabled() || FeatureFlag::NamedAgents.is_enabled() {
            let scope_display = match key.scope {
                ApiKeyScope::Personal => "Personal",
                ApiKeyScope::Team => "Team",
                ApiKeyScope::Agent => "Agent",
            };
            row.add_child(
                Expanded::new(
                    1.,
                    Container::new(
                        Text::new_inline(scope_display, appearance.ui_font_family(), 12.)
                            .with_color(appearance.theme().nonactive_ui_text_color().into())
                            .finish(),
                    )
                    .with_padding(Padding::uniform(8.))
                    .finish(),
                )
                .finish(),
            );
        }
        row.add_child(
            Expanded::new(
                1.,
                Container::new(
                    Text::new_inline(created, appearance.ui_font_family(), 12.)
                        .with_color(appearance.theme().nonactive_ui_text_color().into())
                        .finish(),
                )
                .with_padding(Padding::uniform(8.))
                .finish(),
            )
            .finish(),
        );
        row.add_child(
            Expanded::new(
                1.,
                Container::new(
                    Text::new_inline(last_used, appearance.ui_font_family(), 12.)
                        .with_color(appearance.theme().nonactive_ui_text_color().into())
                        .finish(),
                )
                .with_padding(Padding::uniform(8.))
                .finish(),
            )
            .finish(),
        );
        row.add_child(
            Expanded::new(
                1.,
                Container::new(
                    Text::new_inline(expires_at, appearance.ui_font_family(), 12.)
                        .with_color(appearance.theme().nonactive_ui_text_color().into())
                        .finish(),
                )
                .with_padding(Padding::uniform(8.))
                .finish(),
            )
            .finish(),
        );
        // Expire button column
        let expire_button = view
            .expire_buttons
            .get(&key.uid)
            .map(|handle| ChildView::new(handle).finish())
            // Fallback in case the button is not yet created
            .unwrap_or_else(|| Empty::new().finish());
        row.add_child(Expanded::new(0.5, expire_button).finish());

        Container::new(row.finish())
            .with_vertical_padding(12.)
            .with_border(Border::bottom(1.).with_border_fill(appearance.theme().outline()))
            .finish()
    }

    fn render_zero_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            Align::new(
                Flex::column()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        ConstrainedBox::new(
                            Icon::Key
                                .to_warpui_icon(appearance.theme().nonactive_ui_text_color())
                                .finish(),
                        )
                        .with_width(48.)
                        .with_height(48.)
                        .finish(),
                    )
                    .with_child(
                        Container::new(
                            Text::new(
                                "No API Keys",
                                appearance.ui_font_family(),
                                SUBHEADER_FONT_SIZE,
                            )
                            .with_color(appearance.theme().active_ui_text_color().into())
                            .with_style(Properties::default().weight(Weight::Bold))
                            .finish(),
                        )
                        .with_margin_top(16.)
                        .finish(),
                    )
                    .with_child(
                        Container::new(
                            Text::new(
                                "Create a key to manage external access to Warp",
                                appearance.ui_font_family(),
                                CONTENT_FONT_SIZE,
                            )
                            .with_color(appearance.theme().active_ui_text_color().into())
                            .finish(),
                        )
                        .with_margin_top(8.)
                        .finish(),
                    )
                    .finish(),
            )
            .finish(),
        )
        .with_margin_top(80.)
        .finish()
    }
}

impl SettingsPageMeta for PlatformPageView {
    fn section() -> SettingsSection {
        SettingsSection::OzCloudAPIKeys
    }

    fn should_render(&self, ctx: &AppContext) -> bool {
        let is_anonymous = AuthStateProvider::as_ref(ctx)
            .get()
            .is_anonymous_or_logged_out();

        !is_anonymous && FeatureFlag::APIKeyManagement.is_enabled()
    }

    fn on_page_selected(&mut self, _allow_steal_focus: bool, ctx: &mut ViewContext<Self>) {
        // Always fetch/refresh API keys when page is selected to keep data fresh
        self.fetch_api_keys(ctx);
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

impl From<ViewHandle<PlatformPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<PlatformPageView>) -> Self {
        SettingsPageViewHandle::OzCloudAPIKeys(view_handle)
    }
}

#[cfg(test)]
#[path = "platform_page_tests.rs"]
mod tests;
