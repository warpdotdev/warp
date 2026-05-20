use warp_core::ui::appearance::Appearance;
use warp_terminal::model::BlockId;
use warpui::{
    elements::{
        ConstrainedBox, CrossAxisAlignment, Flex, Hoverable, MainAxisSize, ParentElement,
        Shrinkable, Text,
    },
    platform::Cursor,
    prelude::{Container, MouseStateHandle},
    text_layout::ClipConfig,
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::{
    terminal::{view::PADDING_LEFT, CLIAgent},
    ui_components::icons::Icon,
};

const CHEVRON_SIZE: f32 = 14.;
const COLLAPSED_BOTTOM_PADDING: f32 = 8.;

pub struct HarnessSessionHeader {
    block_id: BlockId,
    cli_name: String,
    is_expanded: bool,
    mouse_state: MouseStateHandle,
}

#[derive(Debug, Clone)]
pub enum HarnessSessionHeaderEvent {
    ToggleCommandGridVisibility(BlockId),
}

impl HarnessSessionHeader {
    pub fn new(block_id: BlockId, cli_agent: Option<CLIAgent>) -> Self {
        let cli_name = cli_agent
            .map(|agent| agent.display_name().to_owned())
            .unwrap_or_else(|| "Agent".to_owned());

        Self {
            block_id,
            cli_name,
            is_expanded: false,
            mouse_state: Default::default(),
        }
    }
}

impl Entity for HarnessSessionHeader {
    type Event = HarnessSessionHeaderEvent;
}

impl View for HarnessSessionHeader {
    fn ui_name() -> &'static str {
        "HarnessSessionHeader"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let text_color = theme.main_text_color(theme.background()).into_solid();

        let chevron_icon = if self.is_expanded {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        };

        let label = format!("Running {}...", self.cli_name);

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(
                Shrinkable::new(
                    1.,
                    Text::new(
                        label,
                        appearance.ai_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(text_color)
                    .soft_wrap(false)
                    .with_clip(ClipConfig::ellipsis())
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Container::new(
                    ConstrainedBox::new(
                        chevron_icon
                            .to_warpui_icon(theme.sub_text_color(theme.background()))
                            .finish(),
                    )
                    .with_width(CHEVRON_SIZE)
                    .with_height(CHEVRON_SIZE)
                    .finish(),
                )
                .finish(),
            );

        let bottom_padding = if self.is_expanded {
            0.
        } else {
            COLLAPSED_BOTTOM_PADDING
        };

        Hoverable::new(self.mouse_state.clone(), move |_| {
            Container::new(row.finish())
                .with_margin_left(*PADDING_LEFT)
                .with_margin_right(*PADDING_LEFT)
                .with_padding_top(4.)
                .with_padding_bottom(bottom_padding)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(HarnessSessionHeaderAction::ToggleCommandGridVisibility)
        })
        .finish()
    }
}

#[derive(Debug, Clone)]
pub enum HarnessSessionHeaderAction {
    ToggleCommandGridVisibility,
}

impl TypedActionView for HarnessSessionHeader {
    type Action = HarnessSessionHeaderAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            HarnessSessionHeaderAction::ToggleCommandGridVisibility => {
                self.is_expanded = !self.is_expanded;
                ctx.emit(HarnessSessionHeaderEvent::ToggleCommandGridVisibility(
                    self.block_id.clone(),
                ));
                ctx.notify();
            }
        }
    }
}
