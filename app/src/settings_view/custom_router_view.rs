#[cfg(feature = "local_fs")]
use std::path::PathBuf;

use warpui::elements::{
    ChildView, ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisAlignment,
    MainAxisSize, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, Entity, SingletonEntity, View, ViewContext, ViewHandle};

use crate::ai::custom_model_routers::{CustomModelRouter, CustomModelRouting};
use crate::ai::llms::{LLMId, LLMPreferences};
use crate::appearance::Appearance;
use crate::settings::AISettings;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::ActionButton;
#[cfg(feature = "local_fs")]
use crate::view_components::action_button::{ButtonSize, DangerSecondaryTheme, SecondaryTheme};
#[cfg(feature = "local_fs")]
const HEADER_BUTTON_HEIGHT: f32 = 28.;

#[cfg(feature = "local_fs")]
#[derive(Debug, Clone)]
pub enum CustomRouterViewAction {
    OpenFile,
    Edit,
    Delete,
}

pub enum CustomRouterViewEvent {
    #[cfg(feature = "local_fs")]
    OpenFile(PathBuf),
    #[cfg(feature = "local_fs")]
    Edit,
    #[cfg(feature = "local_fs")]
    Delete,
}

pub struct CustomRouterView {
    router: CustomModelRouter,
    open_file_button: ViewHandle<ActionButton>,
    edit_button: ViewHandle<ActionButton>,
    delete_button: ViewHandle<ActionButton>,
}

impl CustomRouterView {
    #[cfg(feature = "local_fs")]
    pub fn new(router: CustomModelRouter, ctx: &mut ViewContext<Self>) -> Self {
        let is_any_ai_enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
        let open_file_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Open file", SecondaryTheme)
                .with_icon(Icon::File)
                .with_size(ButtonSize::Small)
                .with_height(HEADER_BUTTON_HEIGHT)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CustomRouterViewAction::OpenFile);
                })
        });
        open_file_button.update(ctx, |button, ctx| {
            button.set_disabled(router.source_path.is_none(), ctx);
        });

        let edit_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Edit", SecondaryTheme)
                .with_icon(Icon::Pencil)
                .with_size(ButtonSize::Small)
                .with_height(HEADER_BUTTON_HEIGHT)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CustomRouterViewAction::Edit);
                })
        });
        edit_button.update(ctx, |button, ctx| {
            button.set_disabled(!is_any_ai_enabled, ctx);
        });

        let delete_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Delete", DangerSecondaryTheme)
                .with_icon(Icon::Trash)
                .with_size(ButtonSize::Small)
                .with_height(HEADER_BUTTON_HEIGHT)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(CustomRouterViewAction::Delete);
                })
        });
        delete_button.update(ctx, |button, ctx| {
            button.set_disabled(!is_any_ai_enabled, ctx);
        });

        ctx.subscribe_to_model(&AISettings::handle(ctx), |me, _, _, ctx| {
            let enabled = AISettings::as_ref(ctx).is_any_ai_enabled(ctx);
            me.edit_button.update(ctx, |button, ctx| {
                button.set_disabled(!enabled, ctx);
            });
            me.delete_button.update(ctx, |button, ctx| {
                button.set_disabled(!enabled, ctx);
            });
            ctx.notify();
        });

        Self {
            router,
            open_file_button,
            edit_button,
            delete_button,
        }
    }

    #[allow(dead_code)]
    pub fn router(&self) -> &CustomModelRouter {
        &self.router
    }

    #[allow(dead_code)]
    pub fn update_router(&mut self, router: CustomModelRouter, ctx: &mut ViewContext<Self>) {
        self.router = router;
        ctx.notify();
    }
}

impl Entity for CustomRouterView {
    type Event = CustomRouterViewEvent;
}

impl View for CustomRouterView {
    fn ui_name() -> &'static str {
        "CustomRouterView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let is_any_ai_enabled = AISettings::as_ref(app).is_any_ai_enabled(app);

        let text_color = if is_any_ai_enabled {
            appearance.theme().active_ui_text_color()
        } else {
            appearance.theme().disabled_ui_text_color()
        };
        let sub_color = if is_any_ai_enabled {
            appearance
                .theme()
                .sub_text_color(appearance.theme().surface_2())
        } else {
            appearance.theme().disabled_ui_text_color()
        };

        // Header row: name + buttons
        let name_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Text::new(
                    self.router.info.display_name.clone(),
                    appearance.ui_font_family(),
                    14.,
                )
                .with_style(Properties::default().weight(Weight::Medium))
                .with_color(text_color.into())
                .finish(),
            )
            .with_child(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        Container::new(ChildView::new(&self.open_file_button).finish())
                            .with_margin_right(8.)
                            .finish(),
                    )
                    .with_child(
                        Container::new(ChildView::new(&self.edit_button).finish())
                            .with_margin_right(8.)
                            .finish(),
                    )
                    .with_child(ChildView::new(&self.delete_button).finish())
                    .finish(),
            )
            .finish();

        // Type label row
        let type_label = match &self.router.routing {
            CustomModelRouting::Complexity(_) => "Complexity-based routing",
            CustomModelRouting::Prompt(_) => "Prompt-based routing",
        };
        let type_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Container::new(
                    ConstrainedBox::new(Icon::Dataflow.to_warpui_icon(sub_color).finish())
                        .with_width(12.)
                        .with_height(12.)
                        .finish(),
                )
                .with_margin_right(6.)
                .finish(),
            )
            .with_child(
                Text::new(type_label, appearance.ui_font_family(), 12.)
                    .with_color(sub_color.into())
                    .finish(),
            )
            .finish();

        // Targets summary
        let targets_row = render_targets_row(
            &self.router.routing,
            appearance,
            sub_color,
            is_any_ai_enabled,
            app,
        );

        Container::new(
            Flex::column()
                .with_child(Container::new(name_row).with_margin_bottom(8.).finish())
                .with_child(Container::new(type_row).with_margin_bottom(4.).finish())
                .with_child(targets_row)
                .finish(),
        )
        .with_background(appearance.theme().surface_2())
        .with_border(
            warpui::elements::Border::new(1.).with_border_fill(appearance.theme().outline()),
        )
        .with_corner_radius(warpui::elements::CornerRadius::with_all(
            warpui::elements::Radius::Pixels(4.),
        ))
        .with_horizontal_padding(16.)
        .with_vertical_padding(12.)
        .finish()
    }
}

#[cfg(feature = "local_fs")]
impl warpui::TypedActionView for CustomRouterView {
    type Action = CustomRouterViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            CustomRouterViewAction::OpenFile => {
                if let Some(path) = self.router.source_path.clone() {
                    ctx.emit(CustomRouterViewEvent::OpenFile(path));
                }
            }
            CustomRouterViewAction::Edit => ctx.emit(CustomRouterViewEvent::Edit),
            CustomRouterViewAction::Delete => ctx.emit(CustomRouterViewEvent::Delete),
        }
    }
}

fn render_targets_row(
    routing: &CustomModelRouting,
    appearance: &Appearance,
    sub_color: warp_core::ui::theme::Fill,
    _is_ai_enabled: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let mut flex = Flex::column();
    match routing {
        CustomModelRouting::Complexity(c) => {
            flex.add_child(render_model_line(
                "Default:",
                model_display_name(&c.default, app),
                appearance,
                sub_color,
            ));
            if let Some(easy) = &c.easy {
                flex.add_child(
                    Container::new(render_model_line(
                        "Easy:",
                        model_display_name(easy, app),
                        appearance,
                        sub_color,
                    ))
                    .with_margin_top(2.)
                    .finish(),
                );
            }
            if let Some(medium) = &c.medium {
                flex.add_child(
                    Container::new(render_model_line(
                        "Medium:",
                        model_display_name(medium, app),
                        appearance,
                        sub_color,
                    ))
                    .with_margin_top(2.)
                    .finish(),
                );
            }
            if let Some(hard) = &c.hard {
                flex.add_child(
                    Container::new(render_model_line(
                        "Hard:",
                        model_display_name(hard, app),
                        appearance,
                        sub_color,
                    ))
                    .with_margin_top(2.)
                    .finish(),
                );
            }
        }
        CustomModelRouting::Prompt(p) => {
            flex.add_child(render_model_line(
                "Default:",
                model_display_name(&p.default_model, app),
                appearance,
                sub_color,
            ));
            let rule_count = p.rules.len();
            if rule_count > 0 {
                let label = if rule_count == 1 {
                    "1 rule".to_string()
                } else {
                    format!("{rule_count} rules")
                };
                flex.add_child(
                    Container::new(
                        Text::new(label, appearance.ui_font_family(), 12.)
                            .with_color(sub_color.into())
                            .finish(),
                    )
                    .with_margin_top(2.)
                    .finish(),
                );
            }
        }
    }
    flex.finish()
}

/// Resolves a concrete model id (e.g. `claude-4-5-haiku`) to its display
/// name/alias (e.g. `claude 4.5 haiku`), falling back to the raw id when the
/// model isn't known to the client.
fn model_display_name(model_id: &str, app: &AppContext) -> String {
    LLMPreferences::as_ref(app)
        .get_llm_info(&LLMId::from(model_id))
        .map(|info| info.display_name.clone())
        .unwrap_or_else(|| model_id.to_string())
}

fn render_model_line(
    label: impl Into<String>,
    model_id: impl Into<String>,
    appearance: &Appearance,
    sub_color: warp_core::ui::theme::Fill,
) -> Box<dyn Element> {
    Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Container::new(
                Text::new(label.into(), appearance.ui_font_family(), 12.)
                    .with_color(sub_color.into())
                    .finish(),
            )
            .with_margin_right(6.)
            .finish(),
        )
        .with_child(
            Text::new(model_id.into(), appearance.ui_font_family(), 12.)
                .with_color(appearance.theme().active_ui_text_color().into())
                .finish(),
        )
        .finish()
}

/// A card rendering a file that failed to parse as a custom model router.
#[cfg(feature = "local_fs")]
pub fn render_router_error_card(
    file_name: impl Into<String>,
    error_message: impl Into<String>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    use warpui::elements::Shrinkable;
    let theme = appearance.theme();
    let error_fill = warp_core::ui::theme::Fill::Solid(theme.ui_error_color());
    let sub = theme.sub_text_color(theme.surface_2());
    let file_name = file_name.into();
    let error_message = error_message.into();

    let name_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Container::new(
                ConstrainedBox::new(Icon::AlertTriangle.to_warpui_icon(error_fill).finish())
                    .with_width(14.)
                    .with_height(14.)
                    .finish(),
            )
            .with_margin_right(8.)
            .finish(),
        )
        .with_child(
            Text::new(file_name, appearance.ui_font_family(), 13.)
                .with_style(Properties::default().weight(Weight::Medium))
                .with_color(theme.active_ui_text_color().into())
                .finish(),
        )
        .finish();

    // Truncate long error messages to keep the card readable.
    let truncated = if error_message.chars().count() > 200 {
        format!("{}…", error_message.chars().take(200).collect::<String>())
    } else {
        error_message.to_string()
    };

    let error_row = Shrinkable::new(
        1.,
        Text::new(truncated, appearance.ui_font_family(), 11.)
            .with_color(sub.into())
            .finish(),
    )
    .finish();

    Container::new(
        Flex::column()
            .with_child(Container::new(name_row).with_margin_bottom(6.).finish())
            .with_child(error_row)
            .finish(),
    )
    .with_background(theme.surface_2())
    .with_border(warpui::elements::Border::new(1.).with_border_fill(error_fill))
    .with_corner_radius(warpui::elements::CornerRadius::with_all(
        warpui::elements::Radius::Pixels(4.),
    ))
    .with_horizontal_padding(16.)
    .with_vertical_padding(10.)
    .finish()
}
