use warp_core::ui::appearance::Appearance;
use warpui::{
    elements::ParentElement,
    prelude::{Container, Empty, Flex, Text},
    AppContext, Element, Entity, ModelHandle, SingletonEntity, View, ViewContext,
};

use crate::{
    ai::blocklist::block::view_impl::{
        query::render_query, WithContentItemSpacing, CONTENT_VERTICAL_PADDING,
    },
    auth::AuthStateProvider,
    terminal::view::ambient_agent::{AmbientAgentViewModel, AmbientAgentViewModelEvent},
    workspace::view::DEFAULT_USER_DISPLAY_NAME,
};

/// Renders the submitted prompt immediately while the environment is still being set up, so cloud mode feels like local agent mode even before the first exchange is available in history.
pub struct CloudModeInitialUserQuery {
    view_model: ModelHandle<AmbientAgentViewModel>,
}

impl CloudModeInitialUserQuery {
    pub fn new(
        view_model: ModelHandle<AmbientAgentViewModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&view_model, |_, _, event, ctx| match event {
            AmbientAgentViewModelEvent::DispatchedAgent
            | AmbientAgentViewModelEvent::ProgressUpdated
            | AmbientAgentViewModelEvent::SessionReady { .. }
            | AmbientAgentViewModelEvent::Failed { .. }
            | AmbientAgentViewModelEvent::NeedsGithubAuth
            | AmbientAgentViewModelEvent::Cancelled => ctx.notify(),
            _ => (),
        });
        Self { view_model }
    }
}

impl Entity for CloudModeInitialUserQuery {
    type Event = ();
}

impl View for CloudModeInitialUserQuery {
    fn ui_name() -> &'static str {
        "InitialUserQuery"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let Some(request) = self.view_model.as_ref(app).request() else {
            return Empty::new().finish();
        };

        render_user_query(&request.prompt, &self.view_model, app)
    }
}

fn render_user_query(
    prompt: &str,
    view_model: &ModelHandle<AmbientAgentViewModel>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let ambient_agent_model = view_model.as_ref(app);
    let auth_state = AuthStateProvider::as_ref(app).get().clone();
    let user_display_name = auth_state
        .username_for_display()
        .unwrap_or_else(|| DEFAULT_USER_DISPLAY_NAME.to_owned());
    let profile_image_url = auth_state.user_photo_url();

    let mut column = Flex::column().with_child(
        render_query(
            prompt,
            &user_display_name,
            profile_image_url.as_ref(),
            None,
            &Default::default(),
            &Default::default(),
            0,
            None,
            false,
            true,
            &[],
            None,
            app,
        )
        .with_content_item_spacing()
        .finish(),
    );

    if ambient_agent_model.error_message().is_some() {
        column.add_child(
            Text::new(
                "Failed",
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .finish()
            .with_agent_output_item_spacing(app)
            .finish(),
        )
    }

    Container::new(column.finish())
        .with_padding_top(CONTENT_VERTICAL_PADDING)
        .finish()
}
