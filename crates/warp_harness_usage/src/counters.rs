use std::collections::BTreeMap;

use serde_json::Value;

use crate::{AttributedUsage, Attribution, Findings, MAX_ATTRIBUTIONS, ReasonCode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Counters<const N: usize> {
    pub(crate) values: [Option<i64>; N],
    overflowed: [bool; N],
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
    pub(crate) total: Counters<N>,
    groups: BTreeMap<Attribution, Counters<N>>,
}

impl<const N: usize> Default for Accounting<N> {
    fn default() -> Self {
        Self {
            total: Counters::default(),
            groups: BTreeMap::new(),
        }
    }
}

impl<const N: usize> Accounting<N> {
    pub(crate) fn omit_missing_fields(&mut self, latest: &Counters<N>) {
        self.groups.retain(|_, usage| {
            usage.omit_missing_fields(latest);
            usage.any()
        });
    }

    pub(crate) fn merge(&mut self, other: Self, findings: &mut Findings) {
        if self.total.any() && other.total.any() && !self.total.same_fields(&other.total) {
            findings.token(ReasonCode::IncompleteInput);
        }
        self.total.add(&other.total, findings);
        for (attribution, usage) in other.groups {
            self.attribute(&usage, &attribution, findings);
        }
    }
    pub(crate) fn attribute(
        &mut self,
        usage: &Counters<N>,
        attribution: &Attribution,
        findings: &mut Findings,
    ) {
        if !self.groups.contains_key(attribution) && self.groups.len() >= MAX_ATTRIBUTIONS {
            findings.limit(ReasonCode::ResourceLimit);
            return;
        }
        self.groups
            .entry(attribution.clone())
            .or_default()
            .add(usage, findings);
    }

    pub(crate) fn groups<T: From<Counters<N>>>(self) -> Vec<AttributedUsage<T>> {
        self.groups
            .into_iter()
            .filter(|(_, usage)| usage.any())
            .map(|(attribution, usage)| AttributedUsage {
                attribution,
                usage: usage.into(),
            })
            .collect()
    }
}
