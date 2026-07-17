"""Infrastructure governance regression tests."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from tools.harness.g7mb_harness import governance


class ShellBudgetTest(unittest.TestCase):
    """Keep new shell files thin and total Bash LOC ratcheted."""

    def test_repository_shell_budget_passes(self) -> None:
        result = governance.shell_budget(governance.repository_root())
        self.assertGreater(result["files"], 0)
        self.assertLessEqual(result["lines"], governance.SHELL_TOTAL_LIMIT)

    def test_new_large_shell_file_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            scripts = root / "scripts"
            scripts.mkdir()
            (scripts / "too-large.sh").write_text("#!/bin/bash\n" + "true\n" * 101)
            with self.assertRaisesRegex(RuntimeError, "too-large"):
                governance.shell_budget(root)


if __name__ == "__main__":
    unittest.main()
