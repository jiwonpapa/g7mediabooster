"""Component coverage ratchet regression tests."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from tools.harness.g7mb_harness import coverage_ratchet


class CoverageRatchetTest(unittest.TestCase):
    """Keep aggregate coverage from hiding a weak critical component."""

    def write_report(self, contents: str) -> Path:
        """Write one temporary LCOV report for the active test."""

        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / "lcov.info"
        path.write_text(contents, encoding="utf-8")
        return path

    def test_component_percentages_are_calculated_independently(self) -> None:
        report = self.write_report(
            "SF:/repo/apps/api/src/lib.rs\nDA:1,1\nDA:2,0\nend_of_record\n"
            "SF:/repo/crates/store/src/lib.rs\nDA:1,1\nend_of_record\n"
        )
        result = coverage_ratchet.enforce(
            report,
            {"apps/api/": 50.0, "crates/store/": 100.0},
        )
        self.assertEqual(result, {"apps/api/": 50.0, "crates/store/": 100.0})

    def test_low_component_fails_even_when_another_component_is_fully_covered(self) -> None:
        report = self.write_report(
            "SF:/repo/apps/api/src/lib.rs\nDA:1,0\nend_of_record\n"
            "SF:/repo/crates/store/src/lib.rs\nDA:1,1\nDA:2,1\nend_of_record\n"
        )
        with self.assertRaisesRegex(RuntimeError, "apps/api/.*below"):
            coverage_ratchet.enforce(
                report,
                {"apps/api/": 50.0, "crates/store/": 100.0},
            )

    def test_missing_component_fails_closed(self) -> None:
        report = self.write_report("SF:/repo/apps/api/src/lib.rs\nDA:1,1\nend_of_record\n")
        with self.assertRaisesRegex(RuntimeError, "crates/store/.*missing"):
            coverage_ratchet.enforce(
                report,
                {"apps/api/": 50.0, "crates/store/": 50.0},
            )


if __name__ == "__main__":
    unittest.main()
