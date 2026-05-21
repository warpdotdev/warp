#![cfg_attr(target_family = "wasm", allow(dead_code, unused_imports))]
// Adding this file level gate as some of the code around editability is not used in WASM yet.

use warp_core::ui::{appearance::Appearance, theme::Fill};
use warp_editor::model::CoreEditorModel;
use warpui::{
    elements::{
        Align, Border, ConstrainedBox, Container, CrossAxisAlignment, Flex, MouseStateHandle,
        ParentElement, Shrinkable,
    },
    presenter::ChildView,
    ui_components::{
        button::ButtonVariant,
        components::{UiComponent, UiComponentStyles},
    },
    units::IntoPixels,
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::{
    editor::InteractionState,
    i18n::{self, I18nKey},
    ui_components::icons::Icon,
    view_components::action_button::{ActionButton, ButtonSize, NakedTheme},
    view_components::find::FIND_BAR_PADDING,
};

use super::model::{CodeEditorModel, CodeEditorModelEvent};

const NAV_BAR_HEIGHT: f32 = 40.;
const NAV_BAR_ICON_SIZE: f32 = 16.;
const NAV_BAR_ICON_PADDING: f32 = 4.;
const NAV_BAR_SEPARATOR_PADDING: f32 = 12.;

// The ratio of rows to offset of when jumping to a diff nav (base is the total number of lines in viewport)
const DIFF_NAV_OFFSET_PIXEL_RATIO: usize = 10;

#[derive(Debug, Clone, Copy)]
pub enum NavBarEvent {
    Close,
}

#[derive(Debug, Clone, Copy)]
pub enum NavBarAction {
    NavigateUp,
    NavigateDown,
    Revert,
    Close,
}

#[derive(Default)]
struct MouseStateHandles {
    close_mouse_state: MouseStateHandle,
    revert_mouse_state: MouseStateHandle,
}

pub enum NavBarBehavior {
    Closable,
    NotClosable,
}

pub struct NavBar {
    model: ModelHandle<CodeEditorModel>,
    behavior: NavBarBehavior,
    mouse_state_handles: MouseStateHandles,
    up_label_button: ViewHandle<ActionButton>,
    down_label_button: ViewHandle<ActionButton>,
}

impl NavBar {
    pub fn new(model: ModelHandle<CodeEditorModel>, ctx: &mut ViewContext<Self>) -> Self {
        ctx.subscribe_to_model(&model, |_, _, event, ctx| {
            if matches!(event, CodeEditorModelEvent::InteractionStateChanged) {
                ctx.notify();
            }
        });

        let up_label_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(i18n::tr(ctx, I18nKey::CommonPrevious), NakedTheme)
                .with_size(ButtonSize::InlineActionHeader)
                .with_icon(Icon::ArrowUp)
                .on_click(|ctx| ctx.dispatch_typed_action(NavBarAction::NavigateUp))
        });

        let down_label_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(i18n::tr(ctx, I18nKey::CommonNext), NakedTheme)
                .with_size(ButtonSize::InlineActionHeader)
                .with_icon(Icon::ArrowDown)
                .on_click(|ctx| ctx.dispatch_typed_action(NavBarAction::NavigateDown))
        });

        Self {
            model,
            behavior: NavBarBehavior::Closable,
            mouse_state_handles: Default::default(),
            up_label_button,
            down_label_button,
        }
    }

    pub fn set_behavior(&mut self, behavior: NavBarBehavior) {
        self.behavior = behavior;
    }

    fn diff_hunk_count(&self, app: &AppContext) -> usize {
        self.model.as_ref(app).diff().as_ref(app).diff_hunk_count()
    }

    pub fn selected_index(&self, app: &AppContext) -> Option<usize> {
        self.model.as_ref(app).focused_diff_index()
    }

    /// Autoscroll until the start of the selected hunk is in the center of the viewport.
    pub fn autoscroll(&self, ctx: &mut ViewContext<Self>) {
        let model = self.model.as_ref(ctx);

        let Some(index) = self.selected_index(ctx) else {
            return;
        };

        let Some(range) = model
            .diff()
            .as_ref(ctx)
            .line_range_by_diff_hunk_index(index)
        else {
            return;
        };

        let character_offset = model.start_of_line_offset(range.start, ctx);

        // Number of lines to offset when autoscrolling to a diff. Keep a minimum of 1 line as context.
        let delta = (model.lines_in_viewport(ctx) / DIFF_NAV_OFFSET_PIXEL_RATIO).max(1);
        let pixel_offset = -(delta as f32 * model.line_height(ctx));

        model
            .render_state()
            .clone()
            .update(ctx, |render_state, _ctx| {
                render_state.request_autoscroll_to_exact_vertical(
                    character_offset,
                    pixel_offset.into_pixels(),
                );
            })
    }

    fn render_match_index(
        &self,
        appearance: &Appearance,
        background: Fill,
        total: usize,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let diff_text = appearance
            .ui_builder()
            .span(i18n::tr(app, I18nKey::CodeHunk))
            .with_style(UiComponentStyles {
                font_color: Some(appearance.theme().sub_text_color(background).into()),
                ..Default::default()
            })
            .with_selectable(false)
            .build()
            .finish();

        let index = (self.selected_index(app).unwrap_or(0) + 1).min(total);
        let text = format!("{index}/{total}");

        let index = Container::new(
            appearance
                .ui_builder()
                .span(text)
                .with_style(UiComponentStyles {
                    font_color: Some(appearance.theme().foreground().into()),
                    ..Default::default()
                })
                .with_selectable(false)
                .build()
                .finish(),
        )
        .with_vertical_padding(4.)
        .with_horizontal_padding(8.)
        .with_border(Border::new(1.).with_border_fill(appearance.theme().surface_2()))
        .finish();

        Align::new(
            Container::new(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(diff_text)
                    .with_child(index)
                    .finish(),
            )
            .with_padding_right(16.)
            .finish(),
        )
        .right()
        .finish()
    }

    fn render_revert_button(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        Container::new(
            appearance
                .ui_builder()
                .button(
                    ButtonVariant::Outlined,
                    self.mouse_state_handles.revert_mouse_state.clone(),
                )
                .with_text_label(i18n::tr(app, I18nKey::CommonReject).to_string())
                .build()
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(NavBarAction::Revert))
                .finish(),
        )
        .with_padding_left(NAV_BAR_SEPARATOR_PADDING)
        .with_padding_right(NAV_BAR_SEPARATOR_PADDING)
        .finish()
    }

    fn render_close_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            appearance
                .ui_builder()
                .close_button(
                    NAV_BAR_ICON_SIZE,
                    self.mouse_state_handles.close_mouse_state.clone(),
                )
                .build()
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(NavBarAction::Close))
                .finish(),
        )
        .with_uniform_padding(NAV_BAR_ICON_PADDING)
        .finish()
    }

    fn render_nav_label(&self, up: bool) -> Box<dyn Element> {
        let button_handle = if up {
            &self.up_label_button
        } else {
            &self.down_label_button
        };

        Container::new(Align::new(ChildView::new(button_handle).finish()).finish())
            .with_padding_right(16.)
            .finish()
    }

    pub fn navigate_up(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.nav_diff_up(ctx);
        });
        self.autoscroll(ctx);
        ctx.notify();
    }

    pub fn navigate_down(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.nav_diff_down(ctx);
        });
        self.autoscroll(ctx);
        ctx.notify();
    }
}

impl Entity for NavBar {
    type Event = NavBarEvent;
}

impl View for NavBar {
    fn ui_name() -> &'static str {
        "NavBar"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let total = self.diff_hunk_count(app);

        let editable = matches!(
            self.model.as_ref(app).interaction_state(),
            InteractionState::Editable
        );

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.,
                    self.render_match_index(
                        appearance,
                        appearance.theme().background(),
                        total,
                        app,
                    ),
                )
                .finish(),
            )
            .with_child(self.render_nav_label(true))
            .with_child(self.render_nav_label(false));

        // Do not render the revert button if there is nothing to revert or the editor is
        // not in an editable interaction state.
        if editable && total > 0 {
            row.add_child(self.render_revert_button(appearance, app));
        }

        if matches!(self.behavior, NavBarBehavior::Closable) {
            row.add_child(self.render_close_button(appearance));
        }

        Container::new(
            ConstrainedBox::new(row.finish())
                .with_height(NAV_BAR_HEIGHT)
                .finish(),
        )
        .with_uniform_padding(FIND_BAR_PADDING)
        .finish()
    }
}

impl TypedActionView for NavBar {
    type Action = NavBarAction;

    fn handle_action(&mut self, action: &NavBarAction, ctx: &mut ViewContext<Self>) {
        match action {
            NavBarAction::Close => ctx.emit(NavBarEvent::Close),
            NavBarAction::NavigateUp => self.navigate_up(ctx),
            NavBarAction::NavigateDown => self.navigate_down(ctx),
            NavBarAction::Revert => {
                self.autoscroll(ctx);
                self.model.update(ctx, |model, ctx| {
                    model.revert_diff_index(ctx);
                });
            }
        }
    }
}
