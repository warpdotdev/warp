use ai::skills::{SkillProvider, SkillReference, SkillScope};
use fuzzy_match::{match_indices_case_insensitive, FuzzyMatchResult};
use ordered_float::OrderedFloat;
use warp_core::ui::icons::Icon;
use warp_core::ui::theme::Fill;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, Highlight, ParentElement, Shrinkable, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::Keystroke;
use warpui::scene::{CornerRadius, Radius};
use warpui::text_layout::ClipConfig;
use warpui::{
    AppContext, Element, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity as _,
};

use crate::ai::skills::SkillManager;
use crate::appearance::Appearance;
use crate::search::data_source::{Query, QueryResult};
use crate::search::mixer::DataSourceRunErrorWrapper;
use crate::search::result_renderer::ItemHighlightState;
use crate::search::{SearchItem, SyncDataSource};
use crate::terminal::cli_agent_sessions::{CLIAgentInputState, CLIAgentSessionsModel};
use crate::terminal::input::inline_menu::{
    default_navigation_message_items, styles as inline_styles, InlineMenuAction,
    InlineMenuMessageArgs, InlineMenuType,
};
use crate::terminal::input::message_bar::{Message, MessageItem};
use crate::terminal::model::session::active_session::{ActiveSession, ActiveSessionEvent};

#[derive(Clone, Debug)]
pub struct AcceptSkill {
    pub skill_name: String,
    pub skill_reference: SkillReference,
}

impl InlineMenuAction for AcceptSkill {
    const MENU_TYPE: InlineMenuType = InlineMenuType::SkillMenu;

    fn produce_inline_menu_message<T>(args: InlineMenuMessageArgs<'_, Self, T>) -> Option<Message> {
        // If no item is selected, show "No skills found" message with escape hint
        if args.inline_menu_model.selected_item().is_none() {
            return Some(Message::new(vec![
                MessageItem::text("No skills found"),
                MessageItem::keystroke(Keystroke {
                    key: "escape".to_owned(),
                    ..Default::default()
                }),
                MessageItem::text(" to dismiss"),
            ]));
        }

        // Otherwise show default navigation hints
        Some(Message::new(default_navigation_message_items(&args)))
    }

    // No details panel - we show inline descriptions instead
}

/// Event emitted when available skills may have changed.
#[derive(Debug, Clone, Copy)]
pub struct UpdatedAvailableSkills;

pub struct SkillSelectorDataSource {
    active_session: ModelHandle<ActiveSession>,
    terminal_view_id: EntityId,
    /// Whether bundled skills should be included in results.
    /// False for `/open-skill` (bundled skills can't be edited), true for `/skills` (they can be invoked).
    include_bundled: bool,
}

impl SkillSelectorDataSource {
    pub fn new(
        active_session: ModelHandle<ActiveSession>,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&active_session, |_, event, ctx| match event {
            // Emit event so the mixer can re-run its query with the new pwd
            ActiveSessionEvent::UpdatedPwd | ActiveSessionEvent::Bootstrapped => {
                ctx.emit(UpdatedAvailableSkills);
            }
        });

        Self {
            active_session,
            terminal_view_id,
            include_bundled: false,
        }
    }

    /// Returns the supported skill providers for the active CLI agent, or `None` if
    /// CLI agent input is not open.
    fn active_cli_agent_providers(&self, app: &AppContext) -> Option<&'static [SkillProvider]> {
        CLIAgentSessionsModel::as_ref(app)
            .session(self.terminal_view_id)
            .filter(|s| matches!(s.input_state, CLIAgentInputState::Open { .. }))
            .map(|s| s.agent.supported_skill_providers())
    }

    pub fn set_include_bundled(&mut self, include_bundled: bool) {
        self.include_bundled = include_bundled;
    }

    /// Get the current working directory location from the active session.
    fn get_current_working_directory(&self, app: &AppContext) -> Option<LocalOrRemotePath> {
        self.active_session
            .as_ref(app)
            .current_working_directory_location(app)
    }
}

impl SyncDataSource for SkillSelectorDataSource {
    type Action = AcceptSkill;

    fn run_query(
        &self,
        query: &Query,
        app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        let cwd = self.get_current_working_directory(app);
        let cli_agent_providers = self.active_cli_agent_providers(app);
        let skills = SkillManager::as_ref(app).get_skills_for_working_directory(cwd.as_ref(), app);

        // Filter out bundled skills when in open mode, since they cannot be opened.
        // Bundled skills are identified by scope rather than reference: local
        // catalog entries are `BundledSkillId`-referenced, but remote catalog
        // entries are path-referenced, and both must be excluded here.
        // When CLI agent input is open, filter to skills that exist in a supported
        // provider folder. We check all paths for the skill name (not just the
        // deduplicated provider) because deduplication may pick a higher-priority
        // provider even when the skill also exists in the CLI agent's folder.
        let skill_manager = SkillManager::as_ref(app);
        let skills: Vec<_> = skills
            .into_iter()
            .filter(|skill| {
                if let Some(providers) = &cli_agent_providers {
                    skill_manager.skill_exists_for_any_provider(skill, providers)
                } else {
                    self.include_bundled || skill.scope != SkillScope::Bundled
                }
            })
            .map(|mut skill| {
                // When a CLI agent is active, re-map the provider to the best
                // supported one so the icon reflects the agent's native provider
                // rather than the global dedup winner.
                if let Some(providers) = &cli_agent_providers {
                    skill.provider = skill_manager.best_supported_provider(&skill, providers);
                }
                skill
            })
            .collect();

        let query_text = query.text.trim();
        if query_text.is_empty() {
            return Ok(skills
                .into_iter()
                .map(|skill| {
                    QueryResult::from(SkillSearchItem::new(
                        skill.name,
                        skill.reference,
                        skill.description,
                        skill.scope,
                        skill.provider,
                        skill.icon_override,
                    ))
                })
                .collect());
        }
        Ok(skills
            .into_iter()
            .filter_map(|skill| {
                let match_result = match_indices_case_insensitive(skill.name.as_str(), query_text)?;
                // Avoid spamming results with extremely weak matches.
                if query_text.len() > 1 && match_result.score < 10 {
                    return None;
                }
                Some(QueryResult::from(
                    SkillSearchItem::new(
                        skill.name,
                        skill.reference,
                        skill.description,
                        skill.scope,
                        skill.provider,
                        skill.icon_override,
                    )
                    .with_name_match_result(Some(match_result.clone()))
                    .with_score(OrderedFloat(match_result.score as f64)),
                ))
            })
            .collect())
    }
}

impl Entity for SkillSelectorDataSource {
    type Event = UpdatedAvailableSkills;
}

#[derive(Clone)]
struct SkillSearchItem {
    skill_name: String,
    skill_reference: SkillReference,
    skill_description: String,
    scope: SkillScope,
    provider: SkillProvider,
    icon_override: Option<Icon>,
    name_match_result: Option<FuzzyMatchResult>,
    score: OrderedFloat<f64>,
}

impl SkillSearchItem {
    fn new(
        skill_name: String,
        skill_reference: SkillReference,
        skill_description: String,
        scope: SkillScope,
        provider: SkillProvider,
        icon_override: Option<Icon>,
    ) -> Self {
        Self {
            skill_name,
            skill_reference,
            skill_description,
            scope,
            provider,
            icon_override,
            name_match_result: None,
            score: OrderedFloat(f64::MIN),
        }
    }

    fn with_name_match_result(mut self, result: Option<FuzzyMatchResult>) -> Self {
        self.name_match_result = result;
        self
    }

    fn with_score(mut self, score: OrderedFloat<f64>) -> Self {
        self.score = score;
        self
    }
}

/// Fixed width for the skill name column (similar to slash commands).
fn skill_name_column_width(app: &AppContext) -> f32 {
    let appearance = Appearance::as_ref(app);
    // Use a reasonable fixed width for skill names
    app.font_cache().em_width(
        appearance.monospace_font_family(),
        inline_styles::font_size(appearance),
    ) * 20.0 // Allow space for skill names like "/analyze-code"
        + 32.0
}

impl SearchItem for SkillSearchItem {
    type Action = AcceptSkill;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon_color = inline_styles::icon_color(appearance);
        let icon_size = inline_styles::font_size(appearance);

        // Use icon_override if set (e.g. Figma skills), otherwise derive from provider.
        let icon = if let Some(override_icon) = self.icon_override {
            override_icon.to_warpui_icon(icon_color).finish()
        } else {
            self.provider
                .icon()
                .to_warpui_icon(self.provider.icon_fill(icon_color))
                .finish()
        };

        Container::new(
            ConstrainedBox::new(icon)
                .with_width(icon_size)
                .with_height(icon_size)
                .finish(),
        )
        .with_margin_right(inline_styles::ICON_MARGIN)
        .finish()
    }

    fn render_item(
        &self,
        _highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let font_size = inline_styles::font_size(appearance);
        let background_color = inline_styles::menu_background_color(app);
        let primary_text_color = inline_styles::primary_text_color(theme, background_color.into());
        let secondary_color = inline_styles::secondary_text_color(theme, background_color.into());

        // Create row layout for inline text
        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        // Skill name with fuzzy match highlighting
        let mut name_text = Text::new_inline(
            self.skill_name.clone(),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(primary_text_color.into())
        .with_clip(ClipConfig::ellipsis());

        if let Some(name_match) = &self.name_match_result {
            if !name_match.matched_indices.is_empty() {
                name_text = name_text.with_single_highlight(
                    Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                    name_match.matched_indices.clone(),
                );
            }
        }

        row.add_child(
            ConstrainedBox::new(name_text.finish())
                .with_width(skill_name_column_width(app))
                .finish(),
        );

        // Description and optional "Project Skill" badge
        // The description should truncate first, badge stays fixed size
        // We wrap the whole description_row in Shrinkable to give it a bounded constraint
        let mut description_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        if !self.skill_description.is_empty() {
            let description_text = Text::new_inline(
                self.skill_description.clone(),
                appearance.ui_font_family(),
                font_size,
            )
            .with_color(secondary_color.into())
            .with_clip(ClipConfig::ellipsis());

            // Use Shrinkable so description truncates before the badge
            description_row.add_child(Shrinkable::new(1., description_text.finish()).finish());
        }

        // "Project Skill" badge for project skills (placed after description)
        if self.scope == SkillScope::Project {
            let badge_font_size = font_size - 4.0;
            // Badge text uses disabled_text_color (40% opacity) per Figma #6d7276
            let badge_text_color =
                inline_styles::disabled_text_color(theme, background_color.into());
            let badge_text = Text::new_inline(
                "Project Skill".to_string(),
                appearance.ui_font_family(),
                badge_font_size,
            )
            .with_color(badge_text_color.into())
            .with_clip(ClipConfig::ellipsis());

            let badge = Container::new(badge_text.finish())
                .with_horizontal_padding(6.0)
                .with_vertical_padding(2.0)
                .with_background(theme.surface_overlay_1())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.0)))
                .with_margin_left(8.0)
                .finish();

            description_row.add_child(badge);
        }

        // Wrap in Shrinkable to provide bounded constraint for inner flexible children
        row.add_child(Shrinkable::new(1., description_row.finish()).finish());

        row.finish()
    }

    fn item_background(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Option<Fill> {
        inline_styles::item_background(highlight_state, appearance)
    }

    // No details panel - we show inline descriptions instead
    fn render_details(&self, _app: &AppContext) -> Option<Box<dyn Element>> {
        None
    }

    fn score(&self) -> OrderedFloat<f64> {
        self.score
    }

    fn accept_result(&self) -> Self::Action {
        AcceptSkill {
            skill_name: self.skill_name.clone(),
            skill_reference: self.skill_reference.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Skill: {}", self.skill_name)
    }
}
