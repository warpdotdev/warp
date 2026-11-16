use warp_core::ui::appearance::Appearance;
use warpui::elements::Element;
use warpui::prelude::Container;
use warpui::scene::Border;
use warpui::{AppContext, Entity, ModelHandle, SingletonEntity, View, ViewContext};

use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewControllerEvent};
use crate::terminal::input::inline_menu::model::InlineMenuModel;
use crate::terminal::input::inline_menu::{
    InlineMenuAction, InlineMenuMessageProvider, InlineMenuPositioner,
};
use crate::terminal::input::message_bar::common::render_standard_message_bar;
use crate::terminal::input::message_bar::{EmptyMessageProducer, MessageProvider};

pub struct InlineMenuMessageBarArgs<A: InlineMenuAction, T: 'static + Send + Sync = ()> {
    pub inline_menu_model: ModelHandle<InlineMenuModel<A, T>>,
    pub agent_view_controller: ModelHandle<AgentViewController>,
    pub positioner: ModelHandle<InlineMenuPositioner>,
}

/// Renders contextual hint text at the bottom of the agent view status bar.
pub struct InlineMenuMessageBar<A: InlineMenuAction, T: 'static + Send + Sync = ()> {
    inline_menu_model: ModelHandle<InlineMenuModel<A, T>>,
    agent_view_controller: ModelHandle<AgentViewController>,
    positioner: ModelHandle<InlineMenuPositioner>,
}

impl<A: InlineMenuAction, T: 'static + Send + Sync> Entity for InlineMenuMessageBar<A, T> {
    type Event = ();
}

impl<A: InlineMenuAction, T: 'static + Send + Sync> InlineMenuMessageBar<A, T> {
    pub fn new(args: InlineMenuMessageBarArgs<A, T>, ctx: &mut ViewContext<Self>) -> Self {
        let InlineMenuMessageBarArgs {
            inline_menu_model,
            agent_view_controller,
            positioner,
        } = args;

        ctx.subscribe_to_model(&inline_menu_model, |_, _, _, ctx| {
            ctx.notify();
        });
        ctx.subscribe_to_model(&agent_view_controller, |_, _, event, ctx| match event {
            AgentViewControllerEvent::EnteredAgentView { .. }
            | AgentViewControllerEvent::ExitedAgentView { .. } => {
                ctx.notify();
            }
            _ => (),
        });

        Self {
            inline_menu_model,
            agent_view_controller,
            positioner,
        }
    }
}

pub const INLINE_MENU_BORDER_WIDTH: f32 = 1.;

impl<A: InlineMenuAction, T: 'static + Send + Sync> View for InlineMenuMessageBar<A, T> {
    fn ui_name() -> &'static str {
        "InlineMenuMessageBar"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let inline_menu_model = self.inline_menu_model.as_ref(app);

        let message = InlineMenuMessageProvider::<A>::default()
            .produce_message(InlineMenuMessageArgs {
                inline_menu_model,
                app,
            })
            .or_else(|| {
                EmptyMessageProducer.produce_message(InlineMenuMessageArgs {
                    inline_menu_model,
                    app,
                })
            })
            .expect("Empty message producer always returns Some().");

        let message_bar = render_standard_message_bar(message, None, app);
        if !self.agent_view_controller.as_ref(app).is_active() {
            let is_rendering_below_input = self
                .positioner
                .as_ref(app)
                .should_render_inline_menu_below_input();
            Container::new(message_bar)
                .with_border(
                    Border::new(INLINE_MENU_BORDER_WIDTH)
                        .with_sides(
                            is_rendering_below_input,
                            false,
                            !is_rendering_below_input,
                            false,
                        )
                        .with_border_fill(Appearance::as_ref(app).theme().outline()),
                )
                .finish()
        } else {
            message_bar
        }
    }
}

/// Arguments for inline menu message producers.
#[derive(Copy, Clone)]
pub struct InlineMenuMessageArgs<'a, A: InlineMenuAction, T = ()> {
    pub inline_menu_model: &'a InlineMenuModel<A, T>,
    pub app: &'a AppContext,
}
