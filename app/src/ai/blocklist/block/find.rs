use std::collections::HashMap;
use std::ops::Range;

use itertools::Itertools;
use markdown_parser::parse_markdown;
use regex::RegexBuilder;

use super::{AIBlock, TextLocation};
use crate::ai::agent::{AIAgentTextSection, MessageId};
use crate::terminal::find::{FindOptions, FindableRichContentView, RichContentMatchId};

/// Represents the location of a find match in an AI block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FindMatchLocation {
    pub(super) text_location: TextLocation,
    pub(super) char_range: Range<usize>,
    /// The message ID that contains this match (for Reasoning/Text blocks).
    pub(super) message_id: Option<MessageId>,
}

/// Encapsulated find-related state relevant to an AI block.
#[derive(Debug, Default, Clone)]
pub(crate) struct FindState {
    /// Matches in this AI block.
    matches: HashMap<RichContentMatchId, FindMatchLocation>,
}

impl FindState {
    pub(super) fn matches_for_location(
        &self,
        location: TextLocation,
    ) -> impl Iterator<Item = &FindMatchLocation> {
        self.matches
            .values()
            .filter(move |find_match_location| find_match_location.text_location == location)
    }

    pub(super) fn location_for_match(&self, id: RichContentMatchId) -> Option<&FindMatchLocation> {
        self.matches.get(&id)
    }
}

impl FindableRichContentView for AIBlock {
    /// Computes find matches within this AI block and updates the block's `FindState`.
    fn run_find(
        &mut self,
        options: &FindOptions,
        ctx: &mut warpui::ViewContext<Self>,
    ) -> Vec<RichContentMatchId> {
        self.clear_matches(ctx);

        let mut new_match_ids = vec![];
        for (i, input) in self.model.inputs_to_render(ctx).iter().enumerate() {
            if let Some(query) = input.user_query() {
                let query_matches = match parse_markdown(&query) {
                    Ok(formatted_query) => formatted_query
                        .lines
                        .iter()
                        .map(|line| compute_find_matches(&line.raw_text(), options))
                        .collect_vec(),
                    Err(_) => vec![compute_find_matches(&query, options)],
                };

                for (line_index, line_matches) in query_matches.into_iter().enumerate() {
                    for find_match_range in line_matches {
                        let id = RichContentMatchId::default();
                        new_match_ids.push(id);
                        self.find_state.matches.insert(
                            id,
                            FindMatchLocation {
                                text_location: TextLocation::Query {
                                    input_index: i,
                                    line_index,
                                },
                                char_range: find_match_range,
                                message_id: None,
                            },
                        );
                    }
                }
            }
        }

        if let Some(output) = self.model.status(ctx).output_to_render() {
            for (section_index, (message_id, text_section)) in output
                .get()
                .all_text_with_message_id()
                .flat_map(|(msg_id, text)| {
                    text.sections.iter().map(move |section| (msg_id, section))
                })
                .enumerate()
            {
                let section_matches = match text_section {
                    AIAgentTextSection::PlainText { text } => match &text.formatted_lines {
                        Some(formatted_text) => {
                            let mut matches: Vec<Vec<Range<usize>>> = vec![];
                            for line in formatted_text.lines() {
                                matches.push(compute_find_matches(line.raw_text(), options));
                            }
                            matches
                        }
                        _ => vec![compute_find_matches(text.text(), options)],
                    },
                    AIAgentTextSection::Code { code, .. } => {
                        vec![compute_find_matches(code.as_str(), options)]
                    }
                    AIAgentTextSection::Table { table } => table
                        .rendered_lines()
                        .into_iter()
                        .map(|line| compute_find_matches(&line, options))
                        .collect(),
                    AIAgentTextSection::Image { image } => {
                        vec![compute_find_matches(&image.markdown_source, options)]
                    }
                    AIAgentTextSection::MermaidDiagram { diagram } => {
                        vec![compute_find_matches(&diagram.markdown_source, options)]
                    }
                };

                for (line_index, frame_matches) in section_matches.into_iter().enumerate() {
                    for find_match_range in frame_matches {
                        let id = RichContentMatchId::default();
                        new_match_ids.push(id);
                        self.find_state.matches.insert(
                            id,
                            FindMatchLocation {
                                text_location: TextLocation::Output {
                                    section_index,
                                    line_index,
                                },
                                char_range: find_match_range,
                                message_id: Some(message_id.clone()),
                            },
                        );
                    }
                }
            }
        }

        ctx.notify();
        new_match_ids
    }

    fn clear_matches(&mut self, ctx: &mut warpui::ViewContext<Self>) {
        self.find_state.matches.clear();
        ctx.notify();
    }
}

/// Computes find matches (represented as character offsets) within the given `text`.
fn compute_find_matches(text: &str, options: &FindOptions) -> Vec<Range<usize>> {
    let Some(query) = options.query.as_ref() else {
        return vec![];
    };
    if options.is_regex_enabled {
        let Ok(regex) = RegexBuilder::new(query.as_str())
            .case_insensitive(!options.is_case_sensitive)
            .build()
        else {
            log::warn!("Attempted to run find on AI block with invalid regex: {query}");
            return vec![];
        };
        regex
            .find_iter(text)
            .map(|m| {
                // Convert the range from byte offset to char offset.
                let char_offset_start = text[..(m.range().start)].chars().count();
                let char_offset_end = text[..(m.range().end)].chars().count();
                char_offset_start..char_offset_end
            })
            .collect_vec()
    } else if options.is_case_sensitive {
        // The length of the query in characters. Note this differs from query.len(), which is
        // length in bytes.
        let query_len_chars = query.chars().count();

        text.match_indices(query.as_ref())
            .map(|(start_bytes, _)| {
                // The start _char_ index (as opposed to byte index).
                let start_chars = text[..start_bytes].chars().count();
                start_chars..start_chars + query_len_chars
            })
            .collect_vec()
    } else {
        // The length of the query in characters. Note this differs from query.len(), which is
        // length in bytes.
        let query = query.to_lowercase();
        let query_len_chars = query.chars().count();

        text.to_lowercase()
            .match_indices(&query)
            .map(|(start_bytes, _)| {
                let start_chars = text[..start_bytes].chars().count();
                start_chars..start_chars + query_len_chars
            })
            .collect_vec()
    }
}
