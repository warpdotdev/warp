use vec1::{vec1, Vec1};
use warp_core::{features::FeatureFlag, ui::builder::AnimatedButtonOptions};
use warpui::{
    elements::{
        Align, Border, ConstrainedBox, Container, CrossAxisAlignment, Element, Flex, Icon,
        MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, SavePosition, Shrinkable,
    },
    fonts::Weight,
    platform::Cursor,
    presenter::ChildView,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    windowing::{StateEvent, WindowManager},
    AppContext, Entity, EntityId, FocusContext, ModelHandle, SingletonEntity, TypedActionView,
    View, ViewContext, ViewHandle, WindowId,
};

use super::{
    keybindings_page::KeybindingsEvent,
    section_views::{
        FOOTER_ICON_SIZE, HEADER_FONT_SIZE, ICON_PADDING, KEYBOARD_ICON_SIZE, SCROLLBAR_OFFSET,
        SECTION_SPACING,
    },
    KeybindingsView, ResourceCenterMainEvent, ResourceCenterMainView, TipsCompleted,
};
use crate::ui_components::{buttons::icon_button, window_focus_dimming::WindowFocusDimming};
use crate::{
    appearance::Appearance,
    changelog_model::ChangelogModel,
    ui_components::icons,
    util::links,
    workspace::{WorkspaceAction, PANEL_HEADER_HEIGHT},
};

// Footer icons
const DOCS_SVG_PATH: &str = "bundled/svg/gitbook-logo.svg";
const SLACK_SVG_PATH: &str = "bundled/svg/slack-logo.svg";
const FEEDBACK_SVG_PATH: &str = "bundled/svg/feedback.svg";

#[derive(Debug, Clone, Copy)]
pub enum ResourceCenterFooterItem {
    Docs,
    Slack,
    Feedback,
}

impl ResourceCenterFooterItem {
    pub fn ui_label(&self) -> &'static str {
        match self {
            ResourceCenterFooterItem::Docs => "Docs",
            ResourceCenterFooterItem::Slack => "Slack",
            ResourceCenterFooterItem::Feedback => "Feedback",
        }
    }

    pub fn svg_path(&self) -> &'static str {
        match self {
            ResourceCenterFooterItem::Docs => DOCS_SVG_PATH,
            ResourceCenterFooterItem::Slack => SLACK_SVG_PATH,
            ResourceCenterFooterItem::Feedback => FEEDBACK_SVG_PATH,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceCenterPage {
    Main,
    Keybindings,
}

#[derive(Clone)]
pub struct ResourceCenterPageView {
    pub page: ResourceCenterPage,
    pub page_view_handle: ResourceCenterViewHandle,
}

#[derive(Clone)]
pub enum ResourceCenterViewHandle {
    Main(ViewHandle<ResourceCenterMainView>),
    Keybindings(ViewHandle<KeybindingsView>),
}

#[derive(Default)]
struct MouseStateHandles {
    navigate_back: MouseStateHandle,
    open_keybindings: MouseStateHandle,
    close: MouseStateHandle,
    // Footer mouse state handles
    view_user_docs: MouseStateHandle,
    join_slack: MouseStateHandle,
    share_feedback: MouseStateHandle,
}

pub enum ResourceCenterEvent {
    Close,
    Escape,
}

pub struct ResourceCenterView {
    button_mouse_states: MouseStateHandles,
    header_dimming_mouse_state: MouseStateHandle,
    current_view_index: usize,
    page_views: Vec1<ResourceCenterPageView>,
    window_id: WindowId,
}

#[derive(Debug, Clone)]
pub enum ResourceCenterAction {
    Close,
    NavigatePage(ResourceCenterPage),
    FooterItemClick(ResourceCenterFooterItem),
}

impl ResourceCenterView {
    pub fn new(
        ctx: &mut ViewContext<Self>,
        tips_completed: ModelHandle<TipsCompleted>,
        changelog_model_handle: ModelHandle<ChangelogModel>,
    ) -> Self {
        let main_view = ResourceCenterPageView {
            page: ResourceCenterPage::Main,
            page_view_handle: ResourceCenterViewHandle::Main(Self::build_main_view(
                ctx,
                tips_completed,
                changelog_model_handle,
            )),
        };
        let keybindings_view = ResourceCenterPageView {
            page: ResourceCenterPage::Keybindings,
            page_view_handle: ResourceCenterViewHandle::Keybindings(Self::build_keybindings_view(
                ctx,
            )),
        };
        // Subscribe to window state changes for focus dimming updates
        let state_handle = WindowManager::handle(ctx);
        ctx.subscribe_to_model(&state_handle, |_me, _, event, ctx| match &event {
            StateEvent::ValueChanged { current, previous } => {
                if WindowManager::did_window_change_focus(ctx.window_id(), current, previous) {
                    ctx.notify();
                }
            }
            StateEvent::WindowZoomFactorChanged { .. } => {}
        });

        let page_views = vec1![main_view, keybindings_view];

        Self {
            button_mouse_states: Default::default(),
            header_dimming_mouse_state: Default::default(),
            current_view_index: 0,
            page_views,
            window_id: ctx.window_id(),
        }
    }

    fn build_main_view(
        ctx: &mut ViewContext<Self>,
        tips_completed: ModelHandle<TipsCompleted>,
        changelog_model_handle: ModelHandle<ChangelogModel>,
    ) -> ViewHandle<ResourceCenterMainView> {
        let main_view = ctx.add_typed_action_view(|ctx| {
            ResourceCenterMainView::new(ctx, tips_completed.clone(), changelog_model_handle)
        });

        ctx.subscribe_to_view(&main_view, move |me, _, event, ctx| {
            me.handle_main_event(event, ctx);
        });

        main_view
    }

    fn handle_main_event(&mut self, event: &ResourceCenterMainEvent, ctx: &mut ViewContext<Self>) {
        match event {
            ResourceCenterMainEvent::Close => {
                ctx.emit(ResourceCenterEvent::Close);
                ctx.notify();
            }
        }
    }

    fn build_keybindings_view(ctx: &mut ViewContext<Self>) -> ViewHandle<KeybindingsView> {
        let keybindings_view = ctx.add_typed_action_view(KeybindingsView::new);

        ctx.subscribe_to_view(&keybindings_view, move |me, _, event, ctx| {
            me.handle_keybindings_event(event, ctx);
        });

        keybindings_view
    }

    fn handle_keybindings_event(&mut self, event: &KeybindingsEvent, ctx: &mut ViewContext<Self>) {
        match event {
            KeybindingsEvent::Escape => {
                ctx.emit(ResourceCenterEvent::Escape);
            }
        }
    }

    pub fn get_current_page(&self) -> ResourceCenterPage {
        self.page_views
            .get(self.current_view_index)
            .map(|x| x.page)
            .expect("Should have a valid page")
    }

    fn focus(&self, ctx: &mut ViewContext<Self>) {
        // Change focus depending on page.
        let current_page_handle = &self.page_views[self.current_view_index].page_view_handle;

        match current_page_handle {
            ResourceCenterViewHandle::Main(_) => {
                // Lets terminal view determine where focus is given.
                ctx.emit(ResourceCenterEvent::Escape);
            }
            ResourceCenterViewHandle::Keybindings(keybindings_view_handle) => {
                ctx.focus(keybindings_view_handle);
            }
        }
    }

    pub fn set_current_page(&mut self, new_page: ResourceCenterPage, ctx: &mut ViewContext<Self>) {
        let position = self
            .page_views
            .iter()
            .position(|page_view| page_view.page == new_page);

        if let Some(new_page_index) = position {
            self.current_view_index = new_page_index;
        }

        self.focus(ctx);
        ctx.notify();
    }

    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
        ctx.emit(ResourceCenterEvent::Close)
    }

    pub fn set_action_target(
        &mut self,
        window_id: WindowId,
        input_id: Option<EntityId>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let ResourceCenterViewHandle::Main(main_handle) = &self.page_views[0].page_view_handle {
            main_handle.update(ctx, |main_view, ctx| {
                main_view.set_action_target(window_id, input_id, ctx);
            });
        }
    }

    fn footer_item_click_action(
        &mut self,
        item: &ResourceCenterFooterItem,
        ctx: &mut ViewContext<Self>,
    ) {
        match item {
            ResourceCenterFooterItem::Docs => ctx.open_url(links::USER_DOCS_URL),
            ResourceCenterFooterItem::Slack => ctx.open_url(links::SLACK_URL),
            // Route feedback through the workspace action so the guided agent experience is
            // launched when AI is available, and the GitHub issue form is opened otherwise.
            ResourceCenterFooterItem::Feedback => {
                ctx.dispatch_typed_action(&WorkspaceAction::SendFeedback)
            }
        }
    }

    fn render_back_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        icon_button(
            appearance,
            crate::ui_components::icons::Icon::ChevronLeft,
            false,
            self.button_mouse_states.navigate_back.clone(),
        )
        .build()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(ResourceCenterAction::NavigatePage(ResourceCenterPage::Main))
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }

    fn render_keyboard_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            appearance
                .ui_builder()
                .animated_button(
                    self.button_mouse_states.open_keybindings.clone(),
                    icons::Icon::Keyboard.into(),
                    AnimatedButtonOptions {
                        size: KEYBOARD_ICON_SIZE,
                        padding: Some(ICON_PADDING),
                        color: Some(appearance.theme().active_ui_text_color().with_opacity(80)),
                        with_accent_animations: false,
                        circular: false,
                    },
                )
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(ResourceCenterAction::NavigatePage(
                        ResourceCenterPage::Keybindings,
                    ))
                })
                .with_cursor(Cursor::PointingHand)
                .finish(),
        )
        .with_padding_right(ICON_PADDING)
        .finish()
    }

    fn render_close_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        SavePosition::new(
            icon_button(
                appearance,
                crate::ui_components::icons::Icon::X,
                false,
                self.button_mouse_states.close.clone(),
            )
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(WorkspaceAction::ToggleResourceCenter))
            .with_cursor(Cursor::PointingHand)
            .finish(),
            "resource_center_close_button",
        )
        .finish()
    }

    fn render_header_contents(&self, appearance: &Appearance) -> Vec<Box<dyn Element>> {
        let current_page = self.page_views.get(self.current_view_index).map(|x| x.page);

        let header_text = match current_page {
            Some(ResourceCenterPage::Keybindings) => "Keyboard Shortcuts".to_string(),
            _ => {
                if FeatureFlag::AvatarInTabBar.is_enabled() {
                    String::new()
                } else {
                    "Warp Essentials".to_string()
                }
            }
        };
        let title = Shrinkable::new(
            1.0,
            Align::new(
                Container::new(
                    appearance
                        .ui_builder()
                        .wrappable_text(header_text, false)
                        .with_style(UiComponentStyles {
                            font_family_id: Some(appearance.ui_font_family()),
                            font_size: Some(HEADER_FONT_SIZE),
                            font_weight: Some(Weight::Semibold),
                            ..Default::default()
                        })
                        .build()
                        .finish(),
                )
                .with_padding_left(6.)
                .finish(),
            )
            .left()
            .finish(),
        )
        .finish();

        // Render header items based on page
        let close_button = self.render_close_button(appearance);
        match current_page {
            Some(ResourceCenterPage::Keybindings) => {
                vec![self.render_back_button(appearance), title, close_button]
            }
            _ => {
                vec![title, self.render_keyboard_button(appearance), close_button]
            }
        }
    }

    fn render_header(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        const HEADER_VERTICAL_PADDING: f32 = 5.;
        const HEADER_HORIZONTAL_PADDING: f32 = 6.;
        let header_body = self.render_header_contents(appearance);

        let header_element = ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_children(header_body)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish(),
            )
            .with_padding_left(HEADER_HORIZONTAL_PADDING)
            .with_padding_right(HEADER_HORIZONTAL_PADDING)
            .with_padding_top(HEADER_VERTICAL_PADDING)
            .with_padding_bottom(HEADER_VERTICAL_PADDING)
            .finish(),
        )
        .with_height(PANEL_HEADER_HEIGHT)
        .finish();

        // Apply dimming if window is not focused
        WindowFocusDimming::apply_panel_header_dimming(
            header_element,
            self.header_dimming_mouse_state.clone(),
            PANEL_HEADER_HEIGHT,
            appearance.theme().surface_1().into(),
            self.window_id,
            app,
        )
    }

    fn render_footer_button(
        &self,
        item: ResourceCenterFooterItem,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let mouse_state = match item {
            ResourceCenterFooterItem::Docs => self.button_mouse_states.view_user_docs.clone(),
            ResourceCenterFooterItem::Slack => self.button_mouse_states.join_slack.clone(),
            ResourceCenterFooterItem::Feedback => self.button_mouse_states.share_feedback.clone(),
        };

        let icon = ConstrainedBox::new(
            Icon::new(
                item.svg_path(),
                appearance.theme().active_ui_detail().into_solid(),
            )
            .finish(),
        )
        .with_height(FOOTER_ICON_SIZE)
        .with_width(FOOTER_ICON_SIZE);

        let button = appearance
            .ui_builder()
            .button(ButtonVariant::Text, mouse_state)
            .with_text_label(item.ui_label().to_string())
            .with_style(
                UiComponentStyles::default().set_padding(Coords::default().left(SCROLLBAR_OFFSET)),
            )
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ResourceCenterAction::FooterItemClick(item));
            })
            .with_cursor(Cursor::PointingHand)
            .finish();

        Flex::row()
            .with_child(Align::new(icon.finish()).finish())
            .with_child(button)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish()
    }

    fn render_footer(&self, appearance: &Appearance) -> Box<dyn Element> {
        let docs_button = self.render_footer_button(ResourceCenterFooterItem::Docs, appearance);
        let slack_button = self.render_footer_button(ResourceCenterFooterItem::Slack, appearance);
        let feedback_button =
            self.render_footer_button(ResourceCenterFooterItem::Feedback, appearance);

        let footer = Flex::row()
            .with_child(docs_button)
            .with_child(slack_button)
            .with_child(feedback_button)
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish();

        Container::new(footer)
            .with_padding_top(SECTION_SPACING)
            .with_padding_bottom(SECTION_SPACING)
            .with_border(Border::top(1.).with_border_fill(appearance.theme().surface_2()))
            .finish()
    }
}

impl Entity for ResourceCenterView {
    type Event = ResourceCenterEvent;
}

impl TypedActionView for ResourceCenterView {
    type Action = ResourceCenterAction;

    fn handle_action(&mut self, action: &ResourceCenterAction, ctx: &mut ViewContext<Self>) {
        use ResourceCenterAction::*;
        match action {
            Close => self.close(ctx),
            NavigatePage(new_page) => self.set_current_page(*new_page, ctx),
            FooterItemClick(item) => self.footer_item_click_action(item, ctx),
        }
    }
}

impl View for ResourceCenterView {
    fn ui_name() -> &'static str {
        "ResourceCenter"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focus(ctx);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let header = self.render_header(appearance, app);
        let footer = self.render_footer(appearance);
        let resource_center_page = &self.page_views[self.current_view_index].page_view_handle;

        let body = match &resource_center_page {
            ResourceCenterViewHandle::Main(main_view_handle) => {
                ChildView::new(main_view_handle).finish()
            }
            ResourceCenterViewHandle::Keybindings(keybindings_view_handle) => {
                ChildView::new(keybindings_view_handle).finish()
            }
        };

        Flex::column()
            .with_child(header)
            .with_child(Shrinkable::new(1., body).finish())
            .with_child(footer)
            .finish()
    }
}
