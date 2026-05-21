use itertools::Itertools as _;
use markdown_parser::{parse_markdown, FormattedText, FormattedTextFragment, FormattedTextLine};
use parking_lot::FairMutex;
use settings::Setting;
use std::{borrow::Cow, cmp::Reverse, path::Path, sync::Arc};
use warp_core::{features::FeatureFlag, report_if_error, ui::Icon};
use warpui::{
    elements::{
        Clipped, Container, CornerRadius, CrossAxisAlignment, Flex, FormattedTextElement,
        HighlightedHyperlink, MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable,
        Text,
    },
    fonts::{Properties, Weight},
    keymap::Keystroke,
    prelude::{Align, ConstrainedBox, Cursor, Empty, Hoverable, MainAxisAlignment, SavePosition},
    scene::Border,
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::{
    ai::{
        active_agent_views_model::{ActiveAgentViewsModel, ConversationOrTaskId},
        agent::conversation::AIConversationId,
        blocklist::{
            agent_view::{
                agent_view_bg_color, AgentViewController, AgentViewEntryOrigin,
                ENTER_AGENT_VIEW_NEW_CONVERSATION_KEYSTROKE,
                ENTER_CLOUD_AGENT_VIEW_NEW_CONVERSATION_KEYSTROKE,
            },
            history_model::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel},
        },
        conversation_navigation::ConversationNavigationData,
    },
    appearance::Appearance,
    changelog_model::{self, ChangelogModel},
    settings::{AISettings, AISettingsChangedEvent},
    terminal::{
        self,
        event::BlockType,
        input::message_bar::{common::render_standard_message, Message, MessageItem},
        model::{
            blocks::BlockHeightItem,
            session::{BootstrapSessionType, Session, SessionType, Sessions},
        },
        model_events::{AnsiHandlerEvent, ModelEvent, ModelEventDispatcher},
        prompt,
        view::{
            ambient_agent::{AmbientAgentViewModel, AmbientAgentViewModelEvent},
            TerminalAction,
        },
        TerminalModel,
    },
    util::time_format::format_approx_duration_from_now_utc,
};

const CLOUD_AGENT_DOCS_URL: &str = "https://docs.warp.dev/agent-platform/cloud-agents/overview";
const OZ_UPDATES_SECTION_HEADER: &str = "What's new in Oz";

// The maximum number of Oz updates from the changelog rendered in-line in the 'What's new in Oz section'.
const MAX_OZ_UPDATE_COUNT: usize = 4;

const MAX_RECENT_CONVERSATION_COUNT: usize = 3;

#[derive(Default)]
struct StateHandles {
    start_new_conversation: MouseStateHandle,
    start_cloud_conversation: MouseStateHandle,
    switch_model: MouseStateHandle,
    exit: MouseStateHandle,
    init_callout: MouseStateHandle,
    oz_updates: MouseStateHandle,
    changelog_link: MouseStateHandle,
    recent_conversations: [MouseStateHandle; MAX_RECENT_CONVERSATION_COUNT],
    update_hyperlinks: Vec<HighlightedHyperlink>,
}

/// Zero state view shown when agent view is active but the conversation has no exchanges yet.
pub struct AgentViewZeroStateBlock {
    conversation_id: AIConversationId,
    origin: AgentViewEntryOrigin,
    agent_view_controller: ModelHandle<AgentViewController>,
    sessions: ModelHandle<Sessions>,
    terminal_model: Arc<FairMutex<TerminalModel>>,
    current_working_directory: Option<String>,
    cached_recent_conversations: Vec<ConversationNavigationData>,
    should_hide: bool,
    should_show_init_callout: bool,
    has_parent_terminal: bool,
    state_handles: StateHandles,
    is_oz_updates_expanded: bool,
}

impl AgentViewZeroStateBlock {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conversation_id: AIConversationId,
        origin: AgentViewEntryOrigin,
        agent_view_controller: ModelHandle<AgentViewController>,
        sessions: &ModelHandle<Sessions>,
        cloud_agent_view_model: Option<&ModelHandle<AmbientAgentViewModel>>,
        terminal_model: Arc<FairMutex<TerminalModel>>,
        model_events_dispatcher: &ModelHandle<ModelEventDispatcher>,
        should_show_init_callout: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let cloud_agent_view_model_clone = cloud_agent_view_model.cloned();

        let model_events_clone = model_events_dispatcher.clone();
        ctx.subscribe_to_model(
            &BlocklistAIHistoryModel::handle(ctx),
            move |me, history_model, event, ctx| {
                if let BlocklistAIHistoryEvent::AppendedExchange {
                    conversation_id, ..
                } = event
                {
                    if *conversation_id == me.conversation_id {
                        me.should_hide = true;
                        ctx.unsubscribe_to_model(&model_events_clone);
                        ctx.unsubscribe_to_model(&history_model);
                        if let Some(cloud_agent_view_model) = cloud_agent_view_model_clone.as_ref()
                        {
                            ctx.unsubscribe_to_model(cloud_agent_view_model);
                        }
                        ctx.notify();
                        return;
                    }
                }

                match event {
                    BlocklistAIHistoryEvent::StartedNewConversation { .. }
                    | BlocklistAIHistoryEvent::AppendedExchange { .. }
                    | BlocklistAIHistoryEvent::UpdatedStreamingExchange { .. }
                    | BlocklistAIHistoryEvent::UpdatedConversationStatus { .. }
                    | BlocklistAIHistoryEvent::RestoredConversations { .. }
                    | BlocklistAIHistoryEvent::RemoveConversation { .. }
                    | BlocklistAIHistoryEvent::DeletedConversation { .. } => ctx.notify(),
                    _ => {}
                }
            },
        );

        let active_agent_views_model = ActiveAgentViewsModel::handle(ctx);
        ctx.subscribe_to_model(&active_agent_views_model, |_, _, _, ctx| {
            ctx.notify();
        });

        let cloud_agent_view_model_clone = cloud_agent_view_model.cloned();
        ctx.subscribe_to_model(
            model_events_dispatcher,
            move |me, model_events_dispatcher, event, ctx| {
                match event {
                    ModelEvent::BlockCompleted(block_completed) => {
                        if matches!(block_completed.block_type, BlockType::User(..))
                            && me.should_hide != me.should_hide(ctx)
                        {
                            me.should_hide = true;
                            ctx.unsubscribe_to_model(&model_events_dispatcher);
                            ctx.unsubscribe_to_model(&BlocklistAIHistoryModel::handle(ctx));
                            if let Some(cloud_agent_view_model) =
                                cloud_agent_view_model_clone.as_ref()
                            {
                                ctx.unsubscribe_to_model(cloud_agent_view_model);
                            }
                            ctx.notify();
                        }
                    }
                    ModelEvent::Handler(AnsiHandlerEvent::Bootstrapped { .. }) => {
                        // Session metadata such as pwd can be unavailable in zero-state until
                        // bootstrap completes; refresh once bootstrapped.
                        ctx.notify();
                    }
                    _ => {}
                }
            },
        );

        if let Some(cloud_agent_view_model) = cloud_agent_view_model {
            let model_events_clone = model_events_dispatcher.clone();
            ctx.subscribe_to_model(cloud_agent_view_model, move |me, model, event, ctx| {
                if me.should_hide {
                    return;
                }

                // Hide the zero state when this pane becomes a local-to-cloud handoff
                // pane (REMOTE-1486). The fresh cloud-mode banner is suppressed because
                // the pane is actually pre-loaded with a forked source conversation, not
                // a brand-new one.
                if matches!(event, AmbientAgentViewModelEvent::PendingHandoffChanged)
                    && model.as_ref(ctx).is_local_to_cloud_handoff()
                {
                    me.should_hide = true;
                } else if FeatureFlag::CloudModeSetupV2.is_enabled() {
                    if matches!(
                        event,
                        AmbientAgentViewModelEvent::DispatchedAgent
                            | AmbientAgentViewModelEvent::Cancelled
                    ) {
                        me.should_hide = true;
                    }
                } else if model.as_ref(ctx).should_show_status_footer() {
                    me.should_hide = true;
                }

                if me.should_hide {
                    ctx.unsubscribe_to_model(&model);
                    ctx.unsubscribe_to_model(&model_events_clone);
                    ctx.unsubscribe_to_model(&BlocklistAIHistoryModel::handle(ctx));
                    ctx.notify();
                }
            });
        }

        let has_parent_terminal =
            cloud_agent_view_model.is_none_or(|model| !model.as_ref(ctx).is_ambient_agent());
        let is_local_to_cloud_handoff = cloud_agent_view_model
            .is_some_and(|model| model.as_ref(ctx).is_local_to_cloud_handoff());
        let changelog_model = ChangelogModel::handle(ctx);
        ctx.subscribe_to_model(&changelog_model, |me, changelog_model, event, ctx| {
            if let changelog_model::Event::ChangelogRequestComplete { .. } = event {
                let oz_update_count = changelog_model
                    .as_ref(ctx)
                    .oz_updates
                    .len()
                    .min(MAX_OZ_UPDATE_COUNT);
                if oz_update_count != me.state_handles.update_hyperlinks.len() {
                    me.state_handles
                        .update_hyperlinks
                        .resize(oz_update_count, Default::default());
                }
            }
        });
        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, event, ctx| {
            let should_rerender_for_oz_updates_visibility = !me.origin.is_cloud_agent()
                && matches!(
                    event,
                    AISettingsChangedEvent::ShouldShowOzUpdatesInZeroState { .. }
                )
                && FeatureFlag::OzChangelogUpdates.is_enabled()
                && !ChangelogModel::as_ref(ctx).oz_updates.is_empty();
            if should_rerender_for_oz_updates_visibility {
                ctx.notify();
            }
        });

        let mut state_handles = StateHandles::default();
        state_handles.update_hyperlinks.resize(
            changelog_model
                .as_ref(ctx)
                .oz_updates
                .len()
                .min(MAX_OZ_UPDATE_COUNT),
            Default::default(),
        );
        let current_working_directory = {
            let terminal_model = terminal_model.lock();
            current_working_directory_for_zero_state(&terminal_model)
        };
        let cached_recent_conversations = current_working_directory
            .as_deref()
            .map(|current_working_directory| {
                Self::recent_conversations_for_working_directory(current_working_directory, ctx)
            })
            .unwrap_or_default();

        Self {
            conversation_id,
            origin,
            agent_view_controller,
            sessions: sessions.clone(),
            terminal_model,
            current_working_directory,
            cached_recent_conversations,
            should_hide: matches!(origin, AgentViewEntryOrigin::AcceptedPassiveCodeDiff)
                || is_local_to_cloud_handoff,
            should_show_init_callout,
            has_parent_terminal,
            state_handles,
            is_oz_updates_expanded: !origin.is_cloud_agent()
                && *AISettings::handle(ctx).as_ref(ctx).should_expand_oz_updates,
        }
    }

    fn should_hide(&self, app: &AppContext) -> bool {
        let Some(conversation_id) = self
            .agent_view_controller
            .as_ref(app)
            .agent_view_state()
            .active_conversation_id()
        else {
            return true;
        };

        if conversation_id != self.conversation_id {
            return true;
        }

        // Don't show zero state if there are visible content items (blocks, rich content)
        // in the agent view. Inline banners alone should not prevent the zero state.
        let has_visible_content = self
            .terminal_model
            .lock()
            .block_list()
            .has_visible_block_height_item_where(|item| {
                !matches!(
                    item,
                    BlockHeightItem::InlineBanner { .. } | BlockHeightItem::Gap(..)
                )
            });

        BlocklistAIHistoryModel::handle(app)
            .as_ref(app)
            .conversation(&conversation_id)
            .is_none_or(|conv| conv.exchange_count() > 0)
            || has_visible_content
    }

    fn active_session(&self, app: &AppContext) -> Option<Arc<Session>> {
        let session_id = self
            .terminal_model
            .lock()
            .block_list()
            .active_block()
            .session_id()?;
        self.sessions.as_ref(app).get(session_id)
    }

    /// Returns the save position ID for this view, used to position overlays.
    fn save_position_id(&self, app: &AppContext) -> Option<String> {
        self.agent_view_controller
            .as_ref(app)
            .agent_view_state()
            .zero_state_position_id()
    }

    fn recent_conversations_for_working_directory(
        current_working_directory: &str,
        app: &AppContext,
    ) -> Vec<ConversationNavigationData> {
        let open_conversation_ids = ActiveAgentViewsModel::as_ref(app)
            .get_all_open_conversation_ids(app)
            .iter()
            .filter_map(ConversationOrTaskId::conversation_id)
            .collect::<std::collections::HashSet<_>>();
        ConversationNavigationData::all_conversations(app)
            .into_iter()
            .filter(|conversation_data| {
                if open_conversation_ids.contains(&conversation_data.id) {
                    return false;
                }

                match conversation_data.latest_working_directory.as_ref() {
                    Some(latest_working_directory) => {
                        latest_working_directory == current_working_directory
                    }
                    None => conversation_data
                        .initial_working_directory
                        .as_ref()
                        .is_some_and(|initial_working_directory| {
                            initial_working_directory == current_working_directory
                        }),
                }
            })
            .sorted_by_key(|conversation_data| Reverse(conversation_data.last_updated))
            .take(MAX_RECENT_CONVERSATION_COUNT)
            .collect()
    }
}

fn format_session_location(session: &Session, working_directory: Option<&str>) -> Option<String> {
    let display_path = display_working_directory(working_directory, session.home_dir())?;
    let session_type = session.session_type();
    let user = session.user();
    let hostname = session.hostname();
    match session_type {
        SessionType::Local => Some(display_path),
        SessionType::WarpifiedRemote { .. } => Some(format!("{user}@{hostname}:{display_path}")),
    }
}

fn display_working_directory(
    working_directory: Option<&str>,
    home_dir: Option<&str>,
) -> Option<String> {
    let working_directory = working_directory?.to_owned();
    let home_dir = home_dir
        .map(ToOwned::to_owned)
        .or_else(|| dirs::home_dir().and_then(|home_dir| home_dir.to_str().map(ToOwned::to_owned)));
    Some(prompt::display_path_string(
        Some(&working_directory),
        home_dir.as_deref(),
    ))
}

impl View for AgentViewZeroStateBlock {
    fn ui_name() -> &'static str {
        "AgentViewZeroState"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if self.should_hide {
            return Empty::new().finish();
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let header_props = if self.origin.is_cloud_agent() {
            HeaderProps {
                title: "New Oz cloud agent conversation".into(),
                description: AgentViewDescription::CloudModeWithDocsLink,
                icon: Icon::OzCloud,
            }
        } else {
            let mut local_description =
                "Send a prompt below to start a new conversation".to_owned();
            let active_session = self.active_session(app);
            let location_label = active_session.as_deref().and_then(|session| {
                format_session_location(session, self.current_working_directory.as_deref())
            });
            if let Some(location_label) = location_label {
                local_description += &format!(" in `{location_label}`");
            }

            HeaderProps {
                title: "New Oz agent conversation".into(),
                description: AgentViewDescription::PlainText(vec![local_description.into()]),
                icon: Icon::Oz,
            }
        };

        let mut content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_children(render_title_and_description(header_props, app));

        if !self.origin.is_cloud_agent() {
            if let Some(oz_updates_section) = render_oz_updates(
                OzUpdatesProps {
                    is_expanded: self.is_oz_updates_expanded,
                    state_handles: &self.state_handles,
                },
                app,
            ) {
                content.add_children([Container::new(oz_updates_section)
                    .with_margin_top(8.)
                    .with_margin_bottom(16.)
                    .finish()]);
            }
        }

        let active_session = self.active_session(app);
        let body = render_body(
            ZeroStateBodyProps {
                origin: self.origin,
                has_parent_terminal: self.has_parent_terminal,
                should_show_init_callout: self.should_show_init_callout,
                recent_conversations: &self.cached_recent_conversations,
                active_session: active_session.as_deref(),
                current_working_directory: self.current_working_directory.as_deref(),
                state_handles: &self.state_handles,
            },
            app,
        );
        let body_item_count = body.len();
        content.add_children(body.into_iter().enumerate().map(|(i, item)| {
            if i == body_item_count - 1 {
                item
            } else {
                Container::new(item).with_margin_bottom(8.).finish()
            }
        }));
        let content = content.finish();

        let show_bottom_border = !self.origin.is_cloud_agent();
        let content = Container::new(content)
            .with_horizontal_padding(*terminal::view::PADDING_LEFT)
            .with_vertical_padding(styles::CONTAINER_VERTICAL_PADDING)
            .with_border(
                Border::new(1.)
                    .with_sides(true, false, show_bottom_border, false)
                    .with_border_fill(theme.outline()),
            )
            .finish();

        if let Some(save_position_id) = self.save_position_id(app) {
            SavePosition::new(content, &save_position_id).finish()
        } else {
            content
        }
    }
}

#[derive(Debug, Clone)]
pub enum AgentViewZeroStateEvent {
    ClickedInitCallout,
    OpenConversation { conversation_id: AIConversationId },
}

impl Entity for AgentViewZeroStateBlock {
    type Event = AgentViewZeroStateEvent;
}

#[derive(Debug, Clone)]
pub enum AgentViewZeroStateAction {
    ClickedInitCallout,
    ToggleOzUpdates,
    OpenConversation { conversation_id: AIConversationId },
}

impl TypedActionView for AgentViewZeroStateBlock {
    type Action = AgentViewZeroStateAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AgentViewZeroStateAction::ClickedInitCallout => {
                ctx.emit(AgentViewZeroStateEvent::ClickedInitCallout);
            }
            AgentViewZeroStateAction::ToggleOzUpdates => {
                let is_expanded = self.is_oz_updates_expanded;
                self.is_oz_updates_expanded = !is_expanded;

                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    report_if_error!(settings
                        .should_expand_oz_updates
                        .set_value(!is_expanded, ctx));
                });
            }
            AgentViewZeroStateAction::OpenConversation { conversation_id } => {
                ctx.emit(AgentViewZeroStateEvent::OpenConversation {
                    conversation_id: *conversation_id,
                });
            }
        }
    }
}

fn current_working_directory_for_zero_state(terminal_model: &TerminalModel) -> Option<String> {
    terminal_model
        .block_list()
        .active_block()
        .pwd()
        .cloned()
        .or_else(|| {
            let is_bootstrapping_remote_shell = terminal_model.has_pending_ssh_session()
                || terminal_model
                    .get_pending_session_info()
                    .as_ref()
                    .is_some_and(|pending_session_info| {
                        matches!(
                            pending_session_info.session_type,
                            BootstrapSessionType::WarpifiedRemote
                        )
                    });
            (!terminal_model.block_list().is_bootstrapped() && !is_bootstrapping_remote_shell)
                .then(|| {
                    terminal_model
                        .session_startup_path()
                        .map(|path| path.to_string_lossy().into_owned())
                })
                .flatten()
        })
}

/// Describes the description content for the header.
enum AgentViewDescription {
    /// Plain text descriptions (used for local agent mode).
    PlainText(Vec<Cow<'static, str>>),
    /// Cloud mode description with "Visit docs" hyperlink.
    CloudModeWithDocsLink,
}

struct HeaderProps {
    title: Cow<'static, str>,
    description: AgentViewDescription,
    icon: Icon,
}

fn render_title_and_description(props: HeaderProps, app: &AppContext) -> Vec<Box<dyn Element>> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let HeaderProps {
        title,
        description,
        icon,
    } = props;

    let title_font_size = styles::title_font_size(appearance);
    let title = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Container::new(
                ConstrainedBox::new(
                    icon.to_warpui_icon(
                        theme
                            .main_text_color(theme.background())
                            .into_solid()
                            .into(),
                    )
                    .finish(),
                )
                .with_height(title_font_size)
                .with_width(title_font_size)
                .finish(),
            )
            .with_margin_right(8.)
            .finish(),
        )
        .with_child(
            Text::new(title, appearance.ui_font_family(), title_font_size)
                .with_color(theme.main_text_color(theme.background()).into_solid())
                .with_style(Properties::default().weight(Weight::Bold))
                .finish(),
        )
        .finish();

    let mut items = vec![];
    items.push(
        Container::new(title)
            .with_margin_bottom(styles::TITLE_MARGIN_BOTTOM)
            .finish(),
    );

    let bg = agent_view_bg_color(app);
    let sub_text_color = theme.sub_text_color(bg.into()).into_solid();
    let main_text_color = theme.main_text_color(bg.into()).into_solid();

    match description {
        AgentViewDescription::PlainText(text_items) => {
            let description_items = text_items.into_iter().map(|description_item| {
                FormattedTextElement::new(
                    parse_markdown(&description_item).expect("is valid markdown"),
                    appearance.monospace_font_size(),
                    appearance.ui_font_family(),
                    appearance.ui_font_family(),
                    sub_text_color,
                    Default::default(),
                )
                .with_inline_code_properties(Some(main_text_color), None)
                .finish()
            });
            items.extend(description_items.map(|rendered_item| {
                Container::new(rendered_item)
                    .with_margin_bottom(styles::TITLE_MARGIN_BOTTOM)
                    .finish()
            }));
        }
        AgentViewDescription::CloudModeWithDocsLink => {
            // First line: plain text.
            items.push(
                Container::new(
                    Text::new(
                        "Run your agent task in an isolated cloud environment.",
                        appearance.ui_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(sub_text_color)
                    .finish(),
                )
                .with_margin_bottom(styles::DESCRIPTION_LINE_MARGIN_BOTTOM)
                .finish(),
            );

            // Second line: text with "Visit docs" hyperlink.
            let description_with_link = FormattedText::new([FormattedTextLine::Line(vec![
                FormattedTextFragment::plain_text(
                    "Use cloud agents to run parallel agents, build agents that run autonomously, and check in on your agents from anywhere. ",
                ),
                FormattedTextFragment::hyperlink("Visit docs", CLOUD_AGENT_DOCS_URL),
            ])]);

            items.push(
                Container::new(
                    FormattedTextElement::new(
                        description_with_link,
                        appearance.monospace_font_size(),
                        appearance.ui_font_family(),
                        appearance.monospace_font_family(),
                        sub_text_color,
                        HighlightedHyperlink::default(),
                    )
                    .with_hyperlink_font_color(theme.accent().into_solid())
                    .register_default_click_handlers(|url, _, ctx| {
                        ctx.open_url(&url.url);
                    })
                    .finish(),
                )
                .with_margin_bottom(-12.)
                .finish(),
            );
        }
    }

    items
}

struct ZeroStateBodyProps<'a> {
    origin: AgentViewEntryOrigin,
    has_parent_terminal: bool,
    should_show_init_callout: bool,
    recent_conversations: &'a [ConversationNavigationData],
    active_session: Option<&'a Session>,
    current_working_directory: Option<&'a str>,
    state_handles: &'a StateHandles,
}

fn render_body(props: ZeroStateBodyProps<'_>, app: &AppContext) -> Vec<Box<dyn Element>> {
    let ZeroStateBodyProps {
        origin,
        has_parent_terminal,
        should_show_init_callout,
        recent_conversations,
        active_session,
        current_working_directory,
        state_handles,
    } = props;

    // Cloud agent mode doesn't show keyboard shortcuts.
    if origin.is_cloud_agent() {
        return vec![];
    }
    let mut body_items = if let Some(recent_conversations_section) =
        render_recent_conversations_section(
            RecentConversationProps {
                recent_conversations,
                active_session,
                current_working_directory,
                state_handles,
            },
            app,
        ) {
        vec![recent_conversations_section]
    } else {
        let mut body_items = vec![
            render_standard_message(
                Message::new(vec![MessageItem::clickable(
                    vec![
                        MessageItem::keystroke(ENTER_AGENT_VIEW_NEW_CONVERSATION_KEYSTROKE.clone()),
                        MessageItem::text("start a new agent conversation"),
                    ],
                    |ctx| {
                        ctx.dispatch_typed_action(TerminalAction::StartNewAgentConversation);
                    },
                    state_handles.start_new_conversation.clone(),
                )]),
                app,
            ),
            render_standard_message(
                Message::new(vec![MessageItem::clickable(
                    vec![
                        MessageItem::keystroke(
                            ENTER_CLOUD_AGENT_VIEW_NEW_CONVERSATION_KEYSTROKE.clone(),
                        ),
                        MessageItem::text("start a new cloud agent conversation"),
                    ],
                    |ctx| {
                        ctx.dispatch_typed_action(TerminalAction::EnterCloudAgentView);
                    },
                    state_handles.start_cloud_conversation.clone(),
                )]),
                app,
            ),
            render_standard_message(
                Message::new(vec![MessageItem::clickable(
                    vec![
                        MessageItem::keystroke(Keystroke {
                            key: "/model".to_owned(),
                            ..Default::default()
                        }),
                        MessageItem::text("switch model"),
                    ],
                    |ctx| {
                        ctx.dispatch_typed_action(TerminalAction::OpenModelSelector);
                    },
                    state_handles.switch_model.clone(),
                )]),
                app,
            ),
        ];

        // Only show "escape to go back" if there's a parent terminal
        if has_parent_terminal {
            body_items.push(render_standard_message(
                Message::new(vec![MessageItem::clickable(
                    vec![
                        MessageItem::keystroke(Keystroke {
                            key: "escape".to_owned(),
                            ..Default::default()
                        }),
                        MessageItem::text("go back to terminal"),
                    ],
                    |ctx| {
                        ctx.dispatch_typed_action(TerminalAction::ExitAgentView);
                    },
                    state_handles.exit.clone(),
                )]),
                app,
            ));
        }

        body_items
    };

    if should_show_init_callout {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let main_text_color = theme
            .main_text_color(agent_view_bg_color(app).into())
            .into_solid();
        let init_message = Message::new(vec![
            MessageItem::keystroke(Keystroke {
                key: "/init".to_owned(),
                ..Default::default()
            }),
            MessageItem::text(
                "to index this codebase and generate an AGENTS.md for optimal performance",
            ),
        ])
        .with_text_color(main_text_color);
        body_items.push(
            Hoverable::new(state_handles.init_callout.clone(), move |_| {
                Container::new(render_standard_message(init_message.clone(), app))
                    .with_background_color(
                        theme
                            .accent()
                            .with_opacity(12)
                            .into_solid_bias_right_color(),
                    )
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .with_vertical_padding(4.)
                    .with_horizontal_padding(4.)
                    .finish()
            })
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(AgentViewZeroStateAction::ClickedInitCallout);
            })
            .with_cursor(Cursor::PointingHand)
            .finish(),
        );
    }

    body_items
}

struct RecentConversationProps<'a> {
    recent_conversations: &'a [ConversationNavigationData],
    active_session: Option<&'a Session>,
    current_working_directory: Option<&'a str>,
    state_handles: &'a StateHandles,
}

fn render_recent_conversations_section(
    props: RecentConversationProps<'_>,
    app: &AppContext,
) -> Option<Box<dyn Element>> {
    let RecentConversationProps {
        recent_conversations,
        active_session,
        current_working_directory,
        state_handles,
    } = props;

    if recent_conversations.is_empty() {
        return None;
    }

    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let last_path_segment = current_working_directory
        .and_then(|working_directory| {
            display_working_directory(
                Some(working_directory),
                active_session.and_then(Session::home_dir),
            )
        })
        .as_deref()
        .and_then(|working_directory| Path::new(working_directory).iter().next_back())
        .map(|path_segment| path_segment.to_string_lossy().into_owned())?;
    let disabled_text_color = theme.disabled_text_color(theme.background()).into_solid();
    let header_font_size = appearance.monospace_font_size() - 4.;
    let section_header = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Container::new(
                Text::new(
                    "RECENT ACTIVITY",
                    appearance.ui_font_family(),
                    header_font_size,
                )
                .with_color(disabled_text_color)
                .with_style(Properties::default().weight(Weight::Semibold))
                .finish(),
            )
            .with_margin_right(6.)
            .finish(),
        )
        .with_child(
            Container::new(
                Text::new(
                    last_path_segment,
                    appearance.ui_font_family(),
                    header_font_size + 1.,
                )
                .with_color(disabled_text_color)
                .finish(),
            )
            .with_background_color(theme.surface_overlay_1().into_solid())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
            .with_horizontal_padding(2.)
            .with_vertical_padding(1.)
            .finish(),
        )
        .finish();
    let mut conversations = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(8.);

    for (i, recent_conversation) in recent_conversations.iter().enumerate() {
        let conversation_id = recent_conversation.id;
        let title = recent_conversation.title.clone();
        let last_updated = recent_conversation.last_updated;

        let row = Hoverable::new(
            state_handles.recent_conversations[i].clone(),
            move |state| {
                let (title_text_color, secondary_text_color) = if state.is_hovered() {
                    (
                        theme.accent().into_solid(),
                        theme.accent().with_opacity(80).into_solid(),
                    )
                } else {
                    (
                        theme.main_text_color(theme.background()).into_solid(),
                        theme.disabled_text_color(theme.background()).into_solid(),
                    )
                };

                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_children([
                        Container::new(
                            Text::new_inline(
                                title.clone(),
                                appearance.ui_font_family(),
                                appearance.monospace_font_size() - 2.,
                            )
                            .with_color(title_text_color)
                            .soft_wrap(false)
                            .finish(),
                        )
                        .with_margin_right(8.)
                        .finish(),
                        Text::new_inline(
                            format_approx_duration_from_now_utc(last_updated.to_utc()),
                            appearance.ui_font_family(),
                            appearance.monospace_font_size() - 1.,
                        )
                        .with_color(secondary_text_color)
                        .soft_wrap(false)
                        .finish(),
                    ])
                    .finish()
            },
        )
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(AgentViewZeroStateAction::OpenConversation {
                conversation_id,
            });
        })
        .with_cursor(Cursor::PointingHand)
        .finish();

        conversations = conversations.with_child(row);
    }

    Some(
        Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(
                Container::new(section_header)
                    .with_margin_bottom(styles::SECTION_HEADER_MARGIN_BOTTOM)
                    .finish(),
            )
            .with_child(conversations.finish())
            .finish(),
    )
}

struct OzUpdatesProps<'a> {
    is_expanded: bool,
    state_handles: &'a StateHandles,
}
fn should_render_oz_updates_section(
    is_oz_changelog_updates_enabled: bool,
    should_show_oz_updates: bool,
    has_oz_updates: bool,
) -> bool {
    is_oz_changelog_updates_enabled && should_show_oz_updates && has_oz_updates
}

fn render_oz_updates(props: OzUpdatesProps<'_>, app: &AppContext) -> Option<Box<dyn Element>> {
    let changelog_model = ChangelogModel::as_ref(app);
    let should_show_oz_updates = *AISettings::as_ref(app)
        .should_show_oz_updates_in_zero_state
        .value();
    if !should_render_oz_updates_section(
        FeatureFlag::OzChangelogUpdates.is_enabled(),
        should_show_oz_updates,
        !changelog_model.oz_updates.is_empty(),
    ) {
        return None;
    }

    let OzUpdatesProps {
        is_expanded,
        state_handles,
    } = props;

    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    let section_header = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Shrinkable::new(
                1.,
                Clipped::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Min)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_constrain_horizontal_bounds_to_parent(true)
                        .with_child(
                            Container::new(
                                ConstrainedBox::new(
                                    if is_expanded {
                                        Icon::ChevronDown
                                    } else {
                                        Icon::ChevronRight
                                    }
                                    .to_warpui_icon(theme.sub_text_color(theme.background()))
                                    .finish(),
                                )
                                .with_height(appearance.monospace_font_size())
                                .with_width(appearance.monospace_font_size())
                                .finish(),
                            )
                            .with_margin_right(4.)
                            .finish(),
                        )
                        .with_child(
                            Container::new(
                                Text::new(
                                    OZ_UPDATES_SECTION_HEADER,
                                    appearance.ui_font_family(),
                                    appearance.monospace_font_size() - 2.,
                                )
                                .with_color(theme.sub_text_color(theme.background()).into_solid())
                                .with_style(Properties::default().weight(Weight::Semibold))
                                .finish(),
                            )
                            .with_margin_right(8.)
                            .finish(),
                        )
                        .with_child(
                            Container::new(
                                Text::new(
                                    if changelog_model.oz_updates.len() == 1 {
                                        "1 update".to_owned()
                                    } else {
                                        format!(
                                            "{} updates",
                                            changelog_model
                                                .oz_updates
                                                .len()
                                                .min(MAX_OZ_UPDATE_COUNT)
                                        )
                                    },
                                    appearance.ui_font_family(),
                                    appearance.monospace_font_size() - 2.,
                                )
                                .with_color(
                                    theme.disabled_text_color(theme.background()).into_solid(),
                                )
                                .finish(),
                            )
                            .with_margin_right(16.)
                            .finish(),
                        )
                        .finish(),
                )
                .finish(),
            )
            .finish(),
        )
        .with_child(
            Shrinkable::new(
                1.,
                Clipped::new(
                    Align::new(
                        Hoverable::new(state_handles.changelog_link.clone(), |state| {
                            let text_color = if state.is_hovered() {
                                theme.sub_text_color(theme.background()).into_solid()
                            } else {
                                theme.disabled_text_color(theme.background()).into_solid()
                            };
                            Flex::row()
                                .with_main_axis_alignment(MainAxisAlignment::End)
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_constrain_horizontal_bounds_to_parent(true)
                                .with_child(
                                    Container::new(
                                        Text::new(
                                            "View changelog",
                                            appearance.ui_font_family(),
                                            appearance.monospace_font_size() - 2.,
                                        )
                                        .with_color(text_color)
                                        .finish(),
                                    )
                                    .with_margin_right(4.)
                                    .finish(),
                                )
                                .with_child(
                                    ConstrainedBox::new(
                                        Icon::Share3
                                            .to_warpui_icon(
                                                theme.sub_text_color(theme.background()),
                                            )
                                            .finish(),
                                    )
                                    .with_width(appearance.monospace_font_size() - 2.)
                                    .with_height(appearance.monospace_font_size() - 2.)
                                    .finish(),
                                )
                                .finish()
                        })
                        .with_reset_cursor_after_click()
                        .on_click(|_, app, _| {
                            const CHANGELOG_URL: &str = "https://docs.warp.dev/changelog";
                            app.open_url(CHANGELOG_URL);
                        })
                        .with_cursor(Cursor::PointingHand)
                        .finish(),
                    )
                    .right()
                    .finish(),
                )
                .finish(),
            )
            .finish(),
        )
        .finish();

    let mut body = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(if is_expanded {
            Container::new(section_header)
                .with_margin_bottom(styles::SECTION_HEADER_MARGIN_BOTTOM)
                .finish()
        } else {
            section_header
        });

    if is_expanded {
        for (i, update) in changelog_model
            .oz_updates
            .iter()
            .enumerate()
            .take(MAX_OZ_UPDATE_COUNT)
        {
            let mut text = FormattedTextElement::new(
                update.clone(),
                appearance.monospace_font_size() - 2.,
                appearance.ui_font_family(),
                appearance.monospace_font_family(),
                theme
                    .main_text_color(agent_view_bg_color(app).into())
                    .into_solid(),
                state_handles
                    .update_hyperlinks
                    .get(i)
                    .cloned()
                    .unwrap_or_default(),
            )
            .register_default_click_handlers(|url, _, ctx| {
                ctx.open_url(&url.url);
            })
            .with_line_height_ratio(1.2)
            .finish();

            if i < changelog_model.oz_updates.len().min(MAX_OZ_UPDATE_COUNT) - 1 {
                text = Container::new(text).with_margin_bottom(8.).finish();
            }
            body.add_child(text);
        }
    }

    Some(
        Hoverable::new(state_handles.oz_updates.clone(), |_| {
            Container::new(body.finish())
                .with_vertical_padding(8.)
                .with_horizontal_padding(12.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_border(Border::all(1.).with_border_fill(theme.surface_overlay_2()))
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .with_reset_cursor_after_click()
        .with_defer_events_to_children()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(AgentViewZeroStateAction::ToggleOzUpdates);
        })
        .finish(),
    )
}

/// Renders the ambient credits banner showing free cloud credits.
/// If `link_mouse_state` is provided, a "Launch cloud agent" link is shown.
pub fn render_ambient_credits_banner(credits: i32, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let font_family = appearance.ui_font_family();
    let font_size = styles::CREDITS_BANNER_FONT_SIZE;

    // Use ANSI terminal colors for the pill styling.
    let text_color = theme.terminal_colors().normal.blue;

    let credits_text = format!("{credits} free cloud agent credits");
    let text = Text::new(credits_text, font_family, font_size)
        .with_color(text_color.into())
        .with_style(Properties::default().weight(Weight::Semibold))
        .soft_wrap(false)
        .finish();

    Container::new(text)
        .with_border(Border::all(1.).with_border_color(text_color.into()))
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
        .with_vertical_padding(2.)
        .with_horizontal_padding(6.)
        .with_margin_left(8.)
        .finish()
}

mod styles {
    use warp_core::ui::appearance::Appearance;

    pub const CONTAINER_VERTICAL_PADDING: f32 = 16.;
    pub const TITLE_MARGIN_BOTTOM: f32 = 8.;
    pub const SECTION_HEADER_MARGIN_BOTTOM: f32 = 8.;
    pub const DESCRIPTION_LINE_MARGIN_BOTTOM: f32 = 6.;
    pub const CREDITS_BANNER_FONT_SIZE: f32 = 12.;

    pub fn title_font_size(appearance: &Appearance) -> f32 {
        appearance.monospace_font_size() + 6.
    }
}

#[cfg(test)]
#[path = "zero_state_block_tests.rs"]
mod tests;
