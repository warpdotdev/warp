#![no_main]
use libfuzzer_sys::fuzz_target;
use std::num::NonZeroU16;
use warp_terminal::model::grid::flat_storage::index::{GraphemeInfo, Index};

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }

    // Use first 2 bytes for dimensions, rest for row content decisions
    let old_cols = (data[0] as usize % 200) + 1;
    let new_cols = (data[1] as usize % 200) + 1;
    let num_rows = ((data[2] as usize) % 100) + 1;

    let ascii_info = GraphemeInfo { cell_width: 1, utf8_bytes: NonZeroU16::new(1).unwrap() };
    let wide_info = GraphemeInfo { cell_width: 2, utf8_bytes: NonZeroU16::new(3).unwrap() };

    let mut index = Index::new(old_cols, Some(num_rows));

    for i in 0..num_rows {
        let mut eb = index.start_row();
        let byte_idx = 3 + (i % (data.len() - 3));
        let row_byte = data[byte_idx];

        let mut cells = 0;
        match row_byte % 4 {
            0 => {
                // Full ASCII row
                while cells < old_cols {
                    eb.process_grapheme_info(ascii_info, &mut index);
                    cells += 1;
                }
            }
            1 => {
                // Wide char row
                while cells + 2 <= old_cols {
                    eb.process_grapheme_info(wide_info, &mut index);
                    cells += 2;
                }
            }
            2 => {
                // Mixed row
                while cells < old_cols {
                    if row_byte.wrapping_add(cells as u8) % 3 == 0 && cells + 2 <= old_cols {
                        eb.process_grapheme_info(wide_info, &mut index);
                        cells += 2;
                    } else {
                        eb.process_grapheme_info(ascii_info, &mut index);
                        cells += 1;
                    }
                }
            }
            _ => {
                // Partial row
                let target = old_cols / 2;
                while cells < target {
                    eb.process_grapheme_info(ascii_info, &mut index);
                    cells += 1;
                }
            }
        }

        if row_byte % 3 != 0 {
            eb.add_trailing_newline();
        }
        eb.append_to_index(&mut index);
    }

    let original_content_len = index.content_len;

    // Rebuild with new columns
    let rebuilt = Index::rebuild(&index, new_cols);

    // Invariant: content_len must be preserved
    assert_eq!(rebuilt.content_len, original_content_len,
        "content_len diverged: old_cols={old_cols}, new_cols={new_cols}, rows={num_rows}");

    // Invariant: no row exceeds new column width
    for row_idx in 0..rebuilt.len() {
        if let Some(infos) = rebuilt.grapheme_infos_for_row(row_idx) {
            let total: usize = infos.map(|i| i.cell_width as usize).sum();
            assert!(total <= new_cols,
                "row {row_idx} has {total} cells, max is {new_cols}");
        }
    }
});
