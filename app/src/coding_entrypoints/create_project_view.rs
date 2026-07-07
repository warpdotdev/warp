use warp_core::send_telemetry_from_ctx;
use warp_core::ui::icons::Icon;
use warpui::elements::{
    ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Expanded, Fill, Flex,
    Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement as _, Radius, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::platform::Cursor;
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity as _, TypedActionView, View,
    ViewContext, ViewHandle,
};

use crate::ai::blocklist::telemetry_banner::should_collect_ai_ugc_telemetry;
use crate::appearance::Appearance;
use crate::coding_entrypoints::glowing_editor::{GlowingEditor, GlowingEditorEvent};
use crate::settings::PrivacySettings;
use crate::TelemetryEvent;

const ICON_MARGIN_LEFT: f32 = 12.;
const ICON_MARGIN_RIGHT: f32 = 8.;
const SUGGESTION_ITEM_PADDING: f32 = 12.;

pub struct CreateProjectView {
    editor: ViewHandle<GlowingEditor>,
    suggestions: Vec<BuildSuggestion>,
    is_ftux: bool,
}

struct BuildSuggestion {
    prompt: &'static str,
    mouse_state: MouseStateHandle,
}

impl CreateProjectView {
    pub fn new(is_ftux: bool, ctx: &mut ViewContext<Self>) -> Self {
        let editor =
            ctx.add_typed_action_view(|ctx| GlowingEditor::new("What do you want to build?", ctx));

        ctx.subscribe_to_view(&editor, move |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        let suggestions = vec![
            BuildSuggestion {
                prompt: "Build a Minesweeper clone in React",
                mouse_state: Default::default(),
            },
            BuildSuggestion {
                prompt: "Code a Node.js server that returns random quotes from a JSON file",
                mouse_state: Default::default(),
            },
            BuildSuggestion {
                prompt: "Write a CSV to JSON converter CLI",
                mouse_state: Default::default(),
            },
            BuildSuggestion {
                prompt: "Create a starter template for a résumé web page",
                mouse_state: Default::default(),
            },
            BuildSuggestion {
                prompt: "Make a Conway's Game of Life simulation",
                mouse_state: Default::default(),
            },
        ];

        Self {
            editor,
            suggestions,
            is_ftux,
        }
    }

    fn handle_editor_event(&mut self, event: &GlowingEditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            GlowingEditorEvent::Submit(prompt) => {
                // Always send metadata event for custom prompts
                send_telemetry_from_ctx!(
                    TelemetryEvent::CreateProjectPromptSubmitted {
                        is_custom_prompt: true,
                        suggested_prompt: None,
                        is_ftux: self.is_ftux,
                    },
                    ctx
                );

                // Send content event only if UGC collection is enabled
                let should_collect_ugc = should_collect_ai_ugc_telemetry(
                    ctx,
                    PrivacySettings::as_ref(ctx).is_telemetry_enabled,
                );
                if should_collect_ugc {
                    send_telemetry_from_ctx!(
                        TelemetryEvent::CreateProjectPromptSubmittedContent {
                            custom_prompt: prompt.clone(),
                        },
                        ctx
                    );
                }

                ctx.emit(CreateProjectEvent::SubmitPrompt(prompt.clone()));
            }
            GlowingEditorEvent::Cancel => {
                self.editor.update(ctx, |editor, ctx| {
                    editor.clear_buffer(ctx);
                });
                ctx.emit(CreateProjectEvent::Cancel)
            }
        }
    }

    fn render_suggestion_item(
        &self,
        appearance: &Appearance,
        suggestion: &BuildSuggestion,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let icon = Icon::MessagePlusSquare;
        let icon_color = theme.terminal_colors().normal.cyan.into();
        let font_family = appearance.ui_font_family();
        let font_size = appearance.ui_font_size();
        let font_color = theme.sub_text_color(theme.background()).into_solid();

        let mouse_state = suggestion.mouse_state.clone();
        let prompt = suggestion.prompt;

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_main_axis_size(MainAxisSize::Max)
            .with_children([
                Container::new(
                    ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
                        .with_height(font_size)
                        .with_width(font_size)
                        .finish(),
                )
                .with_margin_left(ICON_MARGIN_LEFT + 2.)
                .with_margin_right(ICON_MARGIN_RIGHT)
                .finish(),
                Expanded::new(
                    1.,
                    Text::new(prompt, font_family, font_size)
                        .with_color(font_color)
                        .with_style(Properties::default().weight(Weight::Medium))
                        .soft_wrap(false)
                        .finish(),
                )
                .finish(),
            ]);

        Hoverable::new(mouse_state, move |state| {
            Container::new(row.finish())
                .with_vertical_padding(SUGGESTION_ITEM_PADDING)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .with_background(if state.is_hovered() {
                    theme.surface_overlay_1().into()
                } else {
                    Fill::None
                })
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(CreateProjectAction::SuggestionSelected {
                prompt: prompt.to_string(),
            });
        })
        .finish()
    }
}

pub enum CreateProjectEvent {
    SubmitPrompt(String),
    Cancel,
}

impl Entity for CreateProjectView {
    type Event = CreateProjectEvent;
}

#[derive(Clone, Debug)]
pub enum CreateProjectAction {
    SuggestionSelected { prompt: String },
}

impl TypedActionView for CreateProjectView {
    type Action = CreateProjectAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CreateProjectAction::SuggestionSelected { prompt } => {
                // Always send metadata event with suggested prompt content (non-UGC)
                send_telemetry_from_ctx!(
                    TelemetryEvent::CreateProjectPromptSubmitted {
                        is_custom_prompt: false,
                        suggested_prompt: Some(prompt.clone()),
                        is_ftux: self.is_ftux,
                    },
                    ctx
                );
                ctx.emit(CreateProjectEvent::SubmitPrompt(prompt.clone()));
            }
        }
    }
}

impl View for CreateProjectView {
    fn ui_name() -> &'static str {
        "CreateProjectView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.editor);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut column = Flex::column().with_child(ChildView::new(&self.editor).finish());

        if !self.suggestions.is_empty() {
            let suggestions = self
                .suggestions
                .iter()
                .map(|suggestion| self.render_suggestion_item(appearance, suggestion))
                .collect::<Vec<_>>();

            let suggestion_container =
                Container::new(Flex::column().with_children(suggestions).finish())
                    .with_margin_top(8.)
                    .finish();

            column.add_child(suggestion_container);
        }

        column.finish()
    }
}
