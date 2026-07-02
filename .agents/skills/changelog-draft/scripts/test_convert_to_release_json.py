#!/usr/bin/env python3
"""Tests for changelog release JSON conversion."""

import unittest
from unittest import mock

import convert_to_release_json as converter


class ConvertToReleaseJsonTest(unittest.TestCase):
    def test_external_contributor_is_attributed(self) -> None:
        entry = {
            "category": "IMPROVEMENT",
            "text": "Added shell completions",
            "pr_number": 11764,
            "url": "https://github.com/warpdotdev/warp/pull/11764",
            "author": "external-user",
            "author_association": "CONTRIBUTOR",
            "author_is_bot": False,
            "is_external": True,
        }

        self.assertEqual(
            converter.format_entry(entry),
            "Added shell completions ([#11764](https://github.com/warpdotdev/warp/pull/11764)) — [@external-user](https://github.com/external-user) ✨",
        )

    def test_internal_member_keeps_pr_link_without_author_attribution(self) -> None:
        entry = {
            "category": "BUG-FIX",
            "text": "Fixed queued prompt hover state",
            "pr_number": 12067,
            "url": "https://github.com/warpdotdev/warp/pull/12067",
            "author": "harryalbert",
            "author_association": "MEMBER",
            "author_is_bot": False,
            "is_external": True,
        }

        self.assertEqual(
            converter.format_entry(entry),
            "Fixed queued prompt hover state ([#12067](https://github.com/warpdotdev/warp/pull/12067))",
        )

    def test_app_author_keeps_pr_link_without_author_attribution(self) -> None:
        entry = {
            "category": "BUG-FIX",
            "text": "AI responses that contain Markdown tables now render as structured tables",
            "pr_number": 10683,
            "url": "https://github.com/warpdotdev/warp/pull/10683",
            "author": "app/oz-for-oss",
            "author_association": "CONTRIBUTOR",
            "author_is_bot": True,
            "is_external": True,
        }

        self.assertEqual(
            converter.format_entry(entry),
            "AI responses that contain Markdown tables now render as structured tables ([#10683](https://github.com/warpdotdev/warp/pull/10683))",
        )

    def test_convert_can_resolve_missing_internal_author_metadata(self) -> None:
        draft = {
            "entries": [
                {
                    "category": "NEW-FEATURE",
                    "text": "Queue multiple follow-up prompts",
                    "pr_number": 12081,
                    "url": "https://github.com/warpdotdev/warp/pull/12081",
                    "author": "harryalbert",
                    "is_external": True,
                }
            ]
        }

        with mock.patch.object(
            converter,
            "fetch_author_metadata",
            return_value={
                **draft["entries"][0],
                "author_association": "MEMBER",
                "author_is_bot": False,
            },
        ):
            release = converter.convert(draft, resolve_author_metadata=True)

        self.assertEqual(
            release["newFeatures"],
            [
                "Queue multiple follow-up prompts ([#12081](https://github.com/warpdotdev/warp/pull/12081))"
            ],
        )


if __name__ == "__main__":
    unittest.main()
