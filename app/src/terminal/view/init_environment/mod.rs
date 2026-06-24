pub mod mode_selector;

use warpui::elements::{
    ChildView, Container, CornerRadius, CrossAxisAlignment, Empty, Flex, ParentElement, Radius,
    Text,
};
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::ai::agent::icons::yellow_stop_icon;
use crate::ai::blocklist::block::keyboard_navigable_buttons::KeyboardNavigableButtons;
use crate::ai::blocklist::inline_action::inline_action_header::{
    HeaderConfig, INLINE_ACTION_HEADER_VERTICAL_PADDING,
};
use crate::ai::blocklist::inline_action::requested_action::RenderableAction;
use crate::appearance::Appearance;

const EXPLANATION_TEXT: &str = "Environment setup is unavailable in this build.";
const NO_REPOS_HELP_TEXT: &str = "This legacy command is unavailable in this build.";

#[derive(Debug, Clone)]
pub enum InitEnvironmentBlockAction {
    StartSetup,
    Skip,
}

#[derive(Debug)]
pub enum InitEnvironmentBlockEvent {
    StartSetup(Vec<String>, bool),
}

enum SetupState {
    Pending {
        action_view: ViewHandle<KeyboardNavigableButtons>,
    },
    Skipped,
}

pub struct InitEnvironmentBlock {
    setup_state: SetupState,
    repos: Vec<String>,
    use_current_dir: bool,
}

impl InitEnvironmentBlock {
    pub fn try_steal_focus(&self, ctx: &mut ViewContext<Self>) {
        if let SetupState::Pending { action_view } = &self.setup_state {
            ctx.focus(action_view);
        }
    }

    pub fn completed(&self) -> bool {
        matches!(self.setup_state, SetupState::Skipped)
    }

    pub fn handle_ctrl_c(&mut self, ctx: &mut ViewContext<Self>) {
        if self.completed() {
            return;
        }

        // Cancel the active action by transitioning to Skipped state
        if matches!(self.setup_state, SetupState::Pending { .. }) {
            self.setup_state = SetupState::Skipped;
            ctx.notify();
        }
    }

    pub fn new(
        label: String,
        repos: Vec<String>,
        use_current_dir: bool,
        _ctx: &mut ViewContext<Self>,
    ) -> Self {
        let _ = label;

        Self {
            setup_state: SetupState::Skipped,
            repos,
            use_current_dir,
        }
    }

    fn render_pending_step(
        &self,
        action_view: &ViewHandle<KeyboardNavigableButtons>,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // Add help text if we don't have any repos to make it clearer
        if self.repos.is_empty() && !self.use_current_dir {
            let help_text = Text::new(
                NO_REPOS_HELP_TEXT,
                appearance.ui_font_family(),
                appearance.monospace_font_size() - 2.,
            )
            .with_color(theme.nonactive_ui_text_color().into_solid())
            .soft_wrap(true)
            .finish();
            content.add_child(
                Container::new(help_text)
                    .with_margin_bottom(INLINE_ACTION_HEADER_VERTICAL_PADDING)
                    .finish(),
            );
        }
        content.add_child(ChildView::new(action_view).finish());

        RenderableAction::new_with_element(content.finish(), app)
            .with_header(
                HeaderConfig::new(EXPLANATION_TEXT, app)
                    .with_icon(yellow_stop_icon(appearance))
                    .with_corner_radius_override(CornerRadius::with_top(Radius::Pixels(8.)))
                    .with_soft_wrap_title(),
            )
            .with_background_color(theme.surface_1().into_solid())
            .render(app)
            .finish()
    }
}

impl Entity for InitEnvironmentBlock {
    type Event = InitEnvironmentBlockEvent;
}

impl View for InitEnvironmentBlock {
    fn ui_name() -> &'static str {
        "InitEnvironmentBlock"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let rendered_step = match &self.setup_state {
            SetupState::Pending { action_view } => self.render_pending_step(action_view, app),
            SetupState::Skipped => Empty::new().finish(),
        };
        Container::new(rendered_step).with_padding_top(16.).finish()
    }
}

impl TypedActionView for InitEnvironmentBlock {
    type Action = InitEnvironmentBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            InitEnvironmentBlockAction::StartSetup => {
                ctx.emit(InitEnvironmentBlockEvent::StartSetup(
                    self.repos.clone(),
                    self.use_current_dir,
                ));
                ctx.notify();
            }
            InitEnvironmentBlockAction::Skip => {
                self.setup_state = SetupState::Skipped;
                ctx.notify();
            }
        }
    }
}
