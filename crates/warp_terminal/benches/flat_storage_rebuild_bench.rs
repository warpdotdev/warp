use std::num::NonZeroU16;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use warp_terminal::model::grid::flat_storage::{GraphemeInfo, Index};

const ASCII_GRAPHEME_INFO: GraphemeInfo = GraphemeInfo {
    cell_width: 1,
    utf8_bytes: NonZeroU16::new(1).unwrap(),
};

const MULTIBYTE_GRAPHEME_INFO: GraphemeInfo = GraphemeInfo {
    cell_width: 1,
    utf8_bytes: NonZeroU16::new(3).unwrap(),
};

const EMOJI_GRAPHEME_INFO: GraphemeInfo = GraphemeInfo {
    cell_width: 2,
    utf8_bytes: NonZeroU16::new(4).unwrap(),
};

struct Fixture {
    name: &'static str,
    index: Index,
    target_columns: &'static [usize],
}

fn push_graphemes(index: &mut Index, count: usize, info: GraphemeInfo, trailing_newline: bool) {
    let mut entry_builder = index.start_row();
    for _ in 0..count {
        entry_builder.process_grapheme_info_unchecked(info);
    }
    if trailing_newline {
        entry_builder.add_trailing_newline();
    }
    entry_builder.append_to_index(index);
}

fn make_uniform_index(
    rows: usize,
    columns: usize,
    info: GraphemeInfo,
    newline_every_row: bool,
) -> Index {
    let mut index = Index::new(columns, Some(rows));
    let graphemes_per_row = columns / info.cell_width as usize;
    for _ in 0..rows {
        push_graphemes(&mut index, graphemes_per_row, info, newline_every_row);
    }
    index
}

fn make_softwrapped_ascii_index(rows: usize, columns: usize, segment_rows: usize) -> Index {
    let mut index = Index::new(columns, Some(rows));
    for row_idx in 0..rows {
        let trailing_newline = (row_idx + 1) % segment_rows == 0;
        push_graphemes(&mut index, columns, ASCII_GRAPHEME_INFO, trailing_newline);
    }
    index
}

fn make_mixed_index(rows: usize, columns: usize) -> Index {
    let mut index = Index::new(columns, Some(rows));

    for row_idx in 0..rows {
        let mut entry_builder = index.start_row();
        let mut cells = 0;
        let mut pattern_idx = 0;

        while cells < columns {
            let info = match (row_idx + pattern_idx) % 4 {
                0 => ASCII_GRAPHEME_INFO,
                1 => MULTIBYTE_GRAPHEME_INFO,
                _ => EMOJI_GRAPHEME_INFO,
            };

            if cells + info.cell_width as usize > columns {
                pattern_idx += 1;
                continue;
            }

            entry_builder.process_grapheme_info_unchecked(info);
            cells += info.cell_width as usize;
            pattern_idx += 1;
        }

        if row_idx % 3 != 1 {
            entry_builder.add_trailing_newline();
        }
        entry_builder.append_to_index(&mut index);
    }

    index
}

fn total_graphemes(index: &Index) -> u64 {
    (0..index.len())
        .filter_map(|row_idx| index.grapheme_infos_for_row(row_idx))
        .flatten()
        .count() as u64
}

fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            name: "ascii_dense",
            index: make_uniform_index(10_000, 80, ASCII_GRAPHEME_INFO, true),
            target_columns: &[120, 81, 40],
        },
        Fixture {
            name: "wide_dense",
            index: make_uniform_index(10_000, 80, EMOJI_GRAPHEME_INFO, true),
            target_columns: &[120, 79, 7],
        },
        Fixture {
            name: "mixed_rows",
            index: make_mixed_index(8_000, 80),
            target_columns: &[120, 81, 40],
        },
        Fixture {
            name: "softwrapped_ascii",
            index: make_softwrapped_ascii_index(10_000, 80, 32),
            target_columns: &[120, 80, 40],
        },
    ]
}

fn criterion_benchmark(c: &mut Criterion) {
    for fixture in fixtures() {
        let graphemes = total_graphemes(&fixture.index);
        let mut group = c.benchmark_group(format!("flat_storage/rebuild/{}", fixture.name));
        group.throughput(Throughput::Elements(graphemes));

        for &target_columns in fixture.target_columns {
            group.bench_with_input(
                BenchmarkId::new("optimized", target_columns),
                &target_columns,
                |b, &target_columns| {
                    b.iter(|| Index::rebuild(black_box(&fixture.index), black_box(target_columns)))
                },
            );
            group.bench_with_input(
                BenchmarkId::new("baseline", target_columns),
                &target_columns,
                |b, &target_columns| {
                    b.iter(|| {
                        Index::rebuild_baseline(
                            black_box(&fixture.index),
                            black_box(target_columns),
                        )
                    })
                },
            );
        }

        group.finish();
    }
}

criterion_group!(
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = criterion_benchmark
);
criterion_main!(benches);
