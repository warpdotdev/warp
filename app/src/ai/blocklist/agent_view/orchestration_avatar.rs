use warpui::elements::Element;
use warpui::{AppContext, SingletonEntity};

use crate::ai::blocklist::agent_view::orchestration_pill_bar::{
    render_agent_avatar_disc, render_orchestrator_avatar_disc,
};
use crate::appearance::Appearance;

const TRANSCRIPT_AVATAR_SCALE: f32 = 1.25;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OrchestrationAvatar {
    Orchestrator,
    Agent { display_name: String },
}

impl OrchestrationAvatar {
    pub(crate) fn agent(display_name: String) -> Self {
        Self::Agent { display_name }
    }

    pub(crate) fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let size = app.font_cache().line_height(
            appearance.monospace_font_size(),
            appearance.line_height_ratio(),
        ) * TRANSCRIPT_AVATAR_SCALE;

        match self {
            Self::Orchestrator => render_orchestrator_avatar_disc(size, theme, appearance),
            Self::Agent { display_name } => {
                render_agent_avatar_disc(display_name, size, theme, appearance)
            }
        }
    }
}

#[cfg(test)]
#[path = "orchestration_avatar_tests.rs"]
mod tests;
