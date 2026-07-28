//! Claude Code usage, surfaced from the session Claude Code already keeps on disk.
//!
//! Claude's OAuth usage endpoint reports how much of the rolling 5-hour session
//! window (and of the weekly limits) the user has burned through. We poll it with
//! the token Claude Code stores locally — no additional sign-in — so the tab bar
//! can show a live usage percentage next to the other workspace controls.

mod credentials;

use std::time::Duration;

use anyhow::Context as _;
use async_compat::Compat;
use chrono::{DateTime, Utc};
use instant::Instant;
use serde::Deserialize;
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::claude_code_usage::credentials::ClaudeAccessToken;
use crate::features::FeatureFlag;

/// Claude's internal usage endpoint, the same one the Claude Code CLI reads.
const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// Beta header required by the OAuth-authenticated endpoints.
const OAUTH_BETA_HEADER: &str = "oauth-2025-04-20";

/// How often usage is refreshed while a session is readable.
const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// Back-off used when there is no Claude Code session to read; there is nothing
/// to poll until the user runs Claude Code, so check back rarely.
const IDLE_POLL_INTERVAL: Duration = Duration::from_secs(10 * 60);

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Cadence of the Pac-Man chomp.
const CHOMP_FRAME_INTERVAL: Duration = Duration::from_millis(110);

/// How long a chomp burst lasts, so a click always produces a visible bite.
const CHOMP_DURATION: Duration = Duration::from_millis(900);

/// How usage is doing against the session limit. Drives the color of the
/// percentage in the tab bar.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClaudeUsageLevel {
    /// Below 50%.
    Normal,
    /// 50–80%.
    Elevated,
    /// 80–95%.
    High,
    /// Above 95%.
    Critical,
}

impl ClaudeUsageLevel {
    fn from_percent(percent: f32) -> Self {
        if percent < 50. {
            Self::Normal
        } else if percent < 80. {
            Self::Elevated
        } else if percent < 95. {
            Self::High
        } else {
            Self::Critical
        }
    }
}

/// Extra (over-limit) usage, only meaningful when the user has enabled it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClaudeExtraUsage {
    /// Credits spent, in minor currency units (i.e. cents).
    pub used_credits: f64,
    /// Monthly cap, in minor currency units.
    pub monthly_limit: f64,
}

/// A point-in-time view of Claude usage, as shown in the tab bar.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeUsageSnapshot {
    /// Utilization of the rolling 5-hour session window, 0–100.
    pub session_percent: f32,
    pub session_resets_at: Option<DateTime<Utc>>,
    /// Utilization of the weekly limit, 0–100, when the plan has one.
    pub weekly_percent: Option<f32>,
    pub extra_usage: Option<ClaudeExtraUsage>,
}

impl ClaudeUsageSnapshot {
    /// The session percentage, clamped and rounded for display.
    pub fn session_percent_rounded(&self) -> u32 {
        self.session_percent.clamp(0., 100.).round() as u32
    }

    pub fn level(&self) -> ClaudeUsageLevel {
        ClaudeUsageLevel::from_percent(self.session_percent)
    }

    /// A short "2h 18m" style countdown to the session reset, if one is known.
    pub fn time_until_session_reset(&self, now: DateTime<Utc>) -> Option<String> {
        let resets_at = self.session_resets_at?;
        Some(format_countdown(resets_at.signed_duration_since(now)))
    }
}

fn format_countdown(remaining: chrono::TimeDelta) -> String {
    let seconds = remaining.num_seconds();
    if seconds <= 0 {
        return "now".to_string();
    }
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

/// Why usage could not be read. Every variant is caused by the local
/// environment or by Claude's server, so none of them are Sentry-worthy.
#[derive(thiserror::Error, Debug)]
pub enum ClaudeUsageError {
    #[error("No Claude Code session found")]
    NoSession,
    #[error("Claude Code session expired")]
    SessionExpired,
    #[error("Claude rejected the Claude Code session")]
    Unauthorized,
    #[error("Claude returned an unexpected response")]
    BadResponse(reqwest::StatusCode),
    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}

impl ClaudeUsageError {
    /// A single line that can be shown in the tab bar tooltip.
    pub fn user_facing_message(&self) -> String {
        match self {
            Self::NoSession => "Run Claude Code once to show usage here".to_string(),
            Self::SessionExpired | Self::Unauthorized => {
                "Claude Code session expired — run Claude Code to refresh it".to_string()
            }
            Self::BadResponse(status) => format!("Claude returned {status}"),
            Self::Unexpected(_) => "Couldn't reach Claude".to_string(),
        }
    }
}

// MARK: - Usage endpoint

#[derive(Debug, Deserialize)]
struct UsageResponse {
    #[serde(default)]
    five_hour: Option<UsageBucket>,
    #[serde(default)]
    seven_day: Option<UsageBucket>,
    #[serde(default)]
    extra_usage: Option<ExtraUsageResponse>,
}

#[derive(Debug, Deserialize)]
struct UsageBucket {
    #[serde(default)]
    utilization: f32,
    #[serde(default)]
    resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct ExtraUsageResponse {
    #[serde(default)]
    is_enabled: bool,
    #[serde(default)]
    monthly_limit: Option<f64>,
    #[serde(default)]
    used_credits: Option<f64>,
}

impl From<UsageResponse> for ClaudeUsageSnapshot {
    fn from(response: UsageResponse) -> Self {
        let extra_usage = response.extra_usage.and_then(|extra| {
            let (used_credits, monthly_limit) = (extra.used_credits?, extra.monthly_limit?);
            (extra.is_enabled && monthly_limit > 0.).then_some(ClaudeExtraUsage {
                used_credits,
                monthly_limit,
            })
        });
        Self {
            session_percent: response
                .five_hour
                .as_ref()
                .map(|bucket| bucket.utilization)
                .unwrap_or_default(),
            session_resets_at: response.five_hour.and_then(|bucket| bucket.resets_at),
            weekly_percent: response.seven_day.map(|bucket| bucket.utilization),
            extra_usage,
        }
    }
}

/// Reads the local Claude Code session and asks Claude for the current usage.
///
/// `cached_token` is reused while it is still valid so a poll doesn't have to
/// touch the Keychain (which can prompt) once a minute. The token that was
/// actually used is handed back for the next poll.
async fn fetch_usage(
    cached_token: Option<ClaudeAccessToken>,
) -> Result<(ClaudeUsageSnapshot, ClaudeAccessToken), ClaudeUsageError> {
    let access_token = match cached_token {
        Some(token) if token.is_usable(Utc::now()) => token,
        _ => credentials::load_access_token()?,
    };
    let used_token = access_token.clone();

    let response = Compat::new(async move {
        let client = reqwest::Client::builder()
            .https_only(true)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .context("Failed to build the Claude usage HTTP client")?;
        let response = client
            .get(USAGE_URL)
            .bearer_auth(access_token.token)
            .header("anthropic-beta", OAUTH_BETA_HEADER)
            .send()
            .await
            .context("Failed to request Claude usage")?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(ClaudeUsageError::Unauthorized);
        }
        if !status.is_success() {
            return Err(ClaudeUsageError::BadResponse(status));
        }
        response
            .json::<UsageResponse>()
            .await
            .context("Failed to parse the Claude usage response")
            .map_err(ClaudeUsageError::from)
    })
    .await?;

    Ok((response.into(), used_token))
}

// MARK: - Model

/// Tracks Claude Code usage for the tab bar indicator.
pub struct ClaudeCodeUsageModel {
    snapshot: Option<ClaudeUsageSnapshot>,
    /// The most recent failure, kept so the tooltip can explain an empty chip.
    last_error: Option<ClaudeUsageError>,
    is_refreshing: bool,
    /// When the current chomp animation started, and when it should stop. Only
    /// user-initiated refreshes animate: a background poll would otherwise make
    /// the whole tab bar repaint on a timer for no one's benefit.
    chomp_started_at: Option<Instant>,
    chomp_until: Option<Instant>,
    chomp_tick_scheduled: bool,
    /// The session token used by the last poll, reused until it expires.
    cached_token: Option<ClaudeAccessToken>,
}

pub enum ClaudeCodeUsageModelEvent {
    /// Usage, the error state, or the chomp animation frame changed.
    UsageUpdated,
}

/// Who asked for a refresh, which decides whether the Pac-Man chomps.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum RefreshTrigger {
    User,
    Poll,
}

impl Entity for ClaudeCodeUsageModel {
    type Event = ClaudeCodeUsageModelEvent;
}

impl SingletonEntity for ClaudeCodeUsageModel {}

impl ClaudeCodeUsageModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let mut model = Self {
            snapshot: None,
            last_error: None,
            is_refreshing: false,
            chomp_started_at: None,
            chomp_until: None,
            chomp_tick_scheduled: false,
            cached_token: None,
        };
        if FeatureFlag::ClaudeCodeUsageIndicator.is_enabled() {
            // Kick the poll loop off asynchronously; the model isn't in the
            // context yet, so the first fetch can't run inline.
            model.schedule_next_poll(Duration::ZERO, ctx);
        }
        model
    }

    pub fn snapshot(&self) -> Option<&ClaudeUsageSnapshot> {
        self.snapshot.as_ref()
    }

    pub fn last_error(&self) -> Option<&ClaudeUsageError> {
        self.last_error.as_ref()
    }

    /// How far into the chomp animation we are, in frames. `None` while idle,
    /// which renders the resting closed mouth.
    pub fn chomp_frame(&self) -> Option<usize> {
        let started_at = self.chomp_started_at?;
        Some((started_at.elapsed().as_millis() / CHOMP_FRAME_INTERVAL.as_millis()) as usize)
    }

    /// Refreshes in response to the user clicking the indicator, chomping while
    /// it goes.
    pub fn refresh_from_user(&mut self, ctx: &mut ModelContext<Self>) {
        self.refresh(RefreshTrigger::User, ctx);
    }

    /// Fetches usage, then schedules the next poll. Concurrent calls are ignored
    /// so a manual refresh during a poll doesn't double up.
    fn refresh(&mut self, trigger: RefreshTrigger, ctx: &mut ModelContext<Self>) {
        if trigger == RefreshTrigger::User {
            self.start_chomping(ctx);
        }
        if self.is_refreshing {
            return;
        }
        self.is_refreshing = true;

        ctx.spawn(
            fetch_usage(self.cached_token.clone()),
            |model, result, ctx| {
                model.is_refreshing = false;
                let next_poll = match result {
                    Ok((snapshot, token)) => {
                        model.snapshot = Some(snapshot);
                        model.cached_token = Some(token);
                        model.last_error = None;
                        POLL_INTERVAL
                    }
                    Err(err) => {
                        let idle = matches!(
                            err,
                            ClaudeUsageError::NoSession | ClaudeUsageError::SessionExpired
                        );
                        // A rejected token is worth re-reading from disk next time.
                        model.cached_token = None;
                        log::warn!("Failed to refresh Claude Code usage: {err:#}");
                        model.last_error = Some(err);
                        if idle {
                            IDLE_POLL_INTERVAL
                        } else {
                            POLL_INTERVAL
                        }
                    }
                };
                ctx.emit(ClaudeCodeUsageModelEvent::UsageUpdated);
                ctx.notify();
                model.schedule_next_poll(next_poll, ctx);
            },
        );
    }

    fn schedule_next_poll(&mut self, delay: Duration, ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async move {
                Timer::after(delay).await;
            },
            |model, _, ctx| model.refresh(RefreshTrigger::Poll, ctx),
        );
    }

    /// Starts (or extends) the chomp. The minimum burst keeps a chomp visible
    /// even when the refresh resolves from cache in a few milliseconds.
    fn start_chomping(&mut self, ctx: &mut ModelContext<Self>) {
        let now = Instant::now();
        self.chomp_started_at.get_or_insert(now);
        self.chomp_until = Some(now + CHOMP_DURATION);
        if !self.chomp_tick_scheduled {
            self.schedule_chomp_tick(ctx);
        }
    }

    /// Drives the chomp one frame at a time; each tick asks the tab bar to
    /// re-render, and the loop stops once the burst is over.
    fn schedule_chomp_tick(&mut self, ctx: &mut ModelContext<Self>) {
        self.chomp_tick_scheduled = true;
        ctx.spawn(
            async {
                Timer::after(CHOMP_FRAME_INTERVAL).await;
            },
            |model, _, ctx| {
                model.chomp_tick_scheduled = false;
                let still_chomping = model.is_refreshing
                    || model
                        .chomp_until
                        .is_some_and(|until| Instant::now() < until);
                if still_chomping {
                    model.schedule_chomp_tick(ctx);
                } else {
                    model.chomp_started_at = None;
                    model.chomp_until = None;
                }
                ctx.emit(ClaudeCodeUsageModelEvent::UsageUpdated);
                ctx.notify();
            },
        );
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
