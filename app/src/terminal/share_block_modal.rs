use crate::{
    appearance::Appearance,
    editor::{EditorView, Event as EditorEvent, SingleLineEditorOptions, TextOptions},
    send_telemetry_from_ctx,
    server::{
        block::{Block as ServerBlock, DisplaySetting},
        server_api::block::BlockClient,
        telemetry::TelemetryEvent,
    },
    settings::{EnforceMinimumContrast, FontSettings, FontSettingsChangedEvent, PrivacySettings},
    settings_view::SettingsSection,
    terminal::{
        grid_renderer::{self},
        ligature_settings::{should_use_ligature_rendering, LigatureSettings},
        model::{terminal_model::BlockIndex, ObfuscateSecrets},
        safe_mode_settings::get_secret_obfuscation_mode,
        TerminalModel,
    },
    themes::theme::WarpTheme,
    ui_components::icons::Icon,
    util::bindings::CustomAction,
    view_components::ToastFlavor,
    workspace::WorkspaceAction,
};

use super::grid_renderer::CellGlyphCache;
use super::model::grid::RespectDisplayedOutput;
use crate::ai::generate_block_title::api::GenerateBlockTitleRequest;
use crate::editor::EditOrigin;
use crate::settings::AISettings;
use crate::workspaces::user_workspaces::UserWorkspaces;
use anyhow::Result;
use parking_lot::FairMutex;
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use serde::Serialize;
use std::{ops::RangeInclusive, sync::Arc};
use warp_core::features::FeatureFlag;
use warp_core::ui::theme::Fill;
use warpui::r#async::SpawnedFutureHandle;
use warpui::{
    clipboard::ClipboardContent,
    elements::{
        try_rect_with_z, Align, Border, ConstrainedBox, Container, CornerRadius,
        CrossAxisAlignment, Dismiss, Element, Empty, Flex, MainAxisAlignment, MainAxisSize,
        MouseStateHandle, ParentElement, Point, Radius, SavePosition, ScrollData,
        ScrollStateHandle, Scrollable, ScrollableElement, ScrollbarWidth, Shrinkable, Stack, Text,
    },
    event::{DispatchedEvent, ModifiersState},
    fonts::{FamilyId, Properties, Style, Weight},
    keymap::FixedBinding,
    ui_components::{
        button::{ButtonVariant, TextAndIcon, TextAndIconAlignment},
        components::{Coords, UiComponent, UiComponentStyles},
        radio_buttons::{RadioButtonItem, RadioButtonLayout, RadioButtonStateHandle},
    },
    units::{IntoLines, IntoPixels, Lines, Pixels},
    AfterLayoutContext, AppContext, ClipBounds, Entity, Event, EventContext, FocusContext,
    LayoutContext, PaintContext, SingletonEntity, SizeConstraint, TypedActionView, View,
    ViewContext, ViewHandle,
};

const PADDING: f32 = 30.;
const INNER_MARGIN: f32 = 20.;

const MODAL_WIDTH: f32 = 862.;
const BLOCK_TITLE_INPUT_WIDTH: f32 = 800.;

const BLOCK_TITLE_PLACEHOLDER: &str = "Title (optional)";

// TODO(vorporeal): This is 12 in the specs, but I think our 14pt font is a bit
// taller than 14pt?
const VERTICAL_SEPARATOR_HEIGHT: f32 = 32.;
const CHECKBOX_SIZE: f32 = 18.;

const NEW_BUTTON_VERTICAL_PADDING: f32 = 10.;
const NEW_BUTTON_HORIZONTAL_PADDING: f32 = 10.;
const NEW_COPY_BUTTON_WIDTH: f32 = 80.;

const COMMAND_AND_OUTPUT_OPTION: (&str, DisplaySetting) =
    ("Command and Output", DisplaySetting::CommandAndOutput);
const COMMAND_OPTION: (&str, DisplaySetting) = ("Command", DisplaySetting::Command);
const OUTPUT_OPTION: (&str, DisplaySetting) = ("Output", DisplaySetting::Output);

/// This default title is helpful for screen readers.
const DEFAULT_EMBED_TITLE: &str = "embedded warp block";
const BLOCK_CREATION_FAILED_MESSAGE: &str = "Something went wrong. Please try again.";

#[derive(PartialEq)]
enum ShareRequestState {
    None,
    Pending(ShareBlockType),
    Failed,
    Succeeded {
        link: String,
        share_type: ShareBlockType,
    },
}

#[derive(PartialEq, Copy, Clone, Debug, Serialize)]
pub enum ShareBlockType {
    HtmlEmbed,
    Permalink,
}

#[derive(Default)]
struct MouseStateHandles {
    close_modal_hover_state: MouseStateHandle,
    show_prompt_mouse_state: MouseStateHandle,
    get_embed_button_mouse_state: MouseStateHandle,
    create_link_button_mouse_state: MouseStateHandle,
    copy_button_mouse_state: MouseStateHandle,
    manage_permalinks_mouse_state: MouseStateHandle,
    redact_secrets_mouse_state: MouseStateHandle,
}

#[derive(Default, Clone)]
struct EmbedDisplayHandles {
    embed_display_state_handle: RadioButtonStateHandle,
    embed_display_mouse_states: Vec<MouseStateHandle>,
}

pub struct ShareBlockModal {
    /// The model for the session containing the block being shared. This is an `Option` because
    /// the share modal view exists at the PaneGroup level (so that it's sized and positioned
    /// relative to the tab). However, terminal models are session-specific, so this is only
    /// available if the modal is open and displaying a specific session's block.
    model: Option<Arc<FairMutex<TerminalModel>>>,
    block_client: Arc<dyn BlockClient>,
    request_state: ShareRequestState,
    selected_block: Option<BlockIndex>,
    mouse_state_handles: MouseStateHandles,
    /// The number of lines from the top the viewport is scrolled down.
    scroll_top: Lines,
    scroll_state: ScrollStateHandle,
    block_title_editor: ViewHandle<EditorView>,
    embed_display_handles: EmbedDisplayHandles,
    embed_display_options: Vec<(String, DisplaySetting)>,
    show_prompt: bool,
    obfuscate_secrets: ObfuscateSecrets,
    /// We abort the block title generation requests early if the user updated the title text field
    /// before the request completes, rendering the current pending banner request irrelevant.
    title_generation_future_handle: Option<SpawnedFutureHandle>,
}

#[derive(Clone, Copy, Debug)]
pub enum ShareBlockModalAction {
    Close,
    CopyLink,
    CopyEmbed,
    GenerateSharedBlock(ShareBlockType),
    Scroll(Lines),
    ToggleShowPrompt,
    ToggleObfuscateSecrets,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings(vec![
        FixedBinding::custom(
            CustomAction::Copy,
            ShareBlockModalAction::CopyLink,
            "Copy",
            id!(ShareBlockModal::ui_name()),
        ),
        FixedBinding::new(
            "escape",
            ShareBlockModalAction::Close,
            id!(ShareBlockModal::ui_name()),
        ),
    ]);
}

#[derive(PartialEq, Eq)]
pub enum ShareBlockModalEvent {
    Close,
    ShowToast {
        message: String,
        flavor: ToastFlavor,
    },
}

impl ShareBlockModal {
    pub fn new(
        model: Option<Arc<FairMutex<TerminalModel>>>,
        block_client: Arc<dyn BlockClient>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let block_title_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(14.), appearance),
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text(BLOCK_TITLE_PLACEHOLDER, ctx);
            editor
        });
        ctx.subscribe_to_view(&block_title_editor, move |me, _, event, ctx| {
            if matches!(
                event,
                EditorEvent::Paste | EditorEvent::Edited(EditOrigin::UserTyped)
            ) {
                if let Some(handle) = me.title_generation_future_handle.take() {
                    handle.abort();
                }
            }
            ctx.notify();
        });

        let embed_display_handles = EmbedDisplayHandles {
            embed_display_mouse_states: vec![
                Default::default(),
                Default::default(),
                Default::default(),
            ],
            ..Default::default()
        };

        let embed_display_options = [COMMAND_AND_OUTPUT_OPTION, COMMAND_OPTION, OUTPUT_OPTION]
            .map(|(name, display_setting)| (name.to_string(), display_setting))
            .to_vec();

        let ligature_handle = LigatureSettings::handle(ctx);
        ctx.subscribe_to_model(&ligature_handle, |_, _, _, ctx| ctx.notify());

        ctx.subscribe_to_model(&FontSettings::handle(ctx), |_, _, event, ctx| {
            if matches!(
                event,
                FontSettingsChangedEvent::EnforceMinimumContrast { .. }
            ) {
                ctx.notify();
            }
        });

        Self {
            model,
            block_client,
            request_state: ShareRequestState::None,
            selected_block: None,
            mouse_state_handles: Default::default(),
            scroll_top: Lines::zero(),
            scroll_state: Default::default(),
            block_title_editor,
            embed_display_handles,
            embed_display_options,
            show_prompt: false,
            obfuscate_secrets: get_secret_obfuscation_mode(ctx),
            title_generation_future_handle: None,
        }
    }

    fn toggle_show_prompt(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_prompt = !self.show_prompt;
        ctx.notify();
    }

    fn toggle_obfuscate_secrets(&mut self, ctx: &mut ViewContext<Self>) {
        self.obfuscate_secrets = !self.obfuscate_secrets;
        if self.obfuscate_secrets.is_visually_obfuscated() {
            self.scan_selected_block_for_secrets(ctx);
        }
        ctx.notify();
    }

    fn scan_selected_block_for_secrets(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(model) = &self.model else {
            return;
        };
        let Some(selected_block_index) = self.selected_block else {
            return;
        };
        {
            let model = model.lock();
            let Some(selected_block) = model.block_list().block_at(selected_block_index) else {
                return;
            };
            if selected_block.all_bytes_scanned_for_secrets() {
                // We don't need to scan for secrets.
                return;
            }
        }
        let model = model.clone();
        ctx.spawn(
            async move {
                model
                    .lock()
                    .block_list_mut()
                    .scan_block_for_secrets(selected_block_index)
            },
            |_, _, ctx| ctx.notify(),
        );
    }

    pub fn scroll(&mut self, scroll_top: Lines, ctx: &mut ViewContext<Self>) {
        self.scroll_top = scroll_top;
        ctx.notify();
    }

    fn current_display_setting(&self) -> DisplaySetting {
        let selected_idx = self
            .embed_display_handles
            .embed_display_state_handle
            .get_selected_idx()
            .unwrap_or(0);
        self.embed_display_options[selected_idx].1.clone()
    }

    pub fn save_block(&mut self, share_type: ShareBlockType, ctx: &mut ViewContext<Self>) {
        let block_title = self.block_title_editor.as_ref(ctx).buffer_text(ctx);
        let display_setting = self.current_display_setting();

        let server_block = {
            let model = match &self.model {
                Some(model) => model.lock(),
                None => {
                    log::error!("Opened share modal without a model");
                    self.request_state = ShareRequestState::Failed;
                    ctx.notify();
                    return;
                }
            };
            let block = match self
                .selected_block
                .and_then(|block_index| model.block_list().block_at(block_index))
            {
                None => return,
                Some(block) => block,
            };

            if block.render_prompt_on_same_line() {
                if display_setting == DisplaySetting::Output {
                    // We do NOT show the prompt, if showing the output only, even if we are using the combined prompt/command grid.
                    self.show_prompt = false;
                } else {
                    // We must show the prompt, if we're not allowing prompt configuration (due to PS1 with Same Line Prompt).
                    self.show_prompt = true;
                }
            }

            ServerBlock::new(
                block,
                self.show_prompt,
                &display_setting,
                self.obfuscate_secrets,
            )
        };

        self.request_state = ShareRequestState::Pending(share_type);

        send_telemetry_from_ctx!(
            TelemetryEvent::GenerateBlockSharingLink {
                share_type,
                display_setting: display_setting.clone(),
                show_prompt: self.show_prompt,
                redact_secrets: self.obfuscate_secrets.is_visually_obfuscated(),
            },
            ctx
        );
        let block_client = self.block_client.clone();

        let show_prompt = self.show_prompt;
        let _ = ctx.spawn(
            async move {
                block_client
                    .save_block(
                        &server_block,
                        Some(block_title),
                        show_prompt,
                        display_setting,
                    )
                    .await
            },
            match share_type {
                ShareBlockType::HtmlEmbed => Self::on_save_embed_returned,
                ShareBlockType::Permalink => Self::on_save_link_returned,
            },
        );
    }

    fn display_failure_toast(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(ShareBlockModalEvent::ShowToast {
            message: BLOCK_CREATION_FAILED_MESSAGE.to_string(),
            flavor: ToastFlavor::Error,
        });
    }

    fn on_save_link_returned(&mut self, res: Result<String>, ctx: &mut ViewContext<Self>) {
        if let Ok(link) = res {
            self.request_state = ShareRequestState::Succeeded {
                link,
                share_type: ShareBlockType::Permalink,
            };
            self.copy(ctx);
            ctx.notify();
        } else {
            self.request_state = ShareRequestState::Failed;
            self.display_failure_toast(ctx);
        }
    }

    fn on_save_embed_returned(&mut self, res: Result<String>, ctx: &mut ViewContext<Self>) {
        if let Ok(link) = res {
            self.request_state = ShareRequestState::Succeeded {
                link,
                share_type: ShareBlockType::HtmlEmbed,
            };
            self.copy_embed(ctx);
            ctx.notify();
        } else {
            self.request_state = ShareRequestState::Failed;
            self.display_failure_toast(ctx);
        }
    }

    pub fn open_with_model_update(
        &mut self,
        model: Arc<FairMutex<TerminalModel>>,
        block_id: BlockIndex,
        ctx: &mut ViewContext<Self>,
    ) {
        ctx.focus_self();
        self.model = Some(model);
        self.selected_block = Some(block_id);
        self.obfuscate_secrets = get_secret_obfuscation_mode(ctx);
        if self.obfuscate_secrets.is_visually_obfuscated() {
            self.scan_selected_block_for_secrets(ctx);
        }

        if !should_send_title_gen_request(ctx) {
            return;
        }

        // Scope to release the mutex.
        let request = {
            let model = self.model.as_ref().expect("Model should be set").lock();
            let block = match self
                .selected_block
                .and_then(|block_index| model.block_list().block_at(block_index))
            {
                None => {
                    log::error!("Opened block share modal without block");
                    return;
                }
                Some(block) => block,
            };

            let terminal_width: usize = model.block_list().size().columns;
            let (command, output) = block.get_block_content_summary(terminal_width, 100, 200);

            GenerateBlockTitleRequest { command, output }
        };

        let block_client = self.block_client.clone();
        self.title_generation_future_handle = Some(ctx.spawn(
            async move { block_client.generate_shared_block_title(request).await },
            |me, response, ctx| {
                me.title_generation_future_handle = None;
                if let Ok(resp) = response {
                    me.block_title_editor.update(ctx, |editor, ctx| {
                        if !editor.is_dirty(ctx) {
                            editor.set_buffer_text(&resp.title, ctx);
                        }
                    })
                }
            },
        ));
    }

    fn link(&self) -> Option<String> {
        if let ShareRequestState::Succeeded { link, .. } = &self.request_state {
            return Some(link.to_string());
        }
        None
    }

    fn reset(&mut self, ctx: &mut ViewContext<Self>) {
        self.request_state = ShareRequestState::None;
        self.scroll_top = Lines::zero();
        self.selected_block = None;
        self.block_title_editor.update(ctx, |editor, ctx| {
            editor.clear_buffer_and_reset_undo_stack(ctx);
            editor.set_base_buffer_text("".to_string(), ctx);
        });
    }

    pub fn close(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(ShareBlockModalEvent::Close);
        self.reset(ctx);
    }

    pub fn copy(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(link) = self.link() {
            send_telemetry_from_ctx!(
                TelemetryEvent::CopyBlockSharingLink(ShareBlockType::Permalink),
                ctx
            );
            ctx.clipboard().write(ClipboardContent::plain_text(link));
            ctx.emit(ShareBlockModalEvent::ShowToast {
                message: "Link copied.".to_string(),
                flavor: ToastFlavor::Default,
            });
        }
    }

    fn generate_embed_snippet(&self, app: &AppContext) -> Option<String> {
        let link = self.link()?;
        // The URL path for embedded blocks are /block/embed/[BLOCK-ID], but the link given to us by the server is /block/[BLOCK-ID].
        let embed_link = link.replace("/block/", "/block/embed/");
        let model = self.model.clone()?;
        let selected_block_idx = self.selected_block?;
        let model = model.lock();
        let block = model.block_list().block_at(selected_block_idx)?;

        let height = ServerBlock::embed_pixel_height(
            block,
            self.show_prompt,
            &self.current_display_setting(),
        );
        let width = ServerBlock::embed_pixel_width(block);
        let mut title = self.block_title_editor.as_ref(app).buffer_text(app);
        if title.is_empty() {
            title = DEFAULT_EMBED_TITLE.to_string();
        }

        Some(format!(
            "<iframe src=\"{embed_link}\" title=\"{title}\" style=\"width: {width}px; height: {height}px; border:0; overflow:hidden;\" allow=\"clipboard-read; clipboard-write\"></iframe>"
        ))
    }

    pub fn copy_embed(&self, ctx: &mut ViewContext<Self>) {
        let embed_snippet = self.generate_embed_snippet(ctx);
        let Some(embed_snippet) = embed_snippet else {
            log::warn!("Could not generate embed snippet");
            return;
        };
        send_telemetry_from_ctx!(
            TelemetryEvent::CopyBlockSharingLink(ShareBlockType::HtmlEmbed),
            ctx
        );
        ctx.clipboard()
            .write(ClipboardContent::plain_text(embed_snippet));
        ctx.emit(ShareBlockModalEvent::ShowToast {
            message: "Embed code copied.".to_string(),
            flavor: ToastFlavor::Success,
        });
    }

    fn render_close_modal_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        const BUTTON_DIAMETER: f32 = 24.;
        appearance
            .ui_builder()
            .close_button(
                BUTTON_DIAMETER,
                self.mouse_state_handles.close_modal_hover_state.clone(),
            )
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(ShareBlockModalAction::Close))
            .finish()
    }

    fn render_permalink_label(&self, appearance: &Appearance, link_text: &str) -> Box<dyn Element> {
        Shrinkable::new(
            1.,
            Container::new(
                Align::new(
                    Text::new_inline(link_text.to_owned(), appearance.ui_font_family(), 14.)
                        .with_color(appearance.theme().nonactive_ui_text_color().into())
                        .finish(),
                )
                .left()
                .finish(),
            )
            .with_background(appearance.theme().background())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
            .with_vertical_padding(12.)
            .with_horizontal_padding(16.)
            .finish(),
        )
        .finish()
    }

    fn render_embed_label(
        &self,
        appearance: &Appearance,
        embed_snippet: String,
    ) -> Box<dyn Element> {
        ConstrainedBox::new(
            Container::new(
                Text::new(embed_snippet, appearance.monospace_font_family(), 14.)
                    .with_color(appearance.theme().nonactive_ui_text_color().into())
                    .finish(),
            )
            .with_background(appearance.theme().background())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
            .with_vertical_padding(12.)
            .with_horizontal_padding(16.)
            .finish(),
        )
        .with_max_height(88.)
        .finish()
    }

    fn button_style_overrides(&self, appearance: &Appearance) -> UiComponentStyles {
        UiComponentStyles {
            font_size: Some(14.),
            font_family_id: Some(appearance.ui_builder().ui_font_family()),
            font_weight: Some(Weight::Bold),
            padding: Some(Coords {
                top: NEW_BUTTON_VERTICAL_PADDING,
                bottom: NEW_BUTTON_VERTICAL_PADDING,
                left: NEW_BUTTON_HORIZONTAL_PADDING,
                right: NEW_BUTTON_HORIZONTAL_PADDING,
            }),
            ..Default::default()
        }
    }

    fn render_create_block_buttons_row(&self, appearance: &Appearance) -> Box<dyn Element> {
        let create_link_button = self.render_create_block_button(
            appearance,
            "Create link",
            Icon::Link,
            ButtonVariant::Accent,
            self.mouse_state_handles
                .create_link_button_mouse_state
                .clone(),
            ShareBlockType::Permalink,
        );
        let get_embed_button = self.render_create_block_button(
            appearance,
            "Get embed",
            Icon::Code1,
            ButtonVariant::Basic,
            self.mouse_state_handles
                .get_embed_button_mouse_state
                .clone(),
            ShareBlockType::HtmlEmbed,
        );
        Flex::row()
            .with_child(get_embed_button)
            .with_child(create_link_button)
            .finish()
    }

    fn render_create_block_button(
        &self,
        appearance: &Appearance,
        text_label: &str,
        icon: Icon,
        button_variant: ButtonVariant,
        mouse_state_handle: MouseStateHandle,
        share_type: ShareBlockType,
    ) -> Box<dyn Element> {
        let text_and_icon = TextAndIcon::new(
            TextAndIconAlignment::TextFirst,
            if let ShareRequestState::Pending(pending_share_type) = self.request_state {
                if pending_share_type == share_type {
                    "Creating block...".to_string()
                } else {
                    text_label.to_string()
                }
            } else {
                text_label.to_string()
            },
            icon.to_warpui_icon(appearance.theme().active_ui_text_color()),
            MainAxisSize::Max,
            MainAxisAlignment::Center,
            vec2f(16., 16.),
        )
        .with_inner_padding(4.);

        let mut button = appearance
            .ui_builder()
            .button(button_variant, mouse_state_handle)
            .with_style(
                self.button_style_overrides(appearance)
                    .set_margin(Coords {
                        left: 8.,
                        ..Default::default()
                    })
                    .set_width(200.),
            )
            .with_text_and_icon_label(text_and_icon);
        if let ShareRequestState::Pending(pending_share_type) = self.request_state {
            if pending_share_type != share_type {
                // Disable the share button that wasn't selected while request is pending.
                button = button.disabled();
            }
        }

        button
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(ShareBlockModalAction::GenerateSharedBlock(share_type))
            })
            .finish()
    }

    fn render_vertical_separator(&self, appearance: &Appearance) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(Empty::new().finish())
                .with_max_height(VERTICAL_SEPARATOR_HEIGHT)
                .finish(),
        )
        .with_border(Border::left(1.).with_border_fill(appearance.theme().outline()))
        .finish()
    }

    fn render_success_footer(
        &self,
        appearance: &Appearance,
        link_text: &str,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut col = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Start);
        if matches!(
            &self.request_state,
            ShareRequestState::Succeeded {
                share_type: ShareBlockType::Permalink,
                ..
            }
        ) {
            let link_button_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(self.render_permalink_label(appearance, link_text))
                .with_child(self.render_copy_button(ShareBlockModalAction::CopyLink, appearance))
                .finish();
            col.add_child(link_button_row);
        } else {
            let embed_snippet = self
                .generate_embed_snippet(app)
                .unwrap_or("Error generating embed snippet".to_string());
            col.add_child(self.render_embed_label(appearance, embed_snippet));
            col.add_child(
                Align::new(
                    Container::new(
                        self.render_copy_button(ShareBlockModalAction::CopyEmbed, appearance),
                    )
                    .with_margin_top(8.)
                    .finish(),
                )
                .right()
                .finish(),
            );
        }
        col.finish()
    }

    fn render_manage_permalinks_button(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Text,
                self.mouse_state_handles
                    .manage_permalinks_mouse_state
                    .clone(),
            )
            .with_centered_text_label(
                crate::i18n::tr_static(app, "Manage shared blocks").to_string(),
            )
            .with_style(
                self.button_style_overrides(appearance)
                    .set_font_size(12.)
                    .set_padding(Coords {
                        top: 7.,
                        bottom: 7.,
                        left: 12.,
                        right: 12.,
                    })
                    .set_width(170.),
            )
            .build()
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(ShareBlockModalAction::Close);
                ctx.dispatch_typed_action(WorkspaceAction::ShowSettingsPage(
                    SettingsSection::SharedBlocks,
                ));
            });
        if matches!(self.request_state, ShareRequestState::Pending(_)) {
            button = button.disable();
        }
        button.finish()
    }

    fn render_copy_button(
        &self,
        action: ShareBlockModalAction,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let text_and_icon = TextAndIcon::new(
            TextAndIconAlignment::TextFirst,
            "Copy".to_string(),
            Icon::Copy.to_warpui_icon(appearance.theme().active_ui_text_color()),
            MainAxisSize::Max,
            MainAxisAlignment::Center,
            vec2f(16., 16.),
        )
        .with_inner_padding(4.);

        let button = appearance
            .ui_builder()
            .button(
                ButtonVariant::Outlined,
                self.mouse_state_handles.copy_button_mouse_state.clone(),
            )
            .with_style(
                self.button_style_overrides(appearance)
                    .set_margin(Coords {
                        left: 4.,
                        ..Default::default()
                    })
                    .set_width(NEW_COPY_BUTTON_WIDTH),
            )
            .with_text_and_icon_label(text_and_icon)
            .build();

        button
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action))
            .finish()
    }

    fn render_footer(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        SavePosition::new(
            Container::new(match &self.request_state {
                ShareRequestState::Succeeded { link, .. } => {
                    self.render_success_footer(appearance, link.as_str(), app)
                }
                _ => Align::new(self.render_create_block_buttons_row(appearance))
                    .right()
                    .finish(),
            })
            .with_margin_top(INNER_MARGIN)
            .finish(),
            "share_modal:footer",
        )
        .finish()
    }

    fn render_modal(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let theme = appearance.theme();
        let ui_builder = appearance.ui_builder();
        let link_generated = matches!(self.request_state, ShareRequestState::Succeeded { .. });
        let mut column = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        // If we're using the combined prompt/command grid, then "show prompt" should only be configurable if using Warp prompt!
        // Otherwise, we MUST always render the prompt alongside the command (since they're in the same combined grid for PS1).
        let show_prompt_configurable = self
            .model
            .as_ref()
            .map(|model| model.lock())
            .and_then(|model| {
                self.selected_block.and_then(|index| {
                    model
                        .block_list()
                        .block_at(index)
                        .map(|block| !block.render_prompt_on_same_line())
                })
            })
            // Fallback to false (not allowing prompt configuration), if we cannot determine the HonorPS1 status.
            .unwrap_or(false);

        let modal_title_or_block_title = Text::new_inline(
            if link_generated {
                self.block_title_editor.as_ref(app).buffer_text(app)
            } else {
                "Share block".to_string()
            },
            appearance.ui_font_family(),
            24.,
        )
        .with_style(Properties {
            style: Style::Normal,
            weight: Weight::Medium,
        })
        .with_color(theme.active_ui_text_color().into())
        .finish();
        let header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(1., Align::new(modal_title_or_block_title).left().finish())
                    .finish(),
            )
            .with_child(self.render_manage_permalinks_button(appearance, app))
            .with_child(self.render_close_modal_button(appearance))
            .finish();
        column.add_child(
            Container::new(header)
                .with_margin_bottom(INNER_MARGIN)
                .finish(),
        );

        if !link_generated {
            let block_title_editor = Dismiss::new(
                appearance
                    .ui_builder()
                    .text_input(self.block_title_editor.clone())
                    .with_style(UiComponentStyles {
                        width: Some(BLOCK_TITLE_INPUT_WIDTH),
                        padding: Some(Coords {
                            top: 10.,
                            bottom: 10.,
                            left: 16.,
                            right: 12.,
                        }),
                        background: Some(appearance.theme().surface_2().into()),
                        font_size: Some(14.),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish();
            column.add_child(
                Container::new(block_title_editor)
                    .with_margin_bottom(INNER_MARGIN)
                    .finish(),
            );

            let embed_display_radio_buttons = appearance
                .ui_builder()
                .radio_buttons(
                    self.embed_display_handles
                        .embed_display_mouse_states
                        .clone(),
                    self.embed_display_options
                        .iter()
                        .map(|x| RadioButtonItem::text(x.0.clone()))
                        .collect(),
                    self.embed_display_handles
                        .embed_display_state_handle
                        .clone(),
                    Some(0),
                    appearance.ui_font_size(),
                    RadioButtonLayout::Row,
                )
                .build()
                .finish();
            let show_prompt_checkbox = appearance
                .ui_builder()
                .checkbox(
                    self.mouse_state_handles.show_prompt_mouse_state.clone(),
                    Some(CHECKBOX_SIZE),
                )
                .check(self.show_prompt)
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(ShareBlockModalAction::ToggleShowPrompt)
                })
                .finish();
            let show_prompt_description = appearance
                .ui_builder()
                .span(crate::i18n::tr_static(app, "Show prompt").to_string())
                .build()
                .with_margin_left(2.)
                .finish();

            let mut configuration_row =
                Flex::row().with_children([Container::new(embed_display_radio_buttons).finish()]);

            if show_prompt_configurable {
                configuration_row.add_children([
                    self.render_vertical_separator(appearance),
                    Container::new(show_prompt_checkbox)
                        .with_margin_left(5.)
                        .finish(),
                    Container::new(show_prompt_description).finish(),
                ]);
            }

            column.add_child(
                Container::new(
                    configuration_row
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .finish(),
                )
                .with_margin_bottom(INNER_MARGIN)
                .finish(),
            );
        }

        let enforce_minimum_contrast = *FontSettings::as_ref(app).enforce_minimum_contrast;
        let single_block = match &self.model {
            Some(model) => {
                let cell_height = model.lock().block_list().size().cell_height_px();
                let obfuscate_secrets = self.obfuscate_secrets;
                let mut single_block = SingleBlock::new(
                    model.clone(),
                    theme.clone(),
                    appearance.monospace_font_family(),
                    appearance.monospace_font_size(),
                    ui_builder.line_height_ratio(),
                    self.selected_block,
                    self.scroll_top,
                    cell_height,
                    enforce_minimum_contrast,
                    obfuscate_secrets,
                    self.current_display_setting(),
                    self.show_prompt,
                );
                if should_use_ligature_rendering(app) {
                    single_block = single_block.with_ligature_rendering();
                }

                Scrollable::vertical(
                    self.scroll_state.clone(),
                    single_block.finish_scrollable(),
                    ScrollbarWidth::Auto,
                    theme.disabled_text_color(theme.background()).into(),
                    theme.main_text_color(theme.background()).into(),
                    theme.background().into(),
                )
                .with_overlayed_scrollbar()
                .finish()
            }
            None => {
                log::warn!("Tried to render share modal without a model");
                Empty::new().finish()
            }
        };
        column.add_child(Shrinkable::new(1., single_block).finish());

        if !link_generated {
            let redact_secrets_checkbox =
                if PrivacySettings::as_ref(app).is_enterprise_secret_redaction_enabled() {
                    // Force check the checkbox if enterprise secret redaction is enabled.
                    appearance
                        .ui_builder()
                        .checkbox(
                            self.mouse_state_handles.redact_secrets_mouse_state.clone(),
                            Some(CHECKBOX_SIZE),
                        )
                        .check(true)
                        .build()
                        .disable()
                        .finish()
                } else {
                    appearance
                        .ui_builder()
                        .checkbox(
                            self.mouse_state_handles.redact_secrets_mouse_state.clone(),
                            Some(CHECKBOX_SIZE),
                        )
                        .check(self.obfuscate_secrets.is_visually_obfuscated())
                        .build()
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(ShareBlockModalAction::ToggleObfuscateSecrets)
                        })
                        .finish()
                };

            let redact_secrets_description = appearance
                .ui_builder()
                .span(
                    crate::i18n::tr_static(
                        app,
                        "Redact secrets (API keys, passwords, IP addresses, PII etc.)",
                    )
                    .to_string(),
                )
                .build()
                .with_margin_left(4.)
                .finish();
            column.add_child(
                Container::new(
                    Flex::row()
                        .with_children([
                            Container::new(redact_secrets_checkbox).finish(),
                            Container::new(redact_secrets_description).finish(),
                        ])
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .finish(),
                )
                .with_margin_top(24.)
                .finish(),
            );
        }
        column.finish()
    }
}

impl Entity for ShareBlockModal {
    type Event = ShareBlockModalEvent;
}

impl TypedActionView for ShareBlockModal {
    type Action = ShareBlockModalAction;

    fn handle_action(&mut self, action: &ShareBlockModalAction, ctx: &mut ViewContext<Self>) {
        use ShareBlockModalAction::*;

        match action {
            Close => self.close(ctx),
            GenerateSharedBlock(share_type) => self.save_block(*share_type, ctx),
            ToggleShowPrompt => self.toggle_show_prompt(ctx),
            Scroll(top) => self.scroll(*top, ctx),
            CopyLink => self.copy(ctx),
            CopyEmbed => self.copy_embed(ctx),
            ToggleObfuscateSecrets => self.toggle_obfuscate_secrets(ctx),
        }
    }
}

impl View for ShareBlockModal {
    fn ui_name() -> &'static str {
        "ShareBlockModal"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.block_title_editor);
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut stack = Stack::new();
        let appearance = Appearance::as_ref(app);

        let modal = self.render_modal(appearance, app);
        let footer = self.render_footer(appearance, app);

        stack.add_child(
            Align::new(
                Dismiss::new(
                    ConstrainedBox::new(
                        Container::new(
                            Flex::column()
                                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                                .with_child(Shrinkable::new(1., modal).finish())
                                .with_child(footer)
                                .finish(),
                        )
                        .with_background(appearance.theme().surface_2())
                        .with_uniform_padding(PADDING)
                        .with_uniform_margin(35.)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                        .with_border(Border::all(1.).with_border_fill(appearance.theme().outline()))
                        .finish(),
                    )
                    .with_width(MODAL_WIDTH)
                    .with_max_height(627.)
                    .finish(),
                )
                .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(ShareBlockModalAction::Close))
                .finish(),
            )
            .finish(),
        );

        Container::new(stack.finish())
            .with_background_color(Fill::blur().into())
            .finish()
    }
}

fn should_send_title_gen_request(ctx: &ViewContext<ShareBlockModal>) -> bool {
    FeatureFlag::SharedBlockTitleGeneration.is_enabled()
        && AISettings::as_ref(ctx).is_shared_block_title_generation_enabled(ctx)
        && UserWorkspaces::as_ref(ctx).ai_allowed_for_current_team()
}

struct SingleBlock {
    terminal_model: Arc<FairMutex<TerminalModel>>,
    theme: WarpTheme,
    font_family: FamilyId,
    font_size: f32,
    line_height_ratio: f32,
    selected_block: Option<BlockIndex>,
    size: Option<Vector2F>,
    origin: Option<Point>,
    scroll_top: Lines,
    scroll_max: Lines,
    cell_height: Pixels,
    visible_lines: Option<Lines>,
    total_lines: Option<Lines>,
    enforce_minimum_contrast: EnforceMinimumContrast,
    obfuscate_secrets: ObfuscateSecrets,
    use_ligature_rendering: bool,
    display_setting: DisplaySetting,
    show_prompt: bool,
    native_prompt_text: Option<Text>,
}

impl SingleBlock {
    #[allow(clippy::too_many_arguments)]
    fn new(
        terminal_model: Arc<FairMutex<TerminalModel>>,
        theme: WarpTheme,
        font_family: FamilyId,
        font_size: f32,
        line_height_ratio: f32,
        selected_block: Option<BlockIndex>,
        scroll_top: Lines,
        cell_height: Pixels,
        enforce_minimum_contrast: EnforceMinimumContrast,
        obfuscate_secrets: ObfuscateSecrets,
        display_setting: DisplaySetting,
        show_prompt: bool,
    ) -> Self {
        Self {
            terminal_model,
            theme,
            font_family,
            font_size,
            line_height_ratio,
            selected_block,
            size: None,
            origin: None,
            scroll_top,
            scroll_max: Default::default(),
            cell_height,
            visible_lines: None,
            total_lines: None,
            enforce_minimum_contrast,
            obfuscate_secrets,
            use_ligature_rendering: false,
            display_setting,
            show_prompt,
            native_prompt_text: None,
        }
    }

    fn with_ligature_rendering(mut self) -> Self {
        self.use_ligature_rendering = true;
        self
    }

    /// Scroll by a line amount
    fn scroll_by_lines(&mut self, delta: Lines, ctx: &mut EventContext) {
        let scroll_top = (self.scroll_top - delta)
            .max(Lines::zero())
            .min(self.scroll_max);
        ctx.dispatch_typed_action(ShareBlockModalAction::Scroll(scroll_top));
    }

    /// Scroll a precise pixel amount
    fn scroll_by_pixels(&mut self, delta: Pixels, ctx: &mut EventContext) {
        self.scroll_by_lines(delta.to_lines(self.cell_height), ctx);
    }

    fn rect(&self) -> Option<RectF> {
        try_rect_with_z(self.origin, self.size)
    }

    fn paint_single_block(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        ctx.scene.start_layer(ClipBounds::BoundedBy(RectF::new(
            origin,
            self.size()
                .expect("SingleBlock size should be set in layout"),
        )));
        let size = self.size.expect("SingleBlock size should be set in layout");
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(origin, size))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_background(self.theme.background());

        let model = self.terminal_model.lock();
        let size_info = model.block_list().size();
        let colors = model.colors();
        let selected_block = self
            .selected_block
            .expect("SingleBlock should not be trying to paint with no block selected");

        // TODO(alokedesai): Make this more resilient to the block not existing in the model.
        let block = model
            .block_list()
            .block_at(selected_block)
            .expect("block should exist in model");
        let override_colors = model.override_colors();

        let cell_size = Vector2F::new(
            size_info.cell_width_px().as_f32(),
            size_info.cell_height_px().as_f32(),
        );

        let mut glyphs = CellGlyphCache::default();

        let padding_top_rendered = (block.padding_top() - self.scroll_top)
            .max(Lines::zero())
            .min(block.padding_top());

        // We want to iterate through each the components of the block
        // and keep track of the pixel location we're currently painting and
        // where we are in the block's grid.
        //
        // Grid origin represents the current Pixel location we're painting
        // Logical location represents the location we are in the grid.
        let mut grid_origin =
            origin + vec2f(0., cell_size.y() * padding_top_rendered.as_f64() as f32);
        let mut logical_location = block.padding_top();

        let mut prompt_rows_rendered = Lines::zero();
        let mut padding_between_prompt_and_cmd_rendered = Lines::zero();

        if self.show_prompt {
            // If we're rendering Warp prompt (above the command).
            if !block.honor_ps1() {
                if let Some(native_prompt_text) = self.native_prompt_text.as_mut() {
                    if self.scroll_top - padding_top_rendered <= Lines::zero() {
                        native_prompt_text.paint(
                            grid_origin + vec2f(size_info.padding_x_px().as_f32(), 0.),
                            ctx,
                            app,
                        );
                    }
                }
                // The height of the native prompt will always be 1.
                let hidden_prompt_rows_above =
                    (self.scroll_top - padding_top_rendered).clamp(0.into_lines(), 1.into_lines());
                logical_location += 1.into_lines() + block.command_padding_top();
                prompt_rows_rendered = 1.into_lines() - hidden_prompt_rows_above.max(Lines::zero());
            }
            // If PS1 is on and we're using combined grid, we do NOT need padding between prompt and command, since
            // they are part of the same grid! Otherwise, we need to add padding here.
            if !block.render_prompt_on_same_line() {
                padding_between_prompt_and_cmd_rendered = (logical_location - self.scroll_top)
                    .max(Lines::zero())
                    .min(block.command_padding_top());
                grid_origin += vec2f(
                    0.,
                    cell_size.y()
                        * (padding_between_prompt_and_cmd_rendered + prompt_rows_rendered).as_f64()
                            as f32,
                );
            }
        }

        let mut cmd_rows_rendered = Lines::zero();
        let mut padding_middle_rendered = Lines::zero();

        if matches!(
            self.display_setting,
            DisplaySetting::Command | DisplaySetting::CommandAndOutput
        ) {
            let command_grid = block.prompt_and_command_grid().grid_handler();

            let command_number_of_rows = block.prompt_and_command_number_of_rows().into_lines();

            let start_row = (self.scroll_top - logical_location).max(Lines::zero());
            // Note that in the case of the combined prompt/command grid, the prompt_rows_rendered will be 0 (we render
            // the PS1 inside of the combined grid, and do NOT render_grid above). padding_between_prompt_and_cmd_rendered
            // will also be 0.
            let end_row = (start_row
                + self
                    .visible_lines
                    .expect("visible_lines should be set in layout")
                - padding_top_rendered
                - prompt_rows_rendered
                - padding_between_prompt_and_cmd_rendered)
                .min(command_number_of_rows);

            let hidden_cmd_rows_above = (self.scroll_top - padding_top_rendered)
                .max(Lines::zero())
                .min(command_number_of_rows);

            grid_renderer::render_grid(
                command_grid,
                start_row.as_f64() as usize,
                end_row.as_f64() as usize,
                &colors,
                &override_colors,
                &self.theme,
                Properties::default(),
                self.font_family,
                self.font_size,
                self.line_height_ratio,
                cell_size,
                size_info.padding_x_px(),
                grid_origin - vec2f(0., hidden_cmd_rows_above.as_f64() as f32 * cell_size.y()),
                &mut glyphs,
                255,
                None, /* highlighted url */
                None,
                None::<std::iter::Empty<&RangeInclusive<super::model::index::Point>>>,
                None, /* focused match */
                self.enforce_minimum_contrast,
                self.obfuscate_secrets,
                None,
                self.use_ligature_rendering,
                None,
                RespectDisplayedOutput::No,
                &model.image_id_to_metadata,
                None,
                false, // hide_cursor_cell
                ctx,
                app,
            );
            logical_location += command_number_of_rows + block.padding_middle();
            cmd_rows_rendered = (command_number_of_rows - hidden_cmd_rows_above).max(Lines::zero());
            padding_middle_rendered = (logical_location - self.scroll_top)
                .max(Lines::zero())
                .min(block.padding_middle());
            grid_origin += vec2f(
                0.,
                cell_size.y() * (padding_middle_rendered + cmd_rows_rendered).as_f64() as f32,
            );
        }

        let start_row = (self.scroll_top - logical_location).max(Lines::zero());
        let end_row = (start_row
            + self
                .visible_lines
                .expect("visible_lines should be set in layout")
            - padding_top_rendered
            - cmd_rows_rendered
            - padding_middle_rendered)
            .min(block.output_grid().len().into_lines());

        if matches!(
            self.display_setting,
            DisplaySetting::Output | DisplaySetting::CommandAndOutput
        ) {
            grid_renderer::render_grid(
                block.output_grid().grid_handler(),
                start_row.as_f64() as usize,
                end_row.as_f64() as usize,
                &colors,
                &override_colors,
                &self.theme,
                Properties::default(),
                self.font_family,
                self.font_size,
                self.line_height_ratio,
                cell_size,
                size_info.padding_x_px(),
                grid_origin - vec2f(0., start_row.as_f64() as f32 * cell_size.y()),
                &mut glyphs,
                255,
                None, /* highlighted url */
                None,
                None::<std::iter::Empty<&RangeInclusive<super::model::index::Point>>>,
                None, /* focused match */
                self.enforce_minimum_contrast,
                self.obfuscate_secrets,
                None,
                self.use_ligature_rendering,
                None,
                RespectDisplayedOutput::No,
                &model.image_id_to_metadata,
                None,
                false, // hide_cursor_cell
                ctx,
                app,
            );
        }
        ctx.scene.stop_layer();
    }
}

impl Element for SingleBlock {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let model = self.terminal_model.lock();
        let selected_block = self
            .selected_block
            .expect("SingleBlock should not be trying to layout with no block selected");

        // TODO(alokedesai): Make this more resilient to the block not existing in the model.
        let block = model
            .block_list()
            .block_at(selected_block)
            .expect("Block should exist in model");
        let size_info = model.block_list().size();

        if self.show_prompt && !block.honor_ps1() {
            let mut native_prompt_text = Text::new_inline(
                ServerBlock::native_prompt_for_server(block),
                self.font_family,
                self.font_size,
            );
            let appearance = Appearance::as_ref(app);
            let theme = appearance.theme();
            native_prompt_text = native_prompt_text.with_color(
                appearance
                    .theme()
                    .sub_text_color(theme.background())
                    .into_solid(),
            );
            native_prompt_text.layout(constraint, ctx, app);
            self.native_prompt_text = Some(native_prompt_text);
        }

        let block_height =
            block.full_content_height_with_display_options(&self.display_setting, self.show_prompt);

        let height = (block_height.to_pixels(size_info.cell_height_px()))
            .min(constraint.max.y().into_pixels())
            .max(constraint.min.y().into_pixels());

        let visible_lines = height.to_lines(size_info.cell_height_px());

        let size = Vector2F::new(constraint.min.x(), height.as_f32());

        self.scroll_max = (block_height.into_lines() - visible_lines).max(Lines::zero());
        self.visible_lines = Some(visible_lines);
        self.total_lines = Some(block_height.max(Lines::zero()));
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.paint_single_block(origin, ctx, app);
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        if let Some(Event::ScrollWheel {
            position,
            delta,
            precise,
            modifiers: ModifiersState { ctrl: false, .. },
        }) = event.at_z_index(self.z_index().unwrap(), ctx)
        {
            if self.rect().unwrap().contains_point(*position) {
                if *precise {
                    self.scroll_by_pixels(delta.y().into_pixels(), ctx);
                } else {
                    self.scroll_by_lines(delta.y().into_lines(), ctx);
                }
                return true;
            }
        }
        false
    }
}

impl ScrollableElement for SingleBlock {
    fn scroll_data(&self, _app: &AppContext) -> Option<ScrollData> {
        Some(ScrollData {
            scroll_start: self.scroll_top.to_pixels(self.cell_height),
            visible_px: self.visible_lines?.to_pixels(self.cell_height),
            total_size: self.total_lines?.to_pixels(self.cell_height),
        })
    }

    fn scroll(&mut self, delta: Pixels, ctx: &mut EventContext) {
        self.scroll_by_pixels(delta, ctx);
    }
}
