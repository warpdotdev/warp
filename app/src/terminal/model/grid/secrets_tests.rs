use regex::Regex;

use super::*;
use crate::terminal::SizeInfo;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::ansi::{self, Handler as _};
use crate::terminal::model::blockgrid::BlockGrid;
use crate::terminal::model::secrets::RespectObfuscatedSecrets;

fn secret_ranges(grid_handler: &GridHandler) -> Vec<RangeInclusive<Point>> {
    grid_handler
        .secrets
        .ranges()
        .map(|(range, _)| range)
        .collect_vec()
}

fn empty_blockgrid(
    rows: usize,
    columns: usize,
    max_scroll_limit: usize,
    secret_obfuscation_mode: ObfuscateSecrets,
) -> BlockGrid {
    BlockGrid::new(
        SizeInfo::new_without_font_metrics(rows, columns),
        max_scroll_limit,
        ChannelEventListener::new_for_test(),
        secret_obfuscation_mode,
        Default::default(),
    )
}

#[test]
fn test_secret_redacted_after_byte_processing() {
    crate::terminal::model::secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("ABCD").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    let mut blockgrid = empty_blockgrid(5, 10, 1, ObfuscateSecrets::Yes);
    let grid_handler = blockgrid.grid_handler_mut();

    // Nothing has been inserted into the grid, there should be no secrets.
    assert!(grid_handler.secrets.is_empty());

    grid_handler.input_at_cursor("ABCD");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // There should be one secret in the range (0,0)..(0,3).
    assert_eq!(grid_handler.secrets.len(), 1);
    assert_eq!(
        secret_ranges(grid_handler),
        vec![Point::new(0, 0)..=Point::new(0, 3)]
    );

    // Delete one character (by moving the cursor back and replacing the cell's content with a
    // space).
    grid_handler.update_cursor(|cursor| cursor.point = cursor.point.wrapping_sub(10, 1));
    grid_handler.grid_storage_mut().cursor_cell().c = ' ';
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // There should no longer be any secrets ("ABC" does not match the regex).
    assert!(grid_handler.secrets.is_empty());
    assert!(secret_ranges(grid_handler).is_empty());
}

#[test]
fn test_secret_redacted_after_multibyte_prefix() {
    let secret = "AAAAC3NzaC1lZDI1NTE5AAAAIThisIsNotARealKeyItIsJustForTestingWarpRedact";
    crate::terminal::model::secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("AAAAC3NzaC1lZDI1NTE5[A-Za-z0-9+/=]{20,}")
            .expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    let mut blockgrid = empty_blockgrid(10, 10, 1, ObfuscateSecrets::Yes);
    let grid_handler = blockgrid.grid_handler_mut();

    grid_handler.input_at_cursor(&format!("💣 {secret}"));
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // 💣 is a single char; input_at_cursor advances the cursor by one column per
    // char regardless of display width, so the secret starts at col 2 (bomb + space).
    let secret_start = Point::new(0, 2);
    let secret_end = Point::new(7, 1);

    assert_eq!(grid_handler.secrets.len(), 1);
    assert_eq!(secret_ranges(grid_handler), vec![secret_start..=secret_end]);
    assert_eq!(
        grid_handler.bounds_to_string(
            Point::new(0, 0),
            secret_end,
            false,
            RespectObfuscatedSecrets::Yes,
            false,
            RespectDisplayedOutput::No,
        ),
        format!("💣 {}", "*".repeat(secret.len()))
    );
}

#[test]
fn test_secret_with_word_boundaries_redacted_after_multibyte_prefix() {
    crate::terminal::model::secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new(r"\bTOKEN123\b").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    let mut blockgrid = empty_blockgrid(5, 20, 1, ObfuscateSecrets::Yes);
    let grid_handler = blockgrid.grid_handler_mut();

    grid_handler.input_at_cursor("💣 TOKEN123");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // Same column arithmetic as above: bomb = col 0, space = col 1, secret starts col 2.
    let secret_start = Point::new(0, 2);
    let secret_end = Point::new(0, 9);

    assert_eq!(grid_handler.secrets.len(), 1);
    assert_eq!(secret_ranges(grid_handler), vec![secret_start..=secret_end]);
    assert_eq!(
        grid_handler.bounds_to_string(
            Point::new(0, 0),
            secret_end,
            false,
            RespectObfuscatedSecrets::Yes,
            false,
            RespectDisplayedOutput::No,
        ),
        "💣 ********"
    );
}

#[test]
fn test_secret_redacted_after_multiple_byte_processing() {
    crate::terminal::model::secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("ABCD").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    let mut blockgrid = empty_blockgrid(5, 10, 1, ObfuscateSecrets::Yes);
    let grid_handler = blockgrid.grid_handler_mut();

    // Nothing has been inserted into the grid, there should be no secrets.
    assert!(grid_handler.secrets.is_empty());

    grid_handler.input_at_cursor("AB");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // There should no longer be any secrets ("AB" does not match the regex).
    assert!(grid_handler.secrets.is_empty());
    assert!(secret_ranges(grid_handler).is_empty());

    // Insert "C" into the grid.
    grid_handler.input_at_cursor("C");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // There shouldn't be any secrets ("ABC" does not match the regex).
    assert!(grid_handler.secrets.is_empty());
    assert!(secret_ranges(grid_handler).is_empty());

    // Insert "D" into the grid.
    grid_handler.input_at_cursor("D");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // There should be one secret in the range (0,0)..(0,3).
    assert_eq!(grid_handler.secrets.len(), 1);
    assert_eq!(
        secret_ranges(grid_handler),
        vec![Point::new(0, 0)..=Point::new(0, 3)]
    );
}

#[test]
fn test_secret_redaction_unobfuscated_secret_remains_after_byte_processing() -> anyhow::Result<()> {
    crate::terminal::model::secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("ABCD").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    let mut blockgrid = empty_blockgrid(5, 10, 1, ObfuscateSecrets::Yes);
    let grid_handler = blockgrid.grid_handler_mut();

    // Nothing has been inserted into the grid, there should be no secrets.
    assert!(grid_handler.secrets.is_empty());

    grid_handler.input_at_cursor("ABCD");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // There should be one secret in the range (0,0)..(0,3).
    assert_eq!(grid_handler.secrets.len(), 1);
    assert_eq!(
        secret_ranges(grid_handler),
        vec![Point::new(0, 0)..=Point::new(0, 3)]
    );

    let secret_handle = grid_handler
        .secrets
        .iter()
        .map(|(handle, _)| *handle)
        .next()
        .expect("Should be at least one secret in the grid");
    grid_handler.unobfuscate_secret(secret_handle)?;

    for _ in 0..2 {
        grid_handler.update_cursor(|cursor| cursor.point = cursor.point.wrapping_sub(10, 1));
        grid_handler.grid_storage_mut().cursor_cell().c = ' ';
    }

    grid_handler.input_at_cursor("CD");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // There should be one secret still.
    assert_eq!(grid_handler.secrets.len(), 1);
    assert_eq!(
        secret_ranges(grid_handler),
        vec![Point::new(0, 0)..=Point::new(0, 3)]
    );

    let secret = grid_handler
        .secrets
        .iter()
        .map(|(_, secret)| secret.clone())
        .next()
        .expect("Should be at least one secret in the grid");

    assert!(!secret.is_obfuscated());

    Ok(())
}

#[test]
fn test_secret_redaction_secret_remains_after_resize() {
    crate::terminal::model::secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("ABCD").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    let mut blockgrid = empty_blockgrid(2, 5, 2, ObfuscateSecrets::Yes);
    let grid_handler = blockgrid.grid_handler_mut();

    // Nothing has been inserted into the grid, there should be no secrets.
    assert!(grid_handler.secrets.is_empty());

    grid_handler.input_at_cursor("ABCDE");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // There should be one secret in the range (0,0)..(0,3).
    assert_eq!(grid_handler.secrets.len(), 1);
    assert_eq!(
        secret_ranges(grid_handler),
        vec![Point::new(0, 0)..=Point::new(0, 3)]
    );

    // Resize the grid, to only include 2 columns. This will cause the the last two cells in the
    // secret to wrap to the next line.
    let size_info = SizeInfo::new_without_font_metrics(2, 2);
    grid_handler.resize(size_info);
    assert_eq!(grid_handler.total_rows(), 4);

    // The range should now be from (0,0)..=(1,1)
    assert_eq!(grid_handler.secrets.len(), 1);
    assert_eq!(
        secret_ranges(grid_handler),
        vec![Point::new(0, 0)..=Point::new(1, 1)]
    );
}

#[test]
fn test_bytes_processed_for_secrets_after_turning_redaction() {
    crate::terminal::model::secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("abcd").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    let mut blockgrid = empty_blockgrid(2, 4, 2, ObfuscateSecrets::Yes);
    let grid_handler = blockgrid.grid_handler_mut();

    grid_handler.input_at_cursor("abcd");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // There should be one secret in the range (0,0)..(0,3).
    assert_eq!(grid_handler.secrets.len(), 1);
    assert_eq!(
        secret_ranges(grid_handler),
        vec![Point::new(0, 0)..=Point::new(0, 3)]
    );
    assert!(grid_handler.all_bytes_scanned_for_secrets());

    // Turn off secret obfuscation.
    grid_handler.obfuscate_secrets(ObfuscateSecrets::No);

    // There should still only be one secret and all bytes should not be processed.
    grid_handler.input_at_cursor("abcd");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert_eq!(grid_handler.secrets.len(), 1);
    assert!(!grid_handler.all_bytes_scanned_for_secrets());
}

#[test]
fn test_bytes_processed_for_secrets_after_turning_redaction_on() {
    crate::terminal::model::secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("abcd").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    let mut blockgrid = empty_blockgrid(2, 4, 2, ObfuscateSecrets::No);
    let grid_handler = blockgrid.grid_handler_mut();

    grid_handler.input_at_cursor("abcd");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // There should no secrets.
    assert_eq!(grid_handler.secrets.len(), 0);
    assert!(!grid_handler.all_bytes_scanned_for_secrets());

    // Turn secret obfuscation on.
    grid_handler.obfuscate_secrets(ObfuscateSecrets::Yes);

    // Insert another secret and assert that all bytes are not processed.
    grid_handler.input_at_cursor("abcd");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert!(!grid_handler.all_bytes_scanned_for_secrets());
}

#[test]
fn test_bytes_processed_for_secrets_after_turning_redaction_off_then_on() {
    crate::terminal::model::secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("abcd").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    let mut blockgrid = empty_blockgrid(3, 4, 2, ObfuscateSecrets::Yes);
    let grid_handler = blockgrid.grid_handler_mut();

    grid_handler.input_at_cursor("abcd");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // There should be one secret in the range (0,0)..(0,3).
    assert_eq!(grid_handler.secrets.len(), 1);
    assert_eq!(
        secret_ranges(grid_handler),
        vec![Point::new(0, 0)..=Point::new(0, 3)]
    );
    assert!(grid_handler.all_bytes_scanned_for_secrets());

    // Turn secret obfuscation off.
    grid_handler.obfuscate_secrets(ObfuscateSecrets::No);
    grid_handler.input_at_cursor("abcd");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert_eq!(grid_handler.secrets.len(), 1);
    assert!(!grid_handler.all_bytes_scanned_for_secrets());

    // Turn secret obfuscation back on, and ensure that all bytes are still not processed.
    grid_handler.obfuscate_secrets(ObfuscateSecrets::Yes);
    grid_handler.input_at_cursor("abcd");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert!(!grid_handler.all_bytes_scanned_for_secrets());
}

#[test]
fn test_bytes_processed_for_secrets_after_turning_redaction_on_then_off() {
    crate::terminal::model::secrets::set_user_and_enterprise_secret_regexes(
        [&Regex::new("abcd").expect("Should be able to construct regex")],
        std::iter::empty(), // No enterprise secrets
    );

    let mut blockgrid = empty_blockgrid(3, 4, 2, ObfuscateSecrets::No);
    let grid_handler = blockgrid.grid_handler_mut();

    grid_handler.input_at_cursor("abcd");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));

    // There should be one secret in the range (0,0)..(0,3).
    assert_eq!(grid_handler.secrets.len(), 0);
    assert!(!grid_handler.all_bytes_scanned_for_secrets());

    // Turn secret obfuscation on.
    grid_handler.obfuscate_secrets(ObfuscateSecrets::Yes);
    grid_handler.input_at_cursor("abcd");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert!(!grid_handler.all_bytes_scanned_for_secrets());

    // Turn secret obfuscation back off, and ensure that all bytes are still not processed.
    grid_handler.obfuscate_secrets(ObfuscateSecrets::No);
    grid_handler.input_at_cursor("abcd");
    grid_handler.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert!(!grid_handler.all_bytes_scanned_for_secrets());
}
