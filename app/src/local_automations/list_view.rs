//! Local Automations list body.
//!
//! Hosted by the Settings → Automations page. Lists automations loaded from
//! the user's `automations/` directory with per-row "Run now" and "Open config"
//! actions, shows error rows for files that failed to parse, and offers a "New"
//! dropdown that either prompts the Warp agent or copies a creation prompt for
//! use with another agent (Claude Code, Codex, ...).

use pathfinder_geometry::vector::vec2f;
use warp_core::paths::home_relative_path;
use warpui::elements::{
    Align, ChildAnchor, ChildView, Container, CrossAxisAlignment, Element, Expanded, Flex,
    MainAxisSize, OffsetPositioning, ParentElement, PositionedElementAnchor,
    PositionedElementOffsetBounds, SavePosition, Stack, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use crate::appearance::Appearance;
use crate::local_automations::{LocalAutomation, LocalAutomationError};
use crate::menu::{Event as MenuEvent, Menu, MenuItemFields};
use crate::user_config::{WarpConfig, WarpConfigUpdateEvent};
use crate::view_components::action_button::{
    ActionButton, NakedTheme, PrimaryTheme, SecondaryTheme,
};
use crate::workspace::WorkspaceAction;

const ROW_SPACING: f32 = 12.;
const DESCRIPTION_FONT_SIZE: f32 = 13.;
const NEW_MENU_WIDTH: f32 = 200.;

/// Save-position anchor for the "New" button so its dropdown can attach below it.
const NEW_BUTTON_POSITION_ID: &str = "local_automations:new_button";

#[derive(Debug, Clone, PartialEq)]
pub enum LocalAutomationsViewAction {
    Run(usize),
    OpenConfig(usize),
    OpenErrorFile(usize),
    ToggleNewMenu,
}

struct AutomationRow {
    automation: LocalAutomation,
    run_button: ViewHandle<ActionButton>,
    open_button: ViewHandle<ActionButton>,
}

struct ErrorRow {
    error: LocalAutomationError,
    open_button: ViewHandle<ActionButton>,
}

/// Local Automations list body used by the Settings page.
pub struct LocalAutomationsView {
    rows: Vec<AutomationRow>,
    error_rows: Vec<ErrorRow>,
    new_button: ViewHandle<ActionButton>,
    new_menu: ViewHandle<Menu<WorkspaceAction>>,
    show_new_menu: bool,
}

impl LocalAutomationsView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        // Keep the list in sync while the view is open (files can change on
        // disk via the skill or manual edits).
        ctx.subscribe_to_model(&WarpConfig::handle(ctx), |me, _, event, ctx| {
            if matches!(event, WarpConfigUpdateEvent::LocalAutomations) {
                me.refresh(ctx);
            }
        });

        let new_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("New", PrimaryTheme)
                .with_menu(true)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(LocalAutomationsViewAction::ToggleNewMenu);
                })
        });

        let new_menu = ctx.add_typed_action_view(|_| {
            let mut menu = Menu::new().with_drop_shadow();
            menu.set_width(NEW_MENU_WIDTH);
            menu
        });
        // The menu dispatches the selected `WorkspaceAction` itself; we only
        // need to hide the dropdown when it closes.
        ctx.subscribe_to_view(&new_menu, |me, _, event, ctx| {
            if let MenuEvent::Close { .. } = event {
                me.show_new_menu = false;
                ctx.focus_self();
                ctx.notify();
            }
        });

        let mut view = Self {
            rows: Vec::new(),
            error_rows: Vec::new(),
            new_button,
            new_menu,
            show_new_menu: false,
        };
        view.refresh(ctx);
        view
    }

    /// Rebuilds rows from `WarpConfig` and focuses the view. Called when the
    /// settings page is selected.
    pub fn on_open(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_new_menu = false;
        self.refresh(ctx);
        ctx.focus_self();
    }

    fn refresh(&mut self, ctx: &mut ViewContext<Self>) {
        let (automations, errors) = {
            let config = WarpConfig::as_ref(ctx);
            (
                config.local_automations().clone(),
                config.local_automation_errors().clone(),
            )
        };

        self.rows = automations
            .into_iter()
            .enumerate()
            .map(|(index, automation)| {
                let run_button = ctx.add_typed_action_view(|_| {
                    ActionButton::new("Run now", SecondaryTheme).on_click(move |ctx| {
                        ctx.dispatch_typed_action(LocalAutomationsViewAction::Run(index));
                    })
                });
                let open_button = ctx.add_typed_action_view(|_| {
                    ActionButton::new("Open config", NakedTheme).on_click(move |ctx| {
                        ctx.dispatch_typed_action(LocalAutomationsViewAction::OpenConfig(index));
                    })
                });
                AutomationRow {
                    automation,
                    run_button,
                    open_button,
                }
            })
            .collect();

        self.error_rows = errors
            .into_iter()
            .enumerate()
            .map(|(index, error)| {
                let open_button = ctx.add_typed_action_view(|_| {
                    ActionButton::new("Open config", NakedTheme).on_click(move |ctx| {
                        ctx.dispatch_typed_action(LocalAutomationsViewAction::OpenErrorFile(index));
                    })
                });
                ErrorRow { error, open_button }
            })
            .collect();

        ctx.notify();
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let description = Text::new(
            "Jobs that run on this machine. Schedules are saved but don't fire yet; use Run now."
                .to_string(),
            appearance.ui_font_family(),
            DESCRIPTION_FONT_SIZE,
        )
        .with_color(theme.nonactive_ui_text_color().into())
        .soft_wrap(true)
        .finish();

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Expanded::new(1., Align::new(description).left().finish()).finish())
            .with_child(
                SavePosition::new(
                    ChildView::new(&self.new_button).finish(),
                    NEW_BUTTON_POSITION_ID,
                )
                .finish(),
            )
            .finish()
    }

    fn render_automation_row(
        &self,
        row: &AutomationRow,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let automation = &row.automation;

        let mut subtitle_parts = vec![
            automation.runner.display_label().to_string(),
            automation.schedule.clone(),
        ];
        if let Some(source_path) = &automation.source_path {
            subtitle_parts.push(home_relative_path(source_path));
        }
        if !automation.enabled {
            subtitle_parts.push("disabled".to_string());
        }
        let subtitle = subtitle_parts.join(" · ");

        let text_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Text::new_inline(
                    automation.name.clone(),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.active_ui_text_color().into())
                .with_style(Properties::default().weight(Weight::Bold))
                .finish(),
            )
            .with_child(
                Container::new(
                    Text::new_inline(subtitle, appearance.ui_font_family(), DESCRIPTION_FONT_SIZE)
                        .with_color(theme.nonactive_ui_text_color().into())
                        .finish(),
                )
                .with_margin_top(2.)
                .finish(),
            )
            .finish();

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Expanded::new(1., Align::new(text_column).left().finish()).finish())
                .with_child(
                    Container::new(ChildView::new(&row.open_button).finish())
                        .with_margin_left(8.)
                        .finish(),
                )
                .with_child(
                    Container::new(ChildView::new(&row.run_button).finish())
                        .with_margin_left(8.)
                        .finish(),
                )
                .finish(),
        )
        .with_margin_bottom(ROW_SPACING)
        .finish()
    }

    fn render_error_row(&self, row: &ErrorRow, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let text_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(
                Text::new_inline(
                    format!("{} failed to load", row.error.file_name),
                    appearance.ui_font_family(),
                    appearance.ui_font_size(),
                )
                .with_color(theme.ansi_fg_red())
                .finish(),
            )
            .with_child(
                Container::new(
                    Text::new(
                        row.error.error_message.clone(),
                        appearance.ui_font_family(),
                        DESCRIPTION_FONT_SIZE,
                    )
                    .with_color(theme.nonactive_ui_text_color().into())
                    .soft_wrap(true)
                    .finish(),
                )
                .with_margin_top(2.)
                .finish(),
            )
            .finish();

        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Expanded::new(1., Align::new(text_column).left().finish()).finish())
                .with_child(
                    Container::new(ChildView::new(&row.open_button).finish())
                        .with_margin_left(8.)
                        .finish(),
                )
                .finish(),
        )
        .with_margin_bottom(ROW_SPACING)
        .finish()
    }

    fn render_empty_state(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        Text::new(
            format!(
                "Nothing here yet. An automation is a job that runs on this machine; an agent \
                 prompt or a shell command, in a directory you choose. Hit New and an agent will \
                 set one up with you, or drop a TOML file in {}.",
                home_relative_path(&crate::user_config::automations_dir())
            ),
            appearance.ui_font_family(),
            DESCRIPTION_FONT_SIZE,
        )
        .with_color(theme.nonactive_ui_text_color().into())
        .soft_wrap(true)
        .finish()
    }
}

impl Entity for LocalAutomationsView {
    type Event = ();
}

impl View for LocalAutomationsView {
    fn ui_name() -> &'static str {
        "LocalAutomationsView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(self.render_header(appearance))
                    .with_margin_bottom(16.)
                    .finish(),
            );

        if self.rows.is_empty() && self.error_rows.is_empty() {
            content.add_child(self.render_empty_state(appearance));
        } else {
            for row in &self.rows {
                content.add_child(self.render_automation_row(row, appearance));
            }
            for row in &self.error_rows {
                content.add_child(self.render_error_row(row, appearance));
            }
        }

        let page = content.finish();

        if self.show_new_menu {
            let mut stack = Stack::new();
            stack.add_child(page);
            stack.add_positioned_child(
                ChildView::new(&self.new_menu).finish(),
                OffsetPositioning::offset_from_save_position_element(
                    NEW_BUTTON_POSITION_ID,
                    vec2f(0., 4.),
                    PositionedElementOffsetBounds::WindowByPosition,
                    PositionedElementAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            );
            stack.finish()
        } else {
            page
        }
    }
}

impl TypedActionView for LocalAutomationsView {
    type Action = LocalAutomationsViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            LocalAutomationsViewAction::Run(index) => {
                if let Some(row) = self.rows.get(*index) {
                    ctx.dispatch_typed_action(&WorkspaceAction::RunLocalAutomation {
                        automation: row.automation.clone(),
                    });
                }
            }
            LocalAutomationsViewAction::OpenConfig(index) => {
                if let Some(source_path) = self
                    .rows
                    .get(*index)
                    .and_then(|row| row.automation.source_path.clone())
                {
                    ctx.dispatch_typed_action(&WorkspaceAction::OpenLocalAutomationConfig {
                        path: source_path,
                    });
                }
            }
            LocalAutomationsViewAction::OpenErrorFile(index) => {
                if let Some(row) = self.error_rows.get(*index) {
                    ctx.dispatch_typed_action(&WorkspaceAction::OpenLocalAutomationConfig {
                        path: row.error.file_path.clone(),
                    });
                }
            }
            LocalAutomationsViewAction::ToggleNewMenu => {
                self.show_new_menu = !self.show_new_menu;
                if self.show_new_menu {
                    let items = vec![
                        MenuItemFields::new("Create with Warp's Agent")
                            .with_on_select_action(WorkspaceAction::NewLocalAutomationWithWarpAgent)
                            .into_item(),
                        MenuItemFields::new("Copy agent prompt")
                            .with_on_select_action(WorkspaceAction::CopyLocalAutomationPrompt)
                            .into_item(),
                    ];
                    self.new_menu.update(ctx, |menu, ctx| {
                        menu.set_items(items, ctx);
                    });
                    ctx.focus(&self.new_menu);
                } else {
                    ctx.focus_self();
                }
                ctx.notify();
            }
        }
    }
}
