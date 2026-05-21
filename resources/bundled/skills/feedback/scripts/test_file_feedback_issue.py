#!/usr/bin/env python3
"""Unit tests for file_feedback_issue.py.

Run with:
    python3 resources/bundled/skills/feedback/scripts/test_file_feedback_issue.py
"""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import sys
import unittest
import urllib.parse
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock


SCRIPT_DIR = Path(__file__).resolve().parent
MODULE_PATH = SCRIPT_DIR / "file_feedback_issue.py"
TEST_REPO = "warpdotdev/warp"


def load_module():
    spec = importlib.util.spec_from_file_location("file_feedback_issue", MODULE_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ffi = load_module()


class BuildNewIssueUrlTests(unittest.TestCase):
    def test_url_includes_repo_and_title(self):
        url = ffi.build_new_issue_url(TEST_REPO, "Hello world", None)
        self.assertTrue(url.startswith(f"https://{ffi.DEFAULT_HOSTNAME}/{TEST_REPO}/issues/new?"))
        parsed = urllib.parse.urlparse(url)
        qs = urllib.parse.parse_qs(parsed.query)
        self.assertEqual(qs["title"], ["Hello world"])
        self.assertNotIn("body", qs)

    def test_url_includes_body_when_provided(self):
        url = ffi.build_new_issue_url(TEST_REPO, "T", "body text with spaces & symbols?")
        parsed = urllib.parse.urlparse(url)
        qs = urllib.parse.parse_qs(parsed.query)
        self.assertEqual(qs["title"], ["T"])
        self.assertEqual(qs["body"], ["body text with spaces & symbols?"])

    def test_special_characters_are_percent_encoded(self):
        url = ffi.build_new_issue_url(TEST_REPO, "crash: `foo`", "<script>alert(1)</script>")
        # Spaces should be %20, not +, because we use quote_via=quote.
        self.assertIn("%20", url)
        self.assertNotIn("+", url.split("?", 1)[1])
        # Angle brackets and backticks are percent-encoded.
        self.assertIn("%3C", url)
        self.assertIn("%3E", url)


class FallbackToBrowserTests(unittest.TestCase):
    def _run_fallback(
        self,
        title,
        body,
        open_ok=True,
        browser_available=(True, None),
        gh_path=None,
        gh_create_result=(None, "gh not configured"),
    ):
        with mock.patch.object(
            ffi, "open_in_browser", return_value=open_ok
        ) as open_mock, mock.patch.object(
            ffi, "browser_is_available", return_value=browser_available
        ), mock.patch.object(
            ffi, "gh_path_if_authenticated", return_value=gh_path
        ), mock.patch.object(
            ffi, "create_issue_with_gh", return_value=gh_create_result
        ):
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = ffi.fallback_to_browser(TEST_REPO, title, body)
        payload = json.loads(buf.getvalue().strip())
        return rc, payload, open_mock

    def test_short_body_is_embedded_in_url(self):
        rc, payload, open_mock = self._run_fallback("t", "small body")
        self.assertEqual(rc, 0)
        self.assertEqual(payload["status"], "browser_opened")
        self.assertEqual(payload["method"], "browser")
        self.assertEqual(payload["repo"], TEST_REPO)
        # Short bodies fit in the URL; no `body` field is surfaced separately.
        self.assertNotIn("body", payload)
        open_mock.assert_called_once()
        called_url = open_mock.call_args.args[0]
        parsed = urllib.parse.urlparse(called_url)
        qs = urllib.parse.parse_qs(parsed.query)
        self.assertEqual(qs["title"], ["t"])
        self.assertEqual(qs["body"], ["small body"])

    def test_long_body_surfaces_body_in_payload_and_opens_title_only_url(self):
        huge_body = "A" * (ffi.MAX_PREFILL_URL_LENGTH + 500)
        rc, payload, open_mock = self._run_fallback("t", huge_body)
        self.assertEqual(rc, 0)
        self.assertEqual(payload["status"], "browser_opened")
        # Long bodies are returned in the payload so the caller can instruct
        # the user to paste them; no clipboard copy happens.
        self.assertEqual(payload["body"], huge_body)
        called_url = open_mock.call_args.args[0]
        parsed = urllib.parse.urlparse(called_url)
        qs = urllib.parse.parse_qs(parsed.query)
        self.assertEqual(qs["title"], ["t"])
        self.assertNotIn("body", qs)

    def test_browser_failure_falls_back_to_gh_when_available(self):
        rc, payload, _ = self._run_fallback(
            "t",
            "small body",
            open_ok=False,
            gh_path="/usr/bin/gh",
            gh_create_result=("https://github.com/warpdotdev/warp/issues/7", None),
        )
        self.assertEqual(rc, 0)
        self.assertEqual(payload["status"], "created")
        self.assertEqual(payload["method"], "gh")
        self.assertEqual(payload["repo"], TEST_REPO)
        self.assertTrue(payload["issue_url"].endswith("/issues/7"))
        self.assertTrue(payload.get("browser_unavailable"))
        self.assertIn("attachment", payload["message"].lower())

    def test_browser_failure_reports_failed_when_gh_also_unavailable(self):
        rc, payload, open_mock = self._run_fallback("t", "small body", open_ok=False)
        # Filing failed, so exit code must be non-zero for shell callers that
        # check `$?` to distinguish "issue filed" from "issue not filed".
        self.assertEqual(rc, 1)
        self.assertEqual(payload["status"], "failed")
        self.assertEqual(payload["method"], "browser")
        self.assertEqual(payload["repo"], TEST_REPO)
        self.assertIn("url", payload)
        # Failure messaging must acknowledge attachments so the user understands
        # why the image workflow couldn't complete.
        self.assertIn("attachment", payload["error"].lower())

    def test_browser_unavailable_falls_back_to_gh_when_available(self):
        rc, payload, open_mock = self._run_fallback(
            "t",
            "small body",
            browser_available=(False, "No DISPLAY"),
            gh_path="/usr/bin/gh",
            gh_create_result=("https://github.com/warpdotdev/warp/issues/8", None),
        )
        self.assertEqual(rc, 0)
        self.assertEqual(payload["status"], "created")
        self.assertEqual(payload["method"], "gh")
        self.assertEqual(payload["repo"], TEST_REPO)
        self.assertTrue(payload["issue_url"].endswith("/issues/8"))
        self.assertTrue(payload.get("browser_unavailable"))
        self.assertIn("No DISPLAY", payload["message"])
        self.assertIn("attachment", payload["message"].lower())
        open_mock.assert_not_called()

    def test_browser_unavailable_reports_failed_when_gh_also_unavailable(self):
        rc, payload, open_mock = self._run_fallback(
            "t",
            "small body",
            browser_available=(False, "No DISPLAY"),
        )
        self.assertEqual(rc, 1)
        self.assertEqual(payload["status"], "failed")
        self.assertEqual(payload["method"], "browser")
        self.assertEqual(payload["repo"], TEST_REPO)
        self.assertIn("No DISPLAY", payload["error"])
        self.assertIn("attachment", payload["error"].lower())
        open_mock.assert_not_called()


class BrowserIsAvailableTests(unittest.TestCase):
    def test_darwin_is_always_available(self):
        with mock.patch.object(ffi.platform, "system", return_value="Darwin"), \
                mock.patch.dict(ffi.os.environ, {}, clear=True):
            ok, reason = ffi.browser_is_available()
        self.assertTrue(ok)
        self.assertIsNone(reason)

    def test_windows_is_always_available(self):
        with mock.patch.object(ffi.platform, "system", return_value="Windows"), \
                mock.patch.dict(ffi.os.environ, {}, clear=True):
            ok, reason = ffi.browser_is_available()
        self.assertTrue(ok)
        self.assertIsNone(reason)

    def test_linux_without_display_is_unavailable(self):
        with mock.patch.object(ffi.platform, "system", return_value="Linux"), \
                mock.patch.dict(ffi.os.environ, {}, clear=True):
            ok, reason = ffi.browser_is_available()
        self.assertFalse(ok)
        self.assertIsNotNone(reason)

    def test_linux_with_display_is_available(self):
        with mock.patch.object(ffi.platform, "system", return_value="Linux"), \
                mock.patch.dict(ffi.os.environ, {"DISPLAY": ":0"}, clear=True):
            ok, reason = ffi.browser_is_available()
        self.assertTrue(ok)
        self.assertIsNone(reason)

    def test_linux_with_wayland_display_is_available(self):
        with mock.patch.object(ffi.platform, "system", return_value="Linux"), \
                mock.patch.dict(ffi.os.environ, {"WAYLAND_DISPLAY": "wayland-0"}, clear=True):
            ok, reason = ffi.browser_is_available()
        self.assertTrue(ok)
        self.assertIsNone(reason)


class FileWithGhTests(unittest.TestCase):
    def _run(self, **mocks):
        patches = []
        for attr, value in mocks.items():
            patches.append(mock.patch.object(ffi, attr, return_value=value))
        buf = io.StringIO()
        with contextlib.ExitStack() as stack:
            for p in patches:
                stack.enter_context(p)
            with redirect_stdout(buf):
                rc = ffi.file_with_gh(TEST_REPO, "hello", "body")
        return rc, json.loads(buf.getvalue().strip())

    def test_reports_unavailable_when_gh_missing(self):
        rc, payload = self._run(gh_path_if_authenticated=None)
        self.assertEqual(rc, 0)
        self.assertEqual(payload["status"], "unavailable")
        self.assertEqual(payload["method"], "gh")
        self.assertEqual(payload["repo"], TEST_REPO)
        self.assertIn("message", payload)

    def test_creates_when_gh_available(self):
        rc, payload = self._run(
            gh_path_if_authenticated="/usr/bin/gh",
            create_issue_with_gh=(
                "https://github.com/warpdotdev/warp/issues/42",
                None,
            ),
        )
        self.assertEqual(rc, 0)
        self.assertEqual(payload["status"], "created")
        self.assertEqual(payload["method"], "gh")
        self.assertEqual(payload["repo"], TEST_REPO)
        self.assertTrue(payload["issue_url"].endswith("/issues/42"))

    def test_reports_failed_when_gh_create_fails(self):
        rc, payload = self._run(
            gh_path_if_authenticated="/usr/bin/gh",
            create_issue_with_gh=(None, "gh failed to create issue"),
        )
        # Filing failed, so exit code must be non-zero.
        self.assertEqual(rc, 1)
        self.assertEqual(payload["status"], "failed")
        self.assertEqual(payload["method"], "gh")
        self.assertEqual(payload["repo"], TEST_REPO)
        self.assertEqual(payload["gh_error"], "gh failed to create issue")


class MainTests(unittest.TestCase):
    """Tests for the --use dispatch in main().

    The caller must pick a filing method explicitly; main() does not fall back
    between the two paths.
    """

    def setUp(self):
        self.body_file = SCRIPT_DIR / "_tmp_body.txt"
        self.body_file.write_text("my body", encoding="utf-8")

    def tearDown(self):
        self.body_file.unlink(missing_ok=True)

    def _run_main(self, argv_extras):
        argv = [
            "file_feedback_issue.py",
            "--title",
            "hello",
            "--repo",
            TEST_REPO,
            "--body-file",
            str(self.body_file),
            *argv_extras,
        ]
        buf = io.StringIO()
        with mock.patch.object(sys, "argv", argv):
            with redirect_stdout(buf):
                rc = ffi.main()
        return rc, json.loads(buf.getvalue().strip())

    def test_use_gh_creates_when_gh_available(self):
        with mock.patch.object(
            ffi, "gh_path_if_authenticated", return_value="/usr/bin/gh"
        ), mock.patch.object(
            ffi,
            "create_issue_with_gh",
            return_value=(
                "https://github.com/warpdotdev/warp/issues/999",
                None,
            ),
        ):
            rc, payload = self._run_main(["--use", "gh"])
        self.assertEqual(rc, 0)
        self.assertEqual(payload["status"], "created")
        self.assertEqual(payload["method"], "gh")
        self.assertEqual(payload["repo"], TEST_REPO)
        self.assertTrue(payload["issue_url"].endswith("/issues/999"))

    def test_use_gh_reports_unavailable_and_does_not_open_browser(self):
        with mock.patch.object(
            ffi, "gh_path_if_authenticated", return_value=None
        ), mock.patch.object(ffi, "open_in_browser", return_value=True) as open_mock:
            rc, payload = self._run_main(["--use", "gh"])
        self.assertEqual(rc, 0)
        self.assertEqual(payload["status"], "unavailable")
        self.assertEqual(payload["method"], "gh")
        self.assertEqual(payload["repo"], TEST_REPO)
        # Critical invariant: --use gh must not silently fall back to the browser.
        open_mock.assert_not_called()

    def test_use_browser_does_not_touch_gh(self):
        with mock.patch.object(
            ffi, "gh_path_if_authenticated"
        ) as gh_mock, mock.patch.object(
            ffi, "create_issue_with_gh"
        ) as create_mock, mock.patch.object(
            ffi, "browser_is_available", return_value=(True, None)
        ), mock.patch.object(ffi, "open_in_browser", return_value=True) as open_mock:
            rc, payload = self._run_main(["--use", "browser"])
        self.assertEqual(rc, 0)
        self.assertEqual(payload["status"], "browser_opened")
        self.assertEqual(payload["method"], "browser")
        self.assertEqual(payload["repo"], TEST_REPO)
        open_mock.assert_called_once()
        gh_mock.assert_not_called()
        create_mock.assert_not_called()

    def test_missing_use_flag_exits_with_argparse_error(self):
        stderr = io.StringIO()
        with mock.patch.object(
            sys,
            "argv",
            [
                "file_feedback_issue.py",
                "--title",
                "hello",
                "--repo",
                TEST_REPO,
                "--body-file",
                str(self.body_file),
            ],
        ), mock.patch.object(sys, "stderr", stderr), contextlib.suppress(SystemExit):
            ffi.main()
        self.assertIn("--use", stderr.getvalue())

    def test_missing_repo_flag_exits_with_argparse_error(self):
        stderr = io.StringIO()
        with mock.patch.object(
            sys,
            "argv",
            [
                "file_feedback_issue.py",
                "--use",
                "gh",
                "--title",
                "hello",
                "--body-file",
                str(self.body_file),
            ],
        ), mock.patch.object(sys, "stderr", stderr), contextlib.suppress(SystemExit):
            ffi.main()
        self.assertIn("--repo", stderr.getvalue())

    def test_invalid_use_value_is_rejected(self):
        stderr = io.StringIO()
        with mock.patch.object(
            sys,
            "argv",
            [
                "file_feedback_issue.py",
                "--use",
                "carrier-pigeon",
                "--title",
                "hello",
                "--repo",
                TEST_REPO,
                "--body-file",
                str(self.body_file),
            ],
        ), mock.patch.object(sys, "stderr", stderr), contextlib.suppress(SystemExit):
            ffi.main()
        self.assertIn("invalid choice", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
