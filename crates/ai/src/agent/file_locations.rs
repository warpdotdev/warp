use std::collections::HashMap;
use std::ops::Range;

use itertools::Itertools as _;
use warp_terminal::shell::ShellLaunchData;

use crate::agent::action_result::FileContext;
use crate::index::locations::CodeContextLocation;
use crate::paths::shell_native_absolute_path;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileLocations {
    pub name: String,
    pub lines: Vec<Range<usize>>,
}

impl FileLocations {
    /// Convert file locations to a user readable format.
    pub fn to_user_message(
        &self,
        shell_launch_data: Option<&ShellLaunchData>,
        current_working_directory: Option<&String>,
        file_line_count: Option<usize>,
    ) -> String {
        let absolute_path =
            shell_native_absolute_path(&self.name, shell_launch_data, current_working_directory);

        if self.lines.is_empty() {
            return absolute_path;
        }

        let line_ranges = self
            .lines
            .iter()
            .filter_map(|range| {
                let (start, end) = match file_line_count {
                    Some(line_count) => (
                        std::cmp::min(range.start, line_count),
                        std::cmp::min(range.end, line_count),
                    ),
                    None => (range.start, range.end),
                };

                if start == 1 && Some(end) == file_line_count {
                    // don't show ranges that are just the entire file
                    None
                } else {
                    Some(format!("{start}-{end}"))
                }
            })
            .collect_vec();

        if line_ranges.is_empty() {
            absolute_path
        } else {
            format!("{} ({})", absolute_path, line_ranges.join(", "))
        }
    }

    /// Expands the line ranges (if any) in both directions by `context_line`. Then the line ranges are sorted
    /// and merged if there are overlaps.
    pub fn expand_surrounding_context(&mut self, context_line: usize) {
        if self.lines.is_empty() {
            return;
        }

        // Expand each range by context_line in both directions
        let mut expanded: Vec<Range<usize>> = self
            .lines
            .iter()
            .map(|r| {
                let start = r.start.saturating_sub(context_line);
                let end = r.end + context_line;
                start..end
            })
            .collect();

        // Sort by start
        expanded.sort_by_key(|r| r.start);

        // Merge overlapping or adjacent ranges
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(expanded.len());
        for range in expanded {
            if let Some(last) = merged.last_mut() {
                if range.start <= last.end {
                    last.end = last.end.max(range.end);
                } else {
                    merged.push(range);
                }
            } else {
                merged.push(range);
            }
        }
        self.lines = merged;
    }
}

impl From<&CodeContextLocation> for FileLocations {
    fn from(location: &CodeContextLocation) -> Self {
        match location {
            CodeContextLocation::WholeFile(path) => Self {
                name: path.to_string_lossy().to_string(),
                lines: vec![],
            },
            CodeContextLocation::Fragment(fragment) => Self {
                name: fragment.path.to_string_lossy().to_string(),
                lines: fragment.line_ranges.clone(),
            },
        }
    }
}

/// Groups a slice of [`FileContext`]s by file name, collecting line ranges from
/// fragments of the same file. Returns one display string per unique file,
/// preserving first-occurrence order.
pub fn group_file_contexts_for_display(
    file_contexts: &[FileContext],
    shell_launch_data: Option<&ShellLaunchData>,
    current_working_directory: Option<&String>,
) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<Range<usize>>> = HashMap::new();

    for fc in file_contexts {
        let entry = groups.entry(fc.file_name.clone()).or_insert_with(|| {
            order.push(fc.file_name.clone());
            Vec::new()
        });
        if let Some(range) = &fc.line_range {
            entry.push(range.clone());
        }
    }

    order
        .iter()
        .map(|file_name| {
            let ranges = groups.get(file_name).unwrap();
            let mut sorted = ranges.clone();
            sorted.sort_by_key(|r| (r.start, r.end));
            let locations = FileLocations {
                name: file_name.clone(),
                lines: sorted,
            };
            locations.to_user_message(shell_launch_data, current_working_directory, None)
        })
        .collect()
}
