use std::collections::HashMap;
use std::ops::Range;
use string_offset::ByteOffset;
use urlocator::{UrlLocation, UrlLocator};
use warpui::elements::PartialClickableElement;

use warpui::platform::Cursor;

use crate::ai::agent::{AIAgentActionType, AIAgentOutput, AIAgentTextSection, ReadFilesRequest};
use crate::ai::blocklist::block::view_impl::output::LinkActionConstructors;
use crate::ai::blocklist::block::TextLocation;
use crate::terminal::links::should_directly_open_link;
use crate::terminal::model::grid::grid_handler::is_file_link_separator;
use crate::terminal::ShellLaunchData;
use warpui::elements::MouseStateHandle;
use warpui::text::char_slice;
use warpui::Action;

cfg_if::cfg_if! {
    if #[cfg(feature = "local_fs")] {
        use std::collections::HashSet;
        use std::path::Path;
        use std::path::PathBuf;
        use warp_util::path::CleanPathResult;
    }
}

pub const RICH_CONTENT_LINK_FIRST_CHAR_POSITION_ID: &str =
    "ai_block:rich_content_link_first_char_position";

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct LinkLocation {
    pub(crate) link_range: Range<usize>,
    pub(crate) location: TextLocation,
}

#[derive(Debug, Default)]
pub(crate) struct DetectedLinksState {
    pub(crate) detected_links_by_location: HashMap<TextLocation, DetectedLinksInTextLocation>,
    // The link that the mouse is currently hovered over.
    pub(crate) currently_hovered_link_location: Option<LinkLocation>,
    // The link that a tooltip is currently open for.
    // This is separate from currently_hovered_link because after clicking
    // on a link to open the tooltip, this link should remain highlighted and the tooltip in place
    // even if we hover over other links.
    pub(crate) link_location_open_tooltip: Option<LinkLocation>,
}

impl DetectedLinksState {
    /// Given a text location and char range, returns the detected link there if any.
    pub fn link_at(
        &self,
        location: &TextLocation,
        range: &Range<usize>,
    ) -> Option<&DetectedLinkType> {
        Some(
            &self
                .detected_links_by_location
                .get(location)?
                .detected_links
                .get(range)?
                .link,
        )
    }

    pub fn update_hovered_link(
        &mut self,
        is_hovering: bool,
        is_selecting: bool,
        link_range: &Range<usize>,
        location: &TextLocation,
    ) {
        if is_hovering && !is_selecting {
            self.currently_hovered_link_location = Some(LinkLocation {
                link_range: link_range.clone(),
                location: *location,
            });
        } else if self.currently_hovered_link_location.as_ref().is_some_and(
            |currently_hovered_link| {
                currently_hovered_link.link_range == *link_range
                    && currently_hovered_link.location == *location
            },
        ) {
            self.currently_hovered_link_location = None;
        }
    }

    /// Replaces all detected links with the given background detection results.
    pub(crate) fn replace_all_links(
        &mut self,
        all_links: HashMap<TextLocation, HashMap<Range<usize>, DetectedLinkType>>,
    ) {
        self.detected_links_by_location.clear();
        self.currently_hovered_link_location = None;
        self.link_location_open_tooltip = None;
        for (location, links) in all_links {
            let entry = self.detected_links_by_location.entry(location).or_default();
            for (range, link) in links {
                entry.detected_links.insert(
                    range,
                    HoverableDetectedLink {
                        link,
                        mouse_state: Default::default(),
                    },
                );
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum DetectedLinkType {
    Url(String),
    #[cfg(feature = "local_fs")]
    FilePath {
        absolute_path: PathBuf,
        line_and_column_num: Option<warp_util::path::LineAndColumnArg>,
    },
}

#[derive(Debug)]
pub(crate) struct HoverableDetectedLink {
    pub(crate) link: DetectedLinkType,
    pub(crate) mouse_state: MouseStateHandle,
}

#[derive(Debug, Default)]
pub(crate) struct DetectedLinksInTextLocation {
    pub(crate) detected_links: HashMap<Range<usize>, HoverableDetectedLink>,
}

pub(crate) fn add_link_detection_mouse_interactions<T: PartialClickableElement, A: Action>(
    mut element: T,
    detected_links_state: &DetectedLinksState,
    link_action_constructors: LinkActionConstructors<A>,
    location: TextLocation,
) -> T {
    if let Some(detected_links) = detected_links_state
        .detected_links_by_location
        .get(&location)
    {
        for (detected_link_range, hoverable_link) in &detected_links.detected_links {
            let detected_link_range_clone = detected_link_range.clone();
            element = element.with_clickable_char_range(
                detected_link_range_clone.clone(),
                move |modifiers, ctx, _app| {
                    if should_directly_open_link(modifiers) {
                        let action = (link_action_constructors.construct_open_link_action)(
                            detected_link_range_clone.clone(),
                            location,
                        );
                        ctx.dispatch_typed_action(action);
                    } else {
                        let action = (link_action_constructors.construct_open_link_tooltip_action)(
                            detected_link_range_clone.clone(),
                            location,
                        );
                        ctx.dispatch_typed_action(action);
                    }
                },
            );
            let detected_link_range_clone = detected_link_range.clone();
            element = element.with_hoverable_char_range(
                detected_link_range_clone.clone(),
                hoverable_link.mouse_state.clone(),
                Some(Cursor::PointingHand),
                move |is_hovering, ctx, _app| {
                    let action = (link_action_constructors.construct_changed_hover_on_link_action)(
                        detected_link_range_clone.clone(),
                        location,
                        is_hovering,
                    );
                    ctx.dispatch_typed_action(action);
                },
            );
        }
    }
    element
}

/// Returns the char ranges of detected URLs in the given text.
fn detect_urls(text: &str) -> Vec<Range<usize>> {
    let mut locator = UrlLocator::new();
    let mut url_ranges = vec![];
    let (mut start, mut end) = (None, None);
    for (i, c) in text.chars().enumerate() {
        // Reference to https://docs.rs/urlocator/latest/urlocator/#example-url-boundaries
        // We know we have fully parsed an url when the locator advances from the `UrlLocation::Url`
        // to the `UrlLocation::Reset` stage.
        match locator.advance(c) {
            UrlLocation::Url(length, end_offset) => {
                end = Some(1 + i - end_offset as usize);
                start = Some(end.unwrap() - length as usize);
            }
            UrlLocation::Reset => {
                if let Some((start, end)) = start.zip(end) {
                    url_ranges.push(start..end)
                }
                start = None;
                end = None;
            }
            _ => (),
        }
    }
    // If the last character completes a valid URL, add it.
    if let Some((start, end)) = start.zip(end) {
        url_ranges.push(start..end)
    }
    url_ranges
}

#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
fn addr_of(s: &str) -> usize {
    s.as_ptr() as usize
}

/// Maximum byte length of a token to search for file paths in. Used as a guard against scanning huge non-path tokens.
/// - Linux PATH_MAX: 4096 bytes.
/// - macOS PATH_MAX: 1024 bytes.
/// - Windows long-path cap: 32,767 UTF-16 units = 98,301 bytes.
const MAX_WORD_LEN_FOR_FILE_PATH: usize = 96 * 1024;
/// Maximum [`is_file_link_separator`] characters per token, to bound candidate substrings.
/// 256 keeps per-token allocations under ~1 MiB and is far above any real path.
const MAX_SEPARATORS_PER_WORD: usize = 256;

/// A separator's byte range in the original word.
///
/// File path candidates start after one separator and end before another. Using [`ByteOffset`]
/// keeps the byte-indexing semantics explicit when separators are multi-byte characters like
/// box-drawing glyphs.
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
type SeparatorByteRange = Range<ByteOffset>;

/// Returns separator byte ranges in `word`, framed by zero-width virtual separators at
/// the start and end of the word. Returns empty if either safety cap is exceeded.
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
fn separator_byte_ranges_for_file_path_search(word: &str) -> Vec<SeparatorByteRange> {
    if word.len() > MAX_WORD_LEN_FOR_FILE_PATH {
        return Vec::new();
    }
    // To include any substrings starting at the beginning of the word, we
    // pretend there's a zero-width separator before the first character.
    let mut separator_byte_ranges = vec![ByteOffset::zero()..ByteOffset::zero()];
    // We use char_indices() to get byte indices of each char which are used to index the string,
    // rather than chars().enumerate() would give char indices.
    for (i, c) in word.char_indices() {
        if is_file_link_separator(c) {
            if separator_byte_ranges.len() > MAX_SEPARATORS_PER_WORD {
                return Vec::new();
            }
            separator_byte_ranges.push(ByteOffset::from(i)..ByteOffset::from(i + c.len_utf8()));
        }
    }
    // Consider trailing periods to be separators. This is because
    // in natural language we might use a file path at the end of a sentence, and want
    // to detect them without including the trailing period. But trailing
    // periods can also be part of a valid file path.
    let word_ends_with_period = word.ends_with('.');
    if word_ends_with_period {
        separator_byte_ranges.push(ByteOffset::from(word.len() - 1)..ByteOffset::from(word.len()));
    }
    // To include any substrings ending at the end of the word, we pretend there's
    // a zero-width separator after the last character.
    separator_byte_ranges.push(ByteOffset::from(word.len())..ByteOffset::from(word.len()));
    separator_byte_ranges
}

/// Given a word with no whitespace in it, returns all the possible file paths within the word
/// from longest to shortest. File paths within a word can be split by [`is_file_link_separator`]
/// characters, and those separators may be part of file paths themselves.
/// Possible file paths begin after a separator and end before a separator.
/// For example, given /path/to/file:16:hello, it will return
/// ["/path/to/file:16:hello", "/path/to/file:16", "/path/to/file", "16:hello", "hello"]
///
/// Tokens exceeding [`MAX_WORD_LEN_FOR_FILE_PATH`] or [`MAX_SEPARATORS_PER_WORD`]
/// yield no candidates to bound the substring enumeration.
#[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
fn possible_file_paths_in_word(word: &str) -> impl Iterator<Item = &str> {
    let separator_byte_ranges = separator_byte_ranges_for_file_path_search(word);
    let mut possible_path_byte_ranges = vec![];
    for (i, start_separator) in separator_byte_ranges.iter().cloned().enumerate() {
        for end_separator in separator_byte_ranges.iter().skip(i + 1).cloned() {
            if start_separator.end < end_separator.start {
                possible_path_byte_ranges.push(start_separator.end..end_separator.start);
            }
        }
    }
    // Sort by longest to shortest.
    possible_path_byte_ranges.sort_by(|a, b| {
        (b.end.as_usize() - b.start.as_usize()).cmp(&(a.end.as_usize() - a.start.as_usize()))
    });
    possible_path_byte_ranges
        .into_iter()
        .map(|range| &word[range.start.as_usize()..range.end.as_usize()])
}

/// Returns a DetectedLink::FilePath if expanded_path is a valid path that actually exists on the file system.
#[cfg(feature = "local_fs")]
fn compute_valid_file_path(
    working_directory: &Path,
    expanded_path: &str,
    files_and_folders_in_working_directory: &HashSet<PathBuf>,
    shell_launch_data: Option<&crate::terminal::ShellLaunchData>,
) -> Option<DetectedLinkType> {
    use crate::util::file::{absolute_path_if_valid, ShellPathType};
    // Scan for line and column number in the current word (left + right).
    let cleaned_path = CleanPathResult::with_line_and_column_number(expanded_path);

    // First try to use the files_and_folders_in_working_directory cache.
    let path = Path::new(&cleaned_path.path);
    if let Some(relative_path) = files_and_folders_in_working_directory.get(path) {
        let absolute_path = working_directory.join(relative_path);
        return Some(DetectedLinkType::FilePath {
            absolute_path,
            line_and_column_num: cleaned_path.line_and_column_num,
        });
    } else if path.components().count() <= 1 {
        // If the path does not contain a separator and isn't in files_and_folders_in_working_directory,
        // we know it isn't a valid path. Return immediately to save a a file system call.
        return None;
    }

    // This does a file system lookup.
    let absolute_path = absolute_path_if_valid(
        &cleaned_path,
        ShellPathType::PlatformNative(working_directory.to_owned()),
        shell_launch_data,
    );

    absolute_path.map(|absolute_path| DetectedLinkType::FilePath {
        absolute_path,
        line_and_column_num: cleaned_path.line_and_column_num,
    })
}

/// Returns a set of all file and folder names in the given directory (relative, not absolute paths).
#[cfg(feature = "local_fs")]
fn get_files_and_folders_in_directory(directory: &Path) -> HashSet<PathBuf> {
    let mut files_and_folders = HashSet::new();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return files_and_folders;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        files_and_folders.insert(PathBuf::from(entry.file_name()));
    }
    files_and_folders
}

/// Returns the detected valid file paths in some text along with their char ranges.
#[cfg(feature = "local_fs")]
pub(crate) fn detect_file_paths(
    working_directory: &str,
    text: &str,
    shell_launch_data: Option<&ShellLaunchData>,
) -> HashMap<Range<usize>, DetectedLinkType> {
    let mut file_paths = HashMap::new();
    // List files in this working_directory
    let working_directory = shell_launch_data
        .and_then(|launch_data| launch_data.maybe_convert_absolute_path(working_directory))
        .unwrap_or_else(|| {
            // Naively attempt to make a pathbuf from this.
            PathBuf::from(working_directory)
        });
    let files_and_folders_in_working_directory =
        get_files_and_folders_in_directory(working_directory.as_path());
    for word in text.split_whitespace() {
        let possible_paths = possible_file_paths_in_word(word);
        // In the word, there can be multiple valid file paths which may or may not overlap.
        // Take the longest one to turn into a link.
        for possible_path in possible_paths {
            // Need to expand the path here as built-in Path lib does not understand tilde.
            let expanded_path = shellexpand::tilde(possible_path);
            if let Some(path_type) = compute_valid_file_path(
                working_directory.as_path(),
                &expanded_path,
                &files_and_folders_in_working_directory,
                shell_launch_data,
            ) {
                let byte_start = addr_of(possible_path) - addr_of(text);
                let byte_end = byte_start + possible_path.len();
                let char_start = text[..byte_start].chars().count();
                let char_end = char_start + possible_path.chars().count();
                file_paths.insert(char_start..char_end, path_type.clone());

                // Check for line ranges after this file path and add them as separate clickable links
                if let Some(line_ranges) = detect_line_ranges_after_file_path(text, byte_end) {
                    // Extract the base file path from the existing path_type
                    if let DetectedLinkType::FilePath { absolute_path, .. } = &path_type {
                        for (line_number, char_range) in line_ranges {
                            // Create a new DetectedLinkType with the same file path but with the line number
                            let line_range_link = DetectedLinkType::FilePath {
                                absolute_path: absolute_path.clone(),
                                line_and_column_num: Some(warp_util::path::LineAndColumnArg {
                                    line_num: line_number as usize,
                                    column_num: None,
                                }),
                            };
                            file_paths.insert(char_range, line_range_link);
                        }
                    }
                }

                break;
            }
        }
    }
    file_paths
}

use string_offset::CharOffset;
use warp_editor::content::buffer::Buffer;
use warpui::text::word_boundaries::WordBoundariesPolicy;

/// Returns the range of the word surrounding the given offset.
pub(crate) fn get_word_range_at_offset(
    buffer: &Buffer,
    offset: CharOffset,
    word_boundary_policy: Option<WordBoundariesPolicy>,
) -> Option<Range<CharOffset>> {
    use warp_editor::content::buffer::{ToBufferCharOffset, ToBufferPoint};
    use warpui::text::words::is_default_word_boundary;
    use warpui::text::TextBuffer;

    let word_boundary_policy = word_boundary_policy.unwrap_or(WordBoundariesPolicy::Default);
    let mut word_found_at: Option<CharOffset> = None;
    let mut cursor_offset = offset;

    if let Ok(chars) = buffer.chars_at(offset) {
        for c in chars {
            if c == '\n' {
                // Do not cross line boundaries when searching for the nearest word
                break;
            }
            if !is_default_word_boundary(c) {
                word_found_at = Some(cursor_offset);
                break;
            }
            // advance one character
            cursor_offset += 1;
        }
    }

    let found_offset = word_found_at?;
    let found_point = found_offset.to_buffer_point(buffer);

    let word_start_point = buffer
        .word_starts_backward_from_offset_inclusive(found_point)
        .ok()
        .map(|iter| iter.with_policy(&word_boundary_policy))
        .and_then(|mut iter| iter.next())
        .unwrap_or(found_point);

    let word_end_point = buffer
        .word_ends_from_offset_exclusive(found_point)
        .ok()
        .map(|iter| iter.with_policy(&word_boundary_policy))
        .and_then(|mut iter| iter.next())
        .unwrap_or(found_point);

    let word_start = word_start_point.to_buffer_char_offset(buffer);
    let word_end = word_end_point.to_buffer_char_offset(buffer);

    if word_start < word_end {
        Some(word_start..word_end)
    } else {
        None
    }
}

/// Parse line ranges from comma-separated text content and return detected ranges.
#[cfg(feature = "local_fs")]
fn parse_line_range(
    potential_range: &str,
    text: &str,
) -> Result<(u32, Range<usize>), &'static str> {
    let potential_range = potential_range.trim();

    // Look for pattern "number-number"
    let dash_pos = potential_range.find('-').ok_or("No dash found in range")?;

    // Extracting starting line number for potential range
    let start_str = potential_range[..dash_pos].trim();
    let start_line = start_str
        .parse::<u32>()
        .map_err(|_| "Failed to parse start line number")?;
    let end_str = potential_range[dash_pos + 1..].trim();
    end_str
        .parse::<u32>()
        .map_err(|_| "Failed to parse end line number")?;

    let range_start_bytes = addr_of(potential_range) - addr_of(text);
    let char_start = text[..range_start_bytes].chars().count();
    let range_end_bytes = range_start_bytes + potential_range.len();
    let char_end = text[..range_end_bytes].chars().count();

    Ok((start_line, char_start..char_end))
}

/// Helper function to detect line ranges that appear after a valid file path.
/// Looks for patterns like "file.rs (1-50, 100-150)" and returns the detected ranges.
/// Returns a vector of (line_number, char_range) tuples.
#[cfg(feature = "local_fs")]
fn detect_line_ranges_after_file_path(
    text: &str,
    file_path_byte_end: usize,
) -> Option<Vec<(u32, Range<usize>)>> {
    let chars_iter = text[file_path_byte_end..]
        .char_indices()
        .map(|(offs, ch)| (offs + file_path_byte_end, ch));

    // Finds an opening parenthesis, allowing some whitespace after file path, or returns None on failure
    let mut paren_start_idx = None;
    for (char_idx, ch) in chars_iter {
        if ch == '(' {
            paren_start_idx = Some(char_idx);
            break;
        } else if !ch.is_whitespace() {
            return None;
        }
    }
    let paren_start_idx = paren_start_idx?;

    // Find the matching closing parenthesis, or returns None on failure
    let paren_end_index = paren_start_idx + text[paren_start_idx..].find(')')?;

    // Extract the content between parentheses, and parse valid line ranges
    let paren_content = &text[paren_start_idx + 1..paren_end_index];
    let mut detected_ranges = Vec::new();

    for potential_range in paren_content.split(',') {
        match parse_line_range(potential_range, text) {
            Ok(range) => detected_ranges.push(range),
            Err(_) => return None,
        }
    }

    (!detected_ranges.is_empty()).then_some(detected_ranges)
}

/// Pre-extracted hyperlinks keyed by text location. Each entry contains the char ranges
/// and URL strings for markdown hyperlinks (e.g. `[text](url)`) found in that location.
type HyperlinksByLocation = Vec<(TextLocation, Vec<(Range<usize>, String)>)>;

/// Collects all text/location pairs and markdown hyperlinks from an AI output.
/// Only reads in-memory data (no filesystem I/O), safe to call on the main thread.
/// The returned data is designed to be fed into `detect_all_links` on a background thread.
/// Returns raw text (no MD formatting) with location to run link detection on, and markdown hyperlinks.
pub(crate) fn collect_output_data_for_link_detection(
    output: &AIAgentOutput,
    current_working_directory: Option<&String>,
    shell_launch_data: Option<&ShellLaunchData>,
) -> (Vec<(String, TextLocation)>, HyperlinksByLocation) {
    let mut texts = Vec::new();
    let mut hyperlinks = Vec::new();

    // Collect action texts (ReadFiles requests)
    for (action_index, action) in output.actions().enumerate() {
        if let AIAgentActionType::ReadFiles(ReadFilesRequest { locations }) = &action.action {
            for (line_index, file_location) in locations.iter().enumerate() {
                texts.push((
                    file_location.to_user_message(
                        shell_launch_data,
                        current_working_directory,
                        None,
                    ),
                    TextLocation::Action {
                        action_index,
                        line_index,
                    },
                ));
            }
        }
    }

    // Collect output text sections and extract hyperlinks from formatted lines
    for (section_index, section) in output
        .all_text()
        .flat_map(|text| text.sections.iter())
        .enumerate()
    {
        match section {
            AIAgentTextSection::PlainText { text } => match &text.formatted_lines {
                Some(formatted_lines) => {
                    for (line_index, line) in formatted_lines.lines().iter().enumerate() {
                        let location = TextLocation::Output {
                            section_index,
                            line_index,
                        };
                        texts.push((line.raw_text().to_owned(), location));

                        let url_hyperlinks = line.hyperlinks();
                        if !url_hyperlinks.is_empty() {
                            hyperlinks.push((location, url_hyperlinks));
                        }
                    }
                }
                _ => {
                    texts.push((
                        text.text().to_owned(),
                        TextLocation::Output {
                            section_index,
                            line_index: 0,
                        },
                    ));
                }
            },
            AIAgentTextSection::Image { image } => {
                texts.push((
                    image.markdown_source.clone(),
                    TextLocation::Output {
                        section_index,
                        line_index: 0,
                    },
                ));
                texts.push((
                    image.source.clone(),
                    TextLocation::Output {
                        section_index,
                        line_index: 1,
                    },
                ));
            }
            AIAgentTextSection::MermaidDiagram { diagram } => {
                texts.push((
                    diagram.markdown_source.clone(),
                    TextLocation::Output {
                        section_index,
                        line_index: 0,
                    },
                ));
            }
            AIAgentTextSection::Code { .. } | AIAgentTextSection::Table { .. } => {}
        }
    }

    (texts, hyperlinks)
}

/// Runs URL and file path detection on the given texts and combines with pre-extracted markdown hyperlinks.
/// Designed to run on a background thread (file path detection does filesystem I/O).
pub(crate) fn detect_all_links(
    texts: &[(String, TextLocation)],
    md_hyperlinks: HyperlinksByLocation,
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    current_working_directory: Option<&String>,
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))] shell_launch_data: Option<
        &ShellLaunchData,
    >,
) -> HashMap<TextLocation, HashMap<Range<usize>, DetectedLinkType>> {
    let mut all_links: HashMap<TextLocation, HashMap<Range<usize>, DetectedLinkType>> =
        HashMap::new();

    for (text, location) in texts {
        let url_ranges = detect_urls(text);
        let mut links = HashMap::new();

        // Detect URLs via regex
        for url_range in &url_ranges {
            if let Some(link_text) = char_slice(text, url_range.start, url_range.end) {
                links.insert(
                    url_range.clone(),
                    DetectedLinkType::Url(link_text.to_owned()),
                );
            }
        }

        // Detect file path links, skipping any that overlap with URLs
        #[cfg(feature = "local_fs")]
        if let Some(cwd) = current_working_directory {
            let file_paths = detect_file_paths(cwd, text, shell_launch_data);
            for (range, link) in file_paths {
                if !url_ranges
                    .iter()
                    .any(|ur| ur.start < range.end && range.start < ur.end)
                {
                    links.insert(range, link);
                }
            }
        }

        if !links.is_empty() {
            all_links.insert(*location, links);
        }
    }

    // Add hyperlinks extracted from formatted markdown text
    for (location, line_hyperlinks) in md_hyperlinks {
        let entry = all_links.entry(location).or_default();
        for (range, url) in line_hyperlinks {
            entry.insert(range, DetectedLinkType::Url(url));
        }
    }

    all_links
}

/// Given some text and its location
/// the detected_links_state.
pub(crate) fn detect_links(
    detected_links_state: &mut DetectedLinksState,
    text: &str,
    text_location: TextLocation,
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))]
    current_working_directory: Option<&String>,
    #[cfg_attr(not(feature = "local_fs"), allow(unused_variables))] shell_launch_data: Option<
        &ShellLaunchData,
    >,
) {
    let url_ranges = detect_urls(text);
    for url_range in &url_ranges {
        let Some(link) = char_slice(text, url_range.start, url_range.end) else {
            continue;
        };
        detected_links_state
            .detected_links_by_location
            .entry(text_location)
            .or_default()
            .detected_links
            .insert(
                url_range.clone(),
                HoverableDetectedLink {
                    link: DetectedLinkType::Url(link.to_owned()),
                    mouse_state: Default::default(),
                },
            );
    }
    #[cfg(feature = "local_fs")]
    if let Some(current_working_directory) = current_working_directory {
        let file_paths = detect_file_paths(current_working_directory, text, shell_launch_data);
        for (range, link) in file_paths {
            // If this file path range overlaps with a URL range, don't add it.
            if url_ranges
                .iter()
                .any(|url_range| url_range.start < range.end && range.start < url_range.end)
            {
                continue;
            }
            detected_links_state
                .detected_links_by_location
                .entry(text_location)
                .or_default()
                .detected_links
                .insert(
                    range,
                    HoverableDetectedLink {
                        link,
                        mouse_state: Default::default(),
                    },
                );
        }
    }
}

#[cfg(test)]
#[path = "link_detection_tests.rs"]
mod tests;
