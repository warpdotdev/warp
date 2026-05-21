use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    ChildView, ClippedScrollStateHandle, ClippedScrollable, Dismiss, ParentElement, ScrollbarWidth,
};
use warpui::fonts::FamilyId;
use warpui::{
    elements::{
        ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DropShadow, Flex,
        MainAxisAlignment, MainAxisSize, Radius, Text,
    },
    fonts::Properties,
    keymap::FixedBinding,
    AppContext, Element, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::menu::{self, Event as MenuEvent, Menu, MenuItemFields};
use crate::terminal::model_events::{ModelEvent, ModelEventDispatcher};
use crate::ui_components::blended_colors;
use crate::ui_components::icons;
use crate::view_components::action_button::{ActionButton, SecondaryTheme};

const MENU_WIDTH: f32 = 300.0;
const MENU_MAX_HEIGHT: f32 = 260.0;

pub struct NodeVersionPopupView {
    install_button: ViewHandle<ActionButton>,
    install_latest_node_button: ViewHandle<ActionButton>,
    has_nvm: bool,
    versions: Vec<String>,
    current_version: Option<String>,
    versions_menu: Option<ViewHandle<Menu<NodeVersionPopupAction>>>,
    scroll_state: ClippedScrollStateHandle,
}

#[derive(Debug, Clone)]
pub enum NodeVersionPopupAction {
    ClosePopup,
    InstallNvm,
    InstallLatestNodeVersion,
    SelectVersion { version: String },
}

#[derive(Debug, Clone)]
pub enum NodeVersionPopupEvent {
    Close,
    InstallNvm,
    InstallLatestNodeVersion,
    SelectVersion { version: String },
}

struct Styles {
    ui_font_family: FamilyId,
    background: Fill,
    main_text_color: ColorU,
    secondary_text_color: ColorU,
    tertiary_text_color: ColorU,
    detail_font_size: f32,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([FixedBinding::new(
        "escape",
        NodeVersionPopupAction::ClosePopup,
        id!(NodeVersionPopupView::ui_name()),
    )]);
}

impl NodeVersionPopupView {
    pub fn new(
        current_version: Option<String>,
        model_events: &ModelHandle<ModelEventDispatcher>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let install_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Install nvm"), SecondaryTheme)
                .with_icon(icons::Icon::Terminal)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(NodeVersionPopupAction::InstallNvm);
                })
        });
        let install_latest_node_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("nvm install node", SecondaryTheme)
                .with_icon(icons::Icon::Terminal)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(NodeVersionPopupAction::InstallLatestNodeVersion);
                })
        });
        let has_nvm = detect_nvm_installed();
        let versions = if has_nvm {
            list_nvm_versions()
        } else {
            Vec::new()
        };

        let versions_menu = if has_nvm {
            let menu_handle = ctx.add_typed_action_view(|ctx| {
                let mut menu = Menu::new().with_width(MENU_WIDTH);
                menu.set_items(Self::menu_items(&versions, current_version.as_deref()), ctx);
                let selected_index =
                    get_selected_version_index(&versions, current_version.as_deref());
                menu.set_selected_by_index(selected_index, ctx);
                menu
            });
            ctx.subscribe_to_view(&menu_handle, |_, _, event, ctx| {
                if let MenuEvent::Close { .. } = event {
                    ctx.emit(NodeVersionPopupEvent::Close);
                }
            });
            Some(menu_handle)
        } else {
            None
        };

        // Subscribe to command execution events to refresh
        // when nvm is installed or a node version is installed
        ctx.subscribe_to_model(model_events, |me, _model, event, ctx| match event {
            ModelEvent::ExecutedInBandCommand(_) | ModelEvent::AfterBlockCompleted(_) => {
                me.refresh(ctx);
            }
            _ => {}
        });

        Self {
            install_button,
            install_latest_node_button,
            has_nvm,
            versions,
            current_version,
            versions_menu,
            scroll_state: Default::default(),
        }
    }

    fn styles(&self, appearance: &Appearance) -> Styles {
        let theme = appearance.theme();
        let background = theme.surface_2();
        let main_text_color = blended_colors::text_main(theme, background);
        let secondary_text_color = blended_colors::text_sub(theme, background);
        let tertiary_text_color = theme.hint_text_color(background).into_solid();
        let detail_font_size = appearance.ui_font_size();
        let ui_font_family = appearance.ui_font_family();

        Styles {
            ui_font_family,
            background,
            main_text_color,
            secondary_text_color,
            tertiary_text_color,
            detail_font_size,
        }
    }

    fn render_install_nvm_empty_state(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let styles = self.styles(appearance);

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        col.add_child(
            Container::new(
                ConstrainedBox::new(
                    icons::Icon::NodeJS
                        .to_warpui_icon(styles.tertiary_text_color.into())
                        .finish(),
                )
                .with_width(24.)
                .with_height(24.)
                .finish(),
            )
            .with_margin_bottom(12.)
            .finish(),
        );

        col.add_child(
            Text::new(
                "Install nvm to enable version switching",
                styles.ui_font_family,
                styles.detail_font_size + 2.,
            )
            .with_style(Properties::default())
            .with_color(styles.secondary_text_color)
            .finish(),
        );

        col.add_child(
            Container::new(
                Text::new(
                    "This menu helps you switch between Node.js versions — but it requires nvm to be installed.",
                    styles.ui_font_family,
                    styles.detail_font_size,
                )
                .with_color(styles.tertiary_text_color)
                .soft_wrap(true)
                .finish(),
            )
            .with_margin_top(6.)
            .with_margin_bottom(12.)
            .with_horizontal_padding(24.)
            .finish(),
        );

        col.add_child(ChildView::new(&self.install_button).finish());

        ConstrainedBox::new(col.finish())
            .with_max_width(MENU_WIDTH)
            .with_max_height(MENU_MAX_HEIGHT)
            .finish()
    }

    fn render_install_latest_node_version_empty_state(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let styles = self.styles(appearance);

        let mut col = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max);

        col.add_child(
            Container::new(
                ConstrainedBox::new(
                    icons::Icon::NodeJS
                        .to_warpui_icon(styles.tertiary_text_color.into())
                        .finish(),
                )
                .with_width(24.)
                .with_height(24.)
                .finish(),
            )
            .with_margin_bottom(12.)
            .finish(),
        );

        // Heading
        col.add_child(
            Text::new(
                "No node versions installed",
                styles.ui_font_family,
                styles.detail_font_size + 2.,
            )
            .with_style(Properties::default())
            .with_color(styles.secondary_text_color)
            .finish(),
        );

        // Subheading
        col.add_child(
            Container::new(
                Text::new(
                    "Try installing versions with nvm",
                    styles.ui_font_family,
                    styles.detail_font_size,
                )
                .with_color(styles.tertiary_text_color)
                .soft_wrap(true)
                .finish(),
            )
            .with_margin_top(6.)
            .with_margin_bottom(12.)
            .with_horizontal_padding(24.)
            .finish(),
        );

        // Button
        col.add_child(ChildView::new(&self.install_latest_node_button).finish());

        ConstrainedBox::new(col.finish())
            .with_max_width(MENU_WIDTH)
            .with_max_height(MENU_MAX_HEIGHT)
            .finish()
    }

    fn render_node_version_selector(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let styles = self.styles(appearance);

        let mut col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        col.add_child(
            Container::new(
                Text::new("Installed", styles.ui_font_family, styles.detail_font_size)
                    .with_style(Properties::default())
                    .with_color(styles.secondary_text_color)
                    .finish(),
            )
            .with_horizontal_padding(12.)
            .finish(),
        );

        if let Some(menu) = &self.versions_menu {
            col.add_child(ChildView::new(menu).finish());
        }

        Container::new(col.finish()).with_padding_top(8.).finish()
    }

    fn menu_items(
        versions: &[String],
        current_version: Option<&str>,
    ) -> Vec<menu::MenuItem<NodeVersionPopupAction>> {
        versions
            .iter()
            .map(|ver| {
                let mut fields = MenuItemFields::new(ver).with_on_select_action(
                    NodeVersionPopupAction::SelectVersion {
                        version: ver.clone(),
                    },
                );
                if is_current_version(ver, current_version) {
                    fields = fields.with_icon(icons::Icon::Check);
                } else {
                    fields = fields.with_indent();
                }
                menu::MenuItem::Item(fields)
            })
            .collect()
    }

    pub fn refresh(&mut self, ctx: &mut ViewContext<Self>) {
        self.has_nvm = detect_nvm_installed();

        self.versions = if self.has_nvm {
            list_nvm_versions()
        } else {
            Vec::new()
        };

        if let Some(menu) = &self.versions_menu {
            menu.update(ctx, |menu, ctx| {
                menu.set_items(
                    Self::menu_items(&self.versions, self.current_version.as_deref()),
                    ctx,
                );
                let selected_index =
                    get_selected_version_index(&self.versions, self.current_version.as_deref());
                menu.set_selected_by_index(selected_index, ctx);
            });
        }

        ctx.notify();
    }
}

impl View for NodeVersionPopupView {
    fn ui_name() -> &'static str {
        "NodeVersionPopup"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let styles = self.styles(appearance);

        let content = if !self.versions.is_empty() {
            self.render_node_version_selector(app)
        } else if self.has_nvm {
            self.render_install_latest_node_version_empty_state(app)
        } else {
            self.render_install_nvm_empty_state(app)
        };

        let scrollable = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            content,
            ScrollbarWidth::Auto,
            appearance.theme().nonactive_ui_detail().into(),
            appearance.theme().active_ui_detail().into(),
            warpui::elements::Fill::None,
        )
        .with_overlayed_scrollbar()
        .finish();

        Dismiss::new(
            ConstrainedBox::new(
                Container::new(scrollable)
                    .with_background(styles.background)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                    .with_drop_shadow(DropShadow::default())
                    .finish(),
            )
            .with_width(MENU_WIDTH)
            .with_max_height(MENU_MAX_HEIGHT)
            .finish(),
        )
        .prevent_interaction_with_other_elements()
        .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(NodeVersionPopupAction::ClosePopup))
        .finish()
    }
}

impl Entity for NodeVersionPopupView {
    type Event = NodeVersionPopupEvent;
}

impl NodeVersionPopupView {
    pub fn focus_content(&self, ctx: &mut ViewContext<Self>) {
        // Focus menu if present to allow keyboard navigation
        if let Some(menu) = &self.versions_menu {
            ctx.focus(menu);
        } else {
            ctx.focus_self();
        }
    }
}

impl TypedActionView for NodeVersionPopupView {
    type Action = NodeVersionPopupAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            NodeVersionPopupAction::ClosePopup => ctx.emit(NodeVersionPopupEvent::Close),
            NodeVersionPopupAction::InstallNvm => ctx.emit(NodeVersionPopupEvent::InstallNvm),
            NodeVersionPopupAction::InstallLatestNodeVersion => {
                ctx.emit(NodeVersionPopupEvent::InstallLatestNodeVersion)
            }
            NodeVersionPopupAction::SelectVersion { version } => {
                ctx.emit(NodeVersionPopupEvent::SelectVersion {
                    version: version.clone(),
                });
                ctx.emit(NodeVersionPopupEvent::Close);
            }
        }
    }
}

// Cross-OS detection of nvm availability
fn detect_nvm_installed() -> bool {
    use std::env;

    // Helper: check if an executable exists in PATH
    fn in_path(candidate: &str) -> bool {
        if let Ok(path_var) = env::var("PATH") {
            for dir in env::split_paths(&path_var) {
                let mut p = dir.clone();
                p.push(candidate);
                if p.is_file() {
                    return true;
                }
            }
        }
        false
    }

    // 1) Windows nvm-windows
    #[cfg(windows)]
    {
        env::var("PATH").is_ok_and(|path_var| path_var.contains("%NVM_HOME%"))
            || env::var("NVM_HOME").is_ok()
    }

    // 2) POSIX shells: nvm is typically a shell function; detect via standard install locations
    #[cfg(not(windows))]
    {
        use std::path::Path;

        // NVM_DIR env var with nvm.sh present
        if let Ok(nvm_dir) = env::var("NVM_DIR") {
            let nvm_sh = Path::new(&nvm_dir).join("nvm.sh");
            if nvm_sh.is_file() {
                return true;
            }
        }

        // Default NVM_DIR ~/.nvm/nvm.sh
        if let Some(home) = dirs::home_dir() {
            let nvm_sh = home.join(".nvm").join("nvm.sh");
            if nvm_sh.is_file() {
                return true;
            }
        }

        // Homebrew locations
        let brew_paths: &[&str] = &["/opt/homebrew/opt/nvm", "/usr/local/opt/nvm"];
        for base in brew_paths {
            let nvm_sh = Path::new(base).join("nvm.sh");
            if nvm_sh.is_file() {
                return true;
            }
        }

        // Fish plugin-based installs
        if let Some(home) = dirs::home_dir() {
            let fish_nvm = home
                .join(".config")
                .join("fish")
                .join("functions")
                .join("nvm.fish");
            if fish_nvm.is_file() {
                return true;
            }
            // macOS possible alt path for fish conf sometimes under Library
            let fish_alt = home
                .join("Library")
                .join("Application Support")
                .join("fish")
                .join("functions")
                .join("nvm.fish");
            if fish_alt.is_file() {
                return true;
            }
        }

        // If an `nvm` shim exists on PATH (rare on unix because it's a function), still check
        if in_path("nvm") {
            return true;
        }

        false
    }
}

// Enumerate installed Node versions managed by nvm (best-effort, cross-OS)
fn list_nvm_versions() -> Vec<String> {
    use std::env;
    use std::path::Path;

    let mut out: Vec<String> = Vec::new();

    #[cfg(windows)]
    {
        if let Ok(nvm_home) = env::var("NVM_HOME") {
            let base = Path::new(&nvm_home);
            if let Ok(read_dir) = std::fs::read_dir(base) {
                for entry in read_dir.flatten() {
                    if let Ok(ft) = entry.file_type() {
                        if ft.is_dir() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            // nvm-windows typically uses folder names like v18.19.1 or 18.19.1
                            if name
                                .chars()
                                .next()
                                .map(|c| c == 'v' || c.is_ascii_digit())
                                .unwrap_or(false)
                            {
                                out.push(name);
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(not(windows))]
    {
        // Prefer $NVM_DIR/versions/node
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(nvm_dir) = env::var("NVM_DIR") {
            candidates.push(Path::new(&nvm_dir).join("versions").join("node"));
        }
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".nvm").join("versions").join("node"));
        }

        for base in candidates {
            if let Ok(read_dir) = std::fs::read_dir(&base) {
                for entry in read_dir.flatten() {
                    if let Ok(ft) = entry.file_type() {
                        if ft.is_dir() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            out.push(name);
                        }
                    }
                }
            }
        }
    }

    // Sort descending so the latest version is first
    out.sort_by(|a, b| b.cmp(a));
    out.dedup();
    out
}

fn normalize_version(ver: &str) -> String {
    ver.trim()
        .strip_prefix('v')
        .unwrap_or(ver.trim())
        .to_string()
}

fn is_current_version(candidate: &str, current: Option<&str>) -> bool {
    current.is_some_and(|cur| normalize_version(candidate) == normalize_version(cur))
}

fn get_selected_version_index(versions: &[String], current_version: Option<&str>) -> usize {
    versions
        .iter()
        .position(|v| is_current_version(v, current_version))
        .unwrap_or(0)
}
