use std::time::Duration;

use instant::Instant;
use warpui::EntityId;

use super::{
    ACKNOWLEDGE_GRACE, BlockedTask, FOCUS_POLL_INTERVAL, NagEngine, NagSummary,
    RANKED_REPEAT_INTERVAL, UNRANKED_DEBOUNCE, UNRANKED_REPEAT_INTERVAL,
};

/// Distinct ids without touching the global entity counter's ordering
/// assumptions — the engine only ever compares them for equality.
fn id(raw: usize) -> EntityId {
    EntityId::from_usize(raw)
}

fn ranked(raw: usize, project: &str) -> BlockedTask {
    BlockedTask {
        id: id(raw),
        ranked: true,
        project: project.to_owned(),
        in_view: false,
    }
}

fn unranked(raw: usize, project: &str) -> BlockedTask {
    BlockedTask {
        ranked: false,
        ..ranked(raw, project)
    }
}

fn in_view(task: BlockedTask) -> BlockedTask {
    BlockedTask {
        in_view: true,
        ..task
    }
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[test]
fn ranked_project_is_announced_immediately() {
    let mut engine = NagEngine::default();
    let now = Instant::now();

    let outcome = engine.poll(&[ranked(1, "warp")], now);

    assert_eq!(outcome.announced, vec![id(1)]);
    // A lone waiter has better copy available than "1 agent waiting": the
    // caller uses the agent's own prompt.
    assert_eq!(outcome.summary, None);
}

#[test]
fn unranked_project_waits_out_the_debounce() {
    let mut engine = NagEngine::default();
    let now = Instant::now();
    let task = [unranked(1, "scratch")];

    assert!(engine.poll(&task, now).announced.is_empty());
    assert!(
        engine
            .poll(&task, now + UNRANKED_DEBOUNCE - secs(1))
            .announced
            .is_empty()
    );
    assert_eq!(
        engine.poll(&task, now + UNRANKED_DEBOUNCE).announced,
        vec![id(1)]
    );
}

/// The debounce is only worth having if a wait that ends inside it costs the
/// user nothing at all — no banner, no sound, no trace.
#[test]
fn debounce_swallows_a_block_that_clears_in_under_a_minute() {
    let mut engine = NagEngine::default();
    let now = Instant::now();

    assert!(
        engine
            .poll(&[unranked(1, "scratch")], now)
            .announced
            .is_empty()
    );
    // The agent unblocked: it is absent from the observed set.
    let outcome = engine.poll(&[], now + secs(30));

    assert!(outcome.announced.is_empty());
    // No tracked task left means no timer: `next_poll` is the observable form
    // of "the engine is idle".
    assert_eq!(outcome.next_poll, None);

    // And a later poll must not resurrect it.
    assert!(engine.poll(&[], now + secs(600)).announced.is_empty());
}

#[test]
fn ranked_project_repeats_every_three_minutes() {
    let mut engine = NagEngine::default();
    let now = Instant::now();
    let task = [ranked(1, "warp")];

    assert_eq!(engine.poll(&task, now).announced, vec![id(1)]);
    assert!(
        engine
            .poll(&task, now + RANKED_REPEAT_INTERVAL - secs(1))
            .announced
            .is_empty()
    );
    assert_eq!(
        engine.poll(&task, now + RANKED_REPEAT_INTERVAL).announced,
        vec![id(1)]
    );
    assert!(
        engine
            .poll(&task, now + RANKED_REPEAT_INTERVAL + secs(1))
            .announced
            .is_empty(),
        "the repeat restarted the cadence from the announcement, not from the block"
    );
}

#[test]
fn unranked_project_repeats_every_fifteen_minutes() {
    let mut engine = NagEngine::default();
    let now = Instant::now();
    let task = [unranked(1, "scratch")];
    let first = now + UNRANKED_DEBOUNCE;

    assert!(engine.poll(&task, now).announced.is_empty());
    assert_eq!(engine.poll(&task, first).announced, vec![id(1)]);
    assert!(
        engine
            .poll(&task, first + RANKED_REPEAT_INTERVAL)
            .announced
            .is_empty(),
        "an unranked project nagged at the ranked cadence"
    );
    assert_eq!(
        engine
            .poll(&task, first + UNRANKED_REPEAT_INTERVAL)
            .announced,
        vec![id(1)]
    );
}

#[test]
fn looking_at_a_blocked_task_silences_its_cycle() {
    let mut engine = NagEngine::default();
    let now = Instant::now();

    assert_eq!(
        engine.poll(&[ranked(1, "warp")], now).announced,
        vec![id(1)]
    );
    // The user switched to the tab; the repeat that was due never fires.
    let watched = [in_view(ranked(1, "warp"))];
    assert!(
        engine
            .poll(&watched, now + RANKED_REPEAT_INTERVAL)
            .announced
            .is_empty()
    );
    assert!(
        engine
            .poll(&watched, now + RANKED_REPEAT_INTERVAL * 4)
            .announced
            .is_empty(),
        "a task the user is watching kept nagging"
    );
}

/// A task first seen while the user is already on its tab was never worth
/// announcing — the banner would be telling them what is on their screen.
#[test]
fn a_task_blocked_in_view_is_never_announced() {
    let mut engine = NagEngine::default();
    let now = Instant::now();

    let outcome = engine.poll(&[in_view(ranked(1, "warp"))], now);

    assert!(outcome.announced.is_empty());
    assert!(
        outcome.next_poll.is_some(),
        "the task must still be tracked, so that looking away re-arms it"
    );
}

#[test]
fn looking_away_re_arms_the_nag_after_the_grace() {
    let mut engine = NagEngine::default();
    let now = Instant::now();
    let watched = [in_view(ranked(1, "warp"))];
    let task = [ranked(1, "warp")];

    engine.poll(&watched, now);
    // Looked away at `now`; the grace runs from the poll that observed it.
    let away = now + secs(10);
    assert!(engine.poll(&task, away).announced.is_empty());
    assert!(
        engine
            .poll(&task, away + ACKNOWLEDGE_GRACE - secs(1))
            .announced
            .is_empty()
    );
    assert_eq!(
        engine.poll(&task, away + ACKNOWLEDGE_GRACE).announced,
        vec![id(1)],
        "the nag never came back after the user walked away"
    );
}

/// Answering the prompt is the only permanent stop, and it must hold even
/// after the user has acknowledged and walked away once.
#[test]
fn leaving_blocked_stops_the_nag_for_good() {
    let mut engine = NagEngine::default();
    let now = Instant::now();
    let task = [ranked(1, "warp")];

    assert_eq!(engine.poll(&task, now).announced, vec![id(1)]);
    // Unblocked: gone from the observed set.
    let quiet = engine.poll(&[], now + secs(10));
    assert!(quiet.announced.is_empty());
    assert_eq!(quiet.next_poll, None);

    // Even far past every cadence, silence.
    assert!(
        engine
            .poll(&[], now + UNRANKED_REPEAT_INTERVAL * 4)
            .announced
            .is_empty()
    );
}

/// Spec §8: a session whose pane is closed while blocked (the agent was
/// killed, the tab was closed) must never nag forever. It simply stops being
/// observed, which is the same path as unblocking.
#[test]
fn a_vanished_session_is_forgotten() {
    let mut engine = NagEngine::default();
    let now = Instant::now();

    engine.poll(&[ranked(1, "warp"), ranked(2, "inbox")], now);
    let outcome = engine.poll(&[ranked(2, "inbox")], now + RANKED_REPEAT_INTERVAL);

    assert_eq!(outcome.announced, vec![id(2)]);
    assert!(!outcome.announced.contains(&id(1)));
}

#[test]
fn several_waiters_coalesce_into_one_banner() {
    let mut engine = NagEngine::default();
    let now = Instant::now();

    let outcome = engine.poll(&[ranked(1, "inbox-ai-flow"), ranked(2, "warp")], now);

    assert_eq!(outcome.announced, vec![id(1), id(2)]);
    assert_eq!(
        outcome.summary,
        Some(NagSummary {
            title: "2 agents waiting".to_owned(),
            body: "inbox-ai-flow, warp".to_owned(),
        })
    );
}

#[test]
fn the_coalesced_banner_counts_agents_and_names_projects() {
    let mut engine = NagEngine::default();
    let now = Instant::now();

    let outcome = engine.poll(
        &[
            ranked(1, "inbox-ai-flow"),
            ranked(2, "warp"),
            ranked(3, "dotfiles"),
            ranked(4, "notes"),
        ],
        now,
    );

    assert_eq!(
        outcome.summary,
        Some(NagSummary {
            title: "4 agents waiting".to_owned(),
            body: "inbox-ai-flow, warp +2".to_owned(),
        })
    );
}

/// Two agents blocked in one repo is still one place to go look, so the
/// project earns one mention, not two.
#[test]
fn the_coalesced_banner_names_each_project_once() {
    let mut engine = NagEngine::default();
    let now = Instant::now();

    let outcome = engine.poll(&[ranked(1, "warp"), ranked(2, "warp")], now);

    assert_eq!(
        outcome.summary,
        Some(NagSummary {
            title: "2 agents waiting".to_owned(),
            body: "warp".to_owned(),
        })
    );
}

/// A task still inside its debounce is not yet something the user has been
/// told about, so it must not inflate the count on somebody else's banner.
#[test]
fn a_debouncing_task_is_left_out_of_the_banner() {
    let mut engine = NagEngine::default();
    let now = Instant::now();

    let outcome = engine.poll(&[ranked(1, "warp"), unranked(2, "scratch")], now);

    assert_eq!(outcome.announced, vec![id(1)]);
    assert_eq!(outcome.summary, None);
}

/// An acknowledged task is one the user is looking at; counting it would make
/// the banner claim they are being asked about something they can see.
#[test]
fn an_acknowledged_task_is_left_out_of_the_banner() {
    let mut engine = NagEngine::default();
    let now = Instant::now();

    let outcome = engine.poll(&[ranked(1, "warp"), in_view(ranked(2, "inbox"))], now);

    assert_eq!(outcome.announced, vec![id(1)]);
    assert_eq!(outcome.summary, None);
}

/// One agent falling due speaks for all of them, and everything it spoke for
/// restarts its cadence — otherwise a second banner would follow moments later
/// for the task that was already named in the first.
#[test]
fn a_re_announcement_restarts_every_named_cadence() {
    let mut engine = NagEngine::default();
    let now = Instant::now();
    let tasks = [ranked(1, "warp"), unranked(2, "scratch")];

    // The ranked one fires alone; the unranked one is still debouncing.
    assert_eq!(engine.poll(&tasks, now).announced, vec![id(1)]);
    // Debounce over: both are armed, so both are named.
    let both = now + UNRANKED_DEBOUNCE;
    assert_eq!(engine.poll(&tasks, both).announced, vec![id(1), id(2)]);
    // The ranked one's cadence restarted from the coalesced announcement.
    assert!(
        engine
            .poll(&tasks, both + RANKED_REPEAT_INTERVAL - secs(1))
            .announced
            .is_empty()
    );
    assert_eq!(
        engine.poll(&tasks, both + RANKED_REPEAT_INTERVAL).announced,
        vec![id(1), id(2)],
        "the unranked task should ride along with the ranked one's banner"
    );
}

#[test]
fn nothing_blocked_means_no_timer() {
    let mut engine = NagEngine::default();

    assert_eq!(engine.poll(&[], Instant::now()).next_poll, None);
}

#[test]
fn the_poll_interval_is_capped_so_a_glance_is_noticed() {
    let mut engine = NagEngine::default();
    let now = Instant::now();

    // Armed three minutes out, but the user could walk over at any moment.
    let outcome = engine.poll(&[ranked(1, "warp")], now);
    assert_eq!(outcome.next_poll, Some(FOCUS_POLL_INTERVAL));

    // Watching it has no deadline of its own at all, and still polls.
    let outcome = engine.poll(&[in_view(ranked(1, "warp"))], now + secs(1));
    assert_eq!(outcome.next_poll, Some(FOCUS_POLL_INTERVAL));
}

/// A deadline nearer than the cap wins, so the debounce lands on time rather
/// than at the next 30s boundary after it.
#[test]
fn a_nearer_deadline_beats_the_cap() {
    let mut engine = NagEngine::default();
    let now = Instant::now();
    let task = [unranked(1, "scratch")];

    // First sight arms a 60s debounce, which is further out than the cap.
    assert_eq!(engine.poll(&task, now).next_poll, Some(FOCUS_POLL_INTERVAL));
    // 45s in, the debounce is 15s away and is now the soonest thing to do.
    assert_eq!(engine.poll(&task, now + secs(45)).next_poll, Some(secs(15)));
}

#[test]
fn reset_forgets_everything() {
    let mut engine = NagEngine::default();
    let now = Instant::now();

    engine.poll(&[ranked(1, "warp")], now);
    engine.reset();

    // A fresh cycle, not a resumed one: the task is announced again at once.
    assert_eq!(
        engine.poll(&[ranked(1, "warp")], now + secs(1)).announced,
        vec![id(1)]
    );
}
