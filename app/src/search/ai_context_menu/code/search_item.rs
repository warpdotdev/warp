use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, Highlight, Icon, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

// Import CodeSymbol from the data_source module
use super::data_source::CodeSymbol;
use crate::appearance::Appearance;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::ai_context_menu::{safe_truncate, styles};
use crate::search::item::{IconLocation, SearchItem};
use crate::search::result_renderer::ItemHighlightState;

const MAX_COMBINED_LENGTH: usize = 55;

#[derive(Debug, Clone)]
pub struct CodeSearchItem {
    pub code_symbol: CodeSymbol,
    pub match_result: FuzzyMatchResult,
}

impl SearchItem for CodeSearchItem {
    type Action = AIContextMenuSearchableAction;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Icon::new(
                    "bundled/svg/code-01.svg",
                    _highlight_state.icon_fill(appearance).into_solid(),
                )
                .finish(),
            )
            .with_width(styles::ICON_SIZE)
            .with_height(styles::ICON_SIZE)
            .finish(),
        )
        .with_margin_right(styles::MARGIN_RIGHT)
        .finish()
    }

    fn icon_location(&self, _appearance: &Appearance) -> IconLocation {
        IconLocation::Centered
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // Build the symbol name with type prefix
        let mut symbol_name = String::new();
        if let Some(symbol_type) = &self.code_symbol.symbol.type_prefix {
            symbol_name.push_str(&format!("{symbol_type} "));
        }
        symbol_name.push_str(&self.code_symbol.symbol.name);

        // Get file path for display
        let file_path = self.code_symbol.file_path.to_string_lossy().to_string();
        let mut path_display = file_path.clone();

        // Track truncation for highlight adjustment
        let mut symbol_truncated = false;

        // Ensure combined length is less than MAX_COMBINED_LENGTH characters
        let combined_length = symbol_name.len() + path_display.len();

        if combined_length > MAX_COMBINED_LENGTH {
            // If combined length is too long, prioritize showing the symbol name
            if symbol_name.len() >= MAX_COMBINED_LENGTH {
                // If symbol name itself is too long, truncate it and add ellipsis
                safe_truncate(&mut symbol_name, MAX_COMBINED_LENGTH - 3);
                symbol_name.push_str("...");
                symbol_truncated = true;
                path_display.clear();
            } else {
                // Symbol name fits, truncate path display
                let available_for_path = MAX_COMBINED_LENGTH - symbol_name.len();
                if path_display.len() > available_for_path {
                    let new_path_len = available_for_path.saturating_sub(3);
                    safe_truncate(&mut path_display, new_path_len);
                    path_display.push_str("...");
                }
            }
        }

        // Calculate highlight indices, adjusting for display format
        // The fuzzy matching is done on concatenated "typeprefix" + "symbolname" (no space)
        // But display shows "typeprefix " + "symbolname" (with space)
        // So we need to adjust indices to account for the added space in display
        let symbol_highlights: Vec<usize> = if !symbol_truncated {
            self.match_result
                .matched_indices
                .iter()
                .map(|&i| {
                    if let Some(symbol_type) = &self.code_symbol.symbol.type_prefix {
                        // If we have a type prefix, adjust indices:
                        // - Indices 0 to type_prefix.len()-1 map directly (type prefix part)
                        // - Indices type_prefix.len() and beyond need +1 offset (for the added space)
                        if i < symbol_type.len() {
                            i // Direct mapping for type prefix
                        } else {
                            i + 1 // Add 1 for the space between type and name
                        }
                    } else {
                        i // No type prefix, direct mapping
                    }
                })
                .collect()
        } else {
            // Only include highlights that fall within the truncated range
            self.match_result
                .matched_indices
                .iter()
                .filter_map(|&i| {
                    let adjusted_i = if let Some(symbol_type) = &self.code_symbol.symbol.type_prefix
                    {
                        if i < symbol_type.len() { i } else { i + 1 }
                    } else {
                        i
                    };

                    if adjusted_i < MAX_COMBINED_LENGTH - 3 {
                        Some(adjusted_i)
                    } else {
                        None
                    }
                })
                .collect()
        };

        // Create symbol name text with highlighting
        let mut symbol_text = Text::new(
            symbol_name,
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 1.0,
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid());

        if !symbol_highlights.is_empty() {
            symbol_text = symbol_text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                symbol_highlights,
            );
        }

        // Create path text with lighter color
        let path_text = if !path_display.is_empty() {
            Some(
                Text::new(
                    path_display,
                    appearance.ui_font_family(),
                    appearance.monospace_font_size() - 2.0,
                )
                .with_color(highlight_state.sub_text_fill(appearance).into_solid())
                .finish(),
            )
        } else {
            None
        };

        // Create row with symbol name and path on the same line
        let mut row = Flex::row()
            .with_child(symbol_text.finish())
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(path) = path_text {
            row.add_child(Container::new(path).with_padding_left(8.0).finish());
        }

        row.finish()
    }

    fn render_details(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();

        // Create main text: symbol type + name (e.g., "fn initialize_logger")
        let mut main_text = String::new();
        if let Some(symbol_type) = &self.code_symbol.symbol.type_prefix {
            main_text.push_str(symbol_type);
            main_text.push(' ');
        }
        main_text.push_str(&self.code_symbol.symbol.name);

        // Create sub text: path + line number (e.g., "core/logging.rs (44)")
        let sub_text = format!(
            "{} ({})",
            self.code_symbol.file_path.to_string_lossy(),
            self.code_symbol.symbol.line_number
        );

        // Create main text element - use slightly smaller font that scales with user settings
        let main_text_element = Text::new(
            main_text,
            appearance.monospace_font_family(), // Use monospace font for consistency
            appearance.monospace_font_size() - 1.0, // Slightly smaller than normal
        )
        .with_color(theme.active_ui_text_color().into())
        .with_style(Properties::default().weight(Weight::Medium))
        .finish();

        // Create sub text element - even smaller for sub information
        let sub_text_element = Text::new(
            sub_text,
            appearance.monospace_font_family(), // Use monospace font for consistency
            appearance.monospace_font_size() - 3.0, // Smaller than main text
        )
        .with_color(theme.nonactive_ui_text_color().into())
        .finish();

        // Create modal content with reduced spacing
        let content = Flex::column()
            .with_child(main_text_element)
            .with_child(
                Container::new(sub_text_element)
                    .with_padding_top(2.0)
                    .finish(),
            )
            .finish();

        Some(content)
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> Self::Action {
        // Format the text as "{symbol_type} {symbol_name} in {path}:{line_number}"
        let mut text = String::new();
        if let Some(symbol_type) = &self.code_symbol.symbol.type_prefix {
            text.push_str(symbol_type);
            text.push(' ');
        }
        text.push_str(&self.code_symbol.symbol.name);
        text.push_str(" in ");
        text.push_str(&self.code_symbol.file_path.to_string_lossy());
        text.push(':');
        text.push_str(&self.code_symbol.symbol.line_number.to_string());

        AIContextMenuSearchableAction::InsertText { text }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!(
            "Code symbol: {} in {}:{}",
            self.code_symbol.symbol.name,
            self.code_symbol.file_path.to_string_lossy(),
            self.code_symbol.symbol.line_number
        )
    }
}
