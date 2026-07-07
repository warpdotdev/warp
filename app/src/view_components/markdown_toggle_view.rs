use warpui::elements::{CornerRadius, Fill as UiFill, Radius};
use warpui::presenter::ChildView;
use warpui::ui_components::components::UiComponentStyles;
use warpui::ui_components::segmented_control::{
    LabelConfig, RenderableOptionConfig, SegmentedControl, SegmentedControlEvent,
};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::notebooks::file::MarkdownDisplayMode;

#[derive(Debug, Clone)]
pub enum MarkdownToggleEvent {
    ModeSelected(MarkdownDisplayMode),
}

pub struct MarkdownToggleView {
    segmented_control: ViewHandle<SegmentedControl<MarkdownDisplayMode>>,
}

impl MarkdownToggleView {
    pub fn new(default_mode: MarkdownDisplayMode, ctx: &mut ViewContext<Self>) -> Self {
        let segmented_control = ctx.add_typed_action_view(move |ctx| {
            SegmentedControl::new(
                vec![MarkdownDisplayMode::Rendered, MarkdownDisplayMode::Raw],
                |mode, is_selected, app| {
                    let appearance = Appearance::as_ref(app);
                    let theme = appearance.theme();

                    Some(RenderableOptionConfig {
                        icon_path: "",
                        icon_color: theme.main_text_color(theme.background()).into(),
                        label: Some(LabelConfig {
                            label: match mode {
                                MarkdownDisplayMode::Rendered => "Rendered".into(),
                                MarkdownDisplayMode::Raw => "Raw".into(),
                            },
                            width_override: Some(55.0),
                            color: if is_selected {
                                theme.accent().into()
                            } else {
                                theme.main_text_color(theme.background()).into()
                            },
                        }),
                        tooltip: None,
                        background: if is_selected {
                            UiFill::Solid(theme.surface_3().into())
                        } else {
                            UiFill::None
                        },
                    })
                },
                default_mode,
                markdown_toggle_styles(ctx),
            )
        });

        ctx.subscribe_to_view(&segmented_control, |_, _, event, ctx| {
            let SegmentedControlEvent::OptionSelected(mode) = event;
            ctx.emit(MarkdownToggleEvent::ModeSelected(*mode));
        });

        ctx.subscribe_to_model(&Appearance::handle(ctx), |me, _, _, ctx| {
            me.segmented_control.update(ctx, |segmented_control, ctx| {
                segmented_control.set_styles(markdown_toggle_styles(ctx), ctx);
            });
            ctx.notify();
        });

        Self { segmented_control }
    }

    pub fn set_selected_mode(&mut self, mode: MarkdownDisplayMode, ctx: &mut ViewContext<Self>) {
        self.segmented_control.update(ctx, |control, ctx| {
            control.set_selected_option(mode, ctx);
        });
        ctx.notify();
    }
}

impl View for MarkdownToggleView {
    fn ui_name() -> &'static str {
        "MarkdownToggleView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.segmented_control).finish()
    }
}

impl TypedActionView for MarkdownToggleView {
    type Action = ();

    fn handle_action(&mut self, _action: &Self::Action, ctx: &mut ViewContext<Self>) {
        ctx.notify();
    }
}

impl Entity for MarkdownToggleView {
    type Event = MarkdownToggleEvent;
}

fn markdown_toggle_styles(app: &AppContext) -> UiComponentStyles {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();

    UiComponentStyles {
        font_family_id: Some(appearance.ui_font_family()),
        font_size: Some(appearance.ui_font_size()),
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.0))),
        border_width: Some(1.0),
        border_color: Some(UiFill::Solid(theme.surface_3().into())),
        background: Some(UiFill::Solid(theme.background().into())),
        height: Some(20.0),
        padding: Some(warpui::ui_components::components::Coords::uniform(0.0)),
        margin: Some(warpui::ui_components::components::Coords {
            top: 0.0,
            bottom: 0.0,
            left: 0.0,
            right: 8.0,
        }),
        ..Default::default()
    }
}
