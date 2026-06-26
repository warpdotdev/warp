use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use parking_lot::FairMutex;
use warp::tui_export::{BlockHeightItem, RichContentItem, RichContentType, TerminalModel};
use warpui::EntityId;
use warpui_core::elements::tui::{TuiViewportIndex, TuiViewportIndexPosition};

use super::{block_rows, AgentBlockRegistry, TerminalHistoryIndex, TerminalHistoryItemId};

#[test]
fn terminal_history_index_uses_canonical_block_list_order() {
    let mut model = TerminalModel::mock(None, None);
    model.simulate_block("echo 1", "1\r\n");
    model.simulate_block("echo 2", "2\r\n");
    let expected = model
        .block_list()
        .blocks()
        .iter()
        .filter(|block| block_rows(block, model.block_list()).is_some())
        .map(|block| TerminalHistoryItemId::TerminalBlock(block.id().clone()))
        .collect::<Vec<_>>();
    let index = TerminalHistoryIndex::new(
        Arc::new(FairMutex::new(model)),
        AgentBlockRegistry::new(RefCell::new(HashMap::new())),
        Rc::new(RefCell::new(HashSet::new())),
    );

    let actual = index.with_cursor(TuiViewportIndexPosition::Start, |cursor| {
        let mut items = Vec::new();
        while let Some(item) = cursor.item() {
            items.push(item.id);
            cursor.next();
        }
        items
    });

    assert_eq!(actual, expected);
}

#[test]
fn tui_agent_rich_content_stays_visible_without_gui_agent_view_state() {
    let mut model = TerminalModel::mock(None, None);
    let view_id = EntityId::new();
    model.block_list_mut().append_rich_content(
        RichContentItem::new(Some(RichContentType::AIBlock), view_id, None, false),
        false,
    );
    model
        .block_list_mut()
        .update_rich_content_heights(&HashMap::from([(view_id, 3.0)]));

    let rich_content = model
        .block_list()
        .block_heights()
        .cursor::<(), ()>()
        .find_map(|item| match item {
            BlockHeightItem::RichContent(item) if item.view_id == view_id => Some(item),
            BlockHeightItem::Block(_)
            | BlockHeightItem::Gap(_)
            | BlockHeightItem::RestoredBlockSeparator { .. }
            | BlockHeightItem::InlineBanner { .. }
            | BlockHeightItem::SubshellSeparator { .. }
            | BlockHeightItem::RichContent(_) => None,
        })
        .expect("TUI agent rich content should remain in the canonical block list");

    assert!(!rich_content.should_hide);
    assert!(rich_content.last_laid_out_height.as_f64() > 0.0);
}
