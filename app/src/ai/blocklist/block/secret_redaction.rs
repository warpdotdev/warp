use std::collections::HashMap;
use std::sync::Arc;

use itertools::Itertools;
use similar::DiffableStr;
use warpui::elements::{MouseStateHandle, PartialClickableElement, SecretRange};
use warpui::platform::Cursor;

use super::{AIBlockAction, TextLocation};
use crate::ai::agent::{AIAgentOutput, AIAgentTextSection, AgentOutputText};
use crate::report_error;
use crate::terminal::model::secrets::{SecretLevel, SecretsRegex, SECRETS_REGEX};

pub const SECRET_REDACTION_REPLACEMENT_CHARACTER: &str = "*";

/// Returns the ranges of detected secrets in the given text.
pub(crate) fn find_secrets_in_text(text: &str) -> Vec<SecretRange> {
    find_secrets_in_text_with_levels(text)
        .into_iter()
        .map(|(range, _level)| range)
        .collect()
}

/// Returns the ranges of detected secrets in the given text along with their SecretLevel.
pub(crate) fn find_secrets_in_text_with_levels(text: &str) -> Vec<(SecretRange, SecretLevel)> {
    let secrets_regex: Arc<SecretsRegex> = { SECRETS_REGEX.lock().clone() };

    find_secrets_in_text_with_levels_using_regex(text, &secrets_regex)
}

pub(crate) fn find_secrets_in_text_with_levels_using_regex(
    text: &str,
    secrets_regex: &SecretsRegex,
) -> Vec<(SecretRange, SecretLevel)> {
    let SecretsRegex {
        regex,
        level_metadata,
        ..
    } = secrets_regex;

    let mut secret_ranges = vec![];
    let mut byte_to_char_index = vec![0; text.len() + 1]; // Map byte index to char index

    // Track the current character index while iterating through the string.
    let mut char_index = 0;
    for (byte_index, _) in text.char_indices() {
        byte_to_char_index[byte_index] = char_index;
        char_index += 1;
    }
    byte_to_char_index[text.len()] = char_index; // Map the last byte to the last character index

    // Iterate over the text once, finding all matches against secret regex. Map the byte ranges
    // to character ranges and store them.
    for mat in regex.find_iter(text) {
        let start_byte = mat.start();
        let end_byte = mat.end();
        let start_char = byte_to_char_index[start_byte];
        let end_char = byte_to_char_index[end_byte];

        // Determine which pattern matched by getting the pattern ID and map via counts
        let pattern_id = mat.pattern().as_usize();
        let total_patterns = level_metadata.enterprise_count + level_metadata.user_count;
        if pattern_id >= total_patterns {
            report_error!(
                "Secret level not found for pattern ID",
                extra: { "pattern_id" => %pattern_id }
            );
            continue;
        }
        let secret_level = if pattern_id < level_metadata.enterprise_count {
            SecretLevel::Enterprise
        } else {
            SecretLevel::User
        };

        secret_ranges.push((
            SecretRange {
                char_range: start_char..end_char,
                byte_range: start_byte..end_byte,
            },
            secret_level,
        ));
    }

    // Merge overlapping ranges, preserving the highest priority SecretLevel
    merge_sorted_ranges_with_levels(secret_ranges)
}

/// Merges overlapping ranges while preserving the highest priority SecretLevel
fn merge_sorted_ranges_with_levels(
    ranges: Vec<(SecretRange, SecretLevel)>,
) -> Vec<(SecretRange, SecretLevel)> {
    if ranges.is_empty() {
        return ranges;
    }

    let mut merged_ranges = vec![];
    let mut current_range = ranges[0].0.clone();
    let mut current_level = ranges[0].1;

    for (range, level) in ranges.into_iter().skip(1) {
        // We can merge based on character ranges since non-overlapping character ranges result in non-overlapping byte ranges.
        if range.char_range.start <= current_range.char_range.end {
            // Extend the current range to include the overlapping range.
            current_range.extend_range_end(&range);
            // Keep the highest priority level
            if level.priority() > current_level.priority() {
                current_level = level;
            }
        } else {
            // No overlap, push the current range and move to the next.
            merged_ranges.push((current_range, current_level));
            current_range = range;
            current_level = level;
        }
    }

    // Add the last range.
    merged_ranges.push((current_range, current_level));

    merged_ranges
}

#[derive(Debug, Eq, PartialEq)]
pub struct SecretLocation {
    pub secret_range: SecretRange,
    pub location: TextLocation,
}

#[derive(Clone, Debug)]
pub struct Secret {
    pub secret: String,
    pub is_obfuscated: bool,
    pub mouse_state: MouseStateHandle,
    pub secret_level: SecretLevel,
}

#[derive(Default, Debug)]
pub struct DetectedSecretsInTextLocation {
    pub detected_secrets: HashMap<SecretRange, Secret>,
}

#[derive(Default, Debug)]
pub struct SecretRedactionState {
    /// Last byte index of text we've scanned for secret redaction (avoid re-scanning in the context
    /// of streaming output text). Note we want to redact secrets WHILE streaming to avoid any full secrets
    /// ever being shown on the screen! This applies to the last output step we've currently received.
    last_scanned_secret_redaction_byte_index: usize,
    /// Buffer to hold the last word that's been scanned, since we may need to combine it with the next
    /// tokens we're receiving (since a secret could be split apart across streaming chunks). Words are defined
    /// to be separated by whitespace.
    last_word_to_rescan_for_redaction: String,
    /// Buffer to hold the current line. This is important, as markdown parsing could potentially mutate the entire line.
    current_line_for_redaction: String,
    /// Keeps track of the last step index we've run secret redaction scanning on, while streaming output.
    last_text_section_index_scanned_for_redaction: usize,
    last_line_index_scanned_for_redaction: usize,

    detected_secrets: HashMap<TextLocation, DetectedSecretsInTextLocation>,

    currently_hovered_secret_location: Option<SecretLocation>,

    // This is separate from currently_hovered_secret_location because after clicking
    // on a secret to open the tooltip, this secret should remain highlighted and the tooltip in place
    // even if we hover over other secrets.
    secret_location_open_tooltip: Option<SecretLocation>,
}

impl SecretRedactionState {
    pub fn open_tooltip_location(&self) -> Option<&SecretLocation> {
        self.secret_location_open_tooltip.as_ref()
    }

    pub fn hovered_location(&self) -> Option<&SecretLocation> {
        self.currently_hovered_secret_location.as_ref()
    }

    pub fn has_open_tooltip(&self, location: &TextLocation, range: &SecretRange) -> bool {
        self.open_tooltip_location()
            .is_some_and(|tooltip_location| {
                tooltip_location.location == *location && tooltip_location.secret_range == *range
            })
    }

    pub fn is_hovered(&self, location: &TextLocation, range: &SecretRange) -> bool {
        self.hovered_location().is_some_and(|tooltip_location| {
            tooltip_location.location == *location && tooltip_location.secret_range == *range
        })
    }

    pub fn reset(&mut self) {
        self.last_text_section_index_scanned_for_redaction = 0;
        self.last_scanned_secret_redaction_byte_index = 0;
        self.last_word_to_rescan_for_redaction = Default::default();
    }

    /// Clears secret redaction state for `user_query` locations.
    ///
    /// A bit of an edge case, but this is required when the user accepts a 'suggest new conversation' action
    /// for an existing query. This query becomes the first query in a new conversation, and we prefix '/agent'
    /// to all initial user queries, so detected secret ranges (if any) become stale.
    pub fn clear_user_query_locations(&mut self) {
        self.detected_secrets
            .retain(|location, _| !matches!(location, TextLocation::Query { .. }));

        if self
            .currently_hovered_secret_location
            .as_ref()
            .is_some_and(|location| matches!(location.location, TextLocation::Query { .. }))
        {
            self.currently_hovered_secret_location = None;
        }

        if self
            .secret_location_open_tooltip
            .as_ref()
            .is_some_and(|location| matches!(location.location, TextLocation::Query { .. }))
        {
            self.secret_location_open_tooltip = None;
        }
    }

    pub fn show_secret_tooltip(
        &mut self,
        location: &TextLocation,
        secret_range: &SecretRange,
    ) -> Option<&mut Secret> {
        self.secret_location_open_tooltip = Some(SecretLocation {
            secret_range: secret_range.clone(),
            location: *location,
        });
        self.get_secret_mut(location, secret_range)
    }

    pub fn dismiss_tooltip(&mut self) {
        self.secret_location_open_tooltip = None;
    }

    pub fn set_obfuscated(
        &mut self,
        location: &TextLocation,
        secret_range: &SecretRange,
        is_obfuscated: bool,
    ) {
        if let Some(hoverable_secret_mut) = self.get_secret_mut(location, secret_range) {
            hoverable_secret_mut.is_obfuscated = is_obfuscated;
        }
    }

    pub fn set_hover_state_for_secret(
        &mut self,
        location: &TextLocation,
        secret_range: &SecretRange,
        is_hovering: bool,
    ) {
        if is_hovering {
            self.currently_hovered_secret_location = Some(SecretLocation {
                secret_range: secret_range.clone(),
                location: *location,
            });
        } else if self.currently_hovered_secret_location.as_ref().is_some_and(
            |currently_hovered_secret| {
                currently_hovered_secret.secret_range == *secret_range
                    && currently_hovered_secret.location == *location
            },
        ) {
            self.currently_hovered_secret_location = None;
        }
    }

    pub fn secrets_for_location(
        &self,
        location: &TextLocation,
    ) -> Option<&DetectedSecretsInTextLocation> {
        self.detected_secrets.get(location)
    }

    fn get_secret_mut(
        &mut self,
        location: &TextLocation,
        secret_range: &SecretRange,
    ) -> Option<&mut Secret> {
        self.detected_secrets
            .get_mut(location)
            .and_then(|detected_location| detected_location.detected_secrets.get_mut(secret_range))
    }

    pub fn run_redaction_for_location(
        &mut self,
        text: &str,
        location: TextLocation,
        should_obfuscate: bool,
    ) {
        // Detect secrets in user's query.
        let secret_ranges_with_levels = find_secrets_in_text_with_levels(text);
        for (secret_range, secret_level) in secret_ranges_with_levels {
            if let Some(secret_text) =
                text.get(secret_range.byte_range.start..secret_range.byte_range.end)
            {
                self.detected_secrets
                    .entry(location)
                    .or_default()
                    .detected_secrets
                    .insert(
                        secret_range,
                        Secret {
                            secret: secret_text.to_string(),
                            is_obfuscated: should_obfuscate,
                            mouse_state: Default::default(),
                            secret_level,
                        },
                    );
            }
        }
    }

    pub fn run_incremental_redaction_on_partial_output(
        &mut self,
        output: &AIAgentOutput,
        should_obfuscate: bool,
    ) {
        // Steps are sequentially streamed, hence we always check the last step.
        // Important: all the *lines* in the context are markdown lines, which could be rendered as multiple lines on screen.
        if let Some((section_index, text_section)) = output
            .all_text()
            .flat_map(|text| text.sections.iter())
            .enumerate()
            .last()
        {
            let line_index = match text_section {
                AIAgentTextSection::PlainText {
                    text:
                        AgentOutputText {
                            formatted_lines: Some(text),
                            ..
                        },
                } => text
                    .lines()
                    .iter()
                    .enumerate()
                    .next_back()
                    .map_or(0, |(index, _)| index),
                _ => 0,
            };

            let mut start_of_last_word_byte_index;
            if section_index == self.last_text_section_index_scanned_for_redaction
                && line_index == self.last_line_index_scanned_for_redaction
            {
                // The boundary between what we're done scanning and what we still need to scan.
                start_of_last_word_byte_index = self.last_scanned_secret_redaction_byte_index
                    - self.last_word_to_rescan_for_redaction.len();
                self.detected_secrets
                    // We remove all secrets that are beyond the cutoff boundary where we start rescanning,
                    // specifically to avoid having duplicate secrets from the last word buffer.
                    .retain(|location, secrets| {
                        if let TextLocation::Output {
                            section_index: current_section_index,
                            line_index: current_line_index,
                        } = location
                        {
                            // Only clear secrets from the last step we scanned.
                            if *current_section_index == section_index
                                && *current_line_index == line_index
                            {
                                secrets.detected_secrets.retain(|secret_range, _| {
                                    secret_range.byte_range.start < start_of_last_word_byte_index
                                        && secret_range.byte_range.end
                                            <= start_of_last_word_byte_index
                                });
                            }
                            !secrets.detected_secrets.is_empty()
                        } else {
                            true
                        }
                    });
            } else {
                // Addition needs to happen before subtraction to prevent an intermediate usize value
                // from causing a numeric overflow and panicking, as Rust arithmetic operators are left-associative.
                // TODO: Investigate why, sometimes, the section index is smaller than the last section index scanned.
                let num_sections_to_scan = (section_index + 1)
                    .saturating_sub(self.last_text_section_index_scanned_for_redaction);
                for (rerun_section_index, rerun_section) in output
                    .all_text()
                    .flat_map(|text| text.sections.iter())
                    .enumerate()
                    .skip(self.last_text_section_index_scanned_for_redaction)
                    .take(num_sections_to_scan)
                {
                    if rerun_section_index == self.last_text_section_index_scanned_for_redaction {
                        // First step: rerun secret detection on the step which the secret detection last got ran.
                        if let AIAgentTextSection::PlainText { text } = rerun_section {
                            let end_line_index = if self
                                .last_text_section_index_scanned_for_redaction
                                == section_index
                            {
                                // Only step index changed, we're still in the same step. Run secret detection up to the current line.
                                line_index
                            } else {
                                // We're in a new step - run secret detection up to and include the last line.
                                text.formatted_lines
                                    .as_ref()
                                    .map(|lines| lines.lines().len())
                                    .unwrap_or(0)
                            };
                            // Starting on the last detected line (rerun, in case markdown parsing altered the text)
                            // TODO: Optimization: only run secret detection on the newly part of the line if the parser didn't alter it
                            for rerun_line_index in
                                self.last_line_index_scanned_for_redaction..end_line_index
                            {
                                self.rerun_secret_detection_for_output_line(
                                    rerun_section_index,
                                    rerun_line_index,
                                    text,
                                )
                            }
                        } else {
                            self.detected_secrets.remove(&TextLocation::Output {
                                section_index: rerun_section_index,
                                line_index: 0,
                            });
                        }
                    } else if rerun_section_index == section_index {
                        // Last step: rerun secret detection up until the current line.
                        // This part is already handled in the previous condition if we're in the same step.
                        if let AIAgentTextSection::PlainText { text } = rerun_section {
                            for rerun_line_index in 0..line_index {
                                self.rerun_secret_detection_for_output_line(
                                    rerun_section_index,
                                    rerun_line_index,
                                    text,
                                )
                            }
                        }
                        // No need to clear secrets in an else case - these lines has not been scanned for secrets yet.
                    } else {
                        // Middle sections: run secret detection on all lines.
                        if let AIAgentTextSection::PlainText { text } = rerun_section {
                            for rerun_line_index in 0..text
                                .formatted_lines
                                .as_ref()
                                .map(|lines| lines.lines().len())
                                .unwrap_or(0)
                            {
                                self.rerun_secret_detection_for_output_line(
                                    rerun_section_index,
                                    rerun_line_index,
                                    text,
                                )
                            }
                        }
                        // No need to clear secrets in an else case - these lines has not been scanned for secrets yet.
                    }
                }

                self.last_text_section_index_scanned_for_redaction = section_index;
                self.last_line_index_scanned_for_redaction = line_index;
                self.last_scanned_secret_redaction_byte_index = 0;
                self.last_word_to_rescan_for_redaction.clear();
                self.current_line_for_redaction.clear();
                start_of_last_word_byte_index = 0;
            }

            if let AIAgentTextSection::PlainText { text } = &text_section {
                let text = match &text.formatted_lines {
                    Some(text) => text.lines().iter().last().map(|line| line.raw_text()),
                    _ => None,
                };

                if let Some(text) = text {
                    // Trim the trailing newline as the parser automatically
                    // adds a newline to the end of the text.
                    let text = if text.ends_with_newline() {
                        &text[..text.len() - 1]
                    } else {
                        text
                    };

                    if text.len() >= self.current_line_for_redaction.len()
                        && text.starts_with(&self.current_line_for_redaction)
                    {
                        // If the current line is a prefix of the new text, we can just append the new text.
                        self.current_line_for_redaction
                            .push_str(&text[self.current_line_for_redaction.len()..]);
                    } else {
                        // If the current line is not a prefix of the new text, we need to clear the current line and start over.
                        self.current_line_for_redaction.clear();
                        self.current_line_for_redaction.push_str(text);
                        self.last_scanned_secret_redaction_byte_index = 0;
                        self.last_word_to_rescan_for_redaction.clear();
                        start_of_last_word_byte_index = 0;

                        self.detected_secrets.retain(|location, _| {
                            if let TextLocation::Output {
                                section_index: cur_step_index,
                                line_index: cur_line_index,
                            } = location
                            {
                                // Clear all secrets of the current line in the current step.
                                !(*cur_step_index == section_index && *cur_line_index == line_index)
                            } else {
                                true
                            }
                        });
                    }

                    // Combine the last word in last word buffer with the new text to handle
                    // the case where a secret is split across streaming chunks.
                    let combined_text = format!(
                        "{}{}",
                        &self.last_word_to_rescan_for_redaction,
                        &text[self.last_scanned_secret_redaction_byte_index..]
                    );
                    let secret_ranges_with_levels =
                        find_secrets_in_text_with_levels(&combined_text);

                    for (secret_range, secret_level) in secret_ranges_with_levels {
                        // Adjust the ranges to map correctly within the new text.
                        let adjusted_byte_start =
                            start_of_last_word_byte_index + secret_range.byte_range.start;
                        let adjusted_byte_end =
                            start_of_last_word_byte_index + secret_range.byte_range.end;
                        let adjusted_char_start = text[..adjusted_byte_start].chars().count();
                        let adjusted_char_end = text[..adjusted_byte_end].chars().count();

                        let adjusted_secret_range = SecretRange {
                            char_range: adjusted_char_start..adjusted_char_end,
                            byte_range: adjusted_byte_start..adjusted_byte_end,
                        };

                        if let Some(secret_text) = text.get(adjusted_byte_start..adjusted_byte_end)
                        {
                            self.detected_secrets
                                .entry(TextLocation::Output {
                                    section_index,
                                    line_index,
                                })
                                .or_default()
                                .detected_secrets
                                .insert(
                                    adjusted_secret_range,
                                    Secret {
                                        secret: secret_text.to_string(),
                                        is_obfuscated: should_obfuscate,
                                        mouse_state: Default::default(),
                                        secret_level,
                                    },
                                );
                        }
                    }

                    // Update the last scanned position to the end of the current text.
                    self.last_scanned_secret_redaction_byte_index = text.len();

                    // Extract and store the last word (whitespace-separated) in the last word buffer.
                    if let Some(last_space_byte_index) =
                        combined_text.rfind(|c: char| c.is_whitespace())
                    {
                        let slice_with_space = &combined_text[last_space_byte_index..];
                        let space_offset = slice_with_space
                            .chars()
                            .next()
                            .expect("The whitespace character should be present")
                            .len_utf8();
                        self.last_word_to_rescan_for_redaction =
                            combined_text[last_space_byte_index + space_offset..].to_string();
                    } else {
                        // If no whitespace is found, store the entire combined text as the prefix.
                        self.last_word_to_rescan_for_redaction = combined_text.clone();
                    }
                }
            }
        }
    }

    pub fn run_redaction_on_complete_output(&mut self, output: &AIAgentOutput) {
        // Delete all output secrets as we'll be rescanning the entire output.
        self.detected_secrets
            .retain(|location, _| !matches!(location, TextLocation::Output { .. }));
        for (section_index, section) in output
            .all_text()
            .flat_map(|text| text.sections.iter())
            .enumerate()
        {
            if let AIAgentTextSection::PlainText { text } = section {
                let texts = match &text.formatted_lines {
                    Some(text) => text.lines().iter().map(|line| line.raw_text()).collect(),
                    _ => vec![text.text()],
                };
                for (line_index, text) in texts.iter().enumerate() {
                    let secret_ranges_with_levels = find_secrets_in_text_with_levels(text);
                    for (secret_range, secret_level) in secret_ranges_with_levels {
                        if let Some(secret_text) =
                            text.get(secret_range.byte_range.start..secret_range.byte_range.end)
                        {
                            self.detected_secrets
                                .entry(TextLocation::Output {
                                    section_index,
                                    line_index,
                                })
                                .or_default()
                                .detected_secrets
                                .insert(
                                    secret_range,
                                    Secret {
                                        secret: secret_text.to_string(),
                                        is_obfuscated: true,
                                        mouse_state: Default::default(),
                                        secret_level,
                                    },
                                );
                        }
                    }
                }
            }
        }
    }

    fn rerun_secret_detection_for_output_line(
        &mut self,
        section_index: usize,
        line_index: usize,
        text: &AgentOutputText,
    ) {
        let text_line = text
            .formatted_lines
            .as_ref()
            .and_then(|lines| lines.lines().get(line_index).map(|line| line.raw_text()));

        let entry_location = TextLocation::Output {
            section_index,
            line_index,
        };
        self.detected_secrets.remove(&entry_location);

        if let Some(line_text) = text_line {
            let secret_ranges_with_levels = find_secrets_in_text_with_levels(line_text);
            for (secret_range, secret_level) in secret_ranges_with_levels {
                // No adjustment is needed - we're redoing this entire line
                if let Some(secret_text) = line_text.get(secret_range.byte_range.clone()) {
                    self.detected_secrets
                        .entry(entry_location)
                        .or_default()
                        .detected_secrets
                        .insert(
                            secret_range,
                            Secret {
                                secret: secret_text.to_string(),
                                is_obfuscated: true,
                                mouse_state: Default::default(),
                                secret_level,
                            },
                        );
                }
            }
        }
    }
}

pub(crate) fn redact_secrets_in_element<T: PartialClickableElement>(
    mut element: T,
    detected_secrets: &DetectedSecretsInTextLocation,
    location: TextLocation,
    should_hide: bool,
) -> T {
    // Collect the secrets into a Vec, so we can reverse sort them by starting byte position.
    let secrets: std::iter::Rev<std::vec::IntoIter<(SecretRange, Secret)>> = detected_secrets
        .detected_secrets
        .iter()
        .map(|(range, hoverable)| (range.clone(), hoverable.clone()))
        .sorted_by_key(|(detected_secret_range, _)| detected_secret_range.byte_range.start)
        .rev();

    // Process the secrets in reverse order to avoid issues where we replace multibyte characters with single-width asterisks, changing
    // indices of subsequent secrets.
    for (detected_secret_range, hoverable_secret) in secrets {
        let detected_secret_range_click_clone = detected_secret_range.clone();
        element = element.with_clickable_char_range(
            detected_secret_range_click_clone.char_range.clone(),
            move |_modifiers, ctx, _app| {
                ctx.dispatch_typed_action(AIBlockAction::OpenSecretTooltip {
                    secret_range: detected_secret_range_click_clone.clone(),
                    location,
                });
            },
        );

        if hoverable_secret.is_obfuscated && should_hide {
            element.replace_text_range(
                detected_secret_range.clone(),
                SECRET_REDACTION_REPLACEMENT_CHARACTER
                    .repeat(
                        // Use character range length here! Even wide characters should be replaced by a single *, in rich content
                        // block secret redaction e.g. 码1234 -> *1234 not **1234.
                        detected_secret_range.char_range.end
                            - detected_secret_range.char_range.start,
                    )
                    .into(),
            );
        }

        let detected_secret_range_hover_clone = detected_secret_range.clone();
        element = element.with_hoverable_char_range(
            detected_secret_range_hover_clone.char_range.clone(),
            hoverable_secret.mouse_state.clone(),
            Some(Cursor::PointingHand),
            move |is_hovering, ctx, _app| {
                ctx.dispatch_typed_action(AIBlockAction::ChangedHoverOnSecret {
                    secret_range: detected_secret_range_hover_clone.clone(),
                    location,
                    is_hovering,
                })
            },
        );
    }
    element
}

#[cfg(test)]
#[path = "secret_redaction_tests.rs"]
mod test;
