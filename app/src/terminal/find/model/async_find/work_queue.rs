//! Work queue for the async find background task.
//!
//! The [`FindWorkQueue`] is shared between the main thread (which enqueues work)
//! and the background task (which pulls items via [`FindWorkQueue::pop`]).
//! Internally it uses an [`event_listener::Event`] to efficiently wake the
//! background task when new work is available.

use std::collections::VecDeque;
use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex};

use event_listener::Event;
use warpui::EntityId;

use crate::terminal::block_list_element::GridType;
use crate::terminal::model::blocks::TotalIndex;
use crate::terminal::model::terminal_model::BlockIndex;

use super::BlockInfo;

/// A unit of work for the background find task.
#[derive(Debug, Clone)]
pub enum FindWorkItem {
    /// Scan an entire terminal block.
    FullBlock { block_index: BlockIndex },
    /// Scan a dirty range within a specific grid of a terminal block.
    DirtyRange {
        block_index: BlockIndex,
        grid_type: GridType,
        row_range: RangeInclusive<usize>,
        num_lines_truncated: u64,
    },
    /// Request scanning of an AI block on the main thread.
    AIBlock {
        view_id: EntityId,
        total_index: TotalIndex,
    },
}

/// Error returned by [`FindWorkQueue::pop`] when the queue has been closed.
#[derive(Debug)]
pub struct QueueClosed;

struct FindWorkQueueInner {
    items: VecDeque<FindWorkItem>,
    closed: bool,
}

/// A shared work queue for async find operations.
///
/// The controller enqueues work items from the main thread, and the background
/// task pulls them via the async [`pop`](FindWorkQueue::pop) method. When the
/// queue is empty, `pop` blocks until new work arrives or the queue is closed.
#[derive(Clone)]
pub struct FindWorkQueue {
    inner: Arc<Mutex<FindWorkQueueInner>>,
    event: Arc<Event>,
}

impl FindWorkQueue {
    /// Creates a new empty work queue.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FindWorkQueueInner {
                items: VecDeque::new(),
                closed: false,
            })),
            event: Arc::new(Event::new()),
        }
    }

    /// Populates the queue with initial scan items from a block info list.
    ///
    /// Terminal blocks become [`FindWorkItem::ScanFullBlock`] items and rich
    /// content blocks become [`FindWorkItem::ScanAIBlock`] items. Items are
    /// pushed in the order provided (typically newest-first from
    /// [`collect_block_info`](super::collect_block_info)).
    pub fn enqueue_full_scan(&self, blocks: &[BlockInfo]) {
        let mut inner = self.inner.lock().unwrap();
        for block in blocks {
            let item = match block {
                BlockInfo::Terminal { block_index, .. } => FindWorkItem::FullBlock {
                    block_index: *block_index,
                },
                BlockInfo::RichContent {
                    view_id,
                    total_index,
                } => FindWorkItem::AIBlock {
                    view_id: *view_id,
                    total_index: *total_index,
                },
            };
            inner.items.push_back(item);
        }
        drop(inner);
        // Wake the background task if it is waiting.
        self.event.notify(1);
    }

    /// Enqueues work for a block that has been invalidated.
    ///
    /// If a [`FindWorkItem::ScanFullBlock`] for this block is already pending in
    /// the queue, this is a no-op: the pending scan will pick up the latest
    /// content. Otherwise, the appropriate work item is pushed to the **front**
    /// of the queue so it is processed before remaining initial-scan items.
    pub fn invalidate_block(
        &self,
        block_index: BlockIndex,
        dirty_range: Option<(RangeInclusive<usize>, GridType, u64)>,
    ) {
        let mut inner = self.inner.lock().unwrap();

        // If there is already a pending full scan for this block, do nothing.
        let has_pending_full_scan = inner.items.iter().any(|item| {
            matches!(item, FindWorkItem::FullBlock { block_index: idx } if *idx == block_index)
        });
        if has_pending_full_scan {
            return;
        }

        // Enqueue the appropriate item at the front (high priority).
        let item = match dirty_range {
            Some((row_range, grid_type, num_lines_truncated)) => FindWorkItem::DirtyRange {
                block_index,
                grid_type,
                row_range,
                num_lines_truncated,
            },
            None => FindWorkItem::FullBlock { block_index },
        };
        inner.items.push_front(item);
        drop(inner);
        self.event.notify(1);
    }

    /// Pulls the next work item from the queue.
    ///
    /// If the queue is empty, the returned future blocks until an item is
    /// enqueued or the queue is closed. Returns `Err(QueueClosed)` when the
    /// queue has been closed and no items remain.
    ///
    /// The returned `bool` indicates whether the queue was empty immediately
    /// after the pop (checked atomically within the same lock scope).
    pub async fn pop(&self) -> Result<(FindWorkItem, bool), QueueClosed> {
        loop {
            // Check for an available item or closed state.
            {
                let mut inner = self.inner.lock().unwrap();
                if let Some(item) = inner.items.pop_front() {
                    let is_empty = inner.items.is_empty();
                    return Ok((item, is_empty));
                }
                if inner.closed {
                    return Err(QueueClosed);
                }
            }

            // Queue is empty and not closed. Register a listener before
            // re-checking to avoid a race between the check and the listen.
            let listener = self.event.listen();

            // Re-check after registering the listener.
            {
                let mut inner = self.inner.lock().unwrap();
                if let Some(item) = inner.items.pop_front() {
                    let is_empty = inner.items.is_empty();
                    return Ok((item, is_empty));
                }
                if inner.closed {
                    return Err(QueueClosed);
                }
            }

            // Wait for a notification.
            listener.await;
        }
    }

    /// Closes the queue, waking any blocked [`pop`](FindWorkQueue::pop) call.
    ///
    /// After closing, `pop` will drain remaining items and then return
    /// `Err(QueueClosed)`.
    pub fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.closed = true;
        drop(inner);
        self.event.notify(usize::MAX);
    }

    /// Removes all pending items from the queue.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.items.clear();
    }

    /// Returns `true` if the queue has no pending items.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().items.is_empty()
    }

    /// Returns the number of pending items.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().items.len()
    }
}
