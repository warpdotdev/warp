"""Tests for classify_pr.py.

Covers:
  - Bot/dependency PR exclusion
  - CI-only, tests-only, docs-only, tooling-only exclusion
  - Mixed internal + user-facing files remaining candidates
  - Stable excluding preview and dogfood flags
  - Preview including preview-flag candidates but excluding dogfood flags
  - Dev not excluding on flag tier
  - Flag-registry touches with no detected flag becoming low-confidence review items
  - Unknown contributors remaining unauthenticated for attribution
  - Explicit changelog marker PRs not requiring agent classification
  - Agent cannot override mechanical rules (enforcement test)
  - Valid candidate classifications are accepted (subjective merge test)
"""

import sys
import os
import unittest

# Add scripts directory to path for direct import
_SCRIPTS_DIR = os.path.abspath(
    os.path.join(os.path.dirname(__file__), "..", "scripts")
)
if _SCRIPTS_DIR not in sys.path:
    sys.path.insert(0, _SCRIPTS_DIR)

import classify_pr  # noqa: E402  (import after sys.path modification)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

FEATURE_FLAGS = {
    "release_flags": ["Autoupdate", "DarkMode"],
    "preview_flags": ["Orchestration", "AgentModeV2"],
    "dogfood_flags": ["LogExpensiveFrames", "DebugPanel"],
}

CONTRIBUTORS = {
    "internal": ["alice", "bob"],
    "external": ["contrib-user"],
    "bot": ["warp-bot", "warp-bot[bot]"],
    "unknown": ["unknown-user"],
}


def _make_pr(
    number: int,
    author: str = "alice",
    title: str = "Fix something",
    body: str = "",
    changed_files: list[str] | None = None,
    explicit_entries: list[dict] | None = None,
) -> dict:
    return {
        "number": number,
        "url": f"https://github.com/warpdotdev/warp/pull/{number}",
        "title": title,
        "author": author,
        "body": body,
        "labels": [],
        "merged_at": "2026-06-01T10:00:00Z",
        "explicit_entries": explicit_entries or [],
        "linked_issues": [],
        "changed_files": changed_files or ["app/src/main.rs"],
        "source_repo": "warpdotdev/warp",
    }


def _prs_json(prs: list[dict]) -> dict:
    return {
        "range": {"base": "v0.prev.stable_00", "head": "v0.head.stable_00"},
        "prs": prs,
    }


BOT_BUCKET: set[str] = set(CONTRIBUTORS["bot"])
UNKNOWN_BUCKET: set[str] = set(CONTRIBUTORS["unknown"])


# ---------------------------------------------------------------------------
# Helper to run pass1 on a list of PRs
# ---------------------------------------------------------------------------

def _pass1(prs: list[dict], channel: str = "stable") -> dict:
    return classify_pr.run_pass1(
        prs, channel, FEATURE_FLAGS, BOT_BUCKET, UNKNOWN_BUCKET
    )


def _pass2(prs: list[dict], agent: list[dict], channel: str = "stable") -> dict:
    return classify_pr.run_pass2(
        prs, channel, FEATURE_FLAGS, BOT_BUCKET, UNKNOWN_BUCKET, agent
    )


# ---------------------------------------------------------------------------
# Bot/dependency PR exclusion
# ---------------------------------------------------------------------------

class TestBotExclusion(unittest.TestCase):

    def test_known_bot_excluded(self):
        """PRs from known bots are deterministically excluded."""
        pr = _make_pr(1, author="dependabot[bot]")
        result = _pass1([pr])
        self.assertEqual(len(result["classifications"]), 1)
        c = result["classifications"][0]
        self.assertFalse(c["include"])
        self.assertIn("bot_author", c["matched_rules"])
        self.assertEqual(c["source"], "deterministic")

    def test_warp_bot_excluded(self):
        """PRs from warp-bot are excluded via contributors.json bot bucket."""
        pr = _make_pr(2, author="warp-bot")
        result = _pass1([pr])
        c = result["classifications"][0]
        self.assertFalse(c["include"])
        self.assertIn("bot_author", c["matched_rules"])

    def test_arbitrary_bot_suffix_excluded(self):
        """Any author ending in [bot] is excluded."""
        pr = _make_pr(3, author="some-unknown-bot[bot]")
        result = _pass1([pr])
        c = result["classifications"][0]
        self.assertFalse(c["include"])
        self.assertIn("bot_author", c["matched_rules"])

    def test_internal_author_is_candidate(self):
        """Internal authors are candidates (not excluded by bot rule)."""
        pr = _make_pr(4, author="alice")
        result = _pass1([pr])
        self.assertEqual(len(result["agent_required"]), 1)
        self.assertEqual(len(result["classifications"]), 0)


# ---------------------------------------------------------------------------
# Internal files-only exclusion
# ---------------------------------------------------------------------------

class TestInternalFilesExclusion(unittest.TestCase):

    def test_ci_only_files_excluded(self):
        """PRs touching only .github/workflows/ are excluded."""
        pr = _make_pr(10, changed_files=[".github/workflows/ci.yml"])
        result = _pass1([pr])
        c = result["classifications"][0]
        self.assertFalse(c["include"])
        self.assertIn("internal_files_only", c["matched_rules"])

    def test_tests_only_files_excluded(self):
        """PRs touching only test files are excluded."""
        pr = _make_pr(11, changed_files=["app/tests/main_test.rs", "crates/foo_tests.rs"])
        result = _pass1([pr])
        c = result["classifications"][0]
        self.assertFalse(c["include"])
        self.assertIn("internal_files_only", c["matched_rules"])

    def test_docs_only_files_excluded(self):
        """PRs touching only .md files are excluded."""
        pr = _make_pr(12, changed_files=["README.md", "docs/guide.md"])
        result = _pass1([pr])
        c = result["classifications"][0]
        self.assertFalse(c["include"])
        self.assertIn("internal_files_only", c["matched_rules"])

    def test_tooling_only_scripts_excluded(self):
        """PRs touching only scripts/ are excluded."""
        pr = _make_pr(13, changed_files=["scripts/deploy.sh", "scripts/bootstrap"])
        result = _pass1([pr])
        c = result["classifications"][0]
        self.assertFalse(c["include"])
        self.assertIn("internal_files_only", c["matched_rules"])

    def test_lock_file_only_excluded(self):
        """PRs touching only .lock files are excluded."""
        pr = _make_pr(14, changed_files=["Cargo.lock"])
        result = _pass1([pr])
        c = result["classifications"][0]
        self.assertFalse(c["include"])
        self.assertIn("internal_files_only", c["matched_rules"])

    def test_mixed_files_remain_candidate(self):
        """PRs with a mix of internal and user-facing files remain candidates."""
        pr = _make_pr(
            15,
            changed_files=["README.md", "app/src/user_visible.rs"],
        )
        result = _pass1([pr])
        self.assertEqual(len(result["agent_required"]), 1, "Mixed PR must remain a candidate")
        self.assertEqual(len(result["classifications"]), 0)

    def test_empty_changed_files_is_candidate(self):
        """Empty changed_files list is treated as unknown (candidate)."""
        pr = _make_pr(16, changed_files=[])
        result = _pass1([pr])
        self.assertEqual(len(result["agent_required"]), 1)

    def test_integration_test_files_excluded(self):
        """crates/integration/ paths are internal-only."""
        pr = _make_pr(17, changed_files=["crates/integration/src/lib.rs"])
        result = _pass1([pr])
        c = result["classifications"][0]
        self.assertFalse(c["include"])


# ---------------------------------------------------------------------------
# Feature flag channel gating
# ---------------------------------------------------------------------------

class TestFeatureFlagGating(unittest.TestCase):

    def test_stable_excludes_preview_flag(self):
        """Stable excludes PRs mentioning a preview-only flag."""
        pr = _make_pr(
            20,
            title="Add Orchestration support",
            body="This adds FeatureFlag::Orchestration to the UI",
        )
        result = _pass1([pr], channel="stable")
        c = result["classifications"][0]
        self.assertFalse(c["include"])
        self.assertIn("feature_flag_gate", c["matched_rules"])
        self.assertIn("Orchestration", c["feature_flags"])

    def test_stable_excludes_dogfood_flag(self):
        """Stable excludes PRs mentioning a dogfood-only flag."""
        pr = _make_pr(
            21,
            title="Refactor logging",
            body="Uses FeatureFlag::LogExpensiveFrames for perf tracking",
        )
        result = _pass1([pr], channel="stable")
        c = result["classifications"][0]
        self.assertFalse(c["include"])
        self.assertIn("feature_flag_gate", c["matched_rules"])

    def test_preview_includes_preview_flag_as_candidate(self):
        """Preview channel keeps preview-flagged PRs as candidates (not excluded)."""
        pr = _make_pr(
            22,
            title="Add Orchestration support",
            body="This adds FeatureFlag::Orchestration to the UI",
        )
        result = _pass1([pr], channel="preview")
        # Should be a candidate, not deterministically excluded
        self.assertEqual(len(result["agent_required"]), 1)
        self.assertEqual(len(result["classifications"]), 0)

    def test_preview_excludes_dogfood_flag(self):
        """Preview channel still excludes dogfood-flagged PRs."""
        pr = _make_pr(
            23,
            title="Add debug panel",
            body="Uses FeatureFlag::DebugPanel",
        )
        result = _pass1([pr], channel="preview")
        c = result["classifications"][0]
        self.assertFalse(c["include"])
        self.assertIn("feature_flag_gate", c["matched_rules"])

    def test_dev_does_not_exclude_on_flag_tier(self):
        """Dev channel doesn't exclude based on flag tier."""
        pr = _make_pr(
            24,
            title="Add Orchestration support",
            body="FeatureFlag::Orchestration and FeatureFlag::DebugPanel",
        )
        result = _pass1([pr], channel="dev")
        # No deterministic exclusion for flag gating on dev
        self.assertEqual(len(result["agent_required"]), 1)
        self.assertEqual(len(result["classifications"]), 0)

    def test_release_flag_remains_candidate(self):
        """PRs behind a release flag are candidates (not excluded by any channel)."""
        pr = _make_pr(
            25,
            title="Enable autoupdate",
            body="FeatureFlag::Autoupdate is now in RELEASE_FLAGS",
        )
        result = _pass1([pr], channel="stable")
        self.assertEqual(len(result["agent_required"]), 1)

    def test_flag_file_touch_without_detected_flag_is_low_confidence_candidate(self):
        """Touching crates/warp_features/src/lib.rs without a detected flag
        results in a low-confidence needs_review candidate."""
        pr = _make_pr(
            26,
            body="Refactored feature registry",
            changed_files=["crates/warp_features/src/lib.rs"],
        )
        result = _pass1([pr], channel="stable")
        # Must remain a candidate (not excluded)
        self.assertEqual(len(result["agent_required"]), 1)
        candidate = result["agent_required"][0]
        self.assertTrue(candidate.get("uncertain_flag_touch"))


# ---------------------------------------------------------------------------
# Explicit entries PRs
# ---------------------------------------------------------------------------

class TestExplicitEntries(unittest.TestCase):

    def test_pr_with_explicit_entry_skipped_by_classify(self):
        """PRs with explicit CHANGELOG-* entries are not classified (handled by assemble)."""
        pr = _make_pr(
            30,
            explicit_entries=[{"category": "IMPROVEMENT", "text": "Faster tab switching"}],
        )
        result = _pass1([pr])
        self.assertEqual(len(result["classifications"]), 0)
        self.assertEqual(len(result["agent_required"]), 0)
        self.assertEqual(result["summary"]["skipped_explicit_entries"], 1)

    def test_pr_with_none_entry_skipped(self):
        """PRs with CHANGELOG-NONE are also skipped by classify (handled by assemble)."""
        pr = _make_pr(
            31,
            explicit_entries=[{"category": "NONE", "text": ""}],
        )
        result = _pass1([pr])
        self.assertEqual(len(result["classifications"]), 0)
        self.assertEqual(len(result["agent_required"]), 0)
        self.assertEqual(result["summary"]["skipped_explicit_entries"], 1)


# ---------------------------------------------------------------------------
# Unknown contributors
# ---------------------------------------------------------------------------

class TestUnknownContributors(unittest.TestCase):

    def test_unknown_contributor_becomes_candidate_with_flag(self):
        """Unknown contributors remain candidates (not excluded) but are flagged."""
        pr = _make_pr(40, author="unknown-user")
        result = _pass1([pr])
        self.assertEqual(len(result["agent_required"]), 1)
        self.assertIn("unknown-user", result["unknown_contributors"])
        candidate = result["agent_required"][0]
        self.assertTrue(candidate.get("unknown_contributor"))


# ---------------------------------------------------------------------------
# Enforcement: agent cannot override mechanical rules
# ---------------------------------------------------------------------------

class TestMechanicalEnforcement(unittest.TestCase):

    def test_agent_cannot_override_bot_exclusion(self):
        """Pass 2 must fail non-zero when agent tries to include a bot PR."""
        pr = _make_pr(50, author="dependabot[bot]")
        agent = [
            {
                "pr_number": 50,
                "include": True,
                "category": "IMPROVEMENT",
                "text": "Bumped dep",
                "confidence": "high",
                "rationale": "Seems useful",
                "needs_review": False,
            }
        ]
        with self.assertRaises(SystemExit) as ctx:
            _pass2([pr], agent)
        self.assertNotEqual(ctx.exception.code, 0)

    def test_agent_cannot_override_tooling_exclusion(self):
        """Pass 2 must fail non-zero when agent tries to include a CI-only PR."""
        pr = _make_pr(51, changed_files=[".github/workflows/ci.yml"])
        agent = [
            {
                "pr_number": 51,
                "include": True,
                "category": "IMPROVEMENT",
                "text": "Updated CI",
                "confidence": "medium",
                "rationale": "...",
                "needs_review": False,
            }
        ]
        with self.assertRaises(SystemExit) as ctx:
            _pass2([pr], agent)
        self.assertNotEqual(ctx.exception.code, 0)

    def test_agent_cannot_override_flag_gate_exclusion(self):
        """Pass 2 must fail non-zero when agent tries to include a hidden-flag PR."""
        pr = _make_pr(
            52,
            body="Uses FeatureFlag::Orchestration for preview feature",
        )
        agent = [
            {
                "pr_number": 52,
                "include": True,
                "category": "IMPROVEMENT",
                "text": "New orchestration UI",
                "confidence": "high",
                "rationale": "...",
                "needs_review": False,
            }
        ]
        with self.assertRaises(SystemExit) as ctx:
            _pass2([pr], agent, channel="stable")
        self.assertNotEqual(ctx.exception.code, 0)


# ---------------------------------------------------------------------------
# Subjective merge: valid candidate classifications accepted
# ---------------------------------------------------------------------------

class TestSubjectiveMerge(unittest.TestCase):

    def test_valid_included_entry_accepted(self):
        """Pass 2 accepts a valid agent-included entry."""
        pr = _make_pr(60, author="alice")
        agent = [
            {
                "pr_number": 60,
                "include": True,
                "category": "IMPROVEMENT",
                "text": "Faster tab switching",
                "confidence": "high",
                "rationale": "Clear user-visible performance win",
                "needs_review": False,
            }
        ]
        result = _pass2([pr], agent)
        c = next(c for c in result["classifications"] if c["pr_number"] == 60)
        self.assertTrue(c["include"])
        self.assertEqual(c["category"], "IMPROVEMENT")
        self.assertEqual(c["text"], "Faster tab switching")
        self.assertEqual(c["source"], "agent")

    def test_valid_excluded_entry_accepted(self):
        """Pass 2 accepts a valid agent-excluded entry."""
        pr = _make_pr(61, author="alice")
        agent = [
            {
                "pr_number": 61,
                "include": False,
                "category": None,
                "text": None,
                "confidence": "high",
                "rationale": "Pure refactor, no user-visible change",
                "needs_review": False,
            }
        ]
        result = _pass2([pr], agent)
        c = next(c for c in result["classifications"] if c["pr_number"] == 61)
        self.assertFalse(c["include"])

    def test_low_confidence_forces_needs_review(self):
        """Low confidence agent answers force needs_review=True."""
        pr = _make_pr(62, author="alice")
        agent = [
            {
                "pr_number": 62,
                "include": True,
                "category": "IMPROVEMENT",
                "text": "Maybe faster something",
                "confidence": "low",
                "rationale": "Not sure if user-visible",
                "needs_review": False,  # agent said False, but confidence=low overrides
            }
        ]
        result = _pass2([pr], agent)
        c = next(c for c in result["classifications"] if c["pr_number"] == 62)
        self.assertTrue(c["needs_review"])

    def test_invalid_category_rejected(self):
        """Invalid category in agent answer causes non-zero exit."""
        pr = _make_pr(63, author="alice")
        agent = [
            {
                "pr_number": 63,
                "include": True,
                "category": "INVALID_CATEGORY",
                "text": "Some text",
                "confidence": "high",
                "rationale": "...",
                "needs_review": False,
            }
        ]
        with self.assertRaises(SystemExit) as ctx:
            _pass2([pr], agent)
        self.assertNotEqual(ctx.exception.code, 0)

    def test_empty_text_for_included_entry_rejected(self):
        """Empty changelog text for an included entry causes non-zero exit."""
        pr = _make_pr(64, author="alice")
        agent = [
            {
                "pr_number": 64,
                "include": True,
                "category": "IMPROVEMENT",
                "text": "",  # empty
                "confidence": "high",
                "rationale": "...",
                "needs_review": False,
            }
        ]
        with self.assertRaises(SystemExit) as ctx:
            _pass2([pr], agent)
        self.assertNotEqual(ctx.exception.code, 0)

    def test_missing_agent_classification_becomes_needs_review(self):
        """Candidate PR with no agent classification is marked needs_review."""
        pr = _make_pr(65, author="alice")
        result = _pass2([pr], agent=[])
        c = next(c for c in result["classifications"] if c["pr_number"] == 65)
        self.assertTrue(c["needs_review"])


# ---------------------------------------------------------------------------
# classify_one_pr return value (regression: was returning None instead of candidate)
# ---------------------------------------------------------------------------

class TestClassifyOnePrReturnValue(unittest.TestCase):

    def test_candidate_pr_returns_dict_not_none(self):
        """classify_one_pr must return a candidate dict (not None) for agent-required PRs.

        Regression test: previously the function built the candidate dict and then
        discarded it by returning None, forcing callers to duplicate the logic.
        """
        pr = _make_pr(100, author="alice")
        result = classify_pr.classify_one_pr(
            pr,
            channel="stable",
            feature_flags=FEATURE_FLAGS,
            bot_bucket=BOT_BUCKET,
            unknown_bucket=UNKNOWN_BUCKET,
        )
        self.assertIsNotNone(result, "classify_one_pr must return a candidate dict, not None")
        self.assertIsInstance(result, dict)
        self.assertEqual(result.get("pr_number"), 100)
        self.assertIn("url", result)
        self.assertIn("title", result)
        self.assertIn("changed_files", result)
        self.assertEqual(result.get("source"), "candidate")

    def test_candidate_dict_fields_propagate_to_agent_required(self):
        """Candidate fields (uncertain_flag_touch, unknown_contributor) reach agent_required."""
        pr = _make_pr(
            101,
            body="Refactored feature registry",
            changed_files=["crates/warp_features/src/lib.rs"],
            author="unknown-user",
        )
        result = _pass1([pr])
        self.assertEqual(len(result["agent_required"]), 1)
        candidate = result["agent_required"][0]
        self.assertTrue(candidate.get("uncertain_flag_touch"))
        self.assertTrue(candidate.get("unknown_contributor"))


# ---------------------------------------------------------------------------
# Summary accounting
# ---------------------------------------------------------------------------

class TestSummaryAccounting(unittest.TestCase):

    def test_summary_counts_correct(self):
        """Summary counts match actual classifications."""
        prs = [
            _make_pr(70, author="dependabot[bot]"),  # bot → exclude
            _make_pr(71, changed_files=[".github/workflows/ci.yml"]),  # CI → exclude
            _make_pr(72, author="alice"),  # candidate
            _make_pr(73, explicit_entries=[{"category": "IMPROVEMENT", "text": "x"}]),  # explicit
        ]
        result = _pass1(prs)
        s = result["summary"]
        self.assertEqual(s["skipped_explicit_entries"], 1)
        self.assertEqual(s["deterministic_exclude"], 2)
        self.assertEqual(s["agent_classify"], 1)
        self.assertEqual(s["total_unmarked"], 3)  # 4 total - 1 explicit


if __name__ == "__main__":
    unittest.main()
