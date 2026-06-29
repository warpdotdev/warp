#![cfg_attr(target_family = "wasm", allow(dead_code, unused_imports))]
// Adding this file level gate as some of the code around editability is not used in WASM yet.

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use futures::stream::AbortHandle;
use itertools::Itertools;
use pathfinder_color::ColorU;
use rangemap::RangeMap;
use similar::{ChangeTag, DiffOp, TextDiff};
use string_offset::CharOffset;
use warp_core::ui::theme::{AnsiColorIdentifier, Fill};
use warp_editor::content::edit::TemporaryBlock;
use warp_editor::content::version::BufferVersion;
use warp_editor::multiline::{AnyMultilineString, MultilineStr, MultilineString, LF};
use warp_editor::render::model::{Decoration, LineCount};
use warpui::{Entity, ModelContext};

use super::super::DiffResult;
use crate::appearance::Appearance;
use crate::code::editor::line::EditorLineLocation;
use crate::code::editor::line_iterator::LineIterator;

const OVERLAY_ALPHA: u8 = 56;
const INLINE_OVERLAY_ALPHA: u8 = 71;

/// Get the theme-appropriate add color
pub(crate) fn add_color(appearance: &Appearance) -> ColorU {
    AnsiColorIdentifier::Green
        .to_ansi_color(&appearance.theme().terminal_colors().normal)
        .into()
}

/// Get the theme-appropriate remove color
pub(crate) fn remove_color(appearance: &Appearance) -> ColorU {
    AnsiColorIdentifier::Red
        .to_ansi_color(&appearance.theme().terminal_colors().normal)
        .into()
}

/// Get the theme-appropriate replace color
pub(crate) fn replace_color(appearance: &Appearance) -> ColorU {
    AnsiColorIdentifier::Yellow
        .to_ansi_color(&appearance.theme().terminal_colors().normal)
        .into()
}

/// Get the theme-appropriate remove overlay color
pub(crate) fn remove_overlay_color(appearance: &Appearance) -> ColorU {
    let ansi_color =
        AnsiColorIdentifier::Red.to_ansi_color(&appearance.theme().terminal_colors().normal);
    let mut color: ColorU = ansi_color.into();
    color.a = OVERLAY_ALPHA;
    color
}

/// Get the theme-appropriate add overlay color
pub(crate) fn add_overlay_color(appearance: &Appearance) -> ColorU {
    let ansi_color =
        AnsiColorIdentifier::Green.to_ansi_color(&appearance.theme().terminal_colors().normal);
    let mut color: ColorU = ansi_color.into();
    color.a = OVERLAY_ALPHA;
    color
}

/// Get the theme-appropriate add inline overlay color
pub(crate) fn add_inline_overlay_color(appearance: &Appearance) -> ColorU {
    let ansi_color =
        AnsiColorIdentifier::Green.to_ansi_color(&appearance.theme().terminal_colors().normal);
    let mut color: ColorU = ansi_color.into();
    color.a = INLINE_OVERLAY_ALPHA;
    color
}

/// Get the theme-appropriate remove inline overlay color
pub(crate) fn remove_inline_overlay_color(appearance: &Appearance) -> ColorU {
    let ansi_color =
        AnsiColorIdentifier::Red.to_ansi_color(&appearance.theme().terminal_colors().normal);
    let mut color: ColorU = ansi_color.into();
    color.a = INLINE_OVERLAY_ALPHA;
    color
}

pub enum DiffModelEvent {
    DiffUpdated {
        version: BufferVersion,
        should_recalculate_hidden_lines: bool,
    },
    UnifiedDiffComputed(Rc<DiffResult>),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ChangeType {
    Replacement {
        replaced_range: Range<usize>,
        insertion: Vec<Range<usize>>,
        deletion: Vec<Range<usize>>,
    },
    Addition,
}

#[derive(Debug, Default, Clone)]
pub struct DiffStatus {
    /// A non-deletion change that maps the current range of content to an old range in base.
    change_mapping: RangeMap<usize, ChangeType>,
    /// A deletion that maps a line index in current content to an old range in base.
    deletion_mapping: HashMap<usize, Range<usize>>,
}

impl DiffStatus {
    /// Returns the number of lines added and removed in the current diff.
    pub fn get_diff_lines(&self) -> (usize, usize) {
        let mut lines_added = 0;
        let mut lines_removed = 0;

        // Count changes/additions
        for (new_range, change_type) in self.change_mapping.iter() {
            let new_range_lines =
                (LineCount::from(new_range.end) - LineCount::from(new_range.start)).as_usize();
            match change_type {
                ChangeType::Addition => lines_added += new_range_lines,
                ChangeType::Replacement { replaced_range, .. } => {
                    let old_range_lines = (LineCount::from(replaced_range.end)
                        - LineCount::from(replaced_range.start))
                    .as_usize();
                    lines_added += new_range_lines;
                    lines_removed += old_range_lines;
                }
            }
        }

        // Count deletions
        for range in self.deletion_mapping.values() {
            lines_removed += (LineCount::from(range.end) - LineCount::from(range.start)).as_usize();
        }

        (lines_added, lines_removed)
    }

    /// Retrieve the hunk to render for a given line number.
    pub fn diff_hunk(
        &self,
        line_num: LineCount,
        appearance: &Appearance,
    ) -> Option<DiffHunkDisplay> {
        let line_num = line_num.as_usize();
        if self.deletion_mapping.contains_key(&line_num) {
            return Some(DiffHunkDisplay::Remove(remove_color(appearance)));
        }

        match self.change_mapping.get(&line_num) {
            Some(ChangeType::Replacement { .. }) => Some(DiffHunkDisplay::Replacement {
                collapsed_color: replace_color(appearance),
                add_color: add_color(appearance),
                remove_color: remove_color(appearance),
            }),
            Some(ChangeType::Addition) => Some(DiffHunkDisplay::Add(add_color(appearance))),
            None => None,
        }
    }

    /// Return the range of diff hunk lines (if any) containing the given line.
    /// line_count is the line count assigned in the EditorWrapper element.
    /// This is 0-indexed.
    /// For deleted diff hunks, all lines will have the line_count of the line directly after
    ///   the deleted section in the new current state of the file.
    /// For changed diff hunks, all lines in the old version (removed section of the hunk)
    ///   will have the line number of the first line in the new version of the hunk.
    pub fn removed_diff_range(&self, line_count: LineCount) -> Option<Range<LineCount>> {
        let line_num = line_count.as_usize();
        if self.deletion_mapping.contains_key(&(line_num)) {
            return Some(LineCount::from(line_num)..LineCount::from(line_num));
        }

        // Check if this is a replacement (change with removed lines)
        self.added_diff_range(line_count)
    }

    /// Return the range of diff hunk lines (if any) containing the given line.
    /// line_count is the line count assigned in the EditorWrapper element.
    /// This is 0-indexed.
    pub fn added_diff_range(&self, line_num: LineCount) -> Option<Range<LineCount>> {
        let line_num = line_num.as_usize();
        self.change_mapping
            .get_key_value(&line_num)
            .map(|(key, _)| LineCount::from(key.start)..LineCount::from(key.end))
    }
}

/// The colors used to represent the diff hunks in the editor.
#[derive(Debug, Clone, Copy)]
pub enum DiffHunkDisplay {
    Add(ColorU),
    Replacement {
        collapsed_color: ColorU,
        add_color: ColorU,
        remove_color: ColorU,
    },
    Remove(ColorU),
}

/// A single renderable diff hunk. It contains the information needed to decorate the render model.
///
/// Added/changed line ranges are kept as **logical** (buffer) line ranges here;
/// the consumer ([`CodeEditorModel::refresh_diff_state`]) converts them to the
/// char-offset–anchored [`LineDecoration`]s used for rendering, since that
/// conversion needs access to the live buffer.
#[derive(Debug, Clone)]
pub enum RenderableDiffHunk {
    Add {
        line_range: Range<usize>,
        overlay: Fill,
    },
    Replace {
        line_range: Range<usize>,
        overlay: Fill,
        inline_highlights: Vec<(usize, Range<usize>)>,
        removed_lines: Vec<TemporaryBlock>,
    },
    Deletion {
        removed_lines: Vec<TemporaryBlock>,
    },
}

/// Model that tracks the line-by-line diff status of the editor content.
pub struct DiffModel {
    /// Store base in an Arc to avoid cloning the underlying data on every content change.
    base: Option<Arc<MultilineString<LF>>>,
    status: DiffStatus,
    abort_handle: Option<(AbortHandle, BufferVersion)>,
}

impl DiffModel {
    pub fn new() -> Self {
        Self {
            base: None,
            status: DiffStatus::default(),
            abort_handle: None,
        }
    }

    /// Total number of diff hunks in the current diff model.
    pub fn diff_hunk_count(&self) -> usize {
        self.status.change_mapping.len() + self.status.deletion_mapping.len()
    }

    /// Given an index of a diff hunk, expand it to the range of lines the hunk describes
    /// in the current buffer.
    pub fn line_range_by_diff_hunk_index(&self, index: usize) -> Option<Range<usize>> {
        self.added_or_changed_lines()
            .chain(
                self.status
                    .deletion_mapping
                    .keys()
                    .map(|index| *index..*index),
            )
            .sorted_by(|a, b| Ord::cmp(&a.start, &b.start))
            .nth(index)
    }

    /// Get a single renderable diff hunk by its index.
    pub fn renderable_diff_hunk_by_index<'a>(
        &self,
        index: usize,
        lines: &mut LineIterator<'a, impl Iterator<Item = &'a str>>,
        appearance: &Appearance,
    ) -> Option<RenderableDiffHunk> {
        let (range, is_addition) = self.diff_by_index(index)?;

        if !is_addition {
            let deleted_range = self.status.deletion_mapping.get(&range.start)?;
            let mut removed_lines = Vec::with_capacity(deleted_range.len());
            if let Ok(lines_in_range) = lines.lines_in_range(deleted_range) {
                for line in lines_in_range {
                    let mut content = line.to_string();
                    if !content.ends_with('\n') {
                        content.push('\n');
                    }

                    removed_lines.push(TemporaryBlock {
                        content,
                        insert_before: LineCount::from(range.start),
                        line_decoration: Some(remove_overlay_color(appearance).into()),
                        inline_text_decorations: Vec::new(),
                    });
                }
            }

            Some(RenderableDiffHunk::Deletion { removed_lines })
        } else {
            match self.status.change_mapping.get(&range.start)? {
                ChangeType::Replacement {
                    replaced_range,
                    insertion,
                    deletion,
                } => {
                    let mut removed_lines = Vec::new();

                    // Inline highlight indices are given over the entire multiline range.
                    // We need to split them into per-line decorations.
                    let mut start_char = 0;
                    if let Ok(lines_in_range) = lines.lines_in_range(replaced_range) {
                        for line in lines_in_range {
                            let mut content = line.to_string();
                            if !content.ends_with('\n') {
                                content.push('\n');
                            }

                            let inline_text_decorations = deletion
                                .iter()
                                .filter_map(|inline| {
                                    let line_start = start_char;
                                    let line_end = start_char + line.chars().count();

                                    let overlap_start = inline.start.max(line_start);
                                    let overlap_end = inline.end.min(line_end);

                                    if overlap_start < overlap_end {
                                        Some(
                                            Decoration::new(
                                                CharOffset::from(overlap_start - line_start),
                                                CharOffset::from(overlap_end - line_start),
                                            )
                                            .with_background(Fill::Solid(
                                                remove_inline_overlay_color(appearance),
                                            )),
                                        )
                                    } else {
                                        None
                                    }
                                })
                                .collect_vec();

                            removed_lines.push(TemporaryBlock {
                                content,
                                insert_before: LineCount::from(range.start),
                                line_decoration: Some(remove_overlay_color(appearance).into()),
                                inline_text_decorations,
                            });

                            start_char += line.chars().count() + 1;
                        }
                    }

                    Some(RenderableDiffHunk::Replace {
                        line_range: range.start..range.end,
                        overlay: add_overlay_color(appearance).into(),
                        inline_highlights: insertion
                            .iter()
                            .map(|inline_change| (range.start, inline_change.clone()))
                            .collect(),
                        removed_lines,
                    })
                }
                ChangeType::Addition => Some(RenderableDiffHunk::Add {
                    line_range: range.start..range.end,
                    overlay: add_overlay_color(appearance).into(),
                }),
            }
        }
    }

    /// Returns the number of diff hunks before the given line number.
    pub fn diff_hunk_count_before_line(&self, line: usize) -> usize {
        self.added_or_changed_lines()
            .chain(
                self.status
                    .deletion_mapping
                    .keys()
                    .map(|index| *index..*index + 1),
            )
            .filter(|range| range.start < line)
            .count()
    }

    /// Given a diff hunk index, calculate what is the reverse action that undo this diff.
    pub fn reverse_action_by_diff_hunk_index(
        &self,
        index: usize,
    ) -> Option<(Range<usize>, String)> {
        let line_range = self.line_range_by_diff_hunk_index(index)?;

        if let Some(replaced_range) = self.status.deletion_mapping.get(&line_range.start) {
            let text = self.base_text_by_line_range(replaced_range)?;
            return Some((line_range, text));
        }

        if let Some(change) = self.status.change_mapping.get(&line_range.start) {
            return match change {
                ChangeType::Addition => Some((line_range, "".to_string())),
                ChangeType::Replacement { replaced_range, .. } => {
                    let text = self.base_text_by_line_range(replaced_range)?;
                    return Some((line_range, text));
                }
            };
        }

        None
    }

    /// Given a range of lines, return the corresponding substring in the base text.
    /// Note that all lines will end with a trailing newline.
    fn base_text_by_line_range(&self, replaced_range: &Range<usize>) -> Option<String> {
        let base_text = self.base.as_ref()?;
        let mut text = base_text
            .lines()
            .skip(replaced_range.start)
            .take(replaced_range.len())
            .join("\n");

        text.push('\n');
        Some(text)
    }

    /// Returns the content of a deleted line given the location of a removed line.
    /// Returns None if the info is not for a removed line or if any lookup fails.
    pub fn deleted_line_content(&self, info: &EditorLineLocation) -> Option<String> {
        let index = self.deleted_line_to_base_line_index(info)?;
        self.base_line(index)
    }

    /// Convert a deleted line location to a line index in the base version of the text.
    pub fn deleted_line_to_base_line_index(&self, info: &EditorLineLocation) -> Option<usize> {
        // Extract line_number and index from Removed variant
        let (line_number, index) = match info {
            EditorLineLocation::Removed {
                line_number, index, ..
            } => (line_number.as_usize(), *index),
            _ => return None,
        };

        // First check if this is a pure deletion. Note that deletion maps to line AFTER the removed line.
        // CODE-1638: Use line_number + 1 to align with deletion_mapping's off-by-one convention.
        if let Some(removed_range) = self.status.deletion_mapping.get(&line_number) {
            return Some(removed_range.start + index);
        }

        // Check if this is a replacement (change with removed lines)
        if let Some(ChangeType::Replacement { replaced_range, .. }) =
            self.status.change_mapping.get(&line_number)
        {
            return Some(replaced_range.start + index);
        }

        None
    }

    /// Convert a line index in the base version of the text to an editor line location.
    pub fn base_line_index_to_line_location(&self, index: usize) -> Option<EditorLineLocation> {
        for (line_range, change) in self.status.change_mapping.iter() {
            if let ChangeType::Replacement { replaced_range, .. } = change {
                if replaced_range.contains(&index) {
                    return Some(EditorLineLocation::Removed {
                        // Subtracting 1 as the diff is currently represented as attaching to the _previous line_.
                        line_number: LineCount::from(line_range.start),
                        line_range: LineCount::from(line_range.start)
                            ..LineCount::from(line_range.end),
                        index: index - replaced_range.start,
                    });
                }
            }
        }

        for (line_range, replaced_range) in self.status.deletion_mapping.iter() {
            if replaced_range.contains(&index) {
                return Some(EditorLineLocation::Removed {
                    line_number: LineCount::from(*line_range),
                    line_range: LineCount::from(*line_range)..LineCount::from(*line_range),
                    index: index - replaced_range.start,
                });
            }
        }

        None
    }

    pub fn is_line_added_or_changed(&self, line_num: &LineCount) -> bool {
        let line_num = line_num.as_usize();
        self.status.change_mapping.get(&line_num).is_some()
    }

    pub fn base_line(&self, line_index: usize) -> Option<String> {
        // Extract the line content
        let line = self.base.as_ref()?.lines().nth(line_index)?;
        Some(line.to_string())
    }

    pub fn base_line_count(&self) -> usize {
        self.base
            .as_ref()
            .map(|base| base.lines().count())
            .unwrap_or_default()
    }

    /// Returns an iterator over all the line numbers in the current buffer that are
    /// added or changed from the base buffer.
    pub fn added_or_changed_lines(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.status
            .change_mapping
            .iter()
            .map(|(row_range, _)| row_range.clone())
    }

    pub fn modified_lines(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.added_or_changed_lines().chain(
            self.status
                .deletion_mapping
                .keys()
                .map(|index| *index..*index + 1),
        )
    }

    fn diff_by_index(&self, index: usize) -> Option<(Range<usize>, bool)> {
        self.added_or_changed_lines()
            .map(|range| (range, true))
            .chain(
                self.status
                    .deletion_mapping
                    .keys()
                    .map(|index| (*index..*index + 1, false)),
            )
            .sorted_by(|a, b| Ord::cmp(&a.0.start, &b.0.start))
            .nth(index)
    }

    pub fn diff_status(&self) -> &DiffStatus {
        &self.status
    }

    pub fn set_base(&mut self, base: MultilineString<LF>) {
        self.base = Some(Arc::new(base));
        if let Some((abort_handle, _)) = self.abort_handle.take() {
            abort_handle.abort();
        }
    }

    pub fn base(&self) -> Option<Arc<MultilineString<LF>>> {
        self.base.clone()
    }

    /// Given the new content, compute the new set of diff contents.
    pub fn compute_diff(
        &mut self,
        new: MultilineString<LF>,
        should_recalculate_hidden_lines: bool,
        version: BufferVersion,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some((abort_handle, current_version)) = self.abort_handle.take() {
            // Do not abort a diff computation for the same buffer version. Early return instead.
            if current_version != version {
                abort_handle.abort();
            } else {
                self.abort_handle = Some((abort_handle, version));
                return;
            }
        }

        let Some(base_text) = self.base.clone() else {
            return;
        };

        let handle = ctx
            .spawn(
                async move { Self::compute_diff_internal(&base_text, &new).await },
                move |model, (change_mapping, deletion_mapping), ctx| {
                    model.status.change_mapping = change_mapping;
                    model.status.deletion_mapping = deletion_mapping;
                    log::debug!("diff status updated: {:#?}", &model.status);
                    ctx.emit(DiffModelEvent::DiffUpdated {
                        should_recalculate_hidden_lines,
                        version,
                    });
                },
            )
            .abort_handle();

        self.abort_handle = Some((handle, version));
    }

    #[cfg(test)]
    async fn compute_diff_for_test(&mut self, new: String) {
        let Some(base_text) = self.base.clone() else {
            return;
        };
        let new = AnyMultilineString::infer(new);

        let (change_mapping, deletion_mapping) =
            Self::compute_diff_internal(&base_text, new.to_format().as_ref()).await;
        self.status.change_mapping = change_mapping;
        self.status.deletion_mapping = deletion_mapping;
    }

    pub fn retrieve_unified_diff(
        &mut self,
        new: AnyMultilineString,
        file_name: String,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(base_text) = self.base.clone() else {
            return;
        };

        ctx.spawn(
            async move {
                let new = new.to_format();
                Self::retrieve_unified_diff_internal(&base_text, new.as_ref(), file_name.as_str())
                    .await
            },
            |_, unified_diff, ctx| {
                ctx.emit(DiffModelEvent::UnifiedDiffComputed(Rc::new(unified_diff)));
            },
        );
    }

    async fn retrieve_unified_diff_internal(
        base: &MultilineStr<LF>,
        new: &MultilineStr<LF>,
        file_name: &str,
    ) -> DiffResult {
        if base == new {
            return DiffResult {
                unified_diff: String::new(),
                lines_added: 0,
                lines_removed: 0,
            };
        }

        // Show 3 context lines (standard of git diff).
        let text_diff = TextDiff::from_lines(base.as_str(), new.as_str());

        // Calculate diff statistics.
        let mut lines_added = 0;
        let mut lines_removed = 0;

        for op in text_diff.ops() {
            match op {
                DiffOp::Equal { .. } => (),
                DiffOp::Delete { old_len, .. } => lines_removed += old_len,
                DiffOp::Insert { new_len, .. } => lines_added += new_len,
                DiffOp::Replace {
                    old_len, new_len, ..
                } => {
                    lines_added += new_len;
                    lines_removed += old_len;
                }
            }
        }

        DiffResult {
            unified_diff: text_diff
                .unified_diff()
                .context_radius(3)
                .header(file_name, file_name)
                .missing_newline_hint(false)
                .to_string(),
            lines_added,
            lines_removed,
        }
    }

    async fn compute_diff_internal(
        base: &MultilineStr<LF>,
        new: &MultilineStr<LF>,
    ) -> (RangeMap<usize, ChangeType>, HashMap<usize, Range<usize>>) {
        let diffs = TextDiff::configure()
            .algorithm(similar::Algorithm::Patience)
            .diff_lines(base.as_str(), new.as_str());
        let mut deletion_mapping = HashMap::new();
        let mut change_mapping = RangeMap::new();

        for change in diffs.ops() {
            futures_lite::future::yield_now().await;
            match change {
                DiffOp::Equal { .. } => continue,
                DiffOp::Delete {
                    old_index,
                    old_len,
                    new_index,
                } => {
                    deletion_mapping.insert(*new_index, *old_index..*old_index + *old_len);
                }
                DiffOp::Insert {
                    new_index, new_len, ..
                } => change_mapping.insert(*new_index..*new_index + *new_len, ChangeType::Addition),
                DiffOp::Replace {
                    old_index,
                    old_len,
                    new_index,
                    new_len,
                } => {
                    let (inline_deletion, inline_insertion) = record_replacement(change, &diffs);

                    change_mapping.insert(
                        *new_index..*new_index + *new_len,
                        ChangeType::Replacement {
                            replaced_range: *old_index..*old_index + *old_len,
                            insertion: inline_insertion,
                            deletion: inline_deletion,
                        },
                    )
                }
            }
        }

        coalesce_replacements(&diffs, &mut deletion_mapping, &mut change_mapping);

        (change_mapping, deletion_mapping)
    }
}

/// `similar` can represent a logical replacement as separate Delete + Insert ops at the same
/// `new_index`. When that happens, we end up with a deletion hunk and an addition hunk for
/// the same logical change. Coalesce those into a single Replacement entry.
fn coalesce_replacements<'a>(
    diffs: &'a TextDiff<'a, 'a, 'a, str>,
    deletion_mapping: &mut HashMap<usize, Range<usize>>,
    change_mapping: &mut RangeMap<usize, ChangeType>,
) {
    let mut deletions_to_remove = Vec::new();
    let mut additions_to_remove = Vec::new();
    let mut replacements_to_insert = Vec::new();

    for (&new_index, old_range) in deletion_mapping.iter() {
        let Some((new_range, change)) = change_mapping.get_key_value(&new_index) else {
            continue;
        };

        if new_range.start != new_index {
            continue;
        }

        if !matches!(change, ChangeType::Addition) {
            continue;
        }

        let old_len = old_range.len();
        let new_len = new_range.len();

        let replace_op = DiffOp::Replace {
            old_index: old_range.start,
            old_len,
            new_index,
            new_len,
        };
        let (inline_deletion, inline_insertion) = record_replacement(&replace_op, diffs);

        deletions_to_remove.push(new_index);
        additions_to_remove.push(new_range.clone());
        replacements_to_insert.push((
            new_range.clone(),
            ChangeType::Replacement {
                replaced_range: old_range.clone(),
                insertion: inline_insertion,
                deletion: inline_deletion,
            },
        ));
    }

    for new_range in additions_to_remove {
        change_mapping.remove(new_range);
    }
    deletion_mapping.retain(|new_index, _| !deletions_to_remove.contains(new_index));
    change_mapping.extend(replacements_to_insert);
}

/// Given a replace operation, iterate through its associated diff hunks and collect
/// the deletions and insertions from the generated diff.
/// Return order: (deletions, insertions)
fn record_replacement<'a>(
    replace_op: &DiffOp,
    diffs: &'a TextDiff<'a, 'a, 'a, str>,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let mut old_offset = 0;
    let mut new_offset = 0;
    let mut inline_deletion = Vec::new();
    let mut inline_insertion = Vec::new();

    for inline_diff in diffs.iter_inline_changes(replace_op) {
        match inline_diff.tag() {
            ChangeTag::Equal => {
                log::warn!("Unexpected equal change tag in a replace op");
                continue;
            }
            ChangeTag::Delete => {
                record_inline_diff_as_changes(&mut inline_deletion, &mut old_offset, inline_diff);
            }
            ChangeTag::Insert => {
                record_inline_diff_as_changes(&mut inline_insertion, &mut new_offset, inline_diff);
            }
        }
    }

    (inline_deletion, inline_insertion)
}

fn record_inline_diff_as_changes(
    changes: &mut Vec<Range<usize>>,
    offset: &mut usize,
    inline_diff: similar::InlineChange<str>,
) {
    for (highlight, val) in inline_diff.values() {
        let char_len = val.chars().count();

        if *highlight {
            changes.push(*offset..*offset + char_len);
        }
        *offset += char_len;
    }
}

impl Entity for DiffModel {
    type Event = DiffModelEvent;
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod tests;
