use warpui::elements::{
    Align, ConstrainedBox, Container, CrossAxisAlignment, Element, Flex, Hoverable, Icon,
    MouseState, MouseStateHandle, ParentElement, Shrinkable,
};
use warpui::fonts::Weight;
use warpui::platform::Cursor;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    Action, AppContext, Entity, EntityId, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, WindowId,
};

use super::{
    SectionAction, SectionView, CHEVRON_ICON_SIZE, DESCRIPTION_FONT_SIZE, ELLIPSE_ICON_SIZE,
    ELLIPSE_SVG_PATH, ICON_PADDING, ITEM_PADDING_BOTTOM, SCROLLBAR_OFFSET, SECTION_SPACING,
};
use crate::appearance::Appearance;
use crate::resource_center::main_page::ActionTarget;
use crate::resource_center::{
    complete_tips_and_write_to_user_defaults, skip_tips_and_write_to_user_defaults, FeatureItem,
    FeatureSectionData, Tip, TipsCompleted,
};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::TelemetryEvent;
use crate::settings_view::keybindings::{KeybindingChangedEvent, KeybindingChangedNotifier};
use crate::themes::theme::Fill;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureSection {
    WhatsNew,
    GettingStarted,
    MaximizeWarp,
    AdvancedSetup,
}

impl FeatureSection {
    pub fn section_name_string(&self) -> &'static str {
        match self {
            FeatureSection::WhatsNew => "What's New?",
            FeatureSection::GettingStarted => "Getting Started",
            FeatureSection::MaximizeWarp => "Maximize Warp",
            FeatureSection::AdvancedSetup => "Advanced Setup",
        }
    }
}

#[derive(Default)]
struct FeatureMouseStateHandles {
    item_handles: Vec<MouseStateHandle>,
    top_bar_mouse_state: MouseStateHandle,
}

pub enum FeatureSectionEvent {
    /// Event fired when the tips dialog should close.
    CloseResourceCenter,
    ExpandSection(FeatureSection),
}

pub struct FeatureSectionView {
    pub feature_section_data: FeatureSectionData,
    action_target: ModelHandle<ActionTarget>,
    feature_button_mouse_states: FeatureMouseStateHandles,
    tips_completed: ModelHandle<TipsCompleted>,
    show_tips_progress: bool,
    is_expanded: bool,
}

impl FeatureSectionView {
    fn on_tips_model_changed(
        &mut self,
        _: ModelHandle<TipsCompleted>,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.notify();
    }

    pub fn new(
        feature_section_data: FeatureSectionData,
        action_target: ModelHandle<ActionTarget>,
        ctx: &mut ViewContext<Self>,
        tips_completed: ModelHandle<TipsCompleted>,
        show_tips_progress: bool,
        is_expanded: bool,
    ) -> Self {
        let feature_button_mouse_states = FeatureMouseStateHandles {
            item_handles: feature_section_data
                .items
                .iter()
                .map(|_| Default::default())
                .collect(),
            ..Default::default()
        };

        ctx.observe(&tips_completed, FeatureSectionView::on_tips_model_changed);

        let bindings_notifier = KeybindingChangedNotifier::handle(ctx);
        ctx.subscribe_to_model(&bindings_notifier, |me, _, event, ctx| {
            me.handle_keybinding_changed(event, ctx);
        });

        Self {
            feature_section_data,
            action_target,
            feature_button_mouse_states,
            tips_completed,
            show_tips_progress,
            is_expanded,
        }
    }

    fn handle_keybinding_changed(
        &mut self,
        event: &KeybindingChangedEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            KeybindingChangedEvent::BindingChanged {
                binding_name,
                new_trigger,
            } => {
                if let Some(binding) = self
                    .feature_section_data
                    .items
                    .iter_mut()
                    .find(|data| data.editable_binding_name == Some(binding_name))
                {
                    binding.shortcut.clone_from(new_trigger);
                    ctx.notify();
                }
            }
        }
    }

    pub fn expand_section(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_expanded = true;

        ctx.notify();
    }

    pub fn toggle_expanded(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_expanded = !self.is_expanded;

        ctx.notify();
    }

    // Turns gamification off without rendering completed modal
    pub fn skip_gamified_section(&mut self, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(TelemetryEvent::ResourceCenterTipsSkipped, ctx);
        self.tips_completed.update(ctx, |tips_completed, ctx| {
            skip_tips_and_write_to_user_defaults(tips_completed, ctx);
            ctx.notify();
        });
    }

    // Turns gamification off and renders completed modal
    pub fn complete_gamified_section(&mut self, ctx: &mut ViewContext<Self>) {
        send_telemetry_from_ctx!(TelemetryEvent::ResourceCenterTipsCompleted, ctx);
        self.tips_completed.update(ctx, |tips_completed, ctx| {
            complete_tips_and_write_to_user_defaults(tips_completed, ctx);
            ctx.notify();
        });
    }

    pub fn set_action_target(
        &mut self,
        window_id: WindowId,
        input_id: Option<EntityId>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.action_target.update(ctx, |action_target, ctx| {
            *action_target = ActionTarget::View {
                window_id,
                input_id,
            };
            ctx.notify();
        });
    }

    pub fn dispatch_feature_action(&self, action: &dyn Action, ctx: &mut ViewContext<Self>) {
        let (window_id, input_id) = match self.action_target.as_ref(ctx) {
            ActionTarget::View {
                window_id,
                input_id,
            } => (*window_id, *input_id),
            ActionTarget::None => return,
        };

        if let Some(input_id) = input_id {
            ctx.dispatch_typed_action_for_view(window_id, input_id, action);
        }
    }

    fn render_unread_icon(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(Icon::new(ELLIPSE_SVG_PATH, appearance.theme().accent()).finish())
                .with_height(ELLIPSE_ICON_SIZE)
                .with_width(ELLIPSE_ICON_SIZE)
                .finish(),
        )
        .with_padding_top(ELLIPSE_ICON_SIZE)
        .with_padding_right(ELLIPSE_ICON_SIZE)
        .finish()
    }

    fn render_item_title(&self, item: &FeatureItem, appearance: &Appearance) -> Box<dyn Element> {
        let title_color = appearance.theme().active_ui_text_color();

        Align::new(
            Container::new(
                appearance
                    .ui_builder()
                    .wrappable_text(item.title.to_string(), true)
                    .with_style(UiComponentStyles {
                        font_size: Some(DESCRIPTION_FONT_SIZE),
                        font_color: (Some(title_color.into())),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_padding_top(3.)
            .finish(),
        )
        .left()
        .finish()
    }

    fn render_description(
        &self,
        item: &FeatureItem,
        appearance: &Appearance,
        color: Fill,
    ) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .wrappable_text(item.description.to_string(), true)
            .with_style(UiComponentStyles {
                font_size: Some(DESCRIPTION_FONT_SIZE),
                font_color: Some(color.into()),
                ..Default::default()
            })
            .build()
            .finish()
    }

    pub fn build_feature_item(
        &self,
        item: &FeatureItem,
        appearance: &Appearance,
        state: Option<&MouseState>,
        is_completed: bool,
        show_gamified: bool,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder();
        let mut element = Flex::column();
        let mut element_title = Flex::row();

        // title
        element_title
            .add_child(Shrinkable::new(1., self.render_item_title(item, appearance)).finish());

        // keyboard shortcut
        if let Some(keystroke) = &item.shortcut {
            element_title.add_child(ui_builder.keyboard_shortcut(keystroke).build().finish())
        }

        element.add_child(
            Container::new(
                element_title
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish(),
            )
            .with_padding_bottom(ITEM_PADDING_BOTTOM)
            .finish(),
        );

        let hovered = state.is_some() && state.expect("Expected valid mouse state").is_hovered();
        let description_color = if hovered && matches!(item.feature, Tip::Action(_)) {
            theme.active_ui_text_color()
        } else {
            theme.nonactive_ui_text_color()
        };

        // description
        element.add_child(self.render_description(item, appearance, description_color));

        let mut feature_item = Flex::row();
        if !is_completed && show_gamified {
            feature_item.add_child(self.render_unread_icon(appearance));
        }
        feature_item.add_child(Shrinkable::new(1., element.finish()).finish());

        let margin_left = if is_completed || !show_gamified {
            CHEVRON_ICON_SIZE + ICON_PADDING
        } else {
            SCROLLBAR_OFFSET
        };
        Container::new(feature_item.finish())
            .with_margin_bottom(SECTION_SPACING)
            .with_margin_left(margin_left)
            .finish()
    }

    pub fn render_feature_item(
        &self,
        feature_item: FeatureItem,
        appearance: &Appearance,
        index: usize,
        is_tip_completed: bool,
        show_gamified: bool,
    ) -> Box<dyn Element> {
        match feature_item.feature {
            Tip::Hint(_) => self.build_feature_item(
                &feature_item,
                appearance,
                None,
                is_tip_completed,
                show_gamified,
            ),
            Tip::Action(tip) => {
                let item_element = Hoverable::new(
                    self.feature_button_mouse_states.item_handles[index].clone(),
                    |state| {
                        self.build_feature_item(
                            &feature_item,
                            appearance,
                            Some(state),
                            is_tip_completed,
                            show_gamified,
                        )
                    },
                );
                item_element
                    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(SectionAction::Click(tip)))
                    .with_cursor(Cursor::PointingHand)
                    .finish()
            }
        }
    }
}

impl Entity for FeatureSectionView {
    type Event = FeatureSectionEvent;
}

impl TypedActionView for FeatureSectionView {
    type Action = SectionAction;

    fn handle_action(&mut self, action: &SectionAction, ctx: &mut ViewContext<Self>) {
        match action {
            SectionAction::Click(feature) => {
                let action = ctx
                    .editable_bindings()
                    .find(|action| action.name == feature.editable_binding_name())
                    .map(|action| action.action.clone());

                if let Some(action) = action {
                    self.dispatch_feature_action(action.as_ref(), ctx);
                }
            }
            SectionAction::ToggleExpanded => {
                self.toggle_expanded(ctx);
            }
            SectionAction::CloseResourceCenter => {
                self.toggle_expanded(ctx);
                ctx.emit(FeatureSectionEvent::CloseResourceCenter);
                ctx.notify();
            }
            SectionAction::CompleteGamified => {
                self.complete_gamified_section(ctx);
                self.toggle_expanded(ctx);
            }
            SectionAction::SkipTips => {
                self.skip_gamified_section(ctx);
                self.toggle_expanded(ctx);
            }
            SectionAction::OpenSection(section_name) => {
                ctx.emit(FeatureSectionEvent::ExpandSection(*section_name));
                ctx.notify();
            }
            _ => {}
        }
    }
}

impl SectionView for FeatureSectionView {
    fn is_expanded(&self) -> bool {
        self.is_expanded
    }

    fn toggle_expanded(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_expanded = !self.is_expanded;
        ctx.notify();
    }

    fn section_progress_indicator(
        &self,
        show_gamified: bool,
        appearance: &Appearance,
        ctx: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let tip_count = self.feature_section_data.items.len();
        let tips_completed_count = self
            .feature_section_data
            .tips_completed_count(self.tips_completed.as_ref(ctx));

        // Show progress when section's tips are not yet completed
        if show_gamified && self.show_tips_progress && tips_completed_count != tip_count {
            let progress = format!("{tips_completed_count}/{tip_count}");
            Some(
                appearance
                    .ui_builder()
                    .wrappable_text(progress, false)
                    .with_style(UiComponentStyles {
                        font_family_id: Some(appearance.ui_font_family()),
                        font_size: Some(DESCRIPTION_FONT_SIZE),
                        font_weight: Some(Weight::Semibold),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
        } else {
            None
        }
    }

    fn section_link(&self, _appearance: &Appearance) -> Option<Box<dyn Element>> {
        None
    }
}

impl View for FeatureSectionView {
    fn ui_name() -> &'static str {
        "ResourceCenterFeatureSectionView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let tips_completed = self.tips_completed.as_ref(app);
        let show_gamified = !tips_completed.skipped_or_completed;

        let header = self.render_section_header(
            self.feature_section_data.section_name,
            show_gamified,
            appearance,
            self.feature_button_mouse_states.top_bar_mouse_state.clone(),
            app,
        );

        let mut section = Flex::column().with_child(header);
        if self.is_expanded {
            let mut feature_section = Container::new(
                Flex::column()
                    .with_children(self.feature_section_data.items.iter().enumerate().map(
                        |(index, feature_item)| {
                            self.render_feature_item(
                                feature_item.clone(),
                                appearance,
                                index,
                                tips_completed.features_used.contains(&feature_item.feature),
                                show_gamified,
                            )
                        },
                    ))
                    .finish(),
            );

            if !self.feature_section_data.items.is_empty() {
                feature_section = feature_section.with_uniform_padding(SECTION_SPACING)
            }

            section.add_child(feature_section.finish());
        }

        ConstrainedBox::new(Container::new(section.finish()).finish()).finish()
    }
}
