use chrono::{DateTime, Duration, Local, TimeZone as _};
use warp_core::command::ExitCode;

use super::*;
use crate::terminal::HistoryEntry;
use crate::terminal::model::session::SessionId;

/// Fixed clock all fixtures are evaluated against, so recency comparisons are deterministic.
fn now() -> DateTime<Local> {
    Local.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap()
}

fn days_ago(days: i64) -> DateTime<Local> {
    now() - Duration::days(days)
}

/// A single candidate to be ranked, built up with the setup that matters for a given fixture and
/// defaulted otherwise (untracked timestamp, no session match, successful exit).
struct Scenario {
    command: &'static str,
    query: &'static str,
    start_ts: Option<DateTime<Local>>,
    session_id: Option<SessionId>,
    exit_ok: bool,
    total_execution_count: u32,
    cwd_execution_count: u32,
    pwd_known_execution_count: u32,
}

impl Scenario {
    fn new(command: &'static str, query: &'static str) -> Self {
        Self {
            command,
            query,
            start_ts: None,
            session_id: None,
            exit_ok: true,
            total_execution_count: 0,
            cwd_execution_count: 0,
            pwd_known_execution_count: 0,
        }
    }

    fn days_ago(mut self, days: i64) -> Self {
        self.start_ts = Some(days_ago(days));
        self
    }

    fn session(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    fn exit_failed(mut self) -> Self {
        self.exit_ok = false;
        self
    }

    fn executed(mut self, total_execution_count: u32) -> Self {
        self.total_execution_count = total_execution_count;
        self
    }

    fn cwd_counts(mut self, cwd_execution_count: u32, pwd_known_execution_count: u32) -> Self {
        self.cwd_execution_count = cwd_execution_count;
        self.pwd_known_execution_count = pwd_known_execution_count;
        self
    }

    fn rank(&self, current_session_id: SessionId) -> OrderedFloat<f64> {
        let tokens = tokenize_query(self.query);
        let (_, match_quality) = match_history_command(self.command, &tokens)
            .expect("scenario command should match its own query");

        let mut entry = HistoryEntry::command_only(self.command.to_owned());
        entry.start_ts = self.start_ts;
        entry.session_id = self.session_id;
        entry.exit_code = Some(ExitCode::from(if self.exit_ok { 0 } else { 1 }));

        rank(RankInputs {
            entry: &entry,
            match_quality,
            now: now(),
            current_session_id,
            total_execution_count: self.total_execution_count,
            cwd_execution_count: self.cwd_execution_count,
            pwd_known_execution_count: self.pwd_known_execution_count,
            is_blank_query: false,
        })
        .expect("scenario match quality should clear the score floor")
    }
}

#[test]
fn older_exact_match_outranks_fresher_weak_match() {
    // Guard case for the +/-20% prior swing (PRIOR_MULTIPLIER_SWING): raw Skim already scores an
    // exact whole-line match ("id" against "id", score 51) well above a scattered weak match
    // ("id" against "list docker containers", score 33), and a swing this narrow can't overturn
    // that gap even at its extremes (0.8x for the 30-day-old exact match vs 1.2x for the
    // brand-new weak one). A wider swing (e.g. +/-50%) would flip this.
    let session = SessionId::from(1);
    let old_exact = Scenario::new("id", "id").days_ago(30);
    let new_weak = Scenario::new("list docker containers", "id").days_ago(0);

    assert!(
        old_exact.rank(session) > new_weak.rank(session),
        "a 30-day-old whole-line match must still outrank a brand new scattered match"
    );
}

#[test]
fn recency_breaks_ties_among_equal_quality_substring_matches() {
    // `OS=linux make bar` is a substring match just like the others (not at column 0, raw Skim
    // score 83 vs 91 for the others); being freshest should still win here because the prior
    // multiplier's swing (0.8x for the 10-day-old matches vs 1.2x for the brand new one)
    // outweighs that small raw gap.
    let session = SessionId::from(1);
    let make_foo = Scenario::new("make foo", "make").days_ago(10);
    let make_bar = Scenario::new("make bar", "make").days_ago(10);
    let make_baz = Scenario::new("make baz", "make").days_ago(10);
    let fresh_make_bar = Scenario::new("OS=linux make bar", "make").days_ago(0);

    let fresh_rank = fresh_make_bar.rank(session);
    for older in [&make_foo, &make_bar, &make_baz] {
        assert!(
            fresh_rank > older.rank(session),
            "a fresh substring match should outrank an equally-old, equally-good substring match"
        );
    }
}

#[test]
fn whitespace_tokenization_ands_terms_across_the_command() {
    let tokens = tokenize_query("cd hi orm");
    assert!(match_history_command("cd ~/projects/history_orm", &tokens).is_some());
    assert!(
        match_history_command("cd ~/projects/other", &tokens).is_none(),
        "a candidate missing one AND-ed term should not match"
    );
}

#[test]
fn consecutive_substrings_beat_scattered_boundary_matches() {
    let tokens = tokenize_query("tcp");
    let (contiguous_raw, contiguous) = match_history_command("adb tcpip 5000", &tokens).unwrap();
    let (scattered_raw, scattered) = match_history_command("txjs-cli push", &tokens).unwrap();

    assert!(
        contiguous_raw.score < scattered_raw.score,
        "raw Skim alone should still get this backwards, confirming the bonus is load-bearing"
    );
    assert!(
        contiguous.adjusted_skim > scattered.adjusted_skim,
        "a contiguous substring match should score higher overall than a scattered one: \
         contiguous={contiguous:?}, scattered={scattered:?}"
    );
}

#[test]
fn session_prior_favors_the_current_session() {
    let session = SessionId::from(7);
    let other_session = SessionId::from(8);

    let same_session = Scenario::new("npm test", "npm test")
        .days_ago(1)
        .session(session);
    let different_session = Scenario::new("npm test", "npm test")
        .days_ago(1)
        .session(other_session);

    assert!(same_session.rank(session) > different_session.rank(session));
}

#[test]
fn exit_failure_is_penalized() {
    let session = SessionId::from(1);
    let succeeded = Scenario::new("deploy prod", "deploy prod").days_ago(1);
    let failed = Scenario::new("deploy prod", "deploy prod")
        .days_ago(1)
        .exit_failed();

    assert!(succeeded.rank(session) > failed.rank(session));
}

#[test]
fn frequency_prior_favors_more_common_commands() {
    let session = SessionId::from(1);
    let frequent = Scenario::new("npm run build", "npm run build")
        .days_ago(1)
        .executed(20);
    let rare = Scenario::new("npm run build", "npm run build")
        .days_ago(1)
        .executed(1);

    assert!(frequent.rank(session) > rare.rank(session));
}

#[test]
fn cwd_prior_favors_the_current_directory() {
    let session = SessionId::from(1);
    let run_here_often = Scenario::new("git status", "git status")
        .days_ago(1)
        .cwd_counts(45, 50);
    let run_elsewhere_often = Scenario::new("git status", "git status")
        .days_ago(1)
        .cwd_counts(5, 50);

    assert!(run_here_often.rank(session) > run_elsewhere_often.rank(session));
}

#[test]
fn unknown_cwd_is_neutral_between_a_match_and_a_mismatch() {
    let session = SessionId::from(1);
    let confirmed_match = Scenario::new("git status", "git status")
        .days_ago(1)
        .cwd_counts(50, 50);
    let unknown_cwd = Scenario::new("git status", "git status").days_ago(1);
    let confirmed_mismatch = Scenario::new("git status", "git status")
        .days_ago(1)
        .cwd_counts(0, 50);

    assert!(
        confirmed_match.rank(session) > unknown_cwd.rank(session),
        "no pwd data at all must not outrank a command confirmed to run here"
    );
    assert!(
        unknown_cwd.rank(session) > confirmed_mismatch.rank(session),
        "no pwd data at all must still outrank a command confirmed to run elsewhere"
    );
}

#[test]
fn cwd_prior_smooths_a_single_observation_toward_neutral() {
    // A single matching run and a well-evidenced 45-of-50 track record both look like "always
    // matched" as a raw fraction (1/1 vs 45/50 ~= 0.9), but confidence should scale with
    // evidence: the single observation must not reach (or beat) the well-evidenced one.
    let session = SessionId::from(1);
    let single_observation = Scenario::new("git status", "git status")
        .days_ago(1)
        .cwd_counts(1, 1);
    let well_evidenced = Scenario::new("git status", "git status")
        .days_ago(1)
        .cwd_counts(45, 50);

    assert!(
        well_evidenced.rank(session) > single_observation.rank(session),
        "a single matching observation shouldn't outrank a well-evidenced track record"
    );
}

#[test]
fn cwd_affinity_smooths_toward_neutral_and_converges_with_evidence() {
    // Direct check of the Beta(K, K)-smoothed fraction itself, pinning down the exact values
    // the constants above produce (K = CWD_PRIOR_SMOOTHING_STRENGTH = 3.0).
    assert_eq!(
        cwd_affinity(0, 0),
        0.5,
        "no pwd data at all must be neutral"
    );
    assert!(
        (cwd_affinity(1, 1) - 4.0 / 7.0).abs() < 1e-9,
        "a single matching observation should land close to, not at, neutral"
    );
    assert!(
        (cwd_affinity(45, 50) - 48.0 / 56.0).abs() < 1e-9,
        "a well-evidenced fraction should barely be shrunk from its raw value"
    );
    assert!(
        (cwd_affinity(5, 50) - 8.0 / 56.0).abs() < 1e-9,
        "a well-evidenced low fraction should stay low"
    );
}

#[test]
fn missing_timestamp_ranks_between_confirmed_fresh_and_confirmed_ancient() {
    // A missing timestamp gets a neutral, mid-range recency (MISSING_TIMESTAMP_RECENCY) rather
    // than being guessed at or treated as confirmed-worst. That ordering -- unknown strictly
    // between a confirmed-fresh and a confirmed-ancient entry -- is the entire point of the
    // neutral value, so pin it down here: a future change to the weights could otherwise
    // collapse it without any other test noticing.
    let session = SessionId::from(1);
    let fresh = Scenario::new("ls -la", "ls -la").days_ago(0);
    let untracked = Scenario::new("ls -la", "ls -la");
    let ancient = Scenario::new("ls -la", "ls -la").days_ago(3650);

    assert!(fresh.rank(session) > untracked.rank(session));
    assert!(untracked.rank(session) > ancient.rank(session));
}

#[test]
fn match_score_floor_filters_out_low_quality_matches() {
    let low_quality = MatchQuality {
        adjusted_skim: 3.0,
        adjusted_skim_per_char: 3.0,
    };
    assert!(low_quality.adjusted_skim_per_char < RAW_SKIM_FLOOR_PER_CHAR);

    let entry = HistoryEntry::command_only("noise".to_owned());
    let result = rank(RankInputs {
        entry: &entry,
        match_quality: low_quality,
        now: now(),
        current_session_id: SessionId::from(1),
        total_execution_count: 0,
        cwd_execution_count: 0,
        pwd_known_execution_count: 0,
        is_blank_query: false,
    });

    assert!(
        result.is_none(),
        "a match below the score floor should be filtered out entirely"
    );
}

#[test]
fn blank_query_ignores_priors_and_yields_a_result() {
    let zero_quality = MatchQuality {
        adjusted_skim: 0.0,
        adjusted_skim_per_char: 0.0,
    };
    assert!(zero_quality.adjusted_skim_per_char < RAW_SKIM_FLOOR_PER_CHAR);

    // Deliberately different start_ts: if a blank query only bypassed the score floor (not
    // priors), these would score differently via the recency prior.
    let mut recent = HistoryEntry::command_only("ls -la".to_owned());
    recent.start_ts = Some(now());

    let mut old = HistoryEntry::command_only("ls -la".to_owned());
    old.start_ts = Some(days_ago(30));

    let rank_of = |entry: &HistoryEntry| {
        rank(RankInputs {
            entry,
            match_quality: zero_quality,
            now: now(),
            current_session_id: SessionId::from(1),
            total_execution_count: 0,
            cwd_execution_count: 0,
            pwd_known_execution_count: 0,
            is_blank_query: true,
        })
    };

    let recent_score = rank_of(&recent);
    let old_score = rank_of(&old);

    assert!(
        recent_score.is_some(),
        "a blank query should bypass the score floor so zero-state history isn't dropped"
    );
    assert_eq!(
        recent_score, old_score,
        "a blank query must ignore priors entirely so the mixer's stable sort preserves \
         chronological order, not just bypass the score floor"
    );
}
