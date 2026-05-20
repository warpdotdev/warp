use crate::editor::tests::sample_text;

use super::*;
use crate::editor::EditOrigin;
use anyhow::Error;
use warpui::App;

#[test]
fn test_chars_at() -> Result<()> {
    App::test((), |mut app| async move {
        let text = sample_text(6, 6);
        let buffer = app.add_model(|_| Buffer::new(text));
        let map = app.add_model(|ctx| DisplayMap::new(buffer.clone(), 4, ctx));
        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_for_test(
                vec![
                    Point::new(1, 0)..Point::new(1, 0),
                    Point::new(1, 1)..Point::new(1, 1),
                    Point::new(2, 1)..Point::new(2, 1),
                ],
                "\t",
                EditOrigin::UserInitiated,
                ctx,
            )
        })?;

        map.read(&app, |map, ctx| {
            assert_eq!(
                map.chars_at(DisplayPoint::new(1, 0), ctx)?
                    .take(10)
                    .collect::<String>(),
                "    b   bb"
            );
            assert_eq!(
                map.chars_at(DisplayPoint::new(1, 2), ctx)?
                    .take(10)
                    .collect::<String>(),
                "  b   bbbb"
            );
            assert_eq!(
                map.chars_at(DisplayPoint::new(1, 6), ctx)?
                    .take(13)
                    .collect::<String>(),
                "  bbbbb\nc   c"
            );

            Ok::<(), Error>(())
        })?;

        Ok(())
    })
}

#[test]
fn test_expand_tabs() {
    assert_eq!(expand_tabs("\t".chars(), 0, 4), 0);
    assert_eq!(expand_tabs("\t".chars(), 1, 4), 4);
    assert_eq!(expand_tabs("\ta".chars(), 2, 4), 5);
}

#[test]
fn test_collapse_tabs() {
    assert_eq!(collapse_tabs("\t".chars(), 0, Bias::Left, 4), (0, 0));
    assert_eq!(collapse_tabs("\t".chars(), 0, Bias::Right, 4), (0, 0));
    assert_eq!(collapse_tabs("\t".chars(), 1, Bias::Left, 4), (0, 3));
    assert_eq!(collapse_tabs("\t".chars(), 1, Bias::Right, 4), (1, 0));
    assert_eq!(collapse_tabs("\t".chars(), 2, Bias::Left, 4), (0, 2));
    assert_eq!(collapse_tabs("\t".chars(), 2, Bias::Right, 4), (1, 0));
    assert_eq!(collapse_tabs("\t".chars(), 3, Bias::Left, 4), (0, 1));
    assert_eq!(collapse_tabs("\t".chars(), 3, Bias::Right, 4), (1, 0));
    assert_eq!(collapse_tabs("\t".chars(), 4, Bias::Left, 4), (1, 0));
    assert_eq!(collapse_tabs("\t".chars(), 4, Bias::Right, 4), (1, 0));
    assert_eq!(collapse_tabs("\ta".chars(), 5, Bias::Left, 4), (2, 0));
    assert_eq!(collapse_tabs("\ta".chars(), 5, Bias::Right, 4), (2, 0));
}

#[test]
fn test_max_point() -> Result<()> {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new("aaa\n\t\tbbb"));
        let map = app.add_model(|ctx| DisplayMap::new(buffer.clone(), 4, ctx));
        map.read(&app, |map, app| {
            assert_eq!(map.max_point(app), DisplayPoint::new(1, 11))
        });
        Ok(())
    })
}
