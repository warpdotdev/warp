import io
import unittest

from filter_fingerprint_logs import count_signals, filter_live_logs, summarize_reasons


class FilterLiveLogsTest(unittest.TestCase):
    def test_suppresses_fingerprint_details_but_keeps_cargo_status(self) -> None:
        cargo_log = io.StringIO(
            """
0.1s INFO cargo::core::compiler::fingerprint: err: failed to read `/target/fingerprint`

Caused by:
    No such file or directory
   Compiling warp_core v0.1.0
warning: retained warning
"""
        )
        output = io.StringIO()

        filter_live_logs(cargo_log, output)

        self.assertEqual(
            output.getvalue(),
            "\n   Compiling warp_core v0.1.0\nwarning: retained warning\n",
        )


class SummarizeReasonsTest(unittest.TestCase):
    def test_aggregates_dynamic_fingerprint_paths(self) -> None:
        fingerprint_log = io.StringIO(
            """
0.1s INFO cargo::core::compiler::fingerprint: err: failed to read `/target/debug/.fingerprint/alpha/lib-alpha`
0.2s INFO cargo::core::compiler::fingerprint:     err: failed to read `/different/target/debug/.fingerprint/beta/lib-beta`
"""
        )
        output = io.StringIO()

        summarize_reasons(fingerprint_log, output)

        self.assertEqual(output.getvalue(), "   2 - failed to read Cargo fingerprint\n")

class CountSignalsTest(unittest.TestCase):
    def test_distinguishes_missing_dirty_and_stale_fingerprints(self) -> None:
        fingerprint_log = io.StringIO(
            """
INFO cargo::core::compiler::fingerprint: fingerprint error for warp-core
INFO cargo::core::compiler::fingerprint: fingerprint dirty for warp
INFO cargo::core::compiler::fingerprint: fingerprint dirty for warp-ui
INFO cargo::core::compiler::fingerprint: stale: changed dependency
"""
        )
        output = io.StringIO()

        count_signals(fingerprint_log, output)

        self.assertEqual(output.getvalue(), "1 2 1\n")


if __name__ == "__main__":
    unittest.main()
