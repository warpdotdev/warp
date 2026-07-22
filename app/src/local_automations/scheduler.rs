//! In-process cron scheduler for local automations (Slice B).
//!
//! Evaluates schedules while Warp is running, persists fire/miss state, and
//! queues automations for the workspace to run via the existing Run now path.

use std::collections::{HashSet, VecDeque};
use std::path::Path;

use chrono::Local;
use warpui::r#async::Timer;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::local_automation::LocalAutomation;
use super::run_state::{path_key, LocalAutomationsRunState};
use super::schedule::{
    decide_schedule, next_fire_after, parse_schedule, AutomationScheduleStatus, ScheduleDecision,
    ScheduleEvalInput, TICK_INTERVAL,
};
use crate::features::FeatureFlag;
use crate::user_config::{WarpConfig, WarpConfigUpdateEvent};

/// Why a pending run was enqueued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledRunReason {
    /// Live cron tick or catch-up within the window.
    Schedule,
}

/// Events emitted by [`LocalAutomationsScheduler`].
#[derive(Debug, Clone)]
pub enum LocalAutomationsSchedulerEvent {
    /// One or more automations were queued for the workspace to start.
    PendingUpdated,
    /// Persisted/status view of schedules changed (list UI should refresh).
    StatusUpdated,
}

/// Pending scheduled start for the workspace consumer.
#[derive(Debug, Clone)]
pub struct PendingScheduledRun {
    pub automation: LocalAutomation,
    #[allow(dead_code)] // reserved for future toast/telemetry differentiation
    pub reason: ScheduledRunReason,
}

pub struct LocalAutomationsScheduler {
    state: LocalAutomationsRunState,
    pending: VecDeque<PendingScheduledRun>,
    /// Paths already sitting in `pending` (avoid duplicate queue entries).
    pending_paths: HashSet<String>,
}

impl LocalAutomationsScheduler {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let state = if cfg!(feature = "local_fs") {
            LocalAutomationsRunState::load()
        } else {
            LocalAutomationsRunState::default()
        };

        if FeatureFlag::LocalAutomations.is_enabled() {
            ctx.subscribe_to_model(&WarpConfig::handle(ctx), |me, _, event, ctx| {
                if matches!(event, WarpConfigUpdateEvent::LocalAutomations) {
                    me.evaluate(ctx);
                }
            });
            // First evaluation once WarpConfig may already be loaded (or empty).
            // Defer so WarpConfig singleton is fully constructed.
            let _ = ctx.spawn(
                async {
                    Timer::after(std::time::Duration::from_millis(500)).await;
                },
                |me, (), ctx| {
                    me.evaluate(ctx);
                    Self::schedule_next_tick(ctx);
                },
            );
        }

        Self {
            state,
            pending: VecDeque::new(),
            pending_paths: HashSet::new(),
        }
    }

    fn schedule_next_tick(ctx: &mut ModelContext<Self>) {
        let _ = ctx.spawn(
            async {
                Timer::after(TICK_INTERVAL).await;
            },
            |me, (), ctx| {
                me.evaluate(ctx);
                Self::schedule_next_tick(ctx);
            },
        );
    }

    /// Evaluate all loaded automations and enqueue due runs.
    pub fn evaluate(&mut self, ctx: &mut ModelContext<Self>) {
        if !FeatureFlag::LocalAutomations.is_enabled() {
            return;
        }
        if !cfg!(feature = "local_fs") {
            return;
        }

        let automations = WarpConfig::handle(ctx)
            .as_ref(ctx)
            .local_automations()
            .clone();

        let now = Local::now();
        let mut keep_paths = HashSet::new();
        let mut state_dirty = false;
        let mut pending_added = false;

        for automation in automations {
            let Some(source_path) = automation.source_path.clone() else {
                continue;
            };
            let key = path_key(&source_path);
            keep_paths.insert(key.clone());

            let entry = self.state.entry(&key).cloned().unwrap_or_default();
            let input = ScheduleEvalInput {
                now,
                enabled: automation.enabled,
                schedule_raw: automation.schedule.clone(),
                last_scheduled_fire_at: entry.last_scheduled_fire_at,
                last_missed_at: entry.last_missed_at,
                in_flight_since: entry.in_flight_since,
            };

            // Also skip if already queued.
            if self.pending_paths.contains(&key) {
                continue;
            }

            match decide_schedule(&input) {
                ScheduleDecision::Fire { due_at } => {
                    let entry = self.state.entry_mut(&key);
                    entry.last_scheduled_fire_at = Some(due_at);
                    // Successful scheduled fire clears outstanding miss for UI.
                    if entry.last_missed_at.is_some() {
                        entry.last_missed_at = None;
                    }
                    entry.in_flight_since = Some(now);
                    state_dirty = true;

                    self.pending.push_back(PendingScheduledRun {
                        automation,
                        reason: ScheduledRunReason::Schedule,
                    });
                    self.pending_paths.insert(key);
                    pending_added = true;
                    log::info!(
                        "Local automation schedule fire queued for {} (due_at={due_at})",
                        source_path.display()
                    );
                }
                ScheduleDecision::Missed { due_at } => {
                    let entry = self.state.entry_mut(&key);
                    if entry.last_missed_at != Some(due_at) {
                        entry.last_missed_at = Some(due_at);
                        entry.missed_count = entry.missed_count.saturating_add(1);
                        state_dirty = true;
                        log::info!(
                            "Local automation missed schedule for {} (due_at={due_at})",
                            source_path.display()
                        );
                    }
                }
                ScheduleDecision::Wait { .. }
                | ScheduleDecision::SkipDisabled
                | ScheduleDecision::SkipInFlight
                | ScheduleDecision::Invalid => {}
            }
        }

        // Prune state for removed files.
        let before_len = self.state.by_path.len();
        self.state.prune_to_paths(&keep_paths);
        if self.state.by_path.len() != before_len {
            state_dirty = true;
        }

        if state_dirty {
            self.state.save();
            ctx.emit(LocalAutomationsSchedulerEvent::StatusUpdated);
        }
        if pending_added {
            ctx.emit(LocalAutomationsSchedulerEvent::PendingUpdated);
        }
    }

    /// Pop the next pending scheduled run (single consumer across windows).
    pub fn pop_pending(&mut self) -> Option<PendingScheduledRun> {
        let pending = self.pending.pop_front()?;
        if let Some(path) = pending.automation.source_path.as_ref() {
            self.pending_paths.remove(&path_key(path));
        }
        Some(pending)
    }

    /// Clear in-flight after the workspace attempted to open the run tab.
    pub fn clear_in_flight(&mut self, path: &Path, ctx: &mut ModelContext<Self>) {
        let key = path_key(path);
        if let Some(entry) = self.state.by_path.get_mut(&key) {
            if entry.in_flight_since.is_some() {
                entry.in_flight_since = None;
                self.state.save();
                ctx.emit(LocalAutomationsSchedulerEvent::StatusUpdated);
            }
        }
    }

    /// Status snapshot for list UI.
    pub fn status_for(&self, automation: &LocalAutomation) -> AutomationScheduleStatus {
        let mut status = AutomationScheduleStatus {
            disabled: !automation.enabled,
            ..Default::default()
        };

        match parse_schedule(&automation.schedule) {
            Ok(schedule) => {
                let now = Local::now();
                status.next_at = if automation.enabled {
                    next_fire_after(&schedule, now)
                } else {
                    None
                };
            }
            Err(_) => {
                status.invalid_schedule = true;
            }
        }

        if let Some(path) = automation.source_path.as_ref() {
            if let Some(entry) = self.state.entry(&path_key(path)) {
                status.last_scheduled_fire_at = entry.last_scheduled_fire_at;
                // Outstanding miss: last_missed_at is set and not superseded by a later fire.
                if let Some(missed_at) = entry.last_missed_at {
                    let superseded = entry
                        .last_scheduled_fire_at
                        .is_some_and(|fire| fire >= missed_at);
                    status.missed = !superseded;
                }
            }
        }

        status
    }
}

impl Entity for LocalAutomationsScheduler {
    type Event = LocalAutomationsSchedulerEvent;
}

impl SingletonEntity for LocalAutomationsScheduler {}
