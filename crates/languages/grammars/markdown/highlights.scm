; Warp-authored Markdown highlight query.
;
; arborium bundles an upstream Markdown highlights query that uses nvim-style capture
; names (e.g. `text.title`, `punctuation.special`). Warp only maps a fixed set of capture
; prefixes to theme colors (see `convert_capture_name_to_color` in crates/syntax_tree), so
; this query re-targets the same grammar nodes onto the names Warp understands: keyword,
; function, string, type, comment, property, and tag.
;
; This is the block-level grammar only. Inline emphasis and language-specific highlighting
; inside fenced code blocks require tree-sitter injections, which the editor's highlighter
; does not yet support.

; Headings — the `#` markers (or setext underlines) and the heading text.
[
  (atx_h1_marker)
  (atx_h2_marker)
  (atx_h3_marker)
  (atx_h4_marker)
  (atx_h5_marker)
  (atx_h6_marker)
  (setext_h1_underline)
  (setext_h2_underline)
] @keyword
(atx_heading (inline) @keyword)
(setext_heading (paragraph) @keyword)

; Code — fenced and indented blocks, the fence delimiters, and the info-string language.
[
  (indented_code_block)
  (code_fence_content)
] @string
(fenced_code_block_delimiter) @comment
(info_string (language) @type)

; List and task-list markers.
[
  (list_marker_plus)
  (list_marker_minus)
  (list_marker_star)
  (list_marker_dot)
  (list_marker_parenthesis)
  (task_list_marker_checked)
  (task_list_marker_unchecked)
] @function

; Block quotes and thematic breaks.
[
  (block_quote_marker)
  (thematic_break)
] @comment

; Link reference definitions: `[label]: destination "title"`.
(link_label) @property
(link_destination) @function
(link_title) @string

; Embedded HTML and character escapes/entities.
(html_block) @tag
(backslash_escape) @string
(entity_reference) @string
(numeric_character_reference) @string

; YAML/TOML front matter.
[
  (minus_metadata)
  (plus_metadata)
] @comment

; Pipe-table grid — color the column separators on every row. The delimiter row
; (dashes and all) is structural, so the whole row stays colored.
(pipe_table_header "|" @comment)
(pipe_table_row "|" @comment)
(pipe_table_delimiter_row) @comment
