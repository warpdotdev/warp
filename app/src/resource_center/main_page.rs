use crate::{
    auth::AuthStateProvider,
    changelog_model::ChangelogModel,
    channel::ChannelState,
    features::FeatureFlag,
    resource_center::skip_tips_and_write_to_user_defaults,
    send_telemetry_from_ctx,
    server::telemetry::TelemetryEvent,
    settings::Settings,
    themes::theme::{Blend, Fill as FillTheme},
};
use pathfinder_geometry::vector::vec2f;
use warpui::{
    elements::{
        Align, ClippedScrollStateHandle, ClippedScrollable, Container, CornerRadius, Element,
        Empty, Fill, Flex, Hoverable, Icon, MainAxisAlignment, MainAxisSize, MouseStateHandle,
        ParentElement, Radius, Shrinkable,
    },
    platform::Cursor,
    presenter::ChildView,
    ui_components::{
        button::{ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle, WindowId,
};

use crate::{appearance::Appearance, workspace::WorkspaceAction};

use super::{
    section_views::{
        feature_section::FeatureSectionEvent, SectionViewHandle, BUTTON_PADDING, DETAIL_FONT_SIZE,
        FOOTER_ICON_SIZE, SCROLLBAR_OFFSET, SCROLLBAR_WIDTH, SECTION_SPACING,
        SECTION_SPACING_BOTTOM,
    },
    sections::sections,
    ChangelogSectionView, ContentSectionData, ContentSectionView, FeatureSection,
    FeatureSectionData, FeatureSectionView, Section, TipsCompleted,
};

const SEND_SVG_PATH: &str = "bundled/svg/send.svg";

#[derive(Default)]
struct MouseStateHandles {
    copy_version: MouseStateHandle,
    invite_people: MouseStateHandle,
    skip_tips: MouseStateHandle,
}

pub enum ResourceCenterMainEvent {
    Close,
}

pub struct ResourceCenterMainView {
    button_mouse_states: MouseStateHandles,
    clipped_scroll_state: ClippedScrollStateHandle,
    section_views: Vec<SectionViewHandle>,
    tips_completed: ModelHandle<TipsCompleted>,
}

#[derive(Debug, Clone)]
pub enum ResourceCenterMainAction {
    Close,
    SkipTips,
}

impl ResourceCenterMainView {
    pub fn new(
        ctx: &mut ViewContext<Self>,
        tips_completed: ModelHandle<TipsCompleted>,
        changelog_model_handle: ModelHandle<ChangelogModel>,
    ) -> Self {
        let action_target = ctx.add_model(|_| ActionTarget::None);
        let section_views = Self::initialize_section_views(
            tips_completed.clone(),
            action_target.clone(),
            ctx,
            changelog_model_handle.clone(),
        );
        Self {
            button_mouse_states: Default::default(),
            clipped_scroll_state: Default::default(),
            section_views,
            tips_completed,
        }
    }

    fn initialize_section_views(
        tips_completed: ModelHandle<TipsCompleted>,
        action_target: ModelHandle<ActionTarget>,
        ctx: &mut ViewContext<Self>,
        changelog_model_handle: ModelHandle<ChangelogModel>,
    ) -> Vec<SectionViewHandle> {
        let sections = sections(ctx);

        // Set gamified tips count
        let gamified_tips_count = sections
            .iter()
            .map(|section| {
                let mut count = 0;
                if let Section::Feature(data) = section {
                    count = data.items.len()
                }
                count
            })
            .sum();

        tips_completed.update(ctx, |tips_completed, _ctx| {
            tips_completed.set_gamified_tips_count(gamified_tips_count);
        });

        // Determines if user has completed all tips under Getting Started
        let is_onboarded = sections.iter().any(|section| {
            if let Section::Feature(data) = section {
                let is_section_completed = data.is_section_completed(tips_completed.as_ref(ctx));
                is_section_completed && data.section_name == FeatureSection::GettingStarted
            } else {
                false
            }
        });

        sections
            .iter()
            .map(|section| match section {
                Section::Feature(data) => {
                    let is_tips_completed = tips_completed.as_ref(ctx).skipped_or_completed;
                    let is_expanded = match data.section_name {
                        // Always show What's New section
                        FeatureSection::WhatsNew => true,
                        FeatureSection::GettingStarted => match ChannelState::app_version() {
                            Some(version) => {
                                match Settings::has_changelog_been_shown(version, ctx) {
                                    true => !is_tips_completed && !is_onboarded,
                                    false => false,
                                }
                            }
                            None => !is_tips_completed && !is_onboarded,
                        },
                        // Expand Maximize Warp section once user has completed welcome tips,
                        // and keep open after users have completed/skipped all tips
                        FeatureSection::MaximizeWarp => match ChannelState::app_version() {
                            Some(version) => {
                                match Settings::has_changelog_been_shown(version, ctx) {
                                    true => is_tips_completed || is_onboarded,
                                    false => false,
                                }
                            }
                            None => is_tips_completed || is_onboarded,
                        },
                        _ => false,
                    };

                    // Show tips progress for every section except changelog
                    let show_tips_progress = !matches!(data.section_name, FeatureSection::WhatsNew);

                    SectionViewHandle::Feature(Self::build_feature_section_view(
                        data,
                        action_target.clone(),
                        ctx,
                        tips_completed.clone(),
                        show_tips_progress,
                        is_expanded,
                    ))
                }
                Section::Content(data) => {
                    SectionViewHandle::Content(Self::build_content_section_view(data, ctx))
                }
                Section::Changelog() => SectionViewHandle::Changelog(
                    Self::build_changelog_section_view(changelog_model_handle.clone(), ctx),
                ),
            })
            .collect()
    }

    fn build_feature_section_view(
        section_data: &FeatureSectionData,
        action_target: ModelHandle<ActionTarget>,
        ctx: &mut ViewContext<ResourceCenterMainView>,
        tips_completed: ModelHandle<TipsCompleted>,
        show_tips_progress: bool,
        is_expanded: bool,
    ) -> ViewHandle<FeatureSectionView> {
        let feature_section_view = ctx.add_typed_action_view(|ctx| {
            FeatureSectionView::new(
                section_data.clone(),
                action_target,
                ctx,
                tips_completed.clone(),
                show_tips_progress,
                is_expanded,
            )
        });

        ctx.subscribe_to_view(&feature_section_view, move |me, _, event, ctx| {
            me.handle_feature_section_event(event, ctx);
        });

        feature_section_view
    }

    fn handle_feature_section_event(
        &mut self,
        event: &FeatureSectionEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            FeatureSectionEvent::CloseResourceCenter => {
                ctx.emit(ResourceCenterMainEvent::Close);
                ctx.notify();
            }
            FeatureSectionEvent::ExpandSection(section_name) => {
                for section_view in &self.section_views {
                    match section_view {
                        SectionViewHandle::Feature(feature_view_handle) => {
                            if feature_view_handle
                                .as_ref(ctx)
                                .feature_section_data
                                .section_name
                                == *section_name
                            {
                                feature_view_handle.update(ctx, |view, ctx| {
                                    view.expand_section(ctx);
                                })
                            }
                        }
                        SectionViewHandle::Content(_) => {}
                        SectionViewHandle::Changelog(_) => {}
                    }
                }
                ctx.notify();
            }
        }
    }

    fn build_content_section_view(
        section_data: &ContentSectionData,
        ctx: &mut ViewContext<ResourceCenterMainView>,
    ) -> ViewHandle<ContentSectionView> {
        ctx.add_typed_action_view(|ctx| ContentSectionView::new(section_data.clone(), false, ctx))
    }

    fn build_changelog_section_view(
        changelog_model_handle: ModelHandle<ChangelogModel>,
        ctx: &mut ViewContext<ResourceCenterMainView>,
    ) -> ViewHandle<ChangelogSectionView> {
        let showing_new_changelog = match ChannelState::app_version() {
            Some(version) => !Settings::has_changelog_been_shown(version, ctx),
            None => false,
        };

        ctx.add_typed_action_view(|ctx: &mut ViewContext<_>| {
            ChangelogSectionView::new(changelog_model_handle, showing_new_changelog, ctx)
        })
    }

    pub fn set_action_target(
        &mut self,
        window_id: WindowId,
        input_id: Option<EntityId>,
        ctx: &mut ViewContext<Self>,
    ) {
        for section_view in &self.section_views {
            match section_view {
                SectionViewHandle::Feature(view_handle) => {
                    view_handle.update(ctx, |feature_section_view, ctx| {
                        feature_section_view.set_action_target(window_id, input_id, ctx)
                    });
                }
                SectionViewHandle::Content(_) => {}
                SectionViewHandle::Changelog(_) => {}
            }
        }
    }

    fn render_body(&self, appearance: &Appearance) -> Box<dyn Element> {
        let mut body = Flex::column();

        for section_view in &self.section_views {
            match section_view {
                SectionViewHandle::Feature(feature_view_handle) => {
                    body.add_child(ChildView::new(feature_view_handle).finish());
                }
                SectionViewHandle::Content(section_view_handle) => {
                    body.add_child(ChildView::new(section_view_handle).finish());
                }
                SectionViewHandle::Changelog(section_view_handle) => {
                    body.add_child(ChildView::new(section_view_handle).finish());
                }
            }
        }

        let theme = appearance.theme();

        ClippedScrollable::vertical(
            self.clipped_scroll_state.clone(),
            body.finish(),
            SCROLLBAR_WIDTH,
            theme.disabled_text_color(theme.background()).into(),
            theme.main_text_color(theme.background()).into(),
            Fill::None,
        )
        .finish()
    }

    fn render_current_version(&self, appearance: &Appearance) -> Box<dyn Element> {
        // Use a dummy string for git release tag which is not available on local env
        let version = ChannelState::app_version().unwrap_or("v0.local.testing.string_00");

        let style = UiComponentStyles {
            font_color: Some(appearance.theme().nonactive_ui_text_color().into()),
            ..Default::default()
        };

        let text = appearance
            .ui_builder()
            .wrappable_text(version, true)
            .with_style(style)
            .build()
            .finish();

        let copy_icon = appearance
            .ui_builder()
            .copy_button(
                FOOTER_ICON_SIZE,
                self.button_mouse_states.copy_version.clone(),
            )
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(WorkspaceAction::CopyVersion(version))
            })
            .finish();

        Container::new(
            Flex::row()
                .with_child(Shrinkable::new(1., Align::new(text).left().finish()).finish())
                .with_child(Shrinkable::new(0.2, Align::new(copy_icon).finish()).finish())
                .with_main_axis_size(MainAxisSize::Max)
                .finish(),
        )
        .with_margin_left(SECTION_SPACING)
        .with_margin_bottom(BUTTON_PADDING)
        .with_uniform_padding(BUTTON_PADDING)
        .finish()
    }

    fn render_invite_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        let default_styles = UiComponentStyles {
            font_size: Some(DETAIL_FONT_SIZE),
            font_family_id: Some(appearance.ui_font_family()),
            font_color: Some(appearance.theme().accent().into()),
            border_radius: Some(CornerRadius::with_all(Radius::Percentage(20.))),
            border_width: Some(1.),
            border_color: Some(appearance.theme().accent().into()),
            padding: Some(Coords {
                top: BUTTON_PADDING,
                bottom: BUTTON_PADDING,
                ..Default::default()
            }),
            ..Default::default()
        };

        let hovered_styles = UiComponentStyles {
            background: Some(appearance.theme().accent().into()),
            font_color: Some(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().accent())
                    .into_solid(),
            ),
            ..default_styles
        };

        let clicked_color = appearance.theme().accent().blend(
            &FillTheme::black().with_opacity(*appearance.theme().details().button_click_opacity()),
        );
        let clicked_styles = UiComponentStyles {
            background: Some(clicked_color.into()),
            border_color: Some(clicked_color.into()),
            ..hovered_styles
        };

        Container::new(
            appearance
                .ui_builder()
                .button_with_custom_styles(
                    ButtonVariant::Outlined,
                    self.button_mouse_states.invite_people.clone(),
                    default_styles,
                    Some(hovered_styles),
                    Some(clicked_styles),
                    None,
                )
                .with_text_and_icon_label(
                    TextAndIcon::new(
                        TextAndIconAlignment::IconFirst,
                        "Invite a friend to Warp",
                        Icon::new(SEND_SVG_PATH, appearance.theme().accent()),
                        MainAxisSize::Max,
                        MainAxisAlignment::Center,
                        vec2f(FOOTER_ICON_SIZE, FOOTER_ICON_SIZE),
                    )
                    .with_inner_padding(BUTTON_PADDING),
                )
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(WorkspaceAction::ShowReferralSettingsPage)
                })
                .finish(),
        )
        .with_margin_top(SECTION_SPACING)
        .with_margin_bottom(SECTION_SPACING_BOTTOM)
        .with_margin_left(SECTION_SPACING + SCROLLBAR_OFFSET)
        .with_margin_right(SECTION_SPACING + SCROLLBAR_OFFSET)
        .finish()
    }

    fn render_skip_tips_button(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mark_all_read_label = crate::i18n::tr_static(app, "Mark all as read").to_string();
        Container::new(
            Align::new(
                Hoverable::new(self.button_mouse_states.skip_tips.clone(), |state| {
                    let text_color = if state.is_hovered() {
                        appearance.theme().active_ui_text_color().into_solid()
                    } else {
                        appearance.theme().nonactive_ui_text_color().into_solid()
                    };

                    let style = UiComponentStyles {
                        font_size: Some(DETAIL_FONT_SIZE),
                        font_color: Some(text_color),
                        ..Default::default()
                    };

                    appearance
                        .ui_builder()
                        .wrappable_text(mark_all_read_label.clone(), false)
                        .with_style(style)
                        .build()
                        .finish()
                })
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ResourceCenterMainAction::SkipTips)
                })
                .with_cursor(Cursor::PointingHand)
                .finish(),
            )
            .right()
            .finish(),
        )
        .with_margin_bottom(SECTION_SPACING)
        .with_margin_right(SCROLLBAR_OFFSET + SECTION_SPACING)
        .finish()
    }
}

/// A model for tracking where the events from the resource center view should be dispatched
///
/// Similar to command palette - we need a model to cache the information of where
/// we should send the actions from the resource center features. When the resource center is opened,
/// we cache the current active window ID as well as the input ID of the active
/// tab/pane. By sending all the actions to the input view, we ensure that
/// they propagate correctly. This propagation assumes that each feature action
/// must be in the responder chain. If an action is not in the responder chain
/// (such as a block navigation action) then it won't propagate correctly.
pub enum ActionTarget {
    None,
    View {
        window_id: WindowId,
        input_id: Option<EntityId>,
    },
}

impl Entity for ActionTarget {
    type Event = ();
}

impl Entity for ResourceCenterMainView {
    type Event = ResourceCenterMainEvent;
}

impl TypedActionView for ResourceCenterMainView {
    type Action = ResourceCenterMainAction;

    fn handle_action(&mut self, action: &ResourceCenterMainAction, ctx: &mut ViewContext<Self>) {
        use ResourceCenterMainAction::*;
        match action {
            Close => {
                ctx.notify();
                ctx.emit(ResourceCenterMainEvent::Close);
            }
            SkipTips => {
                send_telemetry_from_ctx!(TelemetryEvent::ResourceCenterTipsSkipped, ctx);
                self.tips_completed.update(ctx, |tips_completed, ctx| {
                    skip_tips_and_write_to_user_defaults(tips_completed, ctx);
                    ctx.notify();
                });
            }
        }
    }
}

impl View for ResourceCenterMainView {
    fn ui_name() -> &'static str {
        "ResourceCenterMain"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let body = self.render_body(appearance);
        let invite_button = self.render_invite_button(appearance);
        let skip_tips = self.render_skip_tips_button(appearance, app);

        let mut main_page = Flex::column();

        if !AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out()
            && !FeatureFlag::AvatarInTabBar.is_enabled()
        {
            main_page = main_page.with_child(invite_button);
        }

        if !self.tips_completed.as_ref(app).skipped_or_completed
            && !FeatureFlag::AvatarInTabBar.is_enabled()
        {
            main_page.add_child(skip_tips);
        }

        main_page = main_page
            .with_child(Shrinkable::new(20., body).finish())
            .with_child(Shrinkable::new(0.1, Empty::new().finish()).finish()); // placeholder to ensure pane extends to bottom of the window

        if FeatureFlag::Autoupdate.is_enabled() && ChannelState::show_autoupdate_menu_items() {
            let current_version = self.render_current_version(appearance);
            main_page.add_child(current_version);
        }

        main_page.finish()
    }
}
