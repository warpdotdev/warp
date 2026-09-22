use std::collections::BTreeMap;

use chrono::{TimeZone, Utc};
use serde_json::Value;

use super::*;

const CLAUDE_FIXTURE: &str = include_str!("fixtures/api/claude.json");
const CODEX_FIXTURE: &str = include_str!("fixtures/api/codex.json");

fn request(snapshot: HarnessUsageSnapshot) -> HarnessUsageRequest {
    HarnessUsageRequest::new(
        7,
        3,
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap(),
        snapshot,
    )
}

#[test]
fn requests_match_contract_fixtures() {
    let claude = request(HarnessUsageSnapshot::ClaudeCode(UsageSnapshot {
        coverage: Coverage {
            token_status: CoverageStatus::Known,
            tool_status: CoverageStatus::Known,
        },
        payload: UsagePayload {
            usage: Some(ClaudeUsage {
                input_tokens: Some(9_007_199_254_740_993),
                output_tokens: Some(0),
                cache_read_input_tokens: Some(2),
                cache_creation_input_tokens: Some(3),
                cache_creation: Some(CacheCreation {
                    ephemeral_5m_input_tokens: Some(3),
                    ephemeral_1h_input_tokens: None,
                }),
            }),
            attribution: vec![AttributedUsage {
                attribution: Attribution {
                    model: Some("claude-test".into()),
                    service_tier: Some("standard".into()),
                    inference_geo: Some("us".into()),
                    speed: Some("fast".into()),
                },
                usage: ClaudeUsage {
                    input_tokens: Some(9_007_199_254_740_993),
                    output_tokens: Some(0),
                    cache_read_input_tokens: None,
                    cache_creation_input_tokens: None,
                    cache_creation: None,
                },
            }],
            tool_calls: Some(ToolCalls {
                total: 2,
                by_name: BTreeMap::from([("Read".into(), 1), ("mcp__test__lookup".into(), 1)]),
            }),
        },
    }));
    let codex = request(HarnessUsageSnapshot::Codex(UsageSnapshot {
        coverage: Coverage {
            token_status: CoverageStatus::Partial,
            tool_status: CoverageStatus::Known,
        },
        payload: UsagePayload {
            usage: Some(CodexUsage {
                input_tokens: Some(10),
                cached_input_tokens: Some(0),
                output_tokens: Some(4),
                reasoning_output_tokens: Some(2),
                total_tokens: Some(14),
            }),
            attribution: vec![AttributedUsage {
                attribution: Attribution {
                    model: Some("codex-test".into()),
                    service_tier: Some("priority".into()),
                    ..Default::default()
                },
                usage: CodexUsage {
                    input_tokens: Some(10),
                    cached_input_tokens: None,
                    output_tokens: Some(4),
                    reasoning_output_tokens: None,
                    total_tokens: None,
                },
            }],
            tool_calls: Some(ToolCalls {
                total: 0,
                by_name: BTreeMap::new(),
            }),
        },
    }));

    for (actual, fixture) in [(claude, CLAUDE_FIXTURE), (codex, CODEX_FIXTURE)] {
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::from_str::<Value>(fixture).unwrap()
        );
    }
}
