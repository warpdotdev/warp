#!/usr/bin/env python3
"""Exercise the bundled zsh native-completion generator in a real ZLE context.

Run with python3 script/test_zsh_native_completions.py; requires zsh and a Unix PTY.
"""

import os
import pty
import re
import select
import shlex
import shutil
import signal
import tempfile
import time
import unittest
from pathlib import Path


BOOTSTRAP = Path(__file__).resolve().parent.parent / "app/assets/bundled/bootstrap/zsh_body.sh"
OSC = re.compile(rb"\x1b\]9280;([^\x07]*)\x07")
READY = b"\x1eREADY\x1f"
DONE = b"\x1eDONE:0:0:0\x1f"


class NativeCompletionsTest(unittest.TestCase):
    def setUp(self):
        zsh = shutil.which("zsh")
        if zsh is None:
            self.fail("zsh is required")
        self.home = tempfile.TemporaryDirectory(prefix="warp-zsh-completions-")
        self.addCleanup(self.home.cleanup)
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            environment = os.environ.copy()
            environment.update(
                HOME=self.home.name,
                ZDOTDIR=self.home.name,
                TERM="xterm-256color",
                WARP_BOOTSTRAPPED="",
                WARP_IS_SUBSHELL="1",
                WARP_SESSION_ID="1",
            )
            os.chdir(self.home.name)
            os.execve(zsh, [zsh, "-dfi"], environment)
        self.addCleanup(self.stop_shell)
        self.run_command(
            f"source {shlex.quote(str(BOOTSTRAP))}; "
            "precmd_functions=(); preexec_functions=(); "
            "PROMPT=''; RPROMPT=''; "
            r"printf '\036READY\037'",
            READY,
        )

    def stop_shell(self):
        os.kill(self.pid, signal.SIGKILL)
        os.waitpid(self.pid, 0)
        os.close(self.fd)

    def run_command(self, command, marker=READY):
        os.write(self.fd, (command + "\n").encode())
        output = bytearray()
        deadline = time.monotonic() + 10
        while marker not in output:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                self.fail(f"zsh did not finish the command: {bytes(output[-2000:])!r}")
            if select.select([self.fd], [], [], remaining)[0]:
                output.extend(os.read(self.fd, 65536))
        return bytes(output)

    def configure(self, command):
        return self.run_command(command + r"; printf '\036READY\037'")

    def complete(self, line):
        output = self.run_command(
            f"warp_run_generator_command_native_completions {line.encode().hex()}; "
            r"printf '\036DONE:%s:%s:%s\037' "
            '"${_WARP_NATIVE_COMPLETIONS_ARMED:-0}" '
            '"${_WARP_NATIVE_COMPLETIONS_ZLE_LINE_INIT_RUNNING:-0}" '
            '"${+COMPADD_OVERRIDE}"',
            DONE,
        )
        packets = OSC.findall(output)
        self.assertTrue(packets, f"no native completion response: {output!r}")
        self.assertEqual(packets[0], b"A")
        self.assertEqual(packets[-1], b"B")
        self.assertEqual(packets.count(b"A"), 1)
        self.assertEqual(packets.count(b"B"), 1)
        return packets

    def test_missing_completion_initialization_returns_empty_response(self):
        output = self.configure(
            r"printf '\036INIT:%s:%s\037' "
            '"${+functions[_generic]}" "${+functions[compdef]}"'
        )
        self.assertIn(b"\x1eINIT:0:0\x1f", output)
        for _ in range(2):
            self.assertEqual(self.complete("dart run ./"), [b"A", b"B"])
        output = self.configure(
            r"printf '\036INIT:%s:%s\037' "
            '"${+functions[_generic]}" "${+functions[compdef]}"'
        )
        self.assertIn(b"\x1eINIT:0:0\x1f", output)

    def test_completion_returning_failure_finishes_response(self):
        self.configure("_generic() { return 1 }")
        self.assertEqual(self.complete("flutter run ./"), [b"A", b"B"])

    def test_completion_runtime_error_finishes_captured_response(self):
        self.configure(
            "autoload -Uz compinit; compinit -u -D; "
            "_warptool() { compadd apple avocado; (( 1 / 0 )) }; "
            "compdef _warptool warptool"
        )
        packets = self.complete("warptool a")
        self.assertEqual(
            packets,
            [b"A", b"S;9,1", b"C;6170706c65", b"C;61766f6361646f", b"B"],
        )
        self.configure("_generic() { return 1 }")
        self.assertEqual(self.complete("warptool a"), [b"A", b"B"])

    def test_initialized_nonzero_completion_preserves_candidates_and_spans(self):
        self.configure(
            "autoload -Uz compinit; compinit -u -D; "
            "_warptool() { compadd apple avocado; return 1 }; "
            "compdef _warptool warptool"
        )
        self.assertEqual(
            self.complete("warptool a"),
            [b"A", b"S;9,1", b"C;6170706c65", b"C;61766f6361646f", b"B"],
        )


if __name__ == "__main__":
    unittest.main()
