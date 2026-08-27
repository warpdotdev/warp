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
/// defaulted otherwise (untracked timestamp, no session/cwd match, successful exit).
struct Scenario {
    command: &'static str,
    query: &'static str,
    start_ts: Option<DateTime<Local>>,
    frequency: u32,
    session_id: Option<SessionId>,
    pwd: Option<&'static str>,
    exit_ok: bool,
    newer_candidate_count: usize,
}

impl Scenario {
    fn new(command: &'static str, query: &'static str) -> Self {
        Self {
            command,
            query,
            start_ts: None,
            frequency: 1,
            session_id: None,
            pwd: None,
            exit_ok: true,
            newer_candidate_count: 0,
        }
    }

    fn days_ago(mut self, days: i64) -> Self {
        self.start_ts = Some(days_ago(days));
        self
    }

    fn frequency(mut self, frequency: u32) -> Self {
        self.frequency = frequency;
        self
    }

    fn newer_candidates(mut self, count: usize) -> Self {
        self.newer_candidate_count = count;
        self
    }

    fn session(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    fn pwd(mut self, pwd: &'static str) -> Self {
        self.pwd = Some(pwd);
        self
    }

    fn exit_failed(mut self) -> Self {
        self.exit_ok = false;
        self
    }

    fn rank(&self, current_session_id: SessionId, current_cwd: Option<&str>) -> OrderedFloat<f64> {
        let tokens = tokenize_query(self.query);
        let (_, match_quality) = match_history_command(self.command, &tokens)
            .expect("scenario command should match its own query");

        let mut entry = HistoryEntry::command_only(self.command.to_owned());
        entry.start_ts = self.start_ts;
        entry.session_id = self.session_id;
        entry.pwd = self.pwd.map(str::to_owned);
        entry.exit_code = Some(ExitCode::from(if self.exit_ok { 0 } else { 1 }));

        rank(RankInputs {
            entry: &entry,
            frequency: self.frequency,
            match_quality,
            now: now(),
            current_session_id,
            current_cwd,
            newer_candidate_count: self.newer_candidate_count,
            is_blank_query: false,
        })
        .expect("scenario match quality should clear the score floor")
    }
}

#[test]
fn older_exact_match_outranks_fresher_weak_match() {
    // Guard case: the ordering gate (exact_line, match_band) must stop history priors from
    // ever letting a fresh weak match outrank an older exact one.
    let session = SessionId::from(1);
    let old_exact = Scenario::new("id", "id").days_ago(30);
    let new_weak = Scenario::new("list docker containers", "id").days_ago(0);

    assert!(
        old_exact.rank(session, None) > new_weak.rank(session, None),
        "a 30-day-old whole-line match must still outrank a brand new scattered match"
    );
}

#[test]
fn recency_breaks_ties_among_equal_quality_substring_matches() {
    // Failure mode 1 (warp#6126, #3430, #5588, #6344): every `make *` candidate used to score
    // identically regardless of recency, so a fresh command could get buried. `OS=linux make
    // bar` is a substring match just like the others (not at column 0), and being freshest
    // should now win.
    let session = SessionId::from(1);
    let make_foo = Scenario::new("make foo", "make").days_ago(10);
    let make_bar = Scenario::new("make bar", "make").days_ago(10);
    let make_baz = Scenario::new("make baz", "make").days_ago(10);
    let fresh_make_bar = Scenario::new("OS=linux make bar", "make").days_ago(0);

    let fresh_rank = fresh_make_bar.rank(session, None);
    for older in [&make_foo, &make_bar, &make_baz] {
        assert!(
            fresh_rank > older.rank(session, None),
            "a fresh substring match should outrank an equally-old, equally-good substring match"
        );
    }
}

#[test]
fn whitespace_tokenization_ands_terms_across_the_command() {
    // Failure mode 2 (warp#4174): a literal space in the query previously had to appear
    // literally in the command, so "cd hi orm" matched nothing in
    // "cd ~/projects/history_orm" despite every term being present.
    let tokens = tokenize_query("cd hi orm");
    assert!(match_history_command("cd ~/projects/history_orm", &tokens).is_some());
    assert!(
        match_history_command("cd ~/projects/other", &tokens).is_none(),
        "a candidate missing one AND-ed term should not match"
    );
}

#[test]
fn consecutive_substrings_beat_scattered_boundary_matches() {
    // Failure mode 3 (warp#1810): Skim's word-boundary bonus made "txjs-cli push" (a scattered
    // match) outscore "adb tcpip 5000" (a tight, contiguous match) for query "tcp"; real fzf
    // reverses this.
    let tokens = tokenize_query("tcp");
    let (_, contiguous) = match_history_command("adb tcpip 5000", &tokens).unwrap();
    let (_, scattered) = match_history_command("txjs-cli push", &tokens).unwrap();

    assert!(contiguous.consecutive > scattered.consecutive);
    assert!(
        contiguous.combined() > scattered.combined(),
        "a contiguous substring match should score higher overall than a scattered one: \
         contiguous={contiguous:?}, scattered={scattered:?}"
    );
}

#[test]
fn frequency_prior_favors_more_common_commands() {
    let session = SessionId::from(1);
    let frequent = Scenario::new("git status", "git status")
        .days_ago(1)
        .frequency(20);
    let rare = Scenario::new("git status", "git status")
        .days_ago(1)
        .frequency(1);

    assert!(frequent.rank(session, None) > rare.rank(session, None));
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

    assert!(same_session.rank(session, None) > different_session.rank(session, None));
}

#[test]
fn cwd_prior_favors_the_current_directory() {
    let session = SessionId::from(1);
    let same_cwd = Scenario::new("npm test", "npm test")
        .days_ago(1)
        .pwd("/repo");
    let different_cwd = Scenario::new("npm test", "npm test")
        .days_ago(1)
        .pwd("/other");

    assert!(same_cwd.rank(session, Some("/repo")) > different_cwd.rank(session, Some("/repo")));
}

#[test]
fn exit_failure_is_penalized() {
    let session = SessionId::from(1);
    let succeeded = Scenario::new("deploy prod", "deploy prod").days_ago(1);
    let failed = Scenario::new("deploy prod", "deploy prod")
        .days_ago(1)
        .exit_failed();

    assert!(succeeded.rank(session, None) > failed.rank(session, None));
}

#[test]
fn missing_timestamp_falls_back_to_list_position_instead_of_reading_as_infinitely_old() {
    // History-file rows with no matching sqlite record have no start_ts. A recent one (near the
    // end of the chronological candidate list) should still read as recent, not vanish behind
    // every timestamped entry.
    let session = SessionId::from(1);
    let recent_untracked = Scenario::new("ls -la", "ls -la").newer_candidates(0);
    let old_untracked = Scenario::new("ls -la", "ls -la").newer_candidates(200);

    assert!(recent_untracked.rank(session, None) > old_untracked.rank(session, None));
}

#[test]
fn match_score_floor_filters_out_low_quality_matches() {
    let low_quality = MatchQuality {
        exact: 0.0,
        skim: 0.1,
        consecutive: 0.1,
        tightness: 0.1,
    };
    assert!(low_quality.combined() < MATCH_SCORE_FLOOR);

    let entry = HistoryEntry::command_only("noise".to_owned());
    let result = rank(RankInputs {
        entry: &entry,
        frequency: 1,
        match_quality: low_quality,
        now: now(),
        current_session_id: SessionId::from(1),
        current_cwd: None,
        newer_candidate_count: 0,
        is_blank_query: false,
    });

    assert!(
        result.is_none(),
        "a match below the score floor should be filtered out entirely"
    );
}

#[test]
fn blank_query_bypasses_the_score_floor() {
    // `SearchMixer` invokes history with an empty query for the zero state (`run_in_zero_state:
    // true`), where match quality is necessarily zero. The floor must not drop every candidate
    // in that case -- ranking should fall back to history priors instead.
    let low_quality = MatchQuality {
        exact: 0.0,
        skim: 0.0,
        consecutive: 0.0,
        tightness: 0.0,
    };
    assert!(low_quality.combined() < MATCH_SCORE_FLOOR);

    let entry = HistoryEntry::command_only("ls -la".to_owned());
    let result = rank(RankInputs {
        entry: &entry,
        frequency: 1,
        match_quality: low_quality,
        now: now(),
        current_session_id: SessionId::from(1),
        current_cwd: None,
        newer_candidate_count: 0,
        is_blank_query: true,
    });

    assert!(
        result.is_some(),
        "a blank query should bypass the score floor so zero-state history isn't dropped"
    );
}
