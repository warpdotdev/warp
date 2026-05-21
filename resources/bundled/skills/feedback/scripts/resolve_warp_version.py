#!/usr/bin/env python3
"""Resolve the bundled Warp version for the feedback skill.

Reads bundled/metadata/version.json relative to this script's location and
prints {"warp_version": "..."} when the version is available, or {} when the
metadata file is missing or unreadable. Always exits 0 so the agent can treat
an empty result as "version unknown" without interpreting a non-zero exit
code.
"""

from __future__ import annotations

import json
from pathlib import Path
import sys


def resolve_version_file() -> Path:
    # This script lives at:
    #   <root>/bundled/skills/feedback/scripts/resolve_warp_version.py
    # Bundled version metadata lives at:
    #   <root>/bundled/metadata/version.json
    script_dir = Path(__file__).resolve().parent
    return script_dir.parent.parent.parent / "metadata" / "version.json"


def read_warp_version(path: Path) -> str | None:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(data, dict):
        return None
    value = data.get("warp_version")
    if isinstance(value, str) and value.strip():
        return value.strip()
    return None


def main() -> int:
    version = read_warp_version(resolve_version_file())
    payload: dict[str, str] = {}
    if version:
        payload["warp_version"] = version
    json.dump(payload, sys.stdout, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
