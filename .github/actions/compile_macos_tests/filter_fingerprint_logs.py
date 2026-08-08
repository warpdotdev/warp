import argparse
import re
import sys
from collections import Counter
from typing import TextIO


ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;]*[mK]")
CARGO_STATUS = re.compile(
    r"^\s*(?:"
    r"Compiling|Checking|Finished|Fresh|Dirty|Building|Blocking|"
    r"Downloading|Downloaded|Updating|Locking|Adding|Removing|"
    r"Packaging|Verifying|Archiving|Installing|Installed|Running|"
    r"Doc-tests|Executable"
    r")\b"
)
FINGERPRINT_LOGGER = "cargo::core::compiler::fingerprint:"
FINGERPRINT_REASON = re.compile(
    rf"{re.escape(FINGERPRINT_LOGGER)}\s+err:\s+(?P<reason>.+)$"
)
BACKTICK_DETAIL = re.compile(r"`[^`]+`")
ABSOLUTE_PATH = re.compile(r"(?<!\w)/(?:[^/\s]+/)+[^/\s]+")
HEX_DETAIL = re.compile(r"\b[0-9a-f]{16,}\b")


def write_live(line: str, output: TextIO) -> None:
    output.write(line)
    output.flush()


def filter_live_logs(input_stream: TextIO, output: TextIO) -> None:
    suppress_continuation = False
    in_cause = False
    for line in input_stream:
        plain = ANSI_ESCAPE.sub("", line).rstrip("\n")

        if FINGERPRINT_LOGGER in plain:
            suppress_continuation = True
            in_cause = False
            continue

        if not suppress_continuation:
            write_live(line, output)
            continue

        if CARGO_STATUS.match(plain) or plain.startswith(("error", "warning")):
            suppress_continuation = False
            in_cause = False
            write_live(line, output)
        elif not plain:
            continue
        elif plain == "Caused by:":
            in_cause = True
        elif in_cause and line[:1].isspace():
            continue
        else:
            suppress_continuation = False
            in_cause = False
            write_live(line, output)


def normalize_reason(reason: str) -> str:
    if reason.startswith("failed to read `") and "/.fingerprint/" in reason:
        return "failed to read Cargo fingerprint"

    reason = BACKTICK_DETAIL.sub("`<detail>`", reason)
    reason = ABSOLUTE_PATH.sub("<path>", reason)
    return HEX_DETAIL.sub("<hash>", reason)


def summarize_reasons(input_stream: TextIO, output: TextIO) -> None:
    counts = Counter(
        normalize_reason(match.group("reason"))
        for line in input_stream
        if (match := FINGERPRINT_REASON.search(line))
    )
    for reason, count in sorted(counts.items(), key=lambda item: (-item[1], item[0])):
        output.write(f"{count:4} - {reason}\n")

def count_signals(input_stream: TextIO, output: TextIO) -> None:
    lines = list(input_stream)
    missing = sum("fingerprint error for " in line for line in lines)
    dirty = sum("fingerprint dirty for " in line for line in lines)
    stale = sum(f"{FINGERPRINT_LOGGER} stale:" in line for line in lines)
    output.write(f"{missing} {dirty} {stale}\n")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--summarize-reasons", action="store_true")
    parser.add_argument("--count-signals", action="store_true")
    args = parser.parse_args()

    if args.summarize_reasons:
        summarize_reasons(sys.stdin, sys.stdout)
    elif args.count_signals:
        count_signals(sys.stdin, sys.stdout)
    else:
        filter_live_logs(sys.stdin, sys.stdout)


if __name__ == "__main__":
    main()
