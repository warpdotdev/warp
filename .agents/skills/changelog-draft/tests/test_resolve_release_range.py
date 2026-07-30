"""Tests for resolve_release_range.py.

Covers _00 head tags, _01/_02 hotfix head tags, channel separation,
invalid tag format, channel mismatch, missing head tag, and no previous cut.
Also includes a real-tag smoke test against the warp repo.
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

# Resolve the scripts directory relative to this test file
_SCRIPTS_DIR = os.path.abspath(
    os.path.join(os.path.dirname(__file__), "..", "scripts")
)
_SCRIPT = os.path.join(_SCRIPTS_DIR, "resolve_release_range.py")


def _run(args: list[str], repo_dir: str | None = None) -> subprocess.CompletedProcess:
    """Run the resolve_release_range.py script with given args."""
    cmd = [sys.executable, _SCRIPT] + args
    return subprocess.run(cmd, capture_output=True, text=True)


class GitFixture:
    """Context manager that creates a minimal git repo with specified tags."""

    def __init__(self, tags: list[str]) -> None:
        self.tags = tags
        self._tmpdir: tempfile.TemporaryDirectory | None = None
        self.repo_dir: str = ""

    def __enter__(self) -> "GitFixture":
        self._tmpdir = tempfile.TemporaryDirectory()
        self.repo_dir = self._tmpdir.name
        # Init repo
        subprocess.run(
            ["git", "init", "--initial-branch=main"],
            cwd=self.repo_dir,
            capture_output=True,
            check=True,
        )
        subprocess.run(
            ["git", "config", "user.email", "test@example.com"],
            cwd=self.repo_dir,
            capture_output=True,
            check=True,
        )
        subprocess.run(
            ["git", "config", "user.name", "Test"],
            cwd=self.repo_dir,
            capture_output=True,
            check=True,
        )
        # Create a single commit so we have something to tag
        subprocess.run(
            ["git", "commit", "--allow-empty", "-m", "init"],
            cwd=self.repo_dir,
            capture_output=True,
            check=True,
        )
        # Create all requested tags
        for tag in self.tags:
            subprocess.run(
                ["git", "tag", tag],
                cwd=self.repo_dir,
                capture_output=True,
                check=True,
            )
        return self

    def __exit__(self, *args: object) -> None:
        if self._tmpdir:
            self._tmpdir.cleanup()


class TestResolveReleaseRange(unittest.TestCase):
    # ------------------------------------------------------------------
    # Successful cases
    # ------------------------------------------------------------------

    def test_head_is_00_tag(self):
        """For a _00 head tag, the base is the previous _00 cut."""
        tags = [
            "v0.2026.06.03.09.49.stable_00",
            "v0.2026.05.27.09.22.stable_00",
            "v0.2026.05.20.09.21.stable_00",
        ]
        with GitFixture(tags) as repo:
            result = _run(
                [
                    "--channel", "stable",
                    "--release-tag", "v0.2026.06.03.09.49.stable_00",
                    "--repo-dir", repo.repo_dir,
                ]
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data["base"], "v0.2026.05.27.09.22.stable_00")
        self.assertEqual(data["head"], "v0.2026.06.03.09.49.stable_00")
        self.assertEqual(
            data["range"],
            "v0.2026.05.27.09.22.stable_00..v0.2026.06.03.09.49.stable_00",
        )

    def test_head_is_01_hotfix_tag(self):
        """For a _01 hotfix, base must skip the _00 of the same cut."""
        tags = [
            "v0.2026.06.03.09.49.stable_00",
            "v0.2026.06.03.09.49.stable_01",
            "v0.2026.05.27.09.22.stable_00",
        ]
        with GitFixture(tags) as repo:
            result = _run(
                [
                    "--channel", "stable",
                    "--release-tag", "v0.2026.06.03.09.49.stable_01",
                    "--repo-dir", repo.repo_dir,
                ]
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        # Should pick previous cut _00, NOT the same-cut _00
        self.assertEqual(data["base"], "v0.2026.05.27.09.22.stable_00")

    def test_head_is_02_hotfix_tag(self):
        """For a _02 hotfix, base must still skip the _00 of the same cut."""
        tags = [
            "v0.2026.06.03.09.49.stable_00",
            "v0.2026.06.03.09.49.stable_01",
            "v0.2026.06.03.09.49.stable_02",
            "v0.2026.05.27.09.22.stable_00",
            "v0.2026.05.20.09.21.stable_00",
        ]
        with GitFixture(tags) as repo:
            result = _run(
                [
                    "--channel", "stable",
                    "--release-tag", "v0.2026.06.03.09.49.stable_02",
                    "--repo-dir", repo.repo_dir,
                ]
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data["base"], "v0.2026.05.27.09.22.stable_00")

    def test_preview_channel_separation(self):
        """Preview _00 tags are separate from stable tags."""
        tags = [
            "v0.2026.06.03.09.49.preview_00",
            "v0.2026.05.27.09.22.preview_00",
            "v0.2026.06.03.09.49.stable_00",  # stable tags should not interfere
        ]
        with GitFixture(tags) as repo:
            result = _run(
                [
                    "--channel", "preview",
                    "--release-tag", "v0.2026.06.03.09.49.preview_00",
                    "--repo-dir", repo.repo_dir,
                ]
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data["base"], "v0.2026.05.27.09.22.preview_00")

    def test_dev_channel(self):
        """Dev channel works the same as stable/preview."""
        tags = [
            "v0.2026.06.03.09.49.dev_00",
            "v0.2026.05.27.09.22.dev_00",
        ]
        with GitFixture(tags) as repo:
            result = _run(
                [
                    "--channel", "dev",
                    "--release-tag", "v0.2026.06.03.09.49.dev_00",
                    "--repo-dir", repo.repo_dir,
                ]
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data["base"], "v0.2026.05.27.09.22.dev_00")

    def test_only_one_previous_cut(self):
        """Works when there is exactly one previous cut."""
        tags = [
            "v0.2026.06.03.09.49.stable_00",
            "v0.2026.05.27.09.22.stable_00",
        ]
        with GitFixture(tags) as repo:
            result = _run(
                [
                    "--channel", "stable",
                    "--release-tag", "v0.2026.06.03.09.49.stable_00",
                    "--repo-dir", repo.repo_dir,
                ]
            )
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data["base"], "v0.2026.05.27.09.22.stable_00")

    # ------------------------------------------------------------------
    # Error cases
    # ------------------------------------------------------------------

    def test_invalid_tag_format(self):
        """Invalid tag format exits non-zero."""
        tags = ["v0.2026.05.27.09.22.stable_00"]
        with GitFixture(tags) as repo:
            result = _run(
                [
                    "--channel", "stable",
                    "--release-tag", "not-a-valid-tag",
                    "--repo-dir", repo.repo_dir,
                ]
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("invalid tag format", result.stderr.lower())

    def test_channel_mismatch(self):
        """Tag channel not matching --channel exits non-zero."""
        tags = [
            "v0.2026.06.03.09.49.stable_00",
            "v0.2026.05.27.09.22.stable_00",
        ]
        with GitFixture(tags) as repo:
            result = _run(
                [
                    "--channel", "preview",
                    "--release-tag", "v0.2026.06.03.09.49.stable_00",
                    "--repo-dir", repo.repo_dir,
                ]
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("channel mismatch", result.stderr.lower())

    def test_missing_head_tag(self):
        """Head tag not in repo exits non-zero."""
        tags = ["v0.2026.05.27.09.22.stable_00"]
        with GitFixture(tags) as repo:
            result = _run(
                [
                    "--channel", "stable",
                    "--release-tag", "v0.2026.06.03.09.49.stable_00",
                    "--repo-dir", repo.repo_dir,
                ]
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not exist", result.stderr.lower())

    def test_no_previous_cut(self):
        """No prior _00 cut exits non-zero."""
        tags = ["v0.2026.06.03.09.49.stable_00"]
        with GitFixture(tags) as repo:
            result = _run(
                [
                    "--channel", "stable",
                    "--release-tag", "v0.2026.06.03.09.49.stable_00",
                    "--repo-dir", repo.repo_dir,
                ]
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no previous", result.stderr.lower())

    def test_no_previous_cut_when_only_later_cuts_exist(self):
        """Only cuts from after the release tag exist → no previous cut error."""
        tags = [
            "v0.2026.06.03.09.49.stable_00",  # later
            "v0.2026.05.27.09.22.stable_00",  # same date as head → skip
            "v0.2026.05.27.09.22.stable_01",  # hotfix, not a _00 cut point
        ]
        # _00 list only has the later tag and the same-prefix tag
        with GitFixture(["v0.2026.06.03.09.49.stable_00",
                         "v0.2026.05.27.09.22.stable_00",
                         "v0.2026.05.27.09.22.stable_01"]) as repo:
            # Run for the _00 of 2026.05.27 — there's nothing before it
            result = _run(
                [
                    "--channel", "stable",
                    "--release-tag", "v0.2026.05.27.09.22.stable_00",
                    "--repo-dir", repo.repo_dir,
                ]
            )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("no previous", result.stderr.lower())

    # ------------------------------------------------------------------
    # Real-tag smoke test
    # ------------------------------------------------------------------

    def test_smoke_real_tags(self):
        """Smoke test against the actual warp repo tags.

        Spec validation criterion #2: for v0.2026.06.03.09.49.stable_00,
        the expected base is v0.2026.05.27.09.22.stable_00.
        """
        # Find the repo root (four directories up from this test file)
        repo_dir = os.path.abspath(
            os.path.join(os.path.dirname(__file__), "..", "..", "..", "..")
        )
        # Skip if the expected tags are not present (e.g. shallow clone)
        check = subprocess.run(
            ["git", "tag", "--list", "v0.2026.06.03.09.49.stable_00"],
            cwd=repo_dir,
            capture_output=True,
            text=True,
        )
        if "v0.2026.06.03.09.49.stable_00" not in check.stdout:
            self.skipTest("Real release tags not available in this checkout")

        result = _run(
            [
                "--channel", "stable",
                "--release-tag", "v0.2026.06.03.09.49.stable_00",
                "--repo-dir", repo_dir,
            ]
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data["base"], "v0.2026.05.27.09.22.stable_00")
        self.assertEqual(data["head"], "v0.2026.06.03.09.49.stable_00")
        self.assertEqual(
            data["range"],
            "v0.2026.05.27.09.22.stable_00..v0.2026.06.03.09.49.stable_00",
        )


if __name__ == "__main__":
    unittest.main()
