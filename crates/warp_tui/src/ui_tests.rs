use warp::appearance::Appearance;
use warpui_core::App;
use warpui_core::elements::tui::{TuiBufferExt, TuiRect, text_width};
use warpui_core::presenter::tui::TuiPresenter;

use super::{conversation_restoring, elide_footer_path, login_placeholder};

#[test]
fn elide_footer_path_keeps_full_path_when_it_fits() {
    // The regression this fixes: with ample width the cwd is shown in full,
    // never eagerly collapsed to `root/…/basename`.
    let path = "/erica/projects/research";
    assert_eq!(elide_footer_path(path, 100), path);
    // A two-component path is likewise untouched when it fits.
    assert_eq!(elide_footer_path("/erica/project", 100), "/erica/project");
}

#[test]
fn elide_footer_path_elides_progressively_as_width_shrinks() {
    let path = "/one/two/three/four/five";
    let full = text_width(path);
    // Just under the full width: elide the least possible while keeping the
    // root and basename, and stay within budget. A prefix-preserving elision
    // that fits must win over a leading-component-free form of equal or greater
    // width, so the root stays visible.
    let budget = full - 1;
    assert_eq!(budget, 23);
    assert_eq!(elide_footer_path(path, budget), "/one/…/three/four/five");
    // Tighter still: the widest prefix-preserving form no longer fits, but a
    // shorter one does. It must be chosen ahead of a wider leading-component-
    // free candidate (`/…/three/four/five`) that also fits, since retaining the
    // root prefix takes priority over preserving one more interior component.
    assert_eq!(elide_footer_path(path, 21), "/one/…/four/five");
    // Much tighter: no prefix-preserving form fits, so we fall back to the
    // leading-component-free `/…/basename`, still within budget.
    let tight = elide_footer_path(path, 8);
    assert!(tight.starts_with('/'), "leading separator kept: {tight:?}");
    assert!(tight.ends_with("five"), "basename preserved: {tight:?}");
    assert!(text_width(&tight) <= 8);
}

#[test]
fn elide_footer_path_truncates_basename_as_last_resort() {
    // Even the minimal `…/basename` cannot fit, so the basename is
    // grapheme-truncated with an ellipsis without panicking or overflowing.
    let elided = elide_footer_path("/one/two/threeeeeeeeeeeeee", 6);
    assert!(text_width(&elided) <= 6, "stays within budget: {elided:?}");
}

#[test]
fn elide_footer_path_handles_windows_paths() {
    assert_eq!(
        elide_footer_path(r"C:\Users\erica\project", 12),
        r"C:\…\project"
    );
}

#[test]
fn conversation_loader_is_centered_and_animated() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        app.read(|app_ctx| {
            let element = conversation_restoring(app_ctx);
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(element, TuiRect::new(0, 0, 60, 7), app_ctx);
            let lines = frame.buffer.to_lines();
            let label = lines
                .iter()
                .find(|line| line.contains("Loading session..."))
                .expect("loading label should render");
            assert!(
                lines.iter().any(|line| {
                    line.contains("Esc or Ctrl-C to cancel and start a new session")
                })
            );

            assert!(
                label.find("Loading session...").is_some_and(|x| x > 0),
                "loading label should be horizontally centered: {label:?}"
            );
            assert!(
                frame.repaint_at.is_some(),
                "loading spinner should schedule a repaint"
            );
        });
    });
}

#[test]
fn login_placeholder_is_centered_for_all_states() {
    const VIEWPORT_COLS: usize = 60;
    const VIEWPORT_ROWS: usize = 8;

    App::test((), |app| async move {
        app.read(|app_ctx| {
            // Every line the placeholder renders for each login substate, so
            // per-line centering is checked — not just the longest line. The
            // shorter lines (e.g. the URI-only and URI+code substates' "Sign in
            // to continue" and "and enter code" lines) are the ones a
            // start-aligned column leaves flush with the block's left edge.
            let states: Vec<(Option<&str>, Option<&str>, Vec<&str>)> = vec![
                (
                    None,
                    None,
                    vec!["Sign in to continue", "Requesting a sign-in link…"],
                ),
                (
                    Some("https://warp.dev/device"),
                    None,
                    vec![
                        "Sign in to continue",
                        "Open https://warp.dev/device in your browser",
                    ],
                ),
                (
                    Some("https://warp.dev/device"),
                    Some("ABC-123"),
                    vec![
                        "Sign in to continue",
                        "Open https://warp.dev/device in your browser",
                        "and enter code: ABC-123",
                    ],
                ),
            ];

            for (verification_uri, user_code, expected_lines) in states {
                let element = login_placeholder(verification_uri, user_code);
                let mut presenter = TuiPresenter::new();
                let frame = presenter.present_element(
                    element,
                    TuiRect::new(0, 0, VIEWPORT_COLS as u16, VIEWPORT_ROWS as u16),
                    app_ctx,
                );
                let lines = frame.buffer.to_lines();

                for expected_line in expected_lines {
                    let row = lines
                        .iter()
                        .position(|line| line.contains(expected_line))
                        .unwrap_or_else(|| {
                            panic!("login state should render {expected_line:?}: {lines:?}")
                        });
                    let rendered_line = &lines[row];
                    let left = rendered_line
                        .find(expected_line)
                        .map(|byte_offset| rendered_line[..byte_offset].chars().count())
                        .unwrap_or_else(|| {
                            panic!("expected login text should be present: {rendered_line:?}")
                        });
                    let text_width = expected_line.chars().count();
                    let right = VIEWPORT_COLS
                        .saturating_sub(left)
                        .saturating_sub(text_width);

                    // Centered means symmetric horizontal padding: the blank
                    // columns on either side of the text differ by at most one
                    // cell (the flex layout's integer-division slack). A
                    // start-aligned column leaves shorter lines flush with the
                    // block's left edge, producing lopsided padding, so this
                    // catches the per-line centering the ticket requires.
                    assert!(
                        left.abs_diff(right) <= 1,
                        "{expected_line:?} should be horizontally centered with symmetric padding \
                         (left={left}, right={right}, width={text_width}, viewport={VIEWPORT_COLS}): \
                         {rendered_line:?}"
                    );
                    assert!(
                        row > 0 && row < VIEWPORT_ROWS,
                        "{expected_line:?} should remain vertically centered: row={row}, lines={lines:?}"
                    );
                }
            }
        });
    });
}
