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


class PythonBudgetTest(unittest.TestCase):
    """Keep the Python orchestration layer bounded as it replaces Bash."""

    def test_repository_python_budget_passes(self) -> None:
        result = governance.python_budget(governance.repository_root())
        self.assertGreater(result["files"], 0)
        self.assertLessEqual(result["lines"], governance.PYTHON_TOTAL_LIMIT)


class SourceBudgetTest(unittest.TestCase):
    """Prevent unreviewed growth of existing and new source monoliths."""

    def test_repository_source_budget_passes(self) -> None:
        result = governance.source_budget(governance.repository_root())
        self.assertGreater(result["files"], 0)

    def test_new_large_source_file_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sources = root / "crates" / "new-crate" / "src"
            sources.mkdir(parents=True)
            (sources / "lib.rs").write_text("//! oversized\n" + "fn item() {}\n" * 500)
            with self.assertRaisesRegex(RuntimeError, "new-crate"):
                governance.source_budget(root)


class RustHarnessDependencyBudgetTest(unittest.TestCase):
    """Keep repository routing commands outside the product build graph."""

    def test_repository_xtask_has_no_product_dependency(self) -> None:
        result = governance.rust_harness_dependency_budget(governance.repository_root())
        self.assertEqual(result["product_dependencies"], 0)

    def test_product_dependency_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            xtask = root / "xtask"
            xtask.mkdir()
            (xtask / "Cargo.toml").write_text(
                '[package]\nname = "xtask"\nversion = "0.1.0"\n'
                '[dependencies]\ng7mb-api = { path = "../apps/g7mb-api" }\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(RuntimeError, "g7mb-api"):
                governance.rust_harness_dependency_budget(root)


if __name__ == "__main__":
    unittest.main()
