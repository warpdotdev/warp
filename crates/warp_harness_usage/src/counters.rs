use serde_json::Value;

use crate::api::{Attribution, RequestUsage, ToolCalls, UsagePayload};
use crate::{Findings, MAX_REQUESTS, ReasonCode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Counters<const N: usize> {
    pub(crate) values: [Option<i64>; N],
    pub(crate) overflowed: [bool; N],
}

impl<const N: usize> Default for Counters<N> {
    fn default() -> Self {
        Self {
            values: [None; N],
            overflowed: [false; N],
        }
    }
}

impl<const N: usize> Counters<N> {
    /// Parse known nonnegative counters while preserving absent fields as unknown.
    pub(crate) fn parse(value: &Value, paths: [&str; N], findings: &mut Findings) -> Option<Self> {
        if !value.is_object() {
            findings.token(ReasonCode::InvalidData);
            return None;
        }
        let mut result = Self::default();
        for (index, path) in paths.into_iter().enumerate() {
            if let Some(value) = value.pointer(path) {
                match value.as_i64().filter(|count| *count >= 0) {
                    Some(count) => result.values[index] = Some(count),
                    None => {
                        findings.token(ReasonCode::InvalidData);
                        return None;
                    }
                }
            }
        }
        if !result.any() {
            findings.token(ReasonCode::IncompleteInput);
            return None;
        }
        Some(result)
    }

    pub(crate) fn any(&self) -> bool {
        self.values.iter().any(Option::is_some)
    }

    pub(crate) fn restore_overflow(&mut self, overflowed: &[bool]) {
        for (index, overflowed) in overflowed.iter().copied().enumerate().take(N) {
            if overflowed {
                self.values[index] = None;
                self.overflowed[index] = true;
            }
        }
    }

    pub(crate) fn add(&mut self, other: &Self, findings: &mut Findings) {
        for index in 0..N {
            if other.overflowed[index] {
                self.values[index] = None;
                self.overflowed[index] = true;
            }
            if self.overflowed[index] {
                continue;
            }
            if let Some(value) = other.values[index] {
                // An overflowed component stays unknown so later observations cannot make it
                // appear exact again.
                let sum = self.values[index].unwrap_or_default().checked_add(value);
                self.values[index] = sum;
                if sum.is_none() {
                    self.overflowed[index] = true;
                    findings.token(ReasonCode::ResourceLimit);
                }
            }
        }
    }

    pub(crate) fn same_fields(&self, other: &Self) -> bool {
        self.values
            .iter()
            .zip(&other.values)
            .all(|(left, right)| left.is_some() == right.is_some())
    }

    pub(crate) fn decreased(&self, previous: &Self) -> bool {
        self.values
            .iter()
            .zip(&previous.values)
            .any(|(current, previous)| {
                matches!((current, previous), (Some(current), Some(previous)) if current < previous)
            })
    }

    pub(crate) fn delta(&self, previous: &Self) -> Self {
        let mut result = Self::default();
        for index in 0..N {
            if let (Some(current), Some(previous)) = (self.values[index], previous.values[index]) {
                result.values[index] = current.checked_sub(previous).filter(|delta| *delta >= 0);
            }
        }
        result
    }

    pub(crate) fn new_fields(&self, previous: &Self) -> Self {
        let mut result = Self::default();
        for index in 0..N {
            if previous.values[index].is_none() {
                result.values[index] = self.values[index];
            }
        }
        result
    }

    pub(crate) fn matches_observed(&self, observed: &Self) -> bool {
        self.values
            .iter()
            .zip(&observed.values)
            .all(|(current, observed)| observed.is_none() || current == observed)
    }

    fn omit_missing_fields(&mut self, latest: &Self) {
        for index in 0..N {
            if latest.values[index].is_none() {
                self.values[index] = None;
                self.overflowed[index] = false;
            }
        }
    }

    pub(crate) fn covers(&self, previous: &Self) -> bool {
        (0..N).all(|index| match (self.values[index], previous.values[index]) {
            (Some(current), Some(previous)) => current >= previous,
            (_, None) => true,
            (None, Some(_)) => false,
        })
    }
}

pub(crate) struct Accounting<const N: usize> {
    /// Aggregate used only to diagnose counter drift and overflow, never emitted in the payload.
    pub(crate) diagnostic_total: Counters<N>,
    requests: Vec<(Attribution, Counters<N>)>,
    unattributed: Counters<N>,
}

impl<const N: usize> Default for Accounting<N> {
    fn default() -> Self {
        Self {
            diagnostic_total: Counters::default(),
            requests: Vec::new(),
            unattributed: Counters::default(),
        }
    }
}

impl<const N: usize> Accounting<N> {
    pub(crate) fn omit_missing_fields(&mut self, latest: &Counters<N>) {
        self.requests.retain_mut(|(_, usage)| {
            usage.omit_missing_fields(latest);
            usage.any()
        });
        self.unattributed.omit_missing_fields(latest);
    }

    pub(crate) fn merge(&mut self, other: Self, findings: &mut Findings) {
        if self.diagnostic_total.any()
            && other.diagnostic_total.any()
            && !self.diagnostic_total.same_fields(&other.diagnostic_total)
        {
            findings.token(ReasonCode::IncompleteInput);
        }
        self.diagnostic_total.add(&other.diagnostic_total, findings);
        self.unattributed.add(&other.unattributed, findings);
        for (attribution, usage) in other.requests {
            self.request(&usage, &attribution, findings);
        }
    }

    pub(crate) fn request(
        &mut self,
        usage: &Counters<N>,
        attribution: &Attribution,
        findings: &mut Findings,
    ) {
        if self.requests.len() >= MAX_REQUESTS {
            self.unattributed.add(usage, findings);
            findings.token(ReasonCode::ResourceLimit);
        } else {
            self.requests.push((attribution.clone(), usage.clone()));
        }
    }

    pub(crate) fn unassigned(&mut self, usage: &Counters<N>, findings: &mut Findings) {
        self.unattributed.add(usage, findings);
    }

    pub(crate) fn has_usage(&self) -> bool {
        self.unattributed.any() || self.requests.iter().any(|(_, usage)| usage.any())
    }

    pub(crate) fn payload<T: From<Counters<N>>>(
        self,
        tool_calls: Option<ToolCalls>,
    ) -> UsagePayload<T> {
        let unattributed_overflowed = self.unattributed.overflowed.to_vec();
        let unattributed_usage = self.unattributed.any().then(|| self.unattributed.into());
        let requests = self
            .requests
            .into_iter()
            .filter(|(_, usage)| usage.any())
            .map(|(attribution, usage)| RequestUsage {
                attribution,
                usage: usage.into(),
            })
            .collect();
        UsagePayload {
            requests,
            unattributed_usage,
            tool_calls,
            unattributed_overflowed,
        }
    }
}
