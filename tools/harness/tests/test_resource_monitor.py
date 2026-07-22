"""Regression tests for bounded native subprocess execution."""

from __future__ import annotations

import os
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness.g7mb_harness.resource_monitor import (
    ResourceLimitError,
    directory_size_kib,
    positive_int_env,
    process_tree_rss,
    run_monitored,
)


class ResourceMonitorTest(unittest.TestCase):
    """Keep parsing, limits, logging, and cleanup deterministic."""

    def test_process_tree_rss_includes_recursive_descendants(self) -> None:
        rows = [(10, 1, 100), (11, 10, 50), (12, 11, 25), (20, 1, 999)]
        self.assertEqual(process_tree_rss(rows, 10), 175)

    def test_positive_integer_environment_is_fail_closed(self) -> None:
        with (
            patch.dict(os.environ, {"G7MB_TEST_LIMIT": "0"}),
            self.assertRaisesRegex(ResourceLimitError, "positive integer"),
        ):
            positive_int_env("G7MB_TEST_LIMIT", 1)

    def test_directory_size_rounds_up_to_kibibytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary)
            (path / "value.bin").write_bytes(b"x" * 1025)
            self.assertEqual(directory_size_kib(path), 2)

    def test_monitored_process_writes_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            usage = run_monitored(
                [sys.executable, "-c", "print('bounded')"],
                cwd=root,
                log_path=root / "process.log",
                timeout_seconds=2,
                max_rss_kib=1_000_000,
                sample_seconds=0.01,
                rss_reader=lambda _pid: 10,
            )
            self.assertIn("bounded", (root / "process.log").read_text())
            self.assertGreaterEqual(usage.elapsed_ms, 0)

    def test_monitored_process_times_out(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            ready = root / "descendant-ready"
            marker = root / "descendant-survived"
            child = (
                "import pathlib,signal,time;signal.signal(signal.SIGTERM,signal.SIG_IGN);"
                f"pathlib.Path({str(ready)!r}).write_text('ready');time.sleep(.3);"
                f"pathlib.Path({str(marker)!r}).write_text('unsafe')"
            )
            parent = (
                "import subprocess,sys,time;"
                "subprocess.Popen([sys.executable,'-c',sys.argv[1]]);time.sleep(2)"
            )
            with self.assertRaisesRegex(ResourceLimitError, "exceeded"):
                run_monitored(
                    [sys.executable, "-c", parent, child],
                    cwd=root,
                    log_path=root / "process.log",
                    timeout_seconds=0.15,
                    max_rss_kib=1_000_000,
                    sample_seconds=0.01,
                    rss_reader=lambda _pid: 10,
                )
            self.assertTrue(ready.exists())
            time.sleep(0.35)
            self.assertFalse(marker.exists())


if __name__ == "__main__":
    unittest.main()
