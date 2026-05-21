use super::*;
use crate::editor::tests::{sample_text, RandomCharIter};
use crate::editor::EditOrigin;
use tests::buffer::RangesWhenEditing;
use warpui::App;

#[test]
fn test_basic_folds() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(sample_text(5, 6)));
        let mut map = app.read(|app| FoldMap::new(buffer.clone(), app));

        app.read(|app| {
            map.fold(
                vec![
                    Point::new(0, 2)..Point::new(2, 2),
                    Point::new(2, 4)..Point::new(4, 1),
                ],
                app,
            )?;
            assert_eq!(map.text(app), "aa…cc…eeeee");
            Ok::<(), anyhow::Error>(())
        })?;

        let edits = buffer.update(&mut app, |buffer, ctx| {
            let start_version = buffer.versions();
            buffer.edit_for_test(
                vec![
                    Point::new(0, 0)..Point::new(0, 1),
                    Point::new(2, 3)..Point::new(2, 3),
                ],
                "123",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            Ok::<_, anyhow::Error>(buffer.edits_since(start_version).collect::<Vec<_>>())
        })?;

        app.read(|app| {
            map.apply_edits(&edits, app)?;
            assert_eq!(map.text(app), "123a…c123c…eeeee");
            Ok::<(), anyhow::Error>(())
        })?;

        let edits = buffer.update(&mut app, |buffer, ctx| {
            let start_version = buffer.versions();
            buffer.edit_for_test(
                Some(Point::new(2, 6)..Point::new(4, 3)),
                "456",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            Ok::<_, anyhow::Error>(buffer.edits_since(start_version).collect::<Vec<_>>())
        })?;

        app.read(|app| {
            map.apply_edits(&edits, app)?;
            assert_eq!(map.text(app), "123a…c123456eee");

            map.unfold(Some(Point::new(0, 4)..Point::new(0, 4)), app)?;
            assert_eq!(map.text(app), "123aaaaa\nbbbbbb\nccc123456eee");

            Ok(())
        })
    })
}

#[test]
fn test_overlapping_folds() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(sample_text(5, 6)));
        app.read(|app| {
            let mut map = FoldMap::new(buffer.clone(), app);
            map.fold(
                vec![
                    Point::new(0, 2)..Point::new(2, 2),
                    Point::new(0, 4)..Point::new(1, 0),
                    Point::new(1, 2)..Point::new(3, 2),
                    Point::new(3, 1)..Point::new(4, 1),
                ],
                app,
            )?;
            assert_eq!(map.text(app), "aa…eeeee");
            Ok(())
        })
    })
}

#[test]
fn test_merging_folds_via_edit() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(sample_text(5, 6)));
        let mut map = app.read(|app| FoldMap::new(buffer.clone(), app));

        app.read(|app| {
            map.fold(
                vec![
                    Point::new(0, 2)..Point::new(2, 2),
                    Point::new(3, 1)..Point::new(4, 1),
                ],
                app,
            )?;
            assert_eq!(map.text(app), "aa…cccc\nd…eeeee");
            Ok::<(), anyhow::Error>(())
        })?;

        let edits = buffer.update(&mut app, |buffer, ctx| {
            let start_version = buffer.versions();
            buffer.edit_for_test(
                Some(Point::new(2, 2)..Point::new(3, 1)),
                "",
                EditOrigin::UserInitiated,
                ctx,
            )?;
            Ok::<_, anyhow::Error>(buffer.edits_since(start_version).collect::<Vec<_>>())
        })?;

        app.read(|app| {
            map.apply_edits(&edits, app)?;
            assert_eq!(map.text(app), "aa…eeeee");
            Ok(())
        })
    })
}

#[test]
fn test_random_folds() -> Result<()> {
    use super::super::buffer::ToPoint;
    use rand::prelude::*;

    for seed in 0..100 {
        println!("{seed:?}");
        let mut rng = StdRng::seed_from_u64(seed);

        App::test((), |mut app| async move {
            let buffer = app.add_model(|_| {
                let len = rng.gen_range(0..10);
                let text = RandomCharIter::new(&mut rng).take(len).collect::<String>();
                Buffer::new(text)
            });
            let mut map = app.read(|app| FoldMap::new(buffer.clone(), app));

            app.read(|app| -> Result<()> {
                let buffer = buffer.as_ref(app);

                let fold_count = rng.gen_range(0..10);
                let mut fold_ranges: Vec<Range<CharOffset>> = Vec::new();
                for _ in 0..fold_count {
                    let end = rng.gen_range(0..buffer.len().as_usize() + 1);
                    let start = rng.gen_range(0..end + 1);
                    fold_ranges.push(CharOffset::from(start)..CharOffset::from(end));
                }

                map.fold(fold_ranges, app)?;

                let mut expected_text = buffer.text();
                for fold_range in map.merged_fold_ranges(app).into_iter().rev() {
                    expected_text
                        .replace_range(fold_range.start.as_usize()..fold_range.end.as_usize(), "…");
                }

                assert_eq!(map.text(app), expected_text);

                for fold_range in map.merged_fold_ranges(app) {
                    let display_point =
                        map.to_display_point(fold_range.start.to_point(buffer).unwrap());
                    assert!(map.is_line_folded(display_point.row()));
                }

                Ok::<(), anyhow::Error>(())
            })?;

            let edits = buffer.update(&mut app, |buffer, ctx| {
                let start_version = buffer.versions();
                let edit_count = rng.gen_range(1..10);
                buffer.randomly_edit(
                    &mut rng,
                    RangesWhenEditing::UseRandomRanges {
                        num_ranges: edit_count,
                    },
                    ctx,
                );
                Ok::<_, anyhow::Error>(buffer.edits_since(start_version).collect::<Vec<_>>())
            })?;

            app.read(|app| {
                map.apply_edits(&edits, app)?;

                let buffer = map.buffer.as_ref(app);
                let mut expected_text = buffer.text();
                for fold_range in map.merged_fold_ranges(app).into_iter().rev() {
                    expected_text
                        .replace_range(fold_range.start.as_usize()..fold_range.end.as_usize(), "…");
                }
                assert_eq!(map.text(app), expected_text);

                Ok::<(), anyhow::Error>(())
            })?;

            Ok::<(), anyhow::Error>(())
        })?;
    }
    Ok(())
}

#[test]
fn test_buffer_rows() -> Result<()> {
    App::test((), |mut app| async move {
        let text = sample_text(6, 6) + "\n";
        let buffer = app.add_model(|_| Buffer::new(text));

        app.read(|app| {
            let mut map = FoldMap::new(buffer.clone(), app);

            map.fold(
                vec![
                    Point::new(0, 2)..Point::new(2, 2),
                    Point::new(3, 1)..Point::new(4, 1),
                ],
                app,
            )?;

            assert_eq!(map.text(app), "aa…cccc\nd…eeeee\nffffff\n");
            assert_eq!(map.buffer_rows(0)?.collect::<Vec<_>>(), vec![0, 3, 5, 6]);
            assert_eq!(map.buffer_rows(3)?.collect::<Vec<_>>(), vec![6]);

            Ok(())
        })
    })
}

#[test]
fn test_desynced_buffer() {
    App::test((), |mut app| async move {
        // Create a buffer with 20 characters
        let buffer = app.add_model(|_| Buffer::new(sample_text(1, 20)));
        app.update(|app| {
            let map = FoldMap::new(buffer.clone(), app);
            buffer.update(app, |buf, ctx| {
                // Update the buffer to only have 10 characters
                buf.edit_for_test(
                    Some(0..20),
                    sample_text(1, 10),
                    EditOrigin::UserInitiated,
                    ctx,
                )
                .unwrap();
            });
            let mut chars = map.chars_at(DisplayPoint::new(0, 10), app).unwrap();

            assert_eq!(
                chars.next(),
                None,
                "Shouldn't attempt to return unavailable characters"
            );
        });
    })
}

impl FoldMap {
    fn text(&self, app: &AppContext) -> String {
        self.chars_at(DisplayPoint(Point::zero()), app)
            .unwrap()
            .collect()
    }

    fn merged_fold_ranges(&self, app: &AppContext) -> Vec<Range<CharOffset>> {
        let buffer = self.buffer.as_ref(app);
        let mut fold_ranges = self
            .folds
            .iter()
            .map(|fold| {
                fold.start.to_char_offset(buffer).unwrap()..fold.end.to_char_offset(buffer).unwrap()
            })
            .peekable();

        let mut merged_ranges = Vec::new();
        while let Some(mut fold_range) = fold_ranges.next() {
            while let Some(next_range) = fold_ranges.peek() {
                if fold_range.end >= next_range.start {
                    if next_range.end > fold_range.end {
                        fold_range.end = next_range.end;
                    }
                    fold_ranges.next();
                } else {
                    break;
                }
            }
            if fold_range.end > fold_range.start {
                merged_ranges.push(fold_range);
            }
        }
        merged_ranges
    }
}
