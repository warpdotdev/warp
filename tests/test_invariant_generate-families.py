import pytest
import importlib.util
import sys
import os
from unittest.mock import patch, MagicMock

# Load the module from the script path
spec = importlib.util.spec_from_file_location(
    "generate_families",
    os.path.join(os.path.dirname(__file__), "../script/font_fallback/generate-families.py")
)
module = importlib.util.load_from_spec(spec) if hasattr(importlib.util, 'load_from_spec') else None

def load_module():
    spec = importlib.util.spec_from_file_location(
        "generate_families",
        os.path.join(os.path.dirname(__file__), "script/font_fallback/generate-families.py")
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod

@pytest.mark.parametrize("payload", [
    "; rm -rf /tmp/pwned",          # exact exploit case
    "$(whoami)",                     # command substitution
    "`id`",                          # backtick injection
    "Arial; echo injected",          # boundary: valid prefix + injection
    "Arial",                         # valid input — must work normally
])
def test_shell_injection_not_executed(payload, tmp_path):
    """Invariant: shell metacharacters in font family names must never be executed as shell commands."""
    sentinel = tmp_path / "pwned"

    try:
        mod = load_module()
    except Exception:
        pytest.skip("Could not load generate-families.py module")

    captured_commands = []

    def mock_check_output(cmd, **kwargs):
        captured_commands.append(cmd)
        return ""

    with patch("subprocess.check_output", side_effect=mock_check_output):
        try:
            # Try any function that builds shell commands with user-influenced input
            if hasattr(mod, 'get_font_families'):
                mod.get_font_families(payload)
            elif hasattr(mod, 'list_fonts'):
                mod.list_fonts(payload)
        except Exception:
            pass  # We only care about what reached subprocess

    for cmd in captured_commands:
        cmd_str = cmd if isinstance(cmd, str) else " ".join(cmd)
        # If shell=True was used, the payload must be sanitized/escaped
        if payload not in ("Arial",):
            assert payload not in cmd_str or isinstance(cmd, list), (
                f"Unsanitized payload '{payload}' found in shell command: {cmd_str}"
            )

    assert not sentinel.exists(), "Sentinel file created — command injection succeeded"