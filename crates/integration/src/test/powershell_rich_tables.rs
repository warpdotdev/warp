//! PowerShell-only coverage for the rich-table prototype.
//!
//! These tests require Warp's session shell to be PowerShell. Linux/macOS CI
//! typically runs bash/zsh, so they skip there. The nearest hermetic layer
//! (bootstrap helper tests + stream unit tests) covers the same behavior on
//! every platform that has `pwsh`.

use sum_tree::SeekBias;
use warp::features::FeatureFlag;
use warp::integration_testing::step::new_step_with_default_assertions;
use warp::integration_testing::terminal::util::{
    ExpectedExitStatus, current_shell_starter_and_version,
};
use warp::integration_testing::terminal::{
    clear_blocklist_to_remove_bootstrapped_blocks, execute_command_for_single_terminal_in_tab,
    wait_until_bootstrapped_single_pane_for_tab,
};
use warp::integration_testing::view_getters::single_terminal_view_for_tab;
use warp::terminal::model::blocks::{BlockHeightItem, TotalIndex};
use warp::terminal::model::rich_content::RichContentType;
use warp::terminal::shell::ShellType;
use warpui_core::async_assert;

use super::new_builder;
use crate::Builder;
use crate::util::{ShellRcType, write_rc_files_for_test};

fn powershell_only() -> bool {
    current_shell_starter_and_version().0.shell_type() == ShellType::PowerShell
}

fn count_powershell_tables(app: &warpui_core::App, window_id: warpui_core::WindowId) -> usize {
    let view = single_terminal_view_for_tab(app, window_id, 0);
    view.read(app, |view, _ctx| {
        let model = view.model.lock();
        let heights = model.block_list().block_heights();
        let mut cursor = heights.cursor::<TotalIndex, ()>();
        cursor.seek(&TotalIndex(0), SeekBias::Left);
        let mut count = 0;
        while let Some(item) = cursor.item() {
            if let BlockHeightItem::RichContent(rich_content) = item
                && rich_content.content_type == Some(RichContentType::PowerShellTable)
            {
                count += 1;
            }
            cursor.next();
        }
        count
    })
}

fn rich_tables_prelude() -> Builder {
    FeatureFlag::PowerShellRichTables.set_enabled(true);
    new_builder()
        .set_should_run_test(powershell_only)
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
}

pub fn test_powershell_rich_tables_implicit_format_and_order() -> Builder {
    rich_tables_prelude()
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "[pscustomobject]@{Name='alpha'; Id=1}, [pscustomobject]@{Name='beta'; Id=2}"
                .to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("implicit objects render as one native table")
                .add_named_assertion("one PowerShell table is mounted", |app, window_id| {
                    let count = count_powershell_tables(app, window_id);
                    async_assert!(
                        count == 1,
                        "expected 1 PowerShell table after implicit output, got {count}"
                    )
                }),
        )
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "[pscustomobject]@{Name='gamma'; Id=3} | Format-Table".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("explicit Format-Table stays plaintext")
                .add_named_assertion(
                    "Format-Table does not mount another native table",
                    |app, window_id| {
                        let count = count_powershell_tables(app, window_id);
                        async_assert!(
                            count == 1,
                            "expected still 1 PowerShell table after Format-Table, got {count}"
                        )
                    },
                )
                .add_named_assertion("Format-Table writes visible plaintext", |app, window_id| {
                    let view = single_terminal_view_for_tab(app, window_id, 0);
                    view.read(app, |view, _ctx| {
                        let output = view
                            .model
                            .lock()
                            .block_list()
                            .last_non_hidden_block()
                            .expect("Format-Table block should exist")
                            .output_to_string();
                        async_assert!(
                            output.contains("gamma"),
                            "Format-Table plaintext should include the object, got {output:?}"
                        )
                    })
                }),
        )
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "[pscustomobject]@{Name='delta'; Id=4}; Write-Output 'after-table'".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("table then plain text keeps both")
                .add_named_assertion(
                    "a second native table is mounted for the object",
                    |app, window_id| {
                        let count = count_powershell_tables(app, window_id);
                        async_assert!(
                            count == 2,
                            "expected 2 PowerShell tables after table-plus-plain, got {count}"
                        )
                    },
                )
                .add_named_assertion("plain text still appears in the block", |app, window_id| {
                    let view = single_terminal_view_for_tab(app, window_id, 0);
                    view.read(app, |view, _ctx| {
                        let output = view
                            .model
                            .lock()
                            .block_list()
                            .last_non_hidden_block()
                            .expect("mixed output block should exist")
                            .output_to_string();
                        async_assert!(
                            output.contains("after-table"),
                            "plain text after a table should remain visible, got {output:?}"
                        )
                    })
                }),
        )
}

pub fn test_powershell_rich_tables_skips_custom_out_default() -> Builder {
    FeatureFlag::PowerShellRichTables.set_enabled(true);
    new_builder()
        .set_should_run_test(powershell_only)
        .with_setup(|utils| {
            write_rc_files_for_test(
                utils.test_dir(),
                r#"
function Out-Default {
    param(
        [Parameter(ValueFromPipeline = $true)]
        [psobject]$InputObject
    )
    process {
        Microsoft.PowerShell.Core\Out-Default
    }
}
"#,
                [ShellRcType::PowerShell],
            );
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(clear_blocklist_to_remove_bootstrapped_blocks())
        .with_step(execute_command_for_single_terminal_in_tab(
            0,
            "[pscustomobject]@{Name='profile'; Id=9}".to_string(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            new_step_with_default_assertions("profile-defined Out-Default is not replaced by Warp")
                .add_named_assertion("no native table is mounted", |app, window_id| {
                    let count = count_powershell_tables(app, window_id);
                    async_assert!(
                        count == 0,
                        "profile Out-Default should keep plaintext, got {count} tables"
                    )
                }),
        )
}
