use anyhow::{Context, Result};
use serde::Deserialize;
use warp_cli::agent::Harness;

const OPENUSAGE_BASE_URL: &str = "http://127.0.0.1:6736/v1/usage";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageSnapshot {
    lines: Vec<UsageLine>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OpenUsageSummary {
    pub(crate) display: String,
    pub(crate) usage_fraction: Option<f32>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum UsageLine {
    Progress {
        label: String,
        used: f64,
        limit: f64,
    },
    Text {
        label: String,
        value: String,
    },
    Badge {
        label: String,
        value: String,
    },
    #[serde(other)]
    Other,
}

pub(crate) async fn provider_summary(harness: Harness) -> Result<Option<OpenUsageSummary>> {
    let Some(provider_id) = provider_id_for_harness(harness) else {
        return Ok(None);
    };
    let url = format!("{OPENUSAGE_BASE_URL}/{provider_id}");
    let response = reqwest::get(url)
        .await
        .context("failed to connect to OpenUsage local API")?;
    if response.status() == reqwest::StatusCode::NO_CONTENT
        || response.status() == reqwest::StatusCode::NOT_FOUND
    {
        return Ok(None);
    }
    let response = response
        .error_for_status()
        .context("OpenUsage local API returned an error")?;
    let snapshot: UsageSnapshot = response
        .json()
        .await
        .context("failed to parse OpenUsage local API response")?;

    let mut usage_fraction = None;
    let lines: Vec<String> = snapshot
        .lines
        .into_iter()
        .filter_map(|line| match line {
            UsageLine::Progress { label, used, limit } if limit > 0.0 => {
                let fraction = (used / limit).clamp(0.0, 1.0) as f32;
                if usage_fraction.is_none() || label.to_ascii_lowercase().contains("total") {
                    usage_fraction = Some(fraction);
                }
                let remaining_pct = ((1.0 - fraction) * 100.0).round() as i32;
                Some(format!("{label}: {remaining_pct}% left"))
            }
            UsageLine::Progress { label, used, .. } => Some(format!("{label}: {used:.0} used")),
            UsageLine::Text { label, value } | UsageLine::Badge { label, value } => {
                Some(format!("{label}: {value}"))
            }
            UsageLine::Other => None,
        })
        .collect();

    if lines.is_empty() {
        Ok(None)
    } else {
        Ok(Some(OpenUsageSummary {
            display: lines.join("\n"),
            usage_fraction,
        }))
    }
}

fn provider_id_for_harness(harness: Harness) -> Option<&'static str> {
    match harness {
        Harness::Claude => Some("claude"),
        Harness::Codex => Some("codex"),
        Harness::Cursor => Some("cursor"),
        Harness::Devin => Some("devin"),
        Harness::Gemini => Some("gemini"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_use_percent_left_and_newlines() {
        let snapshot = UsageSnapshot {
            lines: vec![
                UsageLine::Progress {
                    label: "Total usage".to_string(),
                    used: 19.0,
                    limit: 100.0,
                },
                UsageLine::Progress {
                    label: "Auto usage".to_string(),
                    used: 24.0,
                    limit: 100.0,
                },
                UsageLine::Progress {
                    label: "API usage".to_string(),
                    used: 0.0,
                    limit: 100.0,
                },
            ],
        };

        let mut usage_fraction = None;
        let lines: Vec<String> = snapshot
            .lines
            .into_iter()
            .filter_map(|line| match line {
                UsageLine::Progress { label, used, limit } if limit > 0.0 => {
                    let fraction = (used / limit).clamp(0.0, 1.0) as f32;
                    if usage_fraction.is_none() || label.to_ascii_lowercase().contains("total") {
                        usage_fraction = Some(fraction);
                    }
                    let remaining_pct = ((1.0 - fraction) * 100.0).round() as i32;
                    Some(format!("{label}: {remaining_pct}% left"))
                }
                _ => None,
            })
            .collect();

        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "Total usage: 81% left");
        assert_eq!(lines[1], "Auto usage: 76% left");
        assert_eq!(lines[2], "API usage: 100% left");
        assert_eq!(lines.join("\n"), "Total usage: 81% left\nAuto usage: 76% left\nAPI usage: 100% left");
        assert_eq!(usage_fraction, Some(0.19));
    }
}
