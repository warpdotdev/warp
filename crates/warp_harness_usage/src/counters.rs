use std::collections::BTreeMap;

use serde_json::Value;

use crate::api::{
    Attribution, CostEstimation, CostStatus, Coverage, ThresholdPolicy, ThresholdRule, ToolCalls,
    UsageGroup, UsagePayload, UsageSnapshot, normalize_model,
};
use crate::{Findings, MAX_GROUPS, ReasonCode};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Counters {
    pub(crate) values: [Option<i64>; 6],
}

impl Counters {
    pub(crate) fn parse(value: &Value, paths: [&str; 6], findings: &mut Findings) -> Option<Self> {
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
                    }
                }
            }
        }
        Some(result)
    }

    pub(crate) fn covers(&self, previous: &Self) -> bool {
        self.values
            .iter()
            .zip(previous.values)
            .all(|(current, previous)| match (*current, previous) {
                (Some(current), Some(previous)) => current >= previous,
                (_, None) => true,
                (None, Some(_)) => false,
            })
    }

    pub(crate) fn add_complete(&mut self, other: &Self) -> bool {
        for (sum, count) in self.values.iter_mut().zip(other.values) {
            *sum = match (*sum, count) {
                (Some(sum), Some(count)) => match sum.checked_add(count) {
                    Some(sum) => Some(sum),
                    None => return false,
                },
                _ => None,
            };
        }
        true
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Provider {
    Claude,
    Codex,
}

impl Provider {
    fn output_index(self) -> usize {
        match self {
            Self::Claude => 1,
            Self::Codex => 2,
        }
    }

    fn input(self, usage: &Counters) -> Option<i64> {
        let values = usage.values;
        match self {
            Self::Claude => {
                let [input, output, reads, writes, short, long] = values;
                output?;
                let writes = writes?;
                if (writes > 0 || short.is_some() || long.is_some())
                    && short?.checked_add(long?)? != writes
                {
                    return None;
                }
                input?.checked_add(reads?)?.checked_add(writes)
            }
            Self::Codex => {
                let [input, reads, output, reasoning, total, writes] = values;
                let input = input?;
                let output = output?;
                if reads?.checked_add(writes?)? > input
                    || reasoning.is_some_and(|reasoning| reasoning > output)
                    || total.is_some_and(|total| input.checked_add(output) != Some(total))
                {
                    return None;
                }
                Some(input)
            }
        }
    }
}

struct Group {
    cutoff: Option<i64>,
    pre: Option<Counters>,
    post: Option<Counters>,
}

pub(crate) struct Accounting<'a> {
    provider: Provider,
    policy: Option<&'a ThresholdPolicy>,
    groups: Option<BTreeMap<Attribution, Group>>,
    output: Option<i64>,
    output_overflow: bool,
    output_measured: bool,
    output_partial: bool,
}

impl<'a> Accounting<'a> {
    pub(crate) fn new(provider: Provider, policy: Option<&'a ThresholdPolicy>) -> Self {
        Self {
            provider,
            policy,
            groups: policy.map(|_| BTreeMap::new()),
            output: Some(0),
            output_overflow: false,
            output_measured: false,
            output_partial: false,
        }
    }

    pub(crate) fn invalidate_cost(&mut self) {
        self.groups = None;
    }

    pub(crate) fn request(
        &mut self,
        usage: &Counters,
        attribution: &Attribution,
        findings: &mut Findings,
    ) {
        if !self.output_overflow {
            if let Some(count) = usage.values[self.provider.output_index()] {
                self.output_measured = true;
                self.output = self.output.unwrap_or_default().checked_add(count);
                if self.output.is_none() {
                    self.output_overflow = true;
                    self.invalidate_cost();
                    findings.reason(ReasonCode::ResourceLimit);
                }
            } else {
                self.output_partial = true;
                findings.reason(ReasonCode::IncompleteInput);
            }
        }
        let Some(groups) = &mut self.groups else {
            return;
        };
        let rule = attribution
            .model
            .as_ref()
            .and_then(|model| self.policy?.models.get(&normalize_model(model)));
        let Some(input) = self.provider.input(usage) else {
            self.invalidate_cost();
            findings.reason(ReasonCode::IncompleteInput);
            return;
        };
        let cutoff = match rule {
            Some(ThresholdRule::None) => None,
            Some(ThresholdRule::InputGt { tokens }) => Some(*tokens),
            None => {
                self.invalidate_cost();
                findings.reason(ReasonCode::IncompleteInput);
                return;
            }
        };
        let mut key = attribution.clone();
        key.service_tier = normalized_modifier(key.service_tier, &["", "default", "standard"]);
        key.inference_geo = normalized_modifier(key.inference_geo, &["", "global"]);
        key.speed = normalized_modifier(key.speed, &["", "standard"]);
        if !groups.contains_key(&key) && groups.len() == MAX_GROUPS {
            self.invalidate_cost();
            findings.reason(ReasonCode::ResourceLimit);
            return;
        }
        let group = groups.entry(key).or_insert(Group {
            cutoff,
            pre: None,
            post: None,
        });
        let mut usage = usage.clone();
        if matches!(self.provider, Provider::Claude) && usage.values[3] == Some(0) {
            usage.values[4] = Some(0);
            usage.values[5] = Some(0);
        }
        let band = if cutoff.is_some_and(|cutoff| input > cutoff) {
            &mut group.post
        } else {
            &mut group.pre
        };
        if let Some(sum) = band {
            if !sum.add_complete(&usage) {
                self.invalidate_cost();
                findings.reason(ReasonCode::ResourceLimit);
            }
        } else {
            *band = Some(usage);
        }
    }

    pub(crate) fn finish<T: From<Counters>>(
        mut self,
        tool_calls: Option<ToolCalls>,
        findings: &Findings,
    ) -> UsageSnapshot<T> {
        if findings.tokens_partial || findings.limit_exceeded {
            self.invalidate_cost();
        }
        let cost_estimation = self.groups.map(|groups| CostEstimation {
            groups: groups
                .into_iter()
                .map(|(attribution, group)| UsageGroup {
                    attribution,
                    long_context_threshold_tokens: group.cutoff,
                    pre_threshold: group.pre.map(T::from),
                    post_threshold: group.post.map(T::from),
                })
                .collect(),
        });
        let output = if !self.output_measured
            && (self.output_partial || findings.tokens_partial || findings.limit_exceeded)
        {
            None
        } else {
            self.output
        };
        UsageSnapshot {
            coverage: Coverage {
                cost_status: if cost_estimation.is_some() {
                    CostStatus::Known
                } else {
                    CostStatus::Unavailable
                },
                output_token_status: Findings::status(
                    output.is_some(),
                    self.output_partial || findings.tokens_partial,
                ),
                tool_status: Findings::status(tool_calls.is_some(), findings.tools_partial),
            },
            payload: UsagePayload::new(cost_estimation, output, tool_calls),
        }
    }
}

fn normalized_modifier(value: Option<String>, neutral: &[&str]) -> Option<String> {
    value
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !neutral.contains(&value.as_str()))
}
