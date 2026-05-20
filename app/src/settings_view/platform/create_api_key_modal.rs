use crate::editor::Event as EditorEvent;
use crate::modal::{Modal, ModalViewState};
use crate::server::server_api::auth::{AgentIdentity, AuthClient};
use crate::util::truncation::truncate_from_end;
use crate::workspaces::user_workspaces::UserWorkspaces;
use crate::{
    appearance::Appearance,
    editor::{EditorView, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions, TextOptions},
    view_components::{Dropdown as DropdownView, DropdownItem},
};
use chrono::Utc;
use pathfinder_geometry::vector::vec2f;
use warp_core::features::FeatureFlag;
use warpui::elements::{
    Border, ChildView, ConstrainedBox, Container, CornerRadius, Empty, Fill, Flex,
    MouseStateHandle, ParentElement, Radius, Text,
};
use warpui::elements::{CrossAxisAlignment, Expanded, MainAxisAlignment, MainAxisSize, Padding};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::ui_components::segmented_control::{
    LabelConfig, RenderableOptionConfig, SegmentedControl,
};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

const OZ_AGENTS_URL: &str = "https://oz.warp.dev/agents?new=true";

const LABEL_FONT_SIZE: f32 = 14.;
const INPUT_WIDTH: f32 = 428.; // 460px - (2 * 16px) padding

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApiKeyType {
    Personal,
    Team,
    Agent,
}

impl ApiKeyType {
    fn description(&self) -> &'static str {
        match self {
            ApiKeyType::Personal => {
                "This API key is tied to your user and can make requests against your Warp account."
            }
            ApiKeyType::Team => {
                "This API key is tied to your team and can make requests on behalf of your team."
            }
            ApiKeyType::Agent => {
                "This API key is tied to an agent and can make requests on behalf of the agent."
            }
        }
    }
}

pub struct CreateApiKeyModal {
    name_editor: ViewHandle<EditorView>,
    expiration_dropdown: ViewHandle<DropdownView<CreateApiKeyModalAction>>,
    agent_dropdown: ViewHandle<DropdownView<CreateApiKeyModalAction>>,
    api_key_type_control: ViewHandle<SegmentedControl<ApiKeyType>>,
    expiration: ExpirationOption,
    cancel_button_mouse_state: MouseStateHandle,
    create_button_mouse_state: MouseStateHandle,
    create_agent_button_mouse_state: MouseStateHandle,
    request_state: RequestState,
    raw_key_copied: bool,
    raw_key: Option<String>,
    has_team: bool,
    has_named_agents: bool,
    agents: Vec<AgentIdentity>,
    selected_agent_uid: Option<String>,
    is_loading_agents: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExpirationOption {
    OneDay,
    ThirtyDays,
    NinetyDays,
    Never,
}

impl ExpirationOption {
    fn display_text(&self) -> &'static str {
        match self {
            ExpirationOption::OneDay => "1 day",
            ExpirationOption::ThirtyDays => "30 days",
            ExpirationOption::NinetyDays => "90 days",
            ExpirationOption::Never => "Never",
        }
    }

    fn days(&self) -> Option<i64> {
        match self {
            ExpirationOption::OneDay => Some(1),
            ExpirationOption::ThirtyDays => Some(30),
            ExpirationOption::NinetyDays => Some(90),
            ExpirationOption::Never => None,
        }
    }

    fn all() -> Vec<ExpirationOption> {
        vec![
            ExpirationOption::NinetyDays,
            ExpirationOption::ThirtyDays,
            ExpirationOption::OneDay,
            ExpirationOption::Never,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateApiKeyModalAction {
    Cancel,
    Create,
    CopyRawKey,
    SetExpiration(ExpirationOption),
    SelectAgent(String),
    CreateNewAgent,
}

pub enum CreateApiKeyModalEvent {
    Close,
    Created {
        api_key: warp_graphql::queries::api_keys::ApiKeyProperties,
    },
    Error {
        message: String,
    },
}

#[derive(PartialEq, Eq)]
enum RequestState {
    Idle,
    Pending,
    Succeeded,
}

impl CreateApiKeyModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let font_family = Appearance::as_ref(ctx).ui_font_family();

        let has_team = FeatureFlag::TeamApiKeys.is_enabled()
            && UserWorkspaces::as_ref(ctx).current_team_uid().is_some();
        let has_named_agents = FeatureFlag::NamedAgents.is_enabled();

        let name_editor = ctx.add_typed_action_view(|ctx| {
            let options = SingleLineEditorOptions {
                text: TextOptions {
                    font_family_override: Some(font_family),
                    ..Default::default()
                },
                propagate_and_no_op_vertical_navigation_keys:
                    PropagateAndNoOpNavigationKeys::Always,
                ..Default::default()
            };
            let mut editor = EditorView::single_line(options, ctx);
            editor.set_placeholder_text("Warp API Key", ctx);
            editor
        });

        let expiration_dropdown =
            ctx.add_typed_action_view(DropdownView::<CreateApiKeyModalAction>::new);

        let agent_dropdown =
            ctx.add_typed_action_view(DropdownView::<CreateApiKeyModalAction>::new);
        agent_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_top_bar_max_width(INPUT_WIDTH);
            dropdown.set_menu_width(INPUT_WIDTH, ctx);
        });

        let api_key_type_control = ctx.add_typed_action_view(move |ctx| {
            let options = if has_named_agents {
                vec![ApiKeyType::Personal, ApiKeyType::Agent]
            } else if has_team {
                vec![ApiKeyType::Personal, ApiKeyType::Team]
            } else {
                vec![ApiKeyType::Personal]
            };
            SegmentedControl::new(
                options,
                |key_type, is_selected, app| {
                    let appearance = Appearance::as_ref(app);
                    let theme = appearance.theme();

                    Some(RenderableOptionConfig {
                        icon_path: "",
                        icon_color: theme.active_ui_text_color().into(),
                        label: Some(LabelConfig {
                            label: match key_type {
                                ApiKeyType::Personal => "Personal".into(),
                                ApiKeyType::Team => "Team".into(),
                                ApiKeyType::Agent => "Agent".into(),
                            },
                            width_override: Some(55.0),
                            color: if is_selected {
                                theme.active_ui_text_color().into()
                            } else {
                                theme.nonactive_ui_text_color().into()
                            },
                        }),
                        tooltip: None,
                        background: if is_selected {
                            Fill::Solid(theme.surface_3().into())
                        } else {
                            Fill::None
                        },
                    })
                },
                ApiKeyType::Personal,
                api_key_type_control_styles(ctx),
            )
        });

        ctx.subscribe_to_view(&api_key_type_control, |me, _, _, ctx| {
            let selected = me.api_key_type_control.as_ref(ctx).selected_option();
            if selected == ApiKeyType::Agent && me.agents.is_empty() && !me.is_loading_agents {
                me.fetch_agents(ctx);
            }
            ctx.notify();
            me.name_editor.update(ctx, |_, ctx| ctx.notify());
        });

        ctx.subscribe_to_model(&UserWorkspaces::handle(ctx), |me, _, _, ctx| {
            me.update_has_team(ctx);
        });

        ctx.subscribe_to_view(&name_editor, |me, _, event, ctx| {
            me.handle_name_editor_event(event, ctx);
        });

        let default_expiration = ExpirationOption::NinetyDays;
        let items: Vec<DropdownItem<CreateApiKeyModalAction>> = ExpirationOption::all()
            .into_iter()
            .map(|opt| {
                DropdownItem::new(
                    opt.display_text(),
                    CreateApiKeyModalAction::SetExpiration(opt),
                )
            })
            .collect();
        expiration_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(items, ctx);
            dropdown.set_top_bar_max_width(INPUT_WIDTH);
            dropdown.set_menu_width(INPUT_WIDTH, ctx);
            dropdown.set_selected_by_action(
                CreateApiKeyModalAction::SetExpiration(default_expiration),
                ctx,
            );
        });

        Self {
            name_editor,
            expiration_dropdown,
            agent_dropdown,
            api_key_type_control,
            expiration: default_expiration,
            cancel_button_mouse_state: Default::default(),
            create_button_mouse_state: Default::default(),
            create_agent_button_mouse_state: Default::default(),
            request_state: RequestState::Idle,
            raw_key_copied: false,
            raw_key: None,
            has_team,
            has_named_agents,
            agents: Vec::new(),
            selected_agent_uid: None,
            is_loading_agents: false,
        }
    }

    fn fetch_agents(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_loading_agents = true;
        ctx.notify();

        let server_api = crate::server::server_api::ServerApiProvider::as_ref(ctx).get();
        ctx.spawn(
            async move { server_api.list_agent_identities().await },
            |me, res, ctx| {
                me.is_loading_agents = false;
                match res {
                    Ok(agents) => {
                        me.agents = agents;
                        me.populate_agent_dropdown(ctx);
                    }
                    Err(err) => {
                        log::error!("Failed to load agent identities: {err}");
                        ctx.emit(CreateApiKeyModalEvent::Error {
                            message: "Failed to load agents. Please close and try again."
                                .to_string(),
                        });
                    }
                }
                ctx.notify();
            },
        );
    }

    fn populate_agent_dropdown(&mut self, ctx: &mut ViewContext<Self>) {
        let items: Vec<DropdownItem<CreateApiKeyModalAction>> = self
            .agents
            .iter()
            .filter(|a| a.available)
            .map(|agent| {
                DropdownItem::new(
                    &agent.name,
                    CreateApiKeyModalAction::SelectAgent(agent.uid.clone()),
                )
            })
            .collect();
        self.agent_dropdown.update(ctx, |dropdown, ctx| {
            dropdown.set_items(items, ctx);
        });
    }

    fn create(&mut self, ctx: &mut ViewContext<Self>) {
        if self.request_state == RequestState::Pending {
            return;
        }
        let name = self.name_editor.as_ref(ctx).buffer_text(ctx);

        let final_name = if name.trim().is_empty() {
            "Warp API Key".to_string()
        } else {
            name.trim().to_string()
        };

        self.request_state = RequestState::Pending;
        ctx.notify();

        let expires_at = match self.expiration.days() {
            Some(days) => {
                let t = Utc::now() + chrono::Duration::days(days);
                Some(warp_graphql::scalars::Time::from(t))
            }
            None => None,
        };

        let selected_type = self.api_key_type_control.as_ref(ctx).selected_option();

        let agent_uid = if selected_type == ApiKeyType::Agent {
            match &self.selected_agent_uid {
                Some(uid) => Some(cynic::Id::new(uid.clone())),
                None => {
                    self.request_state = RequestState::Idle;
                    ctx.emit(CreateApiKeyModalEvent::Error {
                        message: "Please select an agent.".to_string(),
                    });
                    ctx.notify();
                    return;
                }
            }
        } else {
            None
        };

        let team_id = if selected_type == ApiKeyType::Team {
            let workspaces = UserWorkspaces::as_ref(ctx);
            match workspaces.current_team_uid() {
                Some(uid) => Some(cynic::Id::new(uid.uid())),
                None => {
                    self.request_state = RequestState::Idle;
                    ctx.emit(CreateApiKeyModalEvent::Error {
                        message:
                            "Unable to create a team API key because there is no current team."
                                .to_string(),
                    });
                    ctx.notify();
                    return;
                }
            }
        } else {
            None
        };

        let server_api = crate::server::server_api::ServerApiProvider::as_ref(ctx).get();
        ctx.spawn(
            async move { server_api.create_api_key(final_name, team_id, agent_uid, expires_at).await },
            |me, res, ctx| {
                match res {
                    Ok(warp_graphql::mutations::generate_api_key::GenerateApiKeyResult::GenerateApiKeyOutput(output)) => {
                        ctx.emit(CreateApiKeyModalEvent::Created { api_key: output.api_key });
                        me.request_state = RequestState::Succeeded;
                        me.raw_key_copied = false;
                        me.raw_key = Some(output.raw_api_key);
                        ctx.notify();
                    }
                    Ok(warp_graphql::mutations::generate_api_key::GenerateApiKeyResult::UserFacingError(e)) => {
                        let msg = warp_graphql::client::get_user_facing_error_message(e);
                        me.request_state = RequestState::Idle;
                        ctx.emit(CreateApiKeyModalEvent::Error { message: msg });
                        ctx.notify();
                    }
                    Ok(warp_graphql::mutations::generate_api_key::GenerateApiKeyResult::Unknown) | Err(_) => {
                        me.request_state = RequestState::Idle;
                        ctx.emit(CreateApiKeyModalEvent::Error { message: "Failed to create API key. Please try again.".to_string() });
                        ctx.notify();
                    }
                }
            },
        );
    }

    fn cancel(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(CreateApiKeyModalEvent::Close);
    }

    pub fn on_close(&mut self, ctx: &mut ViewContext<Self>) {
        self.request_state = RequestState::Idle;
        self.raw_key_copied = false;
        self.raw_key = None;
        self.selected_agent_uid = None;
        self.name_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
        });
    }

    pub fn on_open(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.name_editor);
        if self.has_named_agents {
            self.fetch_agents(ctx);
        }
    }

    fn update_has_team(&mut self, ctx: &mut ViewContext<Self>) {
        let new_has_team = FeatureFlag::TeamApiKeys.is_enabled()
            && UserWorkspaces::as_ref(ctx).current_team_uid().is_some();
        let new_has_named_agents = FeatureFlag::NamedAgents.is_enabled();

        if new_has_team != self.has_team || new_has_named_agents != self.has_named_agents {
            self.has_team = new_has_team;
            self.has_named_agents = new_has_named_agents;
            let options = if new_has_named_agents {
                vec![ApiKeyType::Personal, ApiKeyType::Agent]
            } else if new_has_team {
                vec![ApiKeyType::Personal, ApiKeyType::Team]
            } else {
                vec![ApiKeyType::Personal]
            };
            self.api_key_type_control
                .update(ctx, |control, ctx| control.update_options(options, ctx));
            ctx.notify();
        }
    }

    fn handle_name_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Enter => {
                self.create(ctx);
            }
            EditorEvent::Escape => {
                self.cancel(ctx);
            }
            EditorEvent::Edited(_) => {
                // Re-render when name field changes
                ctx.notify();
            }
            _ => {}
        }
    }

    fn render_success_content(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let button_style = UiComponentStyles {
            font_size: Some(14.),
            padding: Some(Coords::uniform(8.).left(12.).right(12.)),
            ..Default::default()
        };

        let info = Text::new(
            "This secret key is shown only once. Copy and store it securely.",
            appearance.ui_font_family(),
            LABEL_FONT_SIZE,
        )
        .with_color(theme.nonactive_ui_text_color().into())
        .finish();

        // Truncated display of the raw key (copy action uses full value)
        let raw_full = self.raw_key.as_deref().unwrap_or("");
        let display = truncate_from_end(raw_full, 37);
        let raw_key_view = Container::new(
            Text::new_inline(display, appearance.monospace_font_family(), 12.)
                .with_color(theme.active_ui_text_color().into())
                .finish(),
        )
        .with_border(Border::all(1.).with_border_fill(theme.outline()))
        .with_padding(Padding::uniform(8.))
        .with_background(theme.surface_2())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .finish();

        let copy_label = if self.raw_key_copied {
            "Copied"
        } else {
            "Copy"
        };
        let copy_icon = if self.raw_key_copied {
            warp_core::ui::icons::Icon::Check.to_warpui_icon(appearance.theme().background())
        } else {
            warp_core::ui::icons::Icon::Copy
                .to_warpui_icon(appearance.theme().active_ui_text_color())
        };
        let mut copy_button_builder = appearance
            .ui_builder()
            .button(
                if self.raw_key_copied {
                    ButtonVariant::Basic
                } else {
                    ButtonVariant::Outlined
                },
                self.create_button_mouse_state.clone(),
            )
            .with_text_and_icon_label(
                warpui::ui_components::button::TextAndIcon::new(
                    warpui::ui_components::button::TextAndIconAlignment::IconFirst,
                    copy_label,
                    copy_icon,
                    MainAxisSize::Min,
                    MainAxisAlignment::Center,
                    vec2f(14., 14.),
                )
                .with_inner_padding(4.),
            );
        if self.raw_key_copied {
            copy_button_builder = copy_button_builder.with_style(UiComponentStyles {
                background: Some(appearance.theme().ansi_fg_green().into()),
                font_color: Some(appearance.theme().background().into()),
                ..button_style
            });
        } else {
            copy_button_builder = copy_button_builder.with_style(button_style);
        }
        let copy_button = copy_button_builder
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(CreateApiKeyModalAction::CopyRawKey))
            .finish();

        let done_button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Accent,
                self.cancel_button_mouse_state.clone(),
            )
            .with_text_label("Done".to_string())
            .with_style(button_style)
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(CreateApiKeyModalAction::Cancel))
            .finish();

        Flex::column()
            .with_child(Container::new(info).with_margin_bottom(12.).finish())
            .with_child(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(Expanded::new(1., raw_key_view).finish())
                    .with_child(Container::new(copy_button).with_margin_left(8.).finish())
                    .finish(),
            )
            .with_child(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(Expanded::new(1., Empty::new().finish()).finish())
                        .with_child(done_button)
                        .finish(),
                )
                .with_margin_top(12.)
                .finish(),
            )
            .finish()
    }
}

impl Entity for CreateApiKeyModal {
    type Event = CreateApiKeyModalEvent;
}

impl View for CreateApiKeyModal {
    fn ui_name() -> &'static str {
        "CreateApiKeyModal"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let button_style = UiComponentStyles {
            font_size: Some(14.),
            padding: Some(Coords::uniform(8.).left(12.).right(12.)),
            ..Default::default()
        };

        match self.request_state {
            RequestState::Succeeded => self.render_success_content(app),
            _ => {
                let selected_key_type = self.api_key_type_control.as_ref(app).selected_option();

                let description_text = Text::new(
                    selected_key_type.description(),
                    appearance.ui_font_family(),
                    LABEL_FONT_SIZE,
                )
                .with_color(theme.nonactive_ui_text_color().into())
                .finish();

                let name_label = Text::new("Name", appearance.ui_font_family(), LABEL_FONT_SIZE)
                    .with_color(theme.active_ui_text_color().into())
                    .finish();

                let is_pending = self.request_state == RequestState::Pending;

                let is_create_disabled = is_pending
                    || (selected_key_type == ApiKeyType::Agent
                        && (self.selected_agent_uid.is_none() || self.is_loading_agents));

                let mut cancel_button_hover = appearance
                    .ui_builder()
                    .button(
                        ButtonVariant::Secondary,
                        self.cancel_button_mouse_state.clone(),
                    )
                    .with_text_label("Cancel".to_string())
                    .with_style(button_style)
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(CreateApiKeyModalAction::Cancel);
                    });
                if is_pending {
                    cancel_button_hover = cancel_button_hover.disable();
                }
                let cancel_button = cancel_button_hover.finish();

                let mut create_button_hover = appearance
                    .ui_builder()
                    .button(
                        ButtonVariant::Accent,
                        self.create_button_mouse_state.clone(),
                    )
                    .with_text_label(if is_pending {
                        "Creating…".to_string()
                    } else {
                        "Create key".to_string()
                    })
                    .with_style(button_style)
                    .build()
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(CreateApiKeyModalAction::Create);
                    });
                if is_create_disabled {
                    create_button_hover = create_button_hover.disable();
                }
                let create_button = create_button_hover.finish();

                let buttons_row = Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(Expanded::new(1., Empty::new().finish()).finish())
                        .with_child(cancel_button)
                        .with_child(Container::new(create_button).with_margin_left(12.).finish())
                        .finish(),
                )
                .with_margin_top(12.)
                .finish();

                let mut col = Flex::column();

                if self.has_team || self.has_named_agents {
                    let type_label =
                        Text::new("Type", appearance.ui_font_family(), LABEL_FONT_SIZE)
                            .with_color(theme.active_ui_text_color().into())
                            .finish();
                    col.add_child(Container::new(type_label).with_margin_bottom(4.).finish());
                    col.add_child(
                        Container::new(ChildView::new(&self.api_key_type_control).finish())
                            .with_margin_bottom(16.)
                            .finish(),
                    );
                }

                col.add_child(
                    Container::new(description_text)
                        .with_margin_bottom(24.)
                        .finish(),
                );

                if selected_key_type == ApiKeyType::Agent {
                    let agent_label =
                        Text::new("Agent", appearance.ui_font_family(), LABEL_FONT_SIZE)
                            .with_color(theme.active_ui_text_color().into())
                            .finish();
                    col.add_child(Container::new(agent_label).with_margin_bottom(4.).finish());

                    let available_agents: Vec<&AgentIdentity> =
                        self.agents.iter().filter(|a| a.available).collect();

                    if !self.is_loading_agents && available_agents.is_empty() {
                        let empty_text = Text::new(
                            "No agents available. Create one first.",
                            appearance.ui_font_family(),
                            LABEL_FONT_SIZE,
                        )
                        .with_color(theme.nonactive_ui_text_color().into())
                        .finish();

                        let create_agent_button = appearance
                            .ui_builder()
                            .button(
                                ButtonVariant::Secondary,
                                self.create_agent_button_mouse_state.clone(),
                            )
                            .with_text_label("Create agent".to_string())
                            .with_style(button_style)
                            .build()
                            .on_click(|ctx, _, _| {
                                ctx.dispatch_typed_action(CreateApiKeyModalAction::CreateNewAgent);
                            })
                            .finish();

                        col.add_child(
                            Container::new(
                                Flex::column()
                                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                    .with_child(
                                        Container::new(empty_text).with_margin_bottom(8.).finish(),
                                    )
                                    .with_child(create_agent_button)
                                    .finish(),
                            )
                            .with_border(Border::all(1.).with_border_fill(theme.outline()))
                            .with_padding(Padding::uniform(16.))
                            .with_background(theme.surface_2())
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                            .with_margin_bottom(16.)
                            .finish(),
                        );
                    } else {
                        col.add_child(
                            ConstrainedBox::new(
                                Container::new(ChildView::new(&self.agent_dropdown).finish())
                                    .with_margin_bottom(16.)
                                    .finish(),
                            )
                            .with_width(INPUT_WIDTH)
                            .finish(),
                        );
                    }
                }

                col.add_child(Container::new(name_label).with_margin_bottom(4.).finish());
                col.add_child(
                    ConstrainedBox::new(
                        Container::new(ChildView::new(&self.name_editor).finish())
                            .with_border(Border::all(1.).with_border_fill(theme.outline()))
                            .with_padding(Padding::uniform(4.))
                            .with_background(theme.surface_2())
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                            .finish(),
                    )
                    .with_width(INPUT_WIDTH)
                    .finish(),
                );

                let expiration_label =
                    Text::new("Expiration", appearance.ui_font_family(), LABEL_FONT_SIZE)
                        .with_color(theme.active_ui_text_color().into())
                        .finish();

                col.add_child(
                    Container::new(expiration_label)
                        .with_margin_bottom(4.)
                        .with_margin_top(16.)
                        .finish(),
                );
                col.add_child(
                    ConstrainedBox::new(
                        Container::new(ChildView::new(&self.expiration_dropdown).finish())
                            .with_margin_bottom(24.)
                            .finish(),
                    )
                    .with_width(INPUT_WIDTH)
                    .finish(),
                );

                col.add_child(buttons_row);
                col.finish()
            }
        }
    }
}

impl TypedActionView for CreateApiKeyModal {
    type Action = CreateApiKeyModalAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CreateApiKeyModalAction::Cancel => self.cancel(ctx),
            CreateApiKeyModalAction::Create => self.create(ctx),
            CreateApiKeyModalAction::CopyRawKey => {
                let content = self.raw_key.clone().unwrap_or_default();
                ctx.clipboard()
                    .write(warpui::clipboard::ClipboardContent::plain_text(content));
                self.raw_key_copied = true;
                // Success toast
                let window_id = ctx.window_id();
                crate::ToastStack::handle(ctx).update(ctx, |toast_stack, ctx| {
                    let toast = crate::view_components::DismissibleToast::success(
                        "Secret key copied.".to_string(),
                    );
                    toast_stack.add_ephemeral_toast(toast, window_id, ctx);
                });
                ctx.notify();
            }
            CreateApiKeyModalAction::SetExpiration(exp) => {
                // The dropdown component already updates its own selection in response to the
                // menu click; attempting to re-set the selection here causes a circular update.
                self.expiration = *exp;
                ctx.notify();
            }
            CreateApiKeyModalAction::SelectAgent(uid) => {
                self.selected_agent_uid = Some(uid.clone());
                ctx.notify();
            }
            CreateApiKeyModalAction::CreateNewAgent => {
                ctx.open_url(OZ_AGENTS_URL);
            }
        }
    }
}

pub struct CreateApiKeyModalViewState {
    state: ModalViewState<Modal<CreateApiKeyModal>>,
}

impl CreateApiKeyModalViewState {
    pub fn new(state: ModalViewState<Modal<CreateApiKeyModal>>) -> Self {
        Self { state }
    }

    pub fn is_open(&self) -> bool {
        self.state.is_open()
    }

    pub fn render(&self) -> Box<dyn Element> {
        self.state.render()
    }

    pub fn open<T: View>(&mut self, ctx: &mut ViewContext<T>) {
        self.state.open();
        self.state.view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.on_open(ctx);
            });
        });
    }

    pub fn set_title<T: View>(&mut self, title: Option<String>, ctx: &mut ViewContext<T>) {
        self.state.view.update(ctx, |modal, ctx| {
            modal.set_title(title);
            ctx.notify();
        });
        ctx.notify();
    }

    pub fn close<T: View>(&mut self, ctx: &mut ViewContext<T>) {
        self.state.close();
        self.state.view.update(ctx, |modal, ctx| {
            modal.body().update(ctx, |body, ctx| {
                body.on_close(ctx);
            });
        });
    }
}

fn api_key_type_control_styles(app: &AppContext) -> UiComponentStyles {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    UiComponentStyles {
        font_family_id: Some(appearance.ui_font_family()),
        font_size: Some(appearance.ui_font_size()),
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.0))),
        border_width: Some(1.0),
        border_color: Some(Fill::Solid(theme.outline().into())),
        background: Some(Fill::Solid(theme.surface_2().into())),
        height: Some(24.0),
        padding: Some(Coords::uniform(2.0)),
        ..Default::default()
    }
}
