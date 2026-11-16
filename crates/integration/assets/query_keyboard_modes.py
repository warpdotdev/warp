#!/usr/bin/env python3
"""Tests Kitty keyboard protocol apply-mode semantics (set, union, diff) and query responses.

Sends a sequence of CSI = u mode changes (set flags=1, union flags=8, diff flags=1)
with CSI ? u queries after each, then prints the query responses. Used by
test_keyboard_protocol_query_and_apply_modes to verify that the terminal
correctly tracks flag arithmetic (1 → 9 → 8).
"""
import re
import select
import sys
import termios
import time
import tty


def read_query_response(timeout_seconds: float) -> bytes:
    deadline = time.time() + timeout_seconds
    data = b""
    pattern = re.compile(rb"\x1b\[\?[0-9]+u")

    while time.time() < deadline:
        ready, _, _ = select.select([sys.stdin], [], [], 0.05)
        if not ready:
            continue

        chunk = sys.stdin.buffer.read1(64)
        if not chunk:
            break

        data += chunk
        match = pattern.search(data)
        if match is not None:
            return match.group(0)

    return b""


def send(sequence: str) -> None:
    sys.stdout.write(sequence)
    sys.stdout.flush()


def out(msg: str) -> None:
    sys.stdout.write(msg + "\r\n")
    sys.stdout.flush()


old_settings = termios.tcgetattr(sys.stdin)
try:
    tty.setraw(sys.stdin.fileno())

    out("Protocol test starting")

    # Replace with disambiguate-only (1), then union report-all (8), then remove disambiguate (1).
    send("\x1b[=1u")
    send("\x1b[?u")
    response_1 = read_query_response(timeout_seconds=2.0)
    out(f"query_1={response_1!r}")

    send("\x1b[=8;2u")
    send("\x1b[?u")
    response_2 = read_query_response(timeout_seconds=2.0)
    out(f"query_2={response_2!r}")

    send("\x1b[=1;3u")
    send("\x1b[?u")
    response_3 = read_query_response(timeout_seconds=2.0)
    out(f"query_3={response_3!r}")

    out("All queries done. Press Ctrl+C to exit.")

    # Stay alive so the integration test can read output.
    # Use blocking read (like the other test scripts) for immediate Ctrl+C response.
    while True:
        ch = sys.stdin.read(1)
        if not ch or ord(ch) == 3:  # EOF or Ctrl+C
            break
finally:
    termios.tcsetattr(sys.stdin, termios.TCSAFLUSH, old_settings)
    send("\x1b[=0u")
    print("Done!")
