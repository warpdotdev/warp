import pytest
import subprocess
import sys
import os
import tempfile


@pytest.mark.parametrize("malicious_path", [
    "/tmp/font; rm -rf /tmp/pwned; echo ",
    "/tmp/font$(touch /tmp/pwned)",
    "/tmp/font`touch /tmp/pwned`",
    "/tmp/normal_font.ttf",
])
def test_no_shell_injection_in_subprocess_calls(malicious_path):
    """Invariant: subprocess calls must not use shell=True, preventing shell metacharacter injection."""
    script_path = os.path.join(
        os.path.dirname(__file__), "..", "script", "font_fallback", "generate-mappings.py"
    )
    script_path = os.path.normpath(script_path)

    with open(script_path, "r") as f:
        source = f.read()

    # Security invariant: subprocess calls should never use shell=True
    # This prevents shell injection via crafted file paths or config values
    assert "shell=True" not in source, (
        "generate-mappings.py uses shell=True in subprocess calls, "
        "which allows shell metacharacter injection via adversarial inputs like: "
        f"{malicious_path!r}"
    )