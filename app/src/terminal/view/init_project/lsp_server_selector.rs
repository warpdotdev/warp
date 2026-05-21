use crate::ai::agent::icons::yellow_stop_icon;
use crate::ai::blocklist::block::toggleable_items::{ToggleableItemBuilder, ToggleableItemsView};
use crate::ai::blocklist::inline_action::inline_action_header::INLINE_ACTION_HORIZONTAL_PADDING;
use crate::ai::blocklist::inline_action::inline_action_icons::icon_size;
use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use lsp::supported_servers::LSPServerType;
use std::path::PathBuf;

use warpui::{
    elements::{
        Border, ChildView, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Expanded,
        Flex, MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable,
    },
    keymap::Keystroke,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
        keyboard_shortcut::KeyboardShortcut,
        text::Span,
    },
    AppContext, Element, SingletonEntity, TypedActionView, ViewContext, ViewHandle,
};

use super::{InitProjectBlockAction, InitStepBlock};

#[derive(Debug, Clone)]
pub struct LSPServerInfo {
    pub server_type: LSPServerType,
    pub is_installed: bool,
}

/// Creates a ToggleableItemsView configured for LSP server selection.
pub fn create_lsp_server_selector(
    server_info: Vec<LSPServerInfo>,
    repo_path: PathBuf,
    ctx: &mut ViewContext<InitStepBlock>,
) -> ViewHandle<ToggleableItemsView<LSPServerInfo>> {
    let builder = ToggleableItemBuilder::<LSPServerInfo>::new(
        |info, app| {
            let appearance = Appearance::as_ref(app);
            let theme = appearance.theme();
            Span::new(
                info.server_type.binary_name(),
                UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_color: Some(theme.main_text_color(theme.background()).into_solid()),
                    font_size: Some(appearance.monospace_font_size()),
                    ..Default::default()
                },
            )
        },
        // Default all as selected
        |_| true,
    );

    let view = ToggleableItemsView::new(server_info, builder);

    let view_handle = ctx.add_typed_action_view(|_| view);

    // Set up event handling for checkbox selection changes
    let repo_path_for_submit = repo_path.clone();

    ctx.subscribe_to_view(
        &view_handle,
        move |parent_me, lsp_view_handle, event, parent_ctx| {
            use crate::ai::blocklist::block::toggleable_items::ToggleableItemsEvent;

            match event {
                ToggleableItemsEvent::SelectionChanged => {
                    // Notify to trigger re-render (button state may have changed)
                    parent_ctx.notify();
                }
                ToggleableItemsEvent::SubmitRequested => {
                    // Get selected items
                    let selected_items: Vec<LSPServerInfo> = {
                        let view = lsp_view_handle.as_ref(parent_ctx);
                        view.get_selected_items().cloned().collect()
                    };

                    if !selected_items.is_empty() {
                        parent_me.handle_action(
                            &InitProjectBlockAction::SetupLanguageServers {
                                server_info: selected_items,
                                repo_path: repo_path_for_submit.clone(),
                            },
                            parent_ctx,
                        );
                    }
                }
            }
        },
    );

    view_handle
}

/// Renders the complete LSP server selector block with header and checkboxes.
pub fn render_lsp_selector_block(
    action_view: &ViewHandle<ToggleableItemsView<LSPServerInfo>>,
    repo_path: &std::path::Path,
    skip_mouse_state: &MouseStateHandle,
    enable_mouse_state: &MouseStateHandle,
    appearance: &Appearance,
    ctx: &AppContext,
) -> Box<dyn Element> {
    let mut step_content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

    // Header
    let header_background = appearance.theme().surface_2();
    let mut header_row = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    // Left side: icon + text
    let mut left_content = Flex::row()
        .with_main_axis_alignment(MainAxisAlignment::Start)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    left_content.add_child(
        Container::new(
            ConstrainedBox::new(yellow_stop_icon(appearance).finish())
                .with_width(icon_size(ctx))
                .with_height(icon_size(ctx))
                .finish(),
        )
        .with_margin_right(8.)
        .finish(),
    );

    let title_element = Span::new(
        "Would you like to enable available language support for this codebase? This will give you smarter code navigation and inline error checking.",
        UiComponentStyles {
            font_family_id: Some(appearance.ui_font_family()),
            font_color: Some(blended_colors::text_main(appearance.theme(), header_background)),
            font_size: Some(appearance.monospace_font_size()),
            ..Default::default()
        },
    )
    .with_soft_wrap()
    .build()
    .finish();

    left_content.add_child(
        Expanded::new(
            1.,
            Container::new(title_element).with_margin_right(8.).finish(),
        )
        .finish(),
    );

    header_row.add_child(Shrinkable::new(1., left_content.finish()).finish());

    // Right side: buttons with proper action dispatch
    let view = action_view.as_ref(ctx);
    let selected_items: Vec<LSPServerInfo> = view.get_selected_items().cloned().collect();

    let mut buttons_row = Flex::row()
        .with_spacing(4.)
        .with_cross_axis_alignment(CrossAxisAlignment::Center);

    let skip_button = appearance
        .ui_builder()
        .button(ButtonVariant::Text, skip_mouse_state.clone())
        .with_text_label(crate::i18n::tr_static(ctx, "Skip for now").to_string())
        .build()
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(InitProjectBlockAction::SkipLanguageServers);
        })
        .finish();
    buttons_row.add_child(Container::new(skip_button).with_margin_right(4.).finish());

    // Enable button with Enter shortcut
    let any_selected = !selected_items.is_empty();
    let any_needs_download = selected_items.iter().any(|info| !info.is_installed);

    let enable_label = if any_needs_download {
        "Install and enable"
    } else {
        "Enable language support"
    };

    // Create keyboard shortcut for Enter
    let submit_keystroke = Keystroke::parse("enter").expect("can parse enter");

    let submit_shortcut = KeyboardShortcut::new(
        &submit_keystroke,
        UiComponentStyles {
            font_family_id: Some(appearance.ui_font_family()),
            font_size: Some(appearance.monospace_font_size() - 2.),
            font_color: Some(blended_colors::text_sub(
                appearance.theme(),
                appearance.theme().accent(),
            )),
            margin: Some(Coords::default().left(6.)),
            ..Default::default()
        },
    )
    .text_only()
    .build()
    .finish();

    let enable_button_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(
            Span::new(
                enable_label,
                UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_size: Some(appearance.monospace_font_size()),
                    ..Default::default()
                },
            )
            .build()
            .finish(),
        )
        .with_child(submit_shortcut)
        .finish();

    let mut enable_button = appearance
        .ui_builder()
        .button(ButtonVariant::Accent, enable_mouse_state.clone())
        .with_custom_label(enable_button_row);

    if !any_selected {
        enable_button = enable_button.disabled();
    }

    // Capture selected items and repo_path for the click handler
    let selected_items_clone = selected_items.clone();
    let repo_path_clone = repo_path.to_path_buf();
    let enable_button_element = enable_button
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(InitProjectBlockAction::SetupLanguageServers {
                server_info: selected_items_clone.clone(),
                repo_path: repo_path_clone.clone(),
            });
        })
        .finish();

    buttons_row.add_child(
        Container::new(enable_button_element)
            .with_margin_right(4.)
            .finish(),
    );

    header_row.add_child(buttons_row.finish());

    let header_container = Container::new(header_row.finish())
        .with_padding_left(INLINE_ACTION_HORIZONTAL_PADDING)
        .with_padding_right(INLINE_ACTION_HORIZONTAL_PADDING)
        .with_vertical_padding(12.)
        .with_background(header_background)
        .with_corner_radius(CornerRadius::with_top(Radius::Pixels(8.)))
        .finish();

    step_content.add_child(header_container);

    let checkboxes_container = Container::new(ChildView::new(action_view).finish())
        .with_uniform_padding(16.)
        .finish();
    step_content.add_child(checkboxes_container);

    Container::new(step_content.finish())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
        .with_border(Border::all(1.).with_border_fill(appearance.theme().surface_2()))
        .with_background(appearance.theme().surface_1())
        .finish()
}
