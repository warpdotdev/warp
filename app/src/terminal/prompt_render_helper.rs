use crate::ai::blocklist::BlocklistAIInputModel;
use crate::context_chips::display::PromptDisplay;
use crate::context_chips::spacing;
use crate::features::FeatureFlag;
use crate::settings::InputSettings;
use crate::terminal::grid_size_util::grid_compute_baseline_position_fn;
use crate::terminal::ligature_settings::should_use_ligature_rendering;
use crate::terminal::view::TerminalAction;
use crate::themes::theme::PromptColors;
use crate::{appearance::Appearance, terminal::model::blockgrid::BlockGrid};
use settings::Setting as _;

use std::fmt;
use std::num::NonZeroUsize;
use warp_core::semantic_selection::SemanticSelection;
use warpui::elements::{DispatchEventResult, SelectionHandle};
use warpui::ModelAsRef;
use warpui::{
    elements::{Container, Element, EventHandler, SavePosition, SelectableArea, Text},
    fonts::{Properties, Weight},
    presenter::ChildView,
    AppContext, EntityId, ModelHandle, SingletonEntity, ViewHandle,
};

use super::input::InputRenderStateModel;
use super::model::block::Block;
use super::shell::ShellType;
use crate::settings::FontSettings;
use crate::terminal::input::get_input_box_top_border_width;

use super::model::blocks::CachedPromptData;
use super::safe_mode_settings::get_secret_obfuscation_mode;
use super::session_settings::SessionSettings;
use super::settings::TerminalSettings;
use super::{prompt, SizeInfo, TerminalModel};

use crate::terminal::blockgrid_element::BlockGridElement;
use crate::terminal::model::session::Sessions;
use crate::terminal::view::PADDING_LEFT as TERMINAL_VIEW_PADDING_LEFT;

use crate::terminal::model::ObfuscateSecrets;
use warpui::units::Pixels;

/// How long we're willing to wait after precmd for a marker-based prompt to appear before we
/// display an empty prompt grid in the input.
///
/// During this grace period, we will display the same prompt that was displayed in the input when
/// the user submitted the newly-completed command. We use a higher duration for PowerShell and
/// MSYS2 b/c it's significantly slower than bash/fish/zsh on Unix.
fn prompt_marker_grace_period(shell_type: Option<ShellType>, is_msys2: bool) -> chrono::Duration {
    static POSIX_SHELL_PROMPT_MARKER_GRACE_PERIOD: chrono::Duration =
        chrono::Duration::milliseconds(100);
    static POWERSHELL_PROMPT_MARKER_GRACE_PERIOD: chrono::Duration = chrono::Duration::seconds(1);
    static MSYS2_PROMPT_MARKER_GRACE_PERIOD: chrono::Duration = chrono::Duration::seconds(1);
    match (shell_type, is_msys2) {
        (Some(ShellType::PowerShell), _) => POWERSHELL_PROMPT_MARKER_GRACE_PERIOD,
        (_, true) => MSYS2_PROMPT_MARKER_GRACE_PERIOD,
        _ => POSIX_SHELL_PROMPT_MARKER_GRACE_PERIOD,
    }
}

pub const LPROMPT_RIGHT_PADDING_SAME_LINE_PROMPT: f32 = 4.;

pub fn should_render_ps1_prompt(terminal_model: &TerminalModel, app: &AppContext) -> bool {
    let is_classic_input_enabled = InputSettings::as_ref(app).is_classic_input_enabled(app);
    let session_settings = SessionSettings::as_ref(app);

    // In the context of session sharing, these values may differ from the local settings i.e.
    // if the sharer is using PS1 and the viewer is not (using Warp prompt in non-SLP mode).
    // In this case, we still want to render the prompt on the same line (PS1 should ALWAYS be
    // rendered on the same line).
    // Note that the product behavior for session sharing is normally to respect the local settings
    // for prompt cosmetics, but this is an exception!
    let active_block = terminal_model.block_list().active_block();
    let active_block_honor_ps1 = active_block.honor_ps1();

    is_classic_input_enabled && (*session_settings.honor_ps1.value() || active_block_honor_ps1)
}

/// Returns whether the prompt should be rendered on the same line as the input editor's contents.
pub fn should_render_prompt_on_same_line(
    is_universal_developer_input: bool,
    terminal_model: &TerminalModel,
    app: &AppContext,
) -> bool {
    // We render the prompt on the same line, in the input editor, if:
    // 1. The user is using a custom prompt (PS1)
    // 2. The user has the same line prompt setting enabled for their Warp prompt.

    // If universal developer input is enabled, ignore PS1 rendering logic
    if is_universal_developer_input {
        return false;
    }

    let should_render_ps1 = should_render_ps1_prompt(terminal_model, app);

    if FeatureFlag::AgentView.is_enabled() {
        should_render_ps1
    } else {
        let session_settings = SessionSettings::as_ref(app);
        should_render_ps1
            || session_settings
                .saved_prompt
                .value()
                .same_line_prompt_enabled()
    }
}

/// Returns `true` if the shell or AI prompt should be rendered using the editors
/// `EditorDecoratorElements` API.
///
/// The AI prompt is unconditionally rendered above the input.
pub fn should_render_prompt_using_editor_decorator_elements(
    is_universal_developer_input: bool,
    ai_input_model: &ModelHandle<BlocklistAIInputModel>,
    model: &TerminalModel,
    app: &AppContext,
) -> bool {
    should_render_prompt_on_same_line(is_universal_developer_input, model, app)
        && (!ai_input_model.as_ref(app).is_ai_input_enabled()
            || FeatureFlag::AgentView.is_enabled())
}

pub(in crate::terminal) struct PromptAndPadding {
    pub element: PromptAndPaddingElement,
    pub padding_left: f32,
    pub padding_right: f32,
}

pub(in crate::terminal) enum PromptAndPaddingElement {
    // This is boxed because `Text` is large, and without boxing, it
    // bloats the size of the `PromptAndPaddingElement` enum.
    Text(Box<Text>),
    // This is boxed because `BlockGridElement` is large, and without boxing, it
    // bloats the size of the `PromptAndPaddingElement` enum.
    BlockGrid(Box<BlockGridElement>),
    ContextChips(ViewHandle<PromptDisplay>),
}

impl PromptAndPaddingElement {
    pub(in crate::terminal) fn text(&self, ctx: &AppContext) -> String {
        match self {
            Self::Text(text_element) => text_element.text().to_owned(),
            Self::BlockGrid(block_grid_element) => block_grid_element.text(),
            Self::ContextChips(view) => view.as_ref(ctx).text(ctx),
        }
    }

    pub(in crate::terminal) fn render(self) -> Box<dyn Element> {
        match self {
            Self::Text(text_element) => text_element.finish(),
            Self::BlockGrid(block_grid_element) => block_grid_element.finish(),
            Self::ContextChips(view) => ChildView::new(&view).finish(),
        }
    }
}

/// Struct used for storing prompt elements in the default, non same-line
/// prompt case.
pub(super) struct PromptElements {
    pub(super) lprompt: Option<Box<dyn Element>>,
    pub(super) rprompt: Option<Box<dyn Element>>,
}

/// Struct used for storing prompt elements when same-line prompt is toggled on.
pub(super) struct SameLinePromptElements {
    // Top n-1 lines of lprompt.
    pub(super) lprompt_top: Option<Box<dyn Element>>,
    // Bottom (nth) line of lprompt.
    pub(super) lprompt_bottom: Option<Box<dyn Element>>,
    pub(super) rprompt: Option<Box<dyn Element>>,
}
#[derive(Clone)]
pub struct PromptRenderHelper {
    sessions: ModelHandle<Sessions>,
    prompt_parent_view_id: EntityId,

    prompt_view: ViewHandle<PromptDisplay>,
    prompt_selection_state_handle: SelectionHandle,
    input_render_state_model_handle: ModelHandle<InputRenderStateModel>,

    ai_input_model: ModelHandle<BlocklistAIInputModel>,
}

#[derive(Clone, Copy)]
enum PromptSide {
    Left,
    Right,
}

impl fmt::Display for PromptSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PromptSide::Left => f.write_str("prompt_area"),
            PromptSide::Right => f.write_str("rprompt_area"),
        }
    }
}

impl PromptRenderHelper {
    pub(in crate::terminal) fn new(
        sessions: ModelHandle<Sessions>,
        prompt_view_handle: ViewHandle<PromptDisplay>,
        prompt_selection_state_handle: SelectionHandle,
        parent_view_id: EntityId,
        input_render_state_model_handle: ModelHandle<InputRenderStateModel>,
        ai_input_model: ModelHandle<BlocklistAIInputModel>,
    ) -> Self {
        Self {
            sessions,
            prompt_view: prompt_view_handle,
            prompt_selection_state_handle,
            prompt_parent_view_id: parent_view_id,
            input_render_state_model_handle,
            ai_input_model,
        }
    }

    pub fn prompt_view(&self) -> &ViewHandle<PromptDisplay> {
        &self.prompt_view
    }

    /// Returns the block from which we should be retrieving prompt-related data.
    pub(in crate::terminal) fn prompt_block<'a>(
        &self,
        model: &'a TerminalModel,
    ) -> Option<&'a Block> {
        model.prompt_block()
    }

    pub fn prompt_working_dir(&self, model: &TerminalModel, sessions: &Sessions) -> String {
        let block = self.prompt_block(model);
        let home_dir = block.and_then(|block| prompt::home_dir_for_block(block, sessions));
        if model.block_list().is_bootstrapped() {
            prompt::display_path_string(block.and_then(|b| b.pwd()), home_dir.as_deref())
        // If the block list is not bootstrapped and there are SSH sessions in the current
        // terminal session, we should display the "Starting shell..." message when
        // fetching the login_shell information.
        } else {
            self.bootstrapping_shell_message(model, sessions)
        }
    }

    pub fn has_open_chip_menu(&self, app: &AppContext) -> bool {
        self.prompt_view.as_ref(app).has_open_chip_menu(app)
    }

    fn bootstrapping_shell_message(&self, model: &TerminalModel, sessions: &Sessions) -> String {
        use crate::terminal::event::RemoteServerSetupState;

        // If a remote server setup is in progress for the pending session,
        // show a stage-specific message instead of the generic "Starting shell...".
        if let Some(pending_session_id) = model.pending_session_id() {
            if let Some(state) = sessions.remote_server_setup_state(pending_session_id) {
                return match state {
                    RemoteServerSetupState::Checking => "Starting shell...".to_string(),
                    RemoteServerSetupState::Installing {
                        progress_percent: Some(p),
                    } => format!("Installing Warp SSH Extension... ({p}%)"),
                    RemoteServerSetupState::Installing {
                        progress_percent: None,
                    } => "Installing Warp SSH Extension...".to_string(),
                    RemoteServerSetupState::Updating => {
                        "Updating Warp SSH Extension...".to_string()
                    }
                    RemoteServerSetupState::Initializing => "Initializing...".to_string(),
                    RemoteServerSetupState::Ready => "Starting shell...".to_string(),
                    // Failed and Unsupported both fall back to the legacy SSH
                    // flow, so we render the same generic prompt as a normal
                    // SSH session that doesn't have the remote-server extension.
                    RemoteServerSetupState::Failed { .. }
                    | RemoteServerSetupState::Unsupported { .. } => "Starting shell...".to_string(),
                };
            }
        }

        if !sessions.is_empty() {
            "Starting shell...".to_string()
        } else {
            format!("Starting {}...", model.shell_launch_state().display_name())
        }
    }

    /// The bootstrapping shell message, wrapped into a [`Text`] element.
    fn bootstrapping_shell_text(
        &self,
        model: &TerminalModel,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Text {
        let prompt_colors: PromptColors = appearance.theme().clone().into();
        let prompt_message = self.bootstrapping_shell_message(model, self.sessions.as_ref(app));
        Text::new_inline(
            prompt_message,
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(prompt_colors.input_prompt_pwd)
        .with_style(Properties::default().weight(Weight::Bold))
        .with_line_height_ratio(appearance.line_height_ratio())
        .with_compute_baseline_position_fn(grid_compute_baseline_position_fn(
            appearance.monospace_font_family(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn prompt_block_grid_to_prompt_and_padding(
        block_grid: BlockGrid,
        padding_left: f32,
        padding_right: f32,
        appearance: &Appearance,
        obfuscate_secrets: ObfuscateSecrets,
        size_info: SizeInfo,
        app: &AppContext,
    ) -> PromptAndPadding {
        let enforce_minimum_contrast = *FontSettings::as_ref(app).enforce_minimum_contrast;
        let mut block_grid_element = BlockGridElement::new(
            &block_grid,
            appearance,
            enforce_minimum_contrast,
            obfuscate_secrets,
            size_info,
        );
        if should_use_ligature_rendering(app) {
            block_grid_element = block_grid_element.with_ligature_rendering();
        }
        PromptAndPadding {
            element: PromptAndPaddingElement::BlockGrid(Box::new(block_grid_element)),
            padding_left,
            padding_right,
        }
    }

    fn split_prompt_grids(prompt_grid: &BlockGrid) -> (Option<BlockGrid>, Option<BlockGrid>) {
        // We check the ABSOLUTE rows to cursor here, to discern whether it is a single line prompt or multiple lines.
        let rows_to_cursor = prompt_grid.grid_handler().rows_to_cursor();

        if let Some(rows_to_cursor) = NonZeroUsize::new(rows_to_cursor) {
            let (prompt_grid, mut bottom_prompt_grid) = prompt_grid.split(rows_to_cursor);
            if let Some(bottom_prompt_grid) = &mut bottom_prompt_grid {
                bottom_prompt_grid
                    .grid_handler_mut()
                    .truncate_to_cursor_rows();
                bottom_prompt_grid
                    .grid_handler_mut()
                    .truncate_to_cursor_cols();
            }

            (Some(prompt_grid), bottom_prompt_grid)
        } else {
            let mut bottom_prompt_grid = prompt_grid.clone();
            bottom_prompt_grid
                .grid_handler_mut()
                .truncate_to_cursor_rows();
            bottom_prompt_grid
                .grid_handler_mut()
                .truncate_to_cursor_cols();
            (None, Some(bottom_prompt_grid))
        }
    }

    /// Computes the correct prompt layout, based on whether same line prompt is enabled or not,
    /// along with the number of lines in the prompt.
    /// Returns: (lprompt_top, lprompt_bottom, rprompt)
    fn compute_prompt_layout(
        same_line_prompt_enabled: bool,
        prompt_grid: &BlockGrid,
        rprompt_grid: &BlockGrid,
    ) -> (Option<BlockGrid>, Option<BlockGrid>, BlockGrid) {
        if same_line_prompt_enabled {
            let (lprompt_top, lprompt_bottom) = Self::split_prompt_grids(prompt_grid);
            let mut truncated_rprompt_grid = rprompt_grid.clone();
            truncated_rprompt_grid
                .grid_handler_mut()
                .truncate_to_cursor_rows();
            truncated_rprompt_grid
                .grid_handler_mut()
                .truncate_to_cursor_cols();
            (lprompt_top, lprompt_bottom, truncated_rprompt_grid)
        } else {
            (Some(prompt_grid.clone()), None, rprompt_grid.clone())
        }
    }

    /// Core logic for rendering the prompt.
    ///
    /// This method reads the prompt data from the TerminalModel and converts it to
    /// the appropriate renderable PromptAndPadding structs.
    ///
    /// # Returns
    ///
    /// In the case of default (non same-line prompt):
    ///     Returns lprompt, None and rprompt
    /// In the case of same-line prompt with 1 line prompt:
    ///     Returns None, lprompt and rprompt
    /// In the case of same-line prompt with multiple-line prompt:
    ///     Returns lprompt top (n-1 lines), lprompt bottom (nth line) and rprompt
    ///
    /// Note that rprompt's value is returned as None in all above cases,
    /// if should_display_rprompt is false. There are also a variety of bootstrapping/loading
    /// cases which are simply 1 line prompt cases (without an rprompt).
    ///
    pub(in crate::terminal) fn render_prompt(
        &self,
        model: &TerminalModel,
        appearance: &Appearance,
        app: &AppContext,
    ) -> (
        Option<PromptAndPadding>,
        Option<PromptAndPadding>,
        Option<PromptAndPadding>,
    ) {
        let active_block = model.block_list().active_block();
        let is_universal_input =
            InputSettings::as_ref(app).is_universal_developer_input_enabled(app);
        let render_prompt_on_same_line =
            should_render_prompt_on_same_line(is_universal_input, model, app);
        let padding_right = if should_render_prompt_using_editor_decorator_elements(
            is_universal_input,
            &self.ai_input_model,
            model,
            app,
        ) {
            LPROMPT_RIGHT_PADDING_SAME_LINE_PROMPT
        } else {
            *TERMINAL_VIEW_PADDING_LEFT
        };
        // If the active block hasn't received the precmd message, we're waiting for the next
        // prompt. However, we don't want the UI to flicker so we show the previous prompt
        // until the user changes the editor.
        if !active_block.has_received_precmd()
            && app
                .model(&self.input_render_state_model_handle)
                .editor_modified_since_block_finished()
        {
            let prompt = PromptAndPadding {
                element: PromptAndPaddingElement::Text(Box::new(
                    Text::new_inline(
                        "Loading prompt...",
                        appearance.monospace_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(appearance.theme().disabled_ui_text_color().into())
                    .with_line_height_ratio(appearance.line_height_ratio())
                    .with_compute_baseline_position_fn(
                        grid_compute_baseline_position_fn(appearance.monospace_font_family()),
                    ),
                )),
                padding_left: 0.,
                padding_right,
            };
            if render_prompt_on_same_line {
                (None, Some(prompt), None)
            } else {
                (Some(prompt), None, None)
            }
        } else if active_block.honor_ps1()
            && model.block_list().is_bootstrapped()
            && !is_universal_input
        {
            // Only render PS1 directly if the shell is bootstrapped and universal developer input is disabled.
            let prompt_block = self.prompt_block(model).unwrap_or(active_block);
            let shell_type = active_block.shell_host().map(|shell| shell.shell_type);
            let is_msys2 = active_block
                .session_id()
                .and_then(|session_id| self.sessions.as_ref(app).get(session_id))
                .map(|session| session.is_msys2())
                .unwrap_or_default();
            let (lprompt_grid_top, lprompt_grid_bottom, rprompt_grid) =
                match &model.block_list().cached_prompt_data_from_last_user_block() {
                    // If we've cached the prompt from the active block, use our
                    // cached copy instead of the block's current prompt.  This
                    // avoids flicker when a prompt is changed in preexec (e.g.: the
                    // transient prompt feature of p10k).
                    // Or, if the prompt is empty and the block is less than
                    // 50ms old, also use the cached prompt from the previous block (to
                    // avoid flicker in the brief window between receiving precmd
                    // metadata and receiving prompt bytes, when using marker-based
                    // prompts).
                    Some(CachedPromptData {
                        prompt_grid,
                        rprompt_grid,
                        block_creation_time,
                    }) if block_creation_time == prompt_block.creation_ts()
                        || (prompt_block.is_prompt_empty()
                            && chrono::Local::now() - *prompt_block.creation_ts()
                                < prompt_marker_grace_period(shell_type, is_msys2)) =>
                    {
                        Self::compute_prompt_layout(
                            render_prompt_on_same_line,
                            prompt_grid,
                            rprompt_grid,
                        )
                    }
                    // If neither of those conditions apply, simply use the prompt
                    // grid as-is.
                    _ => Self::compute_prompt_layout(
                        render_prompt_on_same_line,
                        prompt_block.prompt_grid(),
                        prompt_block.rprompt_grid(),
                    ),
                };

            // Ignore the default horizontal padding used for grids, as this is
            // already applied by the Input.
            let mut size_info = app.model(&self.input_render_state_model_handle).size_info();
            size_info.padding_x_px = Pixels::zero();

            let obfuscate_secrets: ObfuscateSecrets = get_secret_obfuscation_mode(app);

            let lprompt_top = lprompt_grid_top.map(|grid| {
                Self::prompt_block_grid_to_prompt_and_padding(
                    grid,
                    0.,
                    0.,
                    appearance,
                    obfuscate_secrets,
                    size_info,
                    app,
                )
            });
            let lprompt_bottom = lprompt_grid_bottom.map(|grid| {
                Self::prompt_block_grid_to_prompt_and_padding(
                    grid,
                    0.,
                    0.,
                    appearance,
                    obfuscate_secrets,
                    size_info,
                    app,
                )
            });
            let rprompt = Self::prompt_block_grid_to_prompt_and_padding(
                rprompt_grid,
                0.,
                0.,
                appearance,
                obfuscate_secrets,
                size_info,
                app,
            );
            let rprompt_val = prompt_block
                .should_display_rprompt(&size_info)
                .then_some(rprompt);
            let lprompt_bottom_val = render_prompt_on_same_line
                .then_some(lprompt_bottom)
                .flatten();

            (lprompt_top, lprompt_bottom_val, rprompt_val)

        // If not render the default starting shell message.
        } else if model.block_list().active_block().honor_ps1() && !is_universal_input {
            let prompt = PromptAndPadding {
                element: PromptAndPaddingElement::Text(Box::new(
                    self.bootstrapping_shell_text(model, appearance, app),
                )),
                padding_left: 0.,
                padding_right,
            };
            if render_prompt_on_same_line {
                (None, Some(prompt), None)
            } else {
                (Some(prompt), None, None)
            }
        } else {
            let element = {
                if model.block_list().is_bootstrapped() {
                    PromptAndPaddingElement::ContextChips(self.prompt_view.clone())
                } else {
                    PromptAndPaddingElement::Text(Box::new(
                        self.bootstrapping_shell_text(model, appearance, app),
                    ))
                }
            };

            let prompt = PromptAndPadding {
                element,
                padding_left: 0.,
                padding_right,
            };
            if render_prompt_on_same_line {
                (None, Some(prompt), None)
            } else {
                (Some(prompt), None, None)
            }
        }
    }

    fn render_prompt_area_helper(
        &self,
        terminal_model: &TerminalModel,
        prompt_and_padding: PromptAndPadding,
        appearance: &Appearance,
        prompt_side: PromptSide,
        app: &AppContext,
    ) -> Option<Box<dyn Element>> {
        let is_universal_input =
            InputSettings::as_ref(app).is_universal_developer_input_enabled(app);

        let should_render_prompt_using_editor_decorator_elements =
            should_render_prompt_using_editor_decorator_elements(
                is_universal_input,
                &self.ai_input_model,
                terminal_model,
                app,
            );
        let view_id = self.prompt_parent_view_id;
        let position_id = format!("{prompt_side}_{view_id}");
        let size_info = app.model(&self.input_render_state_model_handle).size_info();
        let terminal_spacing = TerminalSettings::as_ref(app)
            .terminal_input_spacing(appearance.line_height_ratio(), app);
        let prompt_with_padding_container = Container::new(prompt_and_padding.element.render())
            .with_padding_top({
                if should_render_prompt_using_editor_decorator_elements {
                    0.0
                } else {
                    terminal_spacing.block_padding.padding_top * size_info.cell_height_px().as_f32()
                        - get_input_box_top_border_width()
                }
            })
            .with_padding_left(prompt_and_padding.padding_left)
            .with_padding_right(prompt_and_padding.padding_right)
            .with_padding_bottom({
                if should_render_prompt_using_editor_decorator_elements {
                    0.0
                } else {
                    terminal_spacing.prompt_to_editor_padding
                }
            })
            .finish();
        let semantic_selection = SemanticSelection::as_ref(app);
        Some(
            SavePosition::new(
                EventHandler::new(if FeatureFlag::SelectablePrompt.is_enabled() {
                    // TODO(Simon): Distinguish right-clicks performed directly within selected text bounds.
                    // The "Copy" option should only appear when selected text is right-clicked directly.
                    // Right-clicks performed directly within selected text can be handled separately via
                    // the `SelectableArea`'s `on_selection_right_click` function. It may be helpful to
                    // follow the paradigm used for `TerminalAction::BlockListContextMenu`.
                    SelectableArea::new(
                        self.prompt_selection_state_handle.clone(),
                        |_selection_args, _ctx, _| {},
                        prompt_with_padding_container,
                    )
                    .with_word_boundaries_policy(semantic_selection.word_boundary_policy())
                    .with_smart_select_fn(semantic_selection.smart_select_fn())
                    .finish()
                } else {
                    prompt_with_padding_container
                })
                .on_right_mouse_down(move |ctx, _, position| {
                    let position_id = format!("prompt_area_{view_id}");
                    let Some(prompt_rect) = ctx.element_position_by_id(position_id) else {
                        return DispatchEventResult::PropagateToParent;
                    };
                    let offset_position = position - prompt_rect.origin();
                    ctx.dispatch_typed_action(TerminalAction::PromptContextMenu {
                        position_offset_from_prompt: offset_position,
                    });
                    DispatchEventResult::StopPropagation
                })
                .finish(),
                &position_id,
            )
            .finish(),
        )
    }

    pub(in crate::terminal) fn render_universal_developer_input_prompt(
        &self,
        model: &TerminalModel,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let element = {
            if model.block_list().is_bootstrapped() {
                PromptAndPaddingElement::ContextChips(self.prompt_view.clone())
            } else {
                PromptAndPaddingElement::Text(Box::new(
                    self.bootstrapping_shell_text(model, appearance, app),
                ))
            }
        };

        let view_id = self.prompt_parent_view_id;
        let position_id = format!("{}_{}", PromptSide::Left, view_id);
        let size_info = app.model(&self.input_render_state_model_handle).size_info();
        let terminal_spacing = TerminalSettings::as_ref(app)
            .terminal_input_spacing(appearance.line_height_ratio(), app);

        let prompt_with_padding_container = Container::new(element.render())
            .with_padding_top({
                (terminal_spacing.block_padding.padding_top * size_info.cell_height_px().as_f32()
                    - get_input_box_top_border_width())
                    * spacing::UDI_PROMPT_TOP_PADDING_FACTOR
            })
            .finish();

        SavePosition::new(
            EventHandler::new(prompt_with_padding_container)
                .on_right_mouse_down(move |ctx, _, position| {
                    let position_id = format!("prompt_area_{view_id}");
                    let Some(prompt_rect) = ctx.element_position_by_id(position_id) else {
                        return DispatchEventResult::PropagateToParent;
                    };
                    let offset_position = position - prompt_rect.origin();
                    ctx.dispatch_typed_action(TerminalAction::PromptContextMenu {
                        position_offset_from_prompt: offset_position,
                    });
                    DispatchEventResult::StopPropagation
                })
                .finish(),
            &position_id,
        )
        .finish()
    }

    pub(in crate::terminal) fn render_prompt_areas(
        &self,
        model: &TerminalModel,
        appearance: &Appearance,
        app: &AppContext,
    ) -> PromptElements {
        let (lprompt_and_padding_option, _, rprompt_and_padding_option) =
            self.render_prompt(model, appearance, app);
        let lprompt = lprompt_and_padding_option.and_then(|lprompt_top_and_padding| {
            self.render_prompt_area_helper(
                model,
                lprompt_top_and_padding,
                appearance,
                PromptSide::Left,
                app,
            )
        });
        let rprompt = rprompt_and_padding_option.and_then(|rprompt_and_padding| {
            self.render_prompt_area_helper(
                model,
                rprompt_and_padding,
                appearance,
                PromptSide::Right,
                app,
            )
        });
        PromptElements { lprompt, rprompt }
    }

    pub(in crate::terminal) fn render_same_line_prompt_areas(
        &self,
        model: &TerminalModel,
        appearance: &Appearance,
        app: &AppContext,
    ) -> SameLinePromptElements {
        let (
            lprompt_top_and_padding_option,
            lprompt_bottom_and_padding_option,
            rprompt_and_padding_option,
        ) = self.render_prompt(model, appearance, app);
        let lprompt_top = lprompt_top_and_padding_option.and_then(|lprompt_top_and_padding| {
            self.render_prompt_area_helper(
                model,
                lprompt_top_and_padding,
                appearance,
                PromptSide::Left,
                app,
            )
        });
        let lprompt_bottom =
            lprompt_bottom_and_padding_option.and_then(|lprompt_bottom_and_padding| {
                self.render_prompt_area_helper(
                    model,
                    lprompt_bottom_and_padding,
                    appearance,
                    PromptSide::Left,
                    app,
                )
            });
        let rprompt = rprompt_and_padding_option.and_then(|rprompt_and_padding| {
            self.render_prompt_area_helper(
                model,
                rprompt_and_padding,
                appearance,
                PromptSide::Right,
                app,
            )
        });
        SameLinePromptElements {
            lprompt_top,
            lprompt_bottom,
            rprompt,
        }
    }

    #[cfg(feature = "integration_tests")]
    pub fn git_branch(&self, ctx: &AppContext) -> Option<String> {
        self.prompt_view
            .read(ctx, |prompt_display, ctx| prompt_display.git_branch(ctx))
    }
}
