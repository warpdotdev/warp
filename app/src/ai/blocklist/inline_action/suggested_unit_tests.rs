use std::sync::Arc;

use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng as _};
use warp_core::settings::ToggleableSetting;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Align, ConstrainedBox, Container, CrossAxisAlignment, Expanded, Flex, FormattedTextElement,
    HighlightedHyperlink, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement,
    SavePosition, Shrinkable, SizeConstraintCondition, SizeConstraintSwitch, Text,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::ai::agent::{AIAgentActionId, AIIdentifiers};
use crate::ai::predict::prompt_suggestions::{
    ACCEPT_PROMPT_SUGGESTION_KEYBINDING, REJECT_PROMPT_SUGGESTION_KEYSTROKE,
};
use crate::server::telemetry::ToggleCodeSuggestionsSettingSource;
use crate::settings::AISettings;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ButtonSize, KeystrokeSource, NakedTheme, PrimaryTheme,
};
use crate::view_components::compactible_action_button::{
    render_compact_and_regular_button_rows, CompactibleActionButton, MEDIUM_SIZE_SWITCH_THRESHOLD,
};
use crate::{send_telemetry_from_ctx, TelemetryEvent};

const ACCEPT_LABEL: &str = "Generate tests";
const CANCEL_LABEL: &str = "Dismiss";

#[derive(Debug, Clone)]
pub enum SuggestedUnitTestsEvent {
    Accept,
    Cancel,
    Blur,
    OpenSettings,
}

#[derive(Debug, Clone)]
pub enum SuggestedUnitTestsAction {
    Accept,
    Cancel,
    ToggleSetting,
    OpenSettings,
}

pub struct SuggestedUnitTestsView {
    /// Client and server identifiers for the AI output associated with the suggested prompt.
    identifiers: AIIdentifiers,
    action_id: AIAgentActionId,

    is_hidden: bool,
    is_keybindings_hidden: bool,
    should_show_speedbump: bool,
    title: String,
    description: String,
    query: String,
    accept_button: CompactibleActionButton,
    cancel_button: CompactibleActionButton,
    speedbump_mouse_state: MouseStateHandle,
    ai_settings_link_highlight_index: HighlightedHyperlink,

    /// A randomly-generated string prefix to ensure the [`SavePosition`]s in this view are unique.
    position_id_prefix: String,
}

impl SuggestedUnitTestsView {
    pub fn new(
        identifiers: AIIdentifiers,
        action_id: AIAgentActionId,
        query: String,
        title: String,
        description: String,
        should_show_speedbump: bool,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let accept_button = CompactibleActionButton::new(
            ACCEPT_LABEL.to_string(),
            Some(KeystrokeSource::Binding(
                ACCEPT_PROMPT_SUGGESTION_KEYBINDING,
            )),
            ButtonSize::Small,
            SuggestedUnitTestsAction::Accept,
            Icon::Check,
            Arc::new(PrimaryTheme),
            ctx,
        );

        let cancel_button = CompactibleActionButton::new(
            CANCEL_LABEL.to_string(),
            Some(KeystrokeSource::Fixed(
                REJECT_PROMPT_SUGGESTION_KEYSTROKE.clone(),
            )),
            ButtonSize::Small,
            SuggestedUnitTestsAction::Cancel,
            Icon::X,
            Arc::new(NakedTheme),
            ctx,
        );

        let random_str = thread_rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect();

        Self {
            identifiers,
            action_id,
            is_hidden: false,
            is_keybindings_hidden: false,
            should_show_speedbump,
            title,
            description,
            query,
            accept_button,
            cancel_button,
            speedbump_mouse_state: Default::default(),
            ai_settings_link_highlight_index: Default::default(),
            position_id_prefix: random_str,
        }
    }

    pub fn identifiers(&self) -> &AIIdentifiers {
        &self.identifiers
    }

    pub fn action_id(&self) -> &AIAgentActionId {
        &self.action_id
    }

    fn position_id_for_speedbump(&self) -> String {
        format!(
            "SuggestedUnitTestsView-speedbump-{}",
            &self.position_id_prefix
        )
    }

    pub fn is_hidden(&self) -> bool {
        self.is_hidden
    }

    pub fn is_keybindings_hidden(&self) -> bool {
        self.is_keybindings_hidden
    }

    pub fn query(&self) -> Option<String> {
        (!self.query.is_empty()).then(|| self.query.to_string())
    }

    pub fn set_is_hidden(&mut self, is_hidden: bool) {
        self.is_hidden = is_hidden;
    }

    pub fn hide_keybindings(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_keybindings_hidden = true;
        self.accept_button.set_keybinding(None, ctx);
        self.cancel_button.set_keybinding(None, ctx);
        ctx.notify();
    }

    fn render_icon(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                warpui::elements::Icon::new(
                    Icon::Code2.into(),
                    appearance
                        .theme()
                        .main_text_color(appearance.theme().background())
                        .into_solid(),
                )
                .finish(),
            )
            .with_width(appearance.monospace_font_size())
            .with_height(appearance.monospace_font_size())
            .finish(),
        )
        .with_margin_right(8.)
        .finish()
    }

    fn render_buttons(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> (Box<dyn Element>, Box<dyn Element>) {
        {
            render_compact_and_regular_button_rows(
                vec![&self.cancel_button, &self.accept_button],
                None,
                appearance,
                app,
            )
        }
    }

    fn render_header_contents(
        &self,
        buttons: Box<dyn Element>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let background = appearance.theme().background();
        let mut col = Flex::column();

        if !self.title.is_empty() {
            let title = Text::new_inline(
                self.title.to_string(),
                appearance.ui_font_family(),
                appearance.monospace_font_size() + 4.,
            )
            .with_color(appearance.theme().main_text_color(background).into_solid())
            .with_selectable(false)
            .finish();
            col.add_child(title);
        }

        if !self.description.is_empty() {
            let description = Text::new_inline(
                self.description.to_string(),
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(appearance.theme().sub_text_color(background).into_solid())
            .with_selectable(false)
            .finish();
            col.add_child(Container::new(description).with_margin_top(2.).finish());
        }

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(Shrinkable::new(1., Expanded::new(1., col.finish()).finish()).finish())
                .with_child(Align::new(buttons).right().finish())
                .finish(),
        )
        .finish()
    }

    fn render_header(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let (regular_buttons, compact_buttons) = self.render_buttons(appearance, app);
        let regular_header = self.render_header_contents(regular_buttons, appearance);
        let compact_header = self.render_header_contents(compact_buttons, appearance);

        let size_switch_threshold = MEDIUM_SIZE_SWITCH_THRESHOLD * appearance.monospace_ui_scalar();
        SizeConstraintSwitch::new(
            regular_header,
            vec![(
                SizeConstraintCondition::WidthLessThan(size_switch_threshold),
                compact_header,
            )],
        )
        .finish()
    }

    fn render_body(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let header = self.render_header(appearance, app);
        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header);
        if !self.query.is_empty() {
            let query: Box<dyn Element> = Text::new(
                self.query.to_string(),
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(
                appearance
                    .theme()
                    .main_text_color(appearance.theme().background())
                    .into_solid(),
            )
            .with_selectable(true)
            .finish();

            col.add_child(Container::new(query).with_margin_top(6.).finish());
        }

        col.finish()
    }

    fn render_speedbump(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();
        let font_color = theme.sub_text_color(theme.background()).into_solid();
        let font_family = appearance.ui_font_family();
        let font_size = 12.;

        let checked = AISettings::as_ref(app).is_code_suggestions_enabled(app);
        let checkbox = appearance
            .ui_builder()
            .checkbox(self.speedbump_mouse_state.clone(), Some(font_size))
            .check(!checked)
            .with_style(UiComponentStyles {
                font_color: Some(font_color),
                font_size: Some(font_size),
                ..Default::default()
            })
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(SuggestedUnitTestsAction::ToggleSetting);
            })
            .with_cursor(Cursor::PointingHand)
            .finish();

        let checkbox_text = appearance
            .ui_builder()
            .span("Don't show me suggested code banners again")
            .with_style(UiComponentStyles {
                font_color: Some(font_color),
                font_size: Some(font_size),
                padding: Some(Coords::default().left(4.)),
                ..Default::default()
            })
            .build()
            .finish();

        let formatted_text = FormattedTextElement::new(
            FormattedText::new([FormattedTextLine::Line(vec![
                FormattedTextFragment::hyperlink(
                    "Manage suggested code banner settings",
                    "Settings > AI",
                ),
            ])]),
            font_size,
            font_family,
            font_family,
            font_color,
            self.ai_settings_link_highlight_index.clone(),
        )
        .with_hyperlink_font_color(blended_colors::accent_fg_strong(theme).into())
        .register_default_click_handlers(|_, ctx, _| {
            ctx.dispatch_typed_action(SuggestedUnitTestsAction::OpenSettings);
        })
        .finish();

        let container = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_children([checkbox, checkbox_text])
                        .finish(),
                )
                .with_child(formatted_text)
                .finish(),
        )
        .with_padding_top(4.)
        .finish();

        SavePosition::new(container, &self.position_id_for_speedbump()).finish()
    }
}

impl View for SuggestedUnitTestsView {
    fn ui_name() -> &'static str {
        "SuggestedUnitTestsView"
    }

    fn on_focus(&mut self, _focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        // We always want the focus to be in the input, not the view.
        ctx.emit(SuggestedUnitTestsEvent::Blur);
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(self.render_icon(appearance))
            .with_child(Expanded::new(1., self.render_body(appearance, app)).finish())
            .finish();

        if self.should_show_speedbump {
            let speedbump = self.render_speedbump(appearance, app);
            Flex::column()
                .with_child(row)
                .with_child(speedbump)
                .finish()
        } else {
            row
        }
    }
}

impl Entity for SuggestedUnitTestsView {
    type Event = SuggestedUnitTestsEvent;
}

impl TypedActionView for SuggestedUnitTestsView {
    type Action = SuggestedUnitTestsAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SuggestedUnitTestsAction::Accept => ctx.emit(SuggestedUnitTestsEvent::Accept),
            SuggestedUnitTestsAction::Cancel => ctx.emit(SuggestedUnitTestsEvent::Cancel),
            SuggestedUnitTestsAction::ToggleSetting => {
                let checked = AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .code_suggestions_enabled_internal
                        .toggle_and_save_value(ctx)
                });
                ctx.notify();

                if let Ok(checked) = checked {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::ToggleCodeSuggestionsSetting {
                            source: ToggleCodeSuggestionsSettingSource::Speedbump,
                            is_code_suggestions_enabled: checked,
                        },
                        ctx
                    );
                }
            }
            SuggestedUnitTestsAction::OpenSettings => {
                ctx.emit(SuggestedUnitTestsEvent::OpenSettings)
            }
        }
    }
}
