use std::path::PathBuf;
use warpui::AppContext;

use warpui::elements::{
    Align, ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisSize, MouseStateHandle,
    ParentElement, Shrinkable,
};
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::Element;

use crate::appearance::Appearance;
use crate::localization;
use crate::terminal::view::{InlineBannerId, TerminalAction};
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;

/// Uniform padding around the banner
const PADDING: f32 = 12.;

fn text(app: &AppContext, key: &str) -> String {
    localization::text_for_app(app, key)
}

#[derive(PartialEq, Clone)]
pub enum VisibilityState {
    Speedbump,
    Indexing,
}

#[derive(Clone, Copy, Debug)]
pub enum CodebaseIndexSpeedbumpBannerAction {
    ToggleAlwaysAllow,
    AllowIndexing,
    Close,
    ViewStatus,
    DismissForever,
}

/// Maintains the state for the input speedbump banner
/// This banner appears when a user attempts to run a command that might be unsafe
/// and requires explicit confirmation before execution.
pub struct CodebaseIndexSpeedbumpBannerState {
    pub id: InlineBannerId,

    // Mouse state for the checkbox to always allow indexing
    pub checkbox_mouse_state: MouseStateHandle,
    // Mouse state for the allow button that confirms and executes indexing.
    pub allow_button_mouse_state: MouseStateHandle,
    // Mouse state for the close button that dismisses the banner without executing indexing.
    pub close_button_mouse_state: MouseStateHandle,
    // Mouse state for the view status button that shows the code indexing settings page.
    pub view_status_button_mouse_state: MouseStateHandle,
    // Mouse state for the "Don't show again" button that globally dismisses the banner.
    pub dont_show_again_mouse_state: MouseStateHandle,
    // Whether the "Always allow" checkbox is checked.
    pub always_allow_checked: bool,

    // Whether the speedbump banner is visible.
    pub visibility_state: VisibilityState,

    // The path to the repo that the banner is for.
    pub repo_path: PathBuf,
}

impl CodebaseIndexSpeedbumpBannerState {
    #[cfg_attr(not(feature = "local_fs"), expect(unused))]
    pub fn new(id: InlineBannerId, repo_path: PathBuf) -> Self {
        Self {
            id,
            checkbox_mouse_state: Default::default(),
            allow_button_mouse_state: Default::default(),
            close_button_mouse_state: Default::default(),
            view_status_button_mouse_state: Default::default(),
            dont_show_again_mouse_state: Default::default(),
            always_allow_checked: true,
            visibility_state: VisibilityState::Speedbump,
            repo_path,
        }
    }

    pub fn toggle_always_allow_checked(&mut self) {
        self.always_allow_checked = !self.always_allow_checked;
    }

    pub fn show_indexing_banner(&mut self) {
        self.visibility_state = VisibilityState::Indexing;
    }

    fn render_text_column(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let ui_builder = appearance.ui_builder();
        let theme = appearance.theme();

        let mut col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Start);

        let title = ui_builder
            .span(if self.visibility_state == VisibilityState::Speedbump {
                text(app, "agent.codebase_index_speedbump.title")
            } else {
                text(app, "agent.codebase_index_speedbump.indexing_title")
            })
            .with_style(UiComponentStyles {
                font_color: Some(appearance.theme().foreground().into_solid()),
                font_size: Some(appearance.ui_font_size() * 1.2),
                ..Default::default()
            })
            .build()
            .finish();

        col.add_child(title);

        let body = ui_builder
            .span(text(app, "agent.codebase_index_speedbump.description"))
            .with_style(UiComponentStyles {
                font_color: Some(blended_colors::text_sub(theme, theme.surface_1())),
                font_size: Some(appearance.ui_font_size()),
                ..Default::default()
            })
            .build()
            .finish();

        col.add_child(body);

        Container::new(col.finish())
            .with_horizontal_padding(4.)
            .finish()
    }

    pub fn render_codebase_index_speedbump_banner(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder();
        let mut banner = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        let mut left = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        let info_icon = Container::new(
            ConstrainedBox::new(
                Icon::Info
                    .to_warpui_icon(theme.active_ui_text_color())
                    .finish(),
            )
            .with_width(appearance.ui_font_size() * 1.2)
            .with_height(appearance.ui_font_size() * 1.2)
            .finish(),
        )
        .with_padding_right(4.)
        .finish();

        left.add_child(info_icon);
        left.add_child(
            Shrinkable::new(
                1.,
                Align::new(self.render_text_column(appearance, app))
                    .left()
                    .finish(),
            )
            .finish(),
        );

        banner.add_child(Shrinkable::new(1., left.finish()).finish());

        if self.visibility_state == VisibilityState::Speedbump {
            let checkbox = ui_builder
                .checkbox(
                    self.checkbox_mouse_state.clone(),
                    Some(appearance.ui_font_size()),
                )
                .check(self.always_allow_checked)
                .with_style(UiComponentStyles {
                    padding: Some(Coords::default().right(2.)),
                    ..Default::default()
                })
                .build()
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(TerminalAction::CodebaseIndexSpeedbumpBanner(
                        CodebaseIndexSpeedbumpBannerAction::ToggleAlwaysAllow,
                    ));
                })
                .with_cursor(Cursor::PointingHand)
                .finish();

            let checkbox_text = ui_builder
                .span(text(
                    app,
                    "agent.codebase_index_speedbump.allow_automatic_indexing",
                ))
                .with_style(UiComponentStyles {
                    font_color: Some(blended_colors::text_disabled(theme, theme.surface_1())),
                    font_size: Some(appearance.ui_font_size()),
                    ..Default::default()
                })
                .build()
                .finish();

            banner.add_child(
                Container::new(
                    Align::new(
                        Flex::row()
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(checkbox)
                            .with_child(checkbox_text)
                            .finish(),
                    )
                    .finish(),
                )
                .with_horizontal_padding(4.)
                .finish(),
            );
        }

        match self.visibility_state {
            VisibilityState::Speedbump => {
                banner.add_child(
                    Container::new(
                        Align::new(
                            ui_builder
                                .button(
                                    ButtonVariant::Outlined,
                                    self.dont_show_again_mouse_state.clone(),
                                )
                                .with_text_label(text(
                                    app,
                                    "agent.codebase_index_speedbump.dont_show_again",
                                ))
                                .with_style(UiComponentStyles {
                                    font_color: Some(appearance.theme().foreground().into_solid()),
                                    font_size: Some(appearance.ui_font_size()),
                                    padding: Some(Coords {
                                        top: 5.0,
                                        bottom: 5.0,
                                        left: 8.0,
                                        right: 8.0,
                                    }),
                                    ..Default::default()
                                })
                                .build()
                                .on_click(|ctx, _, _| {
                                    ctx.dispatch_typed_action(
                                        TerminalAction::CodebaseIndexSpeedbumpBanner(
                                            CodebaseIndexSpeedbumpBannerAction::DismissForever,
                                        ),
                                    );
                                })
                                .finish(),
                        )
                        .finish(),
                    )
                    .with_horizontal_padding(4.)
                    .finish(),
                );
                banner.add_child(
                    Container::new(
                        Align::new(
                            ui_builder
                                .button(
                                    ButtonVariant::Outlined,
                                    self.allow_button_mouse_state.clone(),
                                )
                                .with_text_label(text(
                                    app,
                                    "agent.codebase_index_speedbump.index_codebase",
                                ))
                                .with_style(UiComponentStyles {
                                    font_color: Some(appearance.theme().foreground().into_solid()),
                                    font_size: Some(appearance.ui_font_size()),
                                    padding: Some(Coords {
                                        top: 5.0,
                                        bottom: 5.0,
                                        left: 8.0,
                                        right: 8.0,
                                    }),
                                    ..Default::default()
                                })
                                .build()
                                .on_click(|ctx, _, _| {
                                    ctx.dispatch_typed_action(
                                        TerminalAction::CodebaseIndexSpeedbumpBanner(
                                            CodebaseIndexSpeedbumpBannerAction::AllowIndexing,
                                        ),
                                    );
                                })
                                .finish(),
                        )
                        .finish(),
                    )
                    .with_horizontal_padding(4.)
                    .finish(),
                );
            }
            VisibilityState::Indexing => {
                banner.add_child(
                    Container::new(
                        Align::new(
                            ui_builder
                                .button(
                                    ButtonVariant::Outlined,
                                    self.view_status_button_mouse_state.clone(),
                                )
                                .with_text_label(text(
                                    app,
                                    "agent.codebase_index_speedbump.view_status",
                                ))
                                .with_style(UiComponentStyles {
                                    font_color: Some(appearance.theme().foreground().into_solid()),
                                    font_size: Some(appearance.ui_font_size()),
                                    padding: Some(Coords {
                                        top: 5.0,
                                        bottom: 5.0,
                                        left: 8.0,
                                        right: 8.0,
                                    }),
                                    ..Default::default()
                                })
                                .build()
                                .on_click(|ctx, _, _| {
                                    ctx.dispatch_typed_action(
                                        TerminalAction::CodebaseIndexSpeedbumpBanner(
                                            CodebaseIndexSpeedbumpBannerAction::ViewStatus,
                                        ),
                                    );
                                })
                                .finish(),
                        )
                        .finish(),
                    )
                    .with_horizontal_padding(4.)
                    .finish(),
                );
            }
        }

        // Add the close button
        banner.add_child(
            Container::new(
                ui_builder
                    .close_button(
                        appearance.ui_font_size(),
                        self.close_button_mouse_state.clone(),
                    )
                    .build()
                    .on_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(TerminalAction::CodebaseIndexSpeedbumpBanner(
                            CodebaseIndexSpeedbumpBannerAction::Close,
                        ));
                    })
                    .finish(),
            )
            .with_padding_left(4.)
            .finish(),
        );

        // Create the final container with background and padding
        Container::new(banner.finish())
            .with_background(theme.surface_1())
            .with_uniform_padding(PADDING)
            .finish()
    }
}
