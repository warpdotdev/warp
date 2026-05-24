use std::collections::VecDeque;

use black_ui::{Entity, ModelContext};

#[derive(Clone, Copy, Debug)]
pub enum QueuedQueryOrigin {
    InitialCloudMode,
}

#[derive(Debug, Clone)]
pub struct QueuedQuery {
    query: String,
    origin: QueuedQueryOrigin,
}

impl QueuedQuery {
    pub fn initial_cloud_mode(query: String) -> Self {
        Self {
            query,
            origin: QueuedQueryOrigin::InitialCloudMode,
        }
    }

    pub fn query(&self) -> &String {
        &self.query
    }

    pub fn origin(&self) -> QueuedQueryOrigin {
        self.origin
    }
}

pub struct QueuedQueryModel {
    queries: VecDeque<QueuedQuery>,
}

impl QueuedQueryModel {
    pub fn new() -> Self {
        Self {
            queries: VecDeque::new(),
        }
    }

    pub fn queue_query(&mut self, query: QueuedQuery, ctx: &mut ModelContext<Self>) {
        self.queries.push_back(query);
        ctx.emit(QueuedQueryEvent::QueuedQuery);
    }

    pub fn pop_query(&mut self, ctx: &mut ModelContext<Self>) -> Option<QueuedQuery> {
        let query = self.queries.pop_front();
        ctx.emit(QueuedQueryEvent::UnqueuedQuery);
        query
    }

    pub fn has_queries(&self) -> bool {
        !self.queries.is_empty()
    }
}

pub enum QueuedQueryEvent {
    QueuedQuery,
    UnqueuedQuery,
}

impl Entity for QueuedQueryModel {
    type Event = QueuedQueryEvent;
}
