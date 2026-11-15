"""Exercise the real zsh widget and event stream in an isolated PTY."""

import os
from pathlib import Path
import pty
import select
import shutil
import signal
import tempfile
import time
import unittest
import uuid


class ShellIntegrationTest(unittest.TestCase):
    def test_command_and_interactive_input(self):
        script = Path(__file__).parents[1] / "App/Shell/integration.zsh"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            startup = root / "startup"
            startup.mkdir()
            shutil.copyfile(script, startup / ".zshrc")
            (root / ".zshrc").write_text("export WARP_TEST_RC=loaded\n")
            (root / "option alpha").touch()
            (root / "option beta").touch()
            (root / "café").touch()
            pid, fd = pty.fork()
            if pid == 0:
                os.chdir(directory)
                os.execve(
                    "/bin/zsh",
                    ["zsh", "-i"],
                    {
                        "HOME": directory,
                        "PATH": "/usr/bin:/bin",
                        "TERM": "xterm-256color",
                        "LANG": "en_US.UTF-8",
                        "ZDOTDIR": str(startup),
                        "WARP_SWIFT_TOKEN": "test",
                        "WARP_SWIFT_SESSION": str(startup),
                    },
                )

            buffered = b""

            def read_until(marker):
                nonlocal buffered
                deadline = time.monotonic() + 5
                while marker not in buffered:
                    timeout = max(0, deadline - time.monotonic())
                    if not select.select([fd], [], [], timeout)[0]:
                        self.fail(f"Missing {marker!r} in {buffered!r}")
                    buffered += os.read(fd, 8192)
                before, buffered = buffered.split(marker, 1)
                return before

            def submit(command):
                (startup / "command").write_text(command)
                os.write(fd, b"\x1b[98~")
                return read_until(b"\x1b]2;warp-swift:test:busy:")

            def complete(draft, cursor):
                request = str(uuid.uuid4()).upper()
                (startup / "completion-request").write_text(f"{request}\n{cursor}\n{draft}")
                os.write(fd, b"\x1b[99~")
                output = read_until(f"\x1b]2;warp-swift:test:completed:{request}\x07".encode())
                self.assertNotIn(b"warp-swift:test:busy:", output)
                result = startup / f"completion-{request}"
                fields = result.read_bytes().split(b"\0")
                self.assertEqual(fields.pop(), b"")
                return [(int(cursor), text.decode()) for cursor, text in zip(fields[::2], fields[1::2])]

            try:
                ready = b"\x1b]2;warp-swift:test:ready\x07"
                read_until(ready)
                self.assertEqual(set(complete("ls option", 9)), {
                    (16, "ls option\\ alpha"), (15, "ls option\\ beta"),
                })
                self.assertEqual(complete("git --ver", 9), [(14, "git --version ")])
                self.assertEqual(complete("ls caf && false", 6), [(8, "ls café  && false")])
                self.assertEqual(complete("ls no-match-77550", 17), [])
                start = submit("printf '\\033[31m%s\\033[0m\\n' \"$WARP_TEST_RC — Unicode\"\nfalse")
                self.assertIn(f"WStest-{pid}-S".encode(), start)
                output = read_until(f"\x1b]2;warp-swift:test:done:{pid}:1:".encode())
                self.assertIn(b"\x1b[31mloaded", output)
                self.assertIn(f"WStest-{pid}-E".encode(), output)
                read_until(ready)
                submit("warpify-zsh")
                read_until(ready)
                submit("exit 7")
                read_until(f"\x1b]2;warp-swift:test:done:{pid}:7:".encode())
                read_until(ready)
                submit("/bin/cat")
                os.write(fd, b"direct input\n")
                read_until(b"direct input")
                read_until(b"direct input")
                os.write(fd, b"\x03")
                read_until(f"\x1b]2;warp-swift:test:done:{pid}:130:".encode())
                read_until(ready)
            finally:
                os.close(fd)
                try:
                    os.kill(pid, signal.SIGHUP)
                except ProcessLookupError:
                    pass
                os.waitpid(pid, 0)


if __name__ == "__main__":
    unittest.main()
