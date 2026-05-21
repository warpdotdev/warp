use super::{
    settings_page::{
        render_page_title, MatchData, PageType, SettingsPageMeta, SettingsPageViewHandle,
        SettingsWidget, HEADER_FONT_SIZE, PAGE_PADDING,
    },
    SettingsSection,
};
use crate::auth::AuthStateProvider;
use crate::{
    appearance::Appearance,
    channel::{Channel, ChannelState},
    i18n::{self, I18nKey},
    menu::{Event as MenuEvent, Event, Menu, MenuItem, MenuItemFields},
    server::{block::Block, server_api::block::BlockClient},
    view_components::ToastFlavor,
};
use anyhow::Result;
use chrono::{DateTime, FixedOffset, Local};
use pathfinder_geometry::vector::vec2f;
use std::sync::Arc;
use warp_core::ui::theme::color::internal_colors;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    clipboard::ClipboardContent,
    elements::{
        Align, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        Dismiss, Expanded, Fill, Flex, Hoverable, Icon, MouseStateHandle, OffsetPositioning,
        ParentAnchor, ParentElement, ParentOffsetBounds, PositionedElementAnchor,
        PositionedElementOffsetBounds, SavePosition, ScrollStateHandle, Scrollable,
        ScrollableElement, Shrinkable, Stack, UniformList, UniformListState,
    },
};
use warpui::{color::ColorU, elements::Radius};
use warpui::{elements::ScrollbarWidth, fonts::Weight};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

const SCROLLBAR_WIDTH: ScrollbarWidth = ScrollbarWidth::Auto;

const UNSHARE_BLOCK_CONFIRMATION_DIALOG_TEXT: &str =
    "Are you sure you want to unshare this block?\n\
\nIt will no longer be accessible by link and will be permanently deleted from Warp servers.";

#[derive(Clone, Debug)]
struct UserOwnedBlock {
    id: String,
    command: String,
    link_mouse_state_handle: MouseStateHandle,
    copy_button_mouse_state_handle: MouseStateHandle,
    overflow_button_mouse_state_handle: MouseStateHandle,
    unshare_request_status: UnshareBlockRequestState,
    time_started: DateTime<FixedOffset>,
}

impl From<Block> for UserOwnedBlock {
    fn from(block: Block) -> Self {
        UserOwnedBlock::new(
            block.id.unwrap_or_default(),
            block.command.unwrap_or_default(),
            block.time_started_term,
        )
    }
}

impl UserOwnedBlock {
    fn new(
        id: impl Into<String>,
        command: impl Into<String>,
        time_started: DateTime<FixedOffset>,
    ) -> Self {
        Self {
            id: id.into(),
            command: command.into(),
            unshare_request_status: UnshareBlockRequestState::NotStarted,
            link_mouse_state_handle: Default::default(),
            copy_button_mouse_state_handle: Default::default(),
            overflow_button_mouse_state_handle: Default::default(),
            time_started,
        }
    }

    fn is_shared(&self) -> bool {
        !matches!(self.unshare_request_status, UnshareBlockRequestState::Done)
    }

    fn block_url(&self) -> String {
        // New block IDs are 22 characters long and are accessible at /block/{id}, whereas as old
        // (hashId) block IDs are 6 characters long and are accessible at /{id}.
        let mut url = if self.id.len() == 22 {
            format!(
                "{}/block/{}",
                ChannelState::server_root_url(),
                self.id.as_str()
            )
        } else {
            format!("{}/{}", ChannelState::server_root_url(), self.id.as_str())
        };

        // If this is a preview build, ensure the link routes to a preview build.
        if matches!(ChannelState::channel(), Channel::Preview) {
            url.push_str("?preview=true");
        }
        url
    }

    fn render_overflow_icon(&self, appearance: &Appearance, index: usize) -> Box<dyn Element> {
        let mut hoverable =
            Hoverable::new(self.overflow_button_mouse_state_handle.clone(), |state| {
                let container = Container::new(
                    ConstrainedBox::new(
                        Icon::new("bundled/svg/overflow.svg", ColorU::new(179, 186, 184, 255))
                            .finish(),
                    )
                    .with_height(20.)
                    .with_width(20.)
                    .finish(),
                )
                .with_uniform_padding(4.)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)));

                let container = if state.is_clicked() || state.is_hovered() {
                    container.with_background(appearance.theme().surface_2())
                } else {
                    container
                };
                container.finish()
            });

        // Disable the overflow button if the request is in flight since the user shouldn't be able
        // to unshare it again.
        if self.unshare_request_status == UnshareBlockRequestState::InFlight {
            hoverable = hoverable.disable();
        } else {
            hoverable = hoverable.on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ShowBlocksAction::OverflowClick(index));
            });
        };

        SavePosition::new(
            hoverable.finish(),
            format!("show_blocks_view:overflow_{index}").as_str(),
        )
        .finish()
    }

    fn copy_link_button(
        &self,
        appearance: &Appearance,
        app: &AppContext,
        block_url: String,
    ) -> Box<dyn Element> {
        let button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Basic,
                self.copy_button_mouse_state_handle.clone(),
            )
            .with_text_label(crate::i18n::tr_static(app, "Copy link").into());

        let button = if self.unshare_request_status == UnshareBlockRequestState::InFlight {
            button.disabled().build()
        } else {
            button.build().on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ShowBlocksAction::CopyUrl(block_url.clone()));
            })
        };

        button.finish()
    }

    fn link_text(
        &self,
        appearance: &Appearance,
        app: &AppContext,
        block_url: String,
    ) -> Box<dyn Element> {
        if self.unshare_request_status == UnshareBlockRequestState::InFlight {
            appearance
                .ui_builder()
                .label(crate::i18n::tr_static(app, "Deleting..."))
                .with_style(
                    UiComponentStyles::default()
                        .set_font_family_id(appearance.monospace_font_family())
                        .set_font_size(14.),
                )
                .build()
                .finish()
        } else {
            appearance
                .ui_builder()
                .link(
                    block_url.clone(),
                    Some(block_url),
                    None,
                    self.link_mouse_state_handle.clone(),
                )
                .soft_wrap(false)
                .with_style(UiComponentStyles::default().set_font_size(14.))
                .build()
                .finish()
        }
    }

    fn render(&self, appearance: &Appearance, app: &AppContext, index: usize) -> Box<dyn Element> {
        let block_url = self.block_url();
        let command = appearance
            .ui_builder()
            .label(self.command.clone())
            .with_style(UiComponentStyles {
                font_size: Some(14.),
                font_family_id: Some(appearance.monospace_font_family()),
                font_weight: Some(Weight::Bold),
                ..Default::default()
            })
            .build()
            .finish();
        let command_row = Container::new(
            Flex::row()
                .with_child(Shrinkable::new(1., Align::new(command).left().finish()).finish())
                .with_child(
                    Container::new(self.render_overflow_icon(appearance, index))
                        .with_padding_left(15.)
                        .finish(),
                )
                .finish(),
        )
        .finish();
        let url_row = Container::new(
            Flex::row()
                .with_child(
                    Shrinkable::new(
                        1.,
                        Container::new(self.link_text(appearance, app, block_url.clone())).finish(),
                    )
                    .finish(),
                )
                .with_child(
                    Shrinkable::new(
                        0.3,
                        Container::new(
                            Align::new(self.copy_link_button(appearance, app, block_url))
                                .right()
                                .finish(),
                        )
                        .finish(),
                    )
                    .finish(),
                )
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .finish();
        let timestamp_row = Container::new(
            appearance
                .ui_builder()
                .label(format!(
                    "Executed on: {}",
                    self.time_started
                        .with_timezone(&Local)
                        .format("%a, %b %-d %Y at %-I:%M %p")
                ))
                .with_style(
                    UiComponentStyles::default()
                        .set_font_color(
                            appearance
                                .theme()
                                .hint_text_color(appearance.theme().surface_2())
                                .into(),
                        )
                        // make it slightly smaller than the url text
                        .set_font_size(appearance.ui_builder().ui_font_size() - 2.),
                )
                .build()
                .finish(),
        )
        .finish();

        Container::new(
            ConstrainedBox::new(
                Flex::column()
                    .with_child(Shrinkable::new(1.0, command_row).finish())
                    .with_child(Shrinkable::new(1., url_row).finish())
                    .with_child(timestamp_row)
                    .finish(),
            )
            .with_max_height(90.)
            .finish(),
        )
        .finish()
    }
}

/// Status for the request to fetch the blocks owned by the current user.
#[derive(Debug)]
enum GetBlocksForUserRequestState {
    NotStarted,
    InFlight,
    Failed,
    Done(Vec<UserOwnedBlock>),
}

fn pad(element: Box<dyn Element>) -> Box<dyn Element> {
    Container::new(element)
        .with_uniform_padding(PAGE_PADDING)
        .finish()
}

impl GetBlocksForUserRequestState {
    fn render(
        &self,
        appearance: &Appearance,
        app: &AppContext,
        list_state: UniformListState,
        scroll_state_handle: ScrollStateHandle,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        match self {
            GetBlocksForUserRequestState::NotStarted => pad(ui_builder
                .label(crate::i18n::tr_static(
                    app,
                    "You don't have any shared blocks yet.",
                ))
                .build()
                .finish()),
            GetBlocksForUserRequestState::InFlight => pad(ui_builder
                .label(crate::i18n::tr_static(app, "Getting blocks..."))
                .build()
                .finish()),
            GetBlocksForUserRequestState::Failed => pad(ui_builder
                .label(crate::i18n::tr_static(
                    app,
                    "Failed to load blocks. Please try again.",
                ))
                .build()
                .finish()),
            GetBlocksForUserRequestState::Done(user_blocks) => {
                let user_blocks = user_blocks.clone();
                // Only consider the unshared blocks when rendering. We don't remove them from the
                // list of blocks so that the indexing is consistent for the lifetime of the app.
                let num_visible_blocks =
                    user_blocks.iter().filter(|block| block.is_shared()).count();

                if num_visible_blocks > 0 {
                    let list =
                        UniformList::new(list_state, num_visible_blocks, move |range, app| {
                            let appearance = Appearance::as_ref(app);
                            user_blocks
                                .iter()
                                .enumerate()
                                .filter(|(_, block)| block.is_shared())
                                .skip(range.start)
                                .take(range.end - range.start)
                                .enumerate()
                                .map(|(visible_index, (index, user_block))| {
                                    let user_block_element =
                                        Container::new(user_block.render(appearance, app, index))
                                            .with_uniform_padding(10.);

                                    // Add a background on alternating blocks.
                                    if visible_index % 2 == 0 {
                                        user_block_element
                                            .with_background(internal_colors::fg_overlay_1(
                                                appearance.theme(),
                                            ))
                                            .finish()
                                    } else {
                                        user_block_element.finish()
                                    }
                                })
                                .collect::<Vec<_>>()
                                .into_iter()
                        });

                    Scrollable::vertical(
                        scroll_state_handle,
                        list.finish_scrollable(),
                        SCROLLBAR_WIDTH,
                        Fill::Solid(appearance.theme().nonactive_ui_detail().into_solid()),
                        Fill::Solid(appearance.theme().active_ui_detail().into_solid()),
                        Fill::None, // Leave the background transparent
                    )
                    .with_padding_start(5.)
                    .finish()
                } else {
                    pad(ui_builder
                        .label(crate::i18n::tr_static(
                            app,
                            "You don't have any shared blocks yet.",
                        ))
                        .build()
                        .finish())
                }
            }
        }
    }
}

/// Status for the request to unshare a block.
#[derive(Debug, Clone, PartialEq)]
enum UnshareBlockRequestState {
    NotStarted,
    InFlight,
    Failed,
    Done,
}

#[derive(Default)]
struct StateHandles {
    scroll_state_handle: ScrollStateHandle,
    confirm_dialog_handle: MouseStateHandle,
    cancel_dialog_handle: MouseStateHandle,
}

/// A view that lists all the blocks owned by the user.
pub struct ShowBlocksView {
    page: PageType<Self>,
    list_state: UniformListState,
    overflow_menu: ViewHandle<Menu<ShowBlocksAction>>,
    overflow_menu_index: Option<usize>,
    get_blocks_for_user_status: GetBlocksForUserRequestState,
    pending_unshared_block_index: Option<usize>,
    block_client: Arc<dyn BlockClient>,
    state_handles: StateHandles,
}

#[derive(Debug, Clone)]
pub enum ShowBlocksAction {
    CopyUrl(String),
    OverflowClick(usize),
    Unshare,
    ConfirmUnshare,
    CancelUnshare,
}

#[derive(Debug, Clone)]
pub enum ShowBlocksEvent {
    ShowToast {
        message: String,
        flavor: ToastFlavor,
    },
}

impl ShowBlocksView {
    pub fn new(block_client: Arc<dyn BlockClient>, ctx: &mut ViewContext<Self>) -> Self {
        let menu = ctx.add_typed_action_view(|ctx| {
            let mut menu = Menu::new().prevent_interaction_with_other_elements();

            menu.set_items(
                vec![MenuItem::Item(
                    MenuItemFields::new(crate::i18n::tr_static(ctx, "Unshare"))
                        .with_on_select_action(ShowBlocksAction::Unshare),
                )],
                ctx,
            );

            menu
        });

        ctx.subscribe_to_view(&menu, move |me, _, event, ctx| {
            me.handle_overflow_menu_event(event, ctx);
        });

        let page = PageType::new_monolith(ShowBlocksWidget::default(), None, false);
        Self {
            page,
            list_state: Default::default(),
            state_handles: Default::default(),
            overflow_menu: menu,
            overflow_menu_index: None,
            get_blocks_for_user_status: GetBlocksForUserRequestState::NotStarted,
            block_client,
            pending_unshared_block_index: None,
        }
    }

    fn load_blocks(&mut self, ctx: &mut ViewContext<Self>) {
        if !matches!(
            self.get_blocks_for_user_status,
            GetBlocksForUserRequestState::InFlight
        ) {
            let block_client = self.block_client.clone();
            self.get_blocks_for_user_status = GetBlocksForUserRequestState::InFlight;
            let _ = ctx.spawn(
                async move {
                    block_client
                        .blocks_owned_by_user()
                        .await
                        .map(|blocks| blocks.into_iter().map(Into::into).collect())
                },
                Self::on_load_complete,
            );
        }
    }

    fn on_load_complete(
        &mut self,
        result: Result<Vec<UserOwnedBlock>>,
        ctx: &mut ViewContext<Self>,
    ) {
        match result {
            Ok(mut blocks) => {
                blocks.sort_by(|b1, b2| b2.time_started.cmp(&b1.time_started));

                self.get_blocks_for_user_status = GetBlocksForUserRequestState::Done(blocks)
            }
            Err(_) => {
                log::info!("Failed to fetch blocks owned by user from server");
                self.get_blocks_for_user_status = GetBlocksForUserRequestState::Failed;
            }
        }
        ctx.notify()
    }

    pub fn copy_url(&mut self, block_url: &str, ctx: &mut ViewContext<Self>) {
        ctx.clipboard()
            .write(ClipboardContent::plain_text(block_url.to_string()));
        ctx.emit(ShowBlocksEvent::ShowToast {
            message: "Link copied.".to_string(),
            flavor: ToastFlavor::Default,
        })
    }

    pub fn overflow_click(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        self.overflow_menu_index = Some(index);
        ctx.notify();
    }

    pub fn cancel_unshare(&mut self, ctx: &mut ViewContext<Self>) {
        self.pending_unshared_block_index = None;
        ctx.notify();
    }

    pub fn confirm_unshare(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(selected_index) = self.pending_unshared_block_index.take() {
            if let GetBlocksForUserRequestState::Done(blocks) = &mut self.get_blocks_for_user_status
            {
                // Only attempt to unshare if there isn't already an inflight request to unshare
                // the block.
                let user_block = &mut blocks[selected_index];
                if !matches!(
                    user_block.unshare_request_status,
                    UnshareBlockRequestState::InFlight
                ) {
                    user_block.unshare_request_status = UnshareBlockRequestState::InFlight;

                    let block_client = self.block_client.clone();
                    let block_id = user_block.id.clone();
                    let _ = ctx.spawn(
                        async move { (block_client.unshare_block(block_id).await, selected_index) },
                        Self::on_block_unshare_complete,
                    );
                }
            }
        }
        ctx.notify();
    }

    pub fn unshare_button_click(&mut self, ctx: &mut ViewContext<Self>) {
        self.pending_unshared_block_index = self.overflow_menu_index.take();
        ctx.notify();
    }

    fn on_block_unshare_complete(
        &mut self,
        (request_result, block_index): (Result<()>, usize),
        ctx: &mut ViewContext<Self>,
    ) {
        log::info!(
            "on_block_unshare_complete with result {:?}",
            &request_result
        );
        if let GetBlocksForUserRequestState::Done(blocks) = &mut self.get_blocks_for_user_status {
            let user_block = &mut blocks[block_index];
            match request_result {
                Ok(_) => {
                    ctx.emit(ShowBlocksEvent::ShowToast {
                        message: "Block was successfully unshared.".to_string(),
                        flavor: ToastFlavor::Success,
                    });
                    user_block.unshare_request_status = UnshareBlockRequestState::Done;
                }
                Err(_) => {
                    ctx.emit(ShowBlocksEvent::ShowToast {
                        message: "Failed to unshare block. Please try again.".to_string(),
                        flavor: ToastFlavor::Error,
                    });
                    user_block.unshare_request_status = UnshareBlockRequestState::Failed;
                }
            }

            ctx.notify();
        }
    }

    fn handle_overflow_menu_event(&mut self, event: &MenuEvent, ctx: &mut ViewContext<Self>) {
        if let Event::Close { via_select_item: _ } = event {
            self.overflow_menu_index = None;
            ctx.notify();
        }
    }
}

impl Entity for ShowBlocksView {
    type Event = ShowBlocksEvent;
}

impl TypedActionView for ShowBlocksView {
    type Action = ShowBlocksAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        use ShowBlocksAction::*;
        match action {
            CopyUrl(url) => self.copy_url(url, ctx),
            OverflowClick(index) => self.overflow_click(*index, ctx),
            Unshare => self.unshare_button_click(ctx),
            ConfirmUnshare => self.confirm_unshare(ctx),
            CancelUnshare => self.cancel_unshare(ctx),
        }
    }
}

impl View for ShowBlocksView {
    fn ui_name() -> &'static str {
        "ShowBlockView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        self.page.render(self, app)
    }
}

impl SettingsPageMeta for ShowBlocksView {
    fn section() -> SettingsSection {
        SettingsSection::SharedBlocks
    }

    fn should_render(&self, ctx: &AppContext) -> bool {
        let is_anonymous = AuthStateProvider::as_ref(ctx)
            .get()
            .is_anonymous_or_logged_out();

        !is_anonymous
    }

    fn on_page_selected(&mut self, _: bool, ctx: &mut ViewContext<Self>) {
        self.load_blocks(ctx);
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

impl From<ViewHandle<ShowBlocksView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<ShowBlocksView>) -> Self {
        SettingsPageViewHandle::SharedBlocks(view_handle)
    }
}

#[derive(Default)]
struct ShowBlocksWidget {}

impl ShowBlocksWidget {
    fn render_confirm_delete_block_dialog(
        &self,
        view: &ShowBlocksView,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_child(
                        Align::new(
                            ui_builder
                                .label(crate::i18n::tr_static(app, "Unshare block"))
                                .with_style(UiComponentStyles {
                                    font_size: Some(appearance.header_font_size()),
                                    ..Default::default()
                                })
                                .build()
                                .finish(),
                        )
                        .top_center()
                        .finish(),
                    )
                    .with_child(
                        Container::new(
                            ui_builder
                                .paragraph(crate::i18n::tr_static(
                                    app,
                                    UNSHARE_BLOCK_CONFIRMATION_DIALOG_TEXT,
                                ))
                                .with_style(UiComponentStyles {
                                    font_size: Some(appearance.ui_font_size() * 1.16),
                                    ..Default::default()
                                })
                                .build()
                                .finish(),
                        )
                        .with_padding_top(9.)
                        .finish(),
                    )
                    .with_child(
                        Container::new(
                            Align::new(
                                Flex::row()
                                    .with_child(
                                        ui_builder
                                            .button(
                                                ButtonVariant::Basic,
                                                view.state_handles.cancel_dialog_handle.clone(),
                                            )
                                            .with_text_label(
                                                crate::i18n::tr_static(app, "Cancel").into(),
                                            )
                                            .build()
                                            .on_click(|ctx, _, _| {
                                                ctx.dispatch_typed_action(
                                                    ShowBlocksAction::CancelUnshare,
                                                );
                                            })
                                            .finish(),
                                    )
                                    .with_child(
                                        Container::new(
                                            ui_builder
                                                .button(
                                                    ButtonVariant::Accent,
                                                    view.state_handles
                                                        .confirm_dialog_handle
                                                        .clone(),
                                                )
                                                .with_text_label(
                                                    crate::i18n::tr_static(app, "Unshare").into(),
                                                )
                                                .build()
                                                .on_click(|ctx, _, _| {
                                                    ctx.dispatch_typed_action(
                                                        ShowBlocksAction::ConfirmUnshare,
                                                    );
                                                })
                                                .finish(),
                                        )
                                        .with_padding_left(10.)
                                        .finish(),
                                    )
                                    .finish(),
                            )
                            .top_center()
                            .finish(),
                        )
                        .with_padding_top(20.)
                        .finish(),
                    )
                    .finish(),
            )
            .with_background(appearance.theme().surface_2())
            .with_uniform_padding(20.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish(),
        )
        .with_height(220.)
        .with_width(300.)
        .finish()
    }
}

impl SettingsWidget for ShowBlocksWidget {
    type View = ShowBlocksView;

    fn search_terms(&self) -> &str {
        "shared blocks"
    }

    fn render(
        &self,
        view: &Self::View,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let element_for_state = view.get_blocks_for_user_status.render(
            appearance,
            app,
            view.list_state.clone(),
            view.state_handles.scroll_state_handle.clone(),
        );

        let inner_container = SavePosition::new(
            Container::new(element_for_state).finish(),
            "show_blocks_view:modal",
        )
        .finish();

        let mut stack = Stack::new().with_child(inner_container);

        if let Some(menu_index) = view.overflow_menu_index {
            stack.add_positioned_overlay_child(
                ChildView::new(&view.overflow_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    format!("show_blocks_view:overflow_{menu_index}").as_str(),
                    vec2f(0., 2.),
                    PositionedElementOffsetBounds::Unbounded,
                    PositionedElementAnchor::BottomLeft,
                    ChildAnchor::TopLeft,
                ),
            );
        }

        if view.pending_unshared_block_index.is_some() {
            stack.add_positioned_child(
                Dismiss::new(
                    Align::new(self.render_confirm_delete_block_dialog(view, appearance, app))
                        .finish(),
                )
                .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(ShowBlocksAction::CancelUnshare))
                .finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 0.),
                    ParentOffsetBounds::ParentByPosition,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
        }

        let header = render_page_title(
            i18n::tr(app, I18nKey::SettingsNavSharedBlocks),
            HEADER_FONT_SIZE,
            appearance,
        );
        let col = Flex::column()
            .with_child(Container::new(header).with_margin_bottom(24.).finish())
            .with_child(Expanded::new(1., stack.finish()).finish());

        col.finish()
    }
}
