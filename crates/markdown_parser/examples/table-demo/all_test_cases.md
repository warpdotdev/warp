# Markdown Table Test Cases

## 01_simple_2x2

| Header 1 | Header 2 |
| --- | --- |
| Cell 1 | Cell 2 |

---

## 02_three_columns

| Name | Age | City |
| --- | --- | --- |
| Alice | 30 | NYC |
| Bob | 25 | LA |

---

## 03_multiple_rows

| ID | Value |
| --- | --- |
| 1 | Apple |
| 2 | Banana |
| 3 | Cherry |
| 4 | Date |
| 5 | Elderberry |

---

## 04_left_aligned

| Left 1 | Left 2 |
| :--- | :--- |
| Short | Text |
| Much longer text | Another |

---

## 05_right_aligned

| Right 1 | Right 2 |
| ---: | ---: |
| Short | Text |
| Much longer text | Another |

---

## 06_center_aligned

| Center 1 | Center 2 |
| :---: | :---: |
| Short | Text |
| Much longer text | Another |

---

## 07_mixed_alignment

| Left | Center | Right |
| :--- | :---: | ---: |
| L | C | R |
| Left-aligned | Centered | Right-aligned |

---

## 08_bold

| Header | Value |
| --- | --- |
| **Bold** | Normal |
| Text | **Bold too** |

---

## 09_italic

| Header | Value |
| --- | --- |
| *Italic* | Normal |
| Text | *Italic too* |

---

## 10_inline_code

| Function | Returns |
| --- | --- |
| `foo()` | `String` |
| `bar()` | `i32` |

---

## 11_links

| Site | URL |
| --- | --- |
| Google | [Link](https://google.com) |
| GitHub | [Link](https://github.com) |

---

## 12_strikethrough

| Item | Status |
| --- | --- |
| ~~Deprecated~~ | Old |
| Active | Current |

---

## 13_mixed_formatting

| Feature | Description |
| --- | --- |
| **Bold** with *italic* | Mixed |
| `code` and **bold** | Combined |
| ~~Strike~~ and *italic* | More |

---

## 14_empty_cells

| A | B | C |
| --- | --- | --- |
|  | filled |  |
| filled |  | filled |

---

## 15_whitespace_cells

| A | B |
| --- | --- |
|   | space |
| tab	 | text |

---

## 16_escaped_pipes

| Expression | Result |
| --- | --- |
| A \| B | OR operation |
| X \| Y \| Z | Multiple |

---

## 17_long_content

| Short | Very Long Content |
| --- | --- |
| A | This is a very long cell with lots of text that should wrap or truncate |
| B | Another cell with substantial content |

---

## 18_html_entities

| Symbol | Code |
| --- | --- |
| &lt; | Less than |
| &gt; | Greater than |
| &amp; | Ampersand |

---

## 19_unicode_emoji

| Icon | Name |
| --- | --- |
| 🚀 | Rocket |
| ⭐ | Star |
| 🎉 | Party |

---

## 20_wide_table

| Build Identifier | Release Channel | Feature Flag State | Workspace Session Token | Active Pane Title | Suggested Command Preview | Generated File Path | Git Branch Name | Pull Request Status | Reviewer Assignment | Telemetry Event Name | Render Mode | Table Layout Strategy | Horizontal Overflow Sentinel | Unbroken Content Sample | Final Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| build_2026_04_08_very_long_identifier_alpha | dogfood_internal_preview_rollout_candidate | markdown_table_horizontal_scroll_enabled | ws_session_token_01_ABCDEFGHIJKLMNOPQRSTUVWXYZ | agent_mode_diff_review_surface_with_extra_context | cargo_nextest_run_no_fail_fast_workspace_markdown_parser | crates/markdown_parser/examples/table-demo/all_test_cases.md | zach/wide-markdown-table-scroll | awaiting_follow_up_visual_regression_check | reviewer_assignment_pending_product_design | markdown_table_rendered_in_example_viewport | constrained_width_preview_panel | preserve_column_intrinsic_widths_before_wrapping | horizontal_scroll_should_be_required_here | SUPERLONGUNBROKENTEXTVALUE0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ | first_row_designed_to_force_width |
| build_2026_04_08_very_long_identifier_beta | stable_candidate_post_validation | markdown_table_horizontal_scroll_enabled | ws_session_token_02_ZYXWVUTSRQPONMLKJIHGFEDCBA | markdown_parser_demo_showing_extreme_width_case | cargo_run_features_with_local_server_markdown_demo | crates/markdown_parser/examples/table-demo/render_snapshot_reference.png | zach/wide-markdown-table-scroll | local_only_validation_before_pr | reviewer_assignment_not_requested_yet | markdown_table_horizontal_scroll_exercised | embedded_example_renderer | keep_headers_verbose_and_cells_intentionally_wide | overflow_region_should_extend_far_past_viewport | ANOTHEREXTREMELYLONGUNBROKENVALUE_for_horizontal_scroll_testing_only | second_row_keeps_pressure_on_layout |
| build_2026_04_08_very_long_identifier_gamma | canary_rollout_with_extra_observability | markdown_table_horizontal_scroll_enabled | ws_session_token_03_0123456789_repeat_repeat | full_width_table_case_for_manual_agent_testing | cargo_clippy_workspace_all_targets_all_features_tests | app/src/features/markdown/table_renderer/visual_debug_reference.rs | zach/wide-markdown-table-scroll | no_pr_needed_for_manual_local_test | reviewer_assignment_not_applicable | markdown_table_scroll_behavior_verified_manually | split_pane_code_review_view | avoid_collapsing_columns_even_with_dense_content | viewport_must_scroll_horizontally_to_reveal_tail_columns | YETANOTHERLONGUNBROKENCONTENTBLOCK_THAT_SHOULD_NOT_WRAP_EASILY | third_row_confirms_consistent_behavior |

---

## 21_deep_table

| ID | Value |
| --- | --- |
| 1 | Row 1 |
| 2 | Row 2 |
| 3 | Row 3 |
| 4 | Row 4 |
| 5 | Row 5 |
| 6 | Row 6 |
| 7 | Row 7 |
| 8 | Row 8 |
| 9 | Row 9 |
| 10 | Row 10 |
| 11 | Row 11 |
| 12 | Row 12 |
| 13 | Row 13 |
| 14 | Row 14 |
| 15 | Row 15 |
| 16 | Row 16 |
| 17 | Row 17 |
| 18 | Row 18 |
| 19 | Row 19 |
| 20 | Row 20 |

---

## 22_large_grid

| C1 | C2 | C3 | C4 | C5 | C6 |
| --- | --- | --- | --- | --- | --- |
| R1C1 | R1C2 | R1C3 | R1C4 | R1C5 | R1C6 |
| R2C1 | R2C2 | R2C3 | R2C4 | R2C5 | R2C6 |
| R3C1 | R3C2 | R3C3 | R3C4 | R3C5 | R3C6 |
| R4C1 | R4C2 | R4C3 | R4C4 | R4C5 | R4C6 |
| R5C1 | R5C2 | R5C3 | R5C4 | R5C5 | R5C6 |
| R6C1 | R6C2 | R6C3 | R6C4 | R6C5 | R6C6 |
| R7C1 | R7C2 | R7C3 | R7C4 | R7C5 | R7C6 |
| R8C1 | R8C2 | R8C3 | R8C4 | R8C5 | R8C6 |
