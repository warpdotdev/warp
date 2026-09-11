use std::collections::{BTreeMap, HashMap};

use crate::{Findings, MAX_IDENTITIES, MAX_TOOL_NAMES, ReasonCode, ToolCalls, identifier};

#[derive(Default)]
pub(crate) struct Tools {
    calls: HashMap<(String, String), Option<String>>,
}

impl Tools {
    pub(crate) fn observe(
        &mut self,
        session: &str,
        id: Option<&str>,
        name: Option<&str>,
        findings: &mut Findings,
    ) {
        let (Some(id), Some(name)) = (
            id.filter(|id| !id.is_empty()),
            name.filter(|name| !name.is_empty()),
        ) else {
            findings.tool(ReasonCode::MissingIdentity);
            return;
        };
        if !identifier(id, findings) || !identifier(name, findings) {
            return;
        }
        let key = (session.to_owned(), id.to_owned());
        if let Some(existing) = self.calls.get_mut(&key) {
            if existing.as_deref().is_some_and(|existing| existing != name) {
                *existing = None;
                findings.tool(ReasonCode::ConflictingTool);
            }
        } else if self.calls.len() >= MAX_IDENTITIES {
            findings.limit(ReasonCode::CollectionLimit);
        } else {
            self.calls.insert(key, Some(name.to_owned()));
        }
    }

    pub(crate) fn finish(self, readable: bool, findings: &mut Findings) -> Option<ToolCalls> {
        let mut by_name = BTreeMap::new();
        let mut total = 0_i64;
        for name in self.calls.into_values().flatten() {
            if !by_name.contains_key(&name) && by_name.len() >= MAX_TOOL_NAMES {
                findings.limit(ReasonCode::CollectionLimit);
                return None;
            }
            let count = by_name.entry(name).or_insert(0_i64);
            let (Some(next_total), Some(next_count)) = (total.checked_add(1), count.checked_add(1))
            else {
                findings.tool(ReasonCode::CounterOverflow);
                return None;
            };
            total = next_total;
            *count = next_count;
        }
        (total > 0 || (readable && !findings.tools_partial)).then_some(ToolCalls { total, by_name })
    }
}
