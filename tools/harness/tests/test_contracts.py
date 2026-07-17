"""Pure validation tests for Python-owned harness contracts."""

from __future__ import annotations

import argparse
import os
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

from tools.harness.g7mb_harness import full_stack, g7_live, package_zipapp
from tools.harness.g7mb_harness.g7_install import Installer
from tools.harness.g7mb_harness.g7_install_support import safe_zip_extract
from tools.harness.g7mb_harness.g7_remote import validate_relative_path


class FullStackOptionTest(unittest.TestCase):
    """Bound expensive full-stack options before starting dependencies."""

    def test_strict_environment_boolean(self) -> None:
        with patch.dict(os.environ, {"G7MB_TEST_BOOL": "true"}):
            self.assertTrue(full_stack.environment_bool("G7MB_TEST_BOOL"))
        with (
            patch.dict(os.environ, {"G7MB_TEST_BOOL": "yes"}),
            self.assertRaisesRegex(RuntimeError, "true or false"),
        ):
            full_stack.environment_bool("G7MB_TEST_BOOL")

    def test_large_bytes_are_bounded(self) -> None:
        with patch.dict(os.environ, {"G7MB_TEST_BYTES": str(5 * 1024 * 1024)}):
            self.assertEqual(full_stack.environment_bytes("G7MB_TEST_BYTES"), 5 * 1024 * 1024)
        with (
            patch.dict(os.environ, {"G7MB_TEST_BYTES": str(5 * 1024 * 1024 * 1024 + 1)}),
            self.assertRaisesRegex(RuntimeError, "between 5MiB and 5GiB"),
        ):
            full_stack.environment_bytes("G7MB_TEST_BYTES")


class G7SafetyTest(unittest.TestCase):
    """Fail closed before SSH or privileged filesystem operations."""

    def test_live_mutation_requires_exact_host_confirmation(self) -> None:
        args = argparse.Namespace(
            host="g7devops",
            app_user="g7devops",
            app_root="/home/g7devops/public_html",
            action="apply",
            confirm=None,
            deployment_id=None,
        )
        with self.assertRaisesRegex(RuntimeError, "--confirm g7devops"):
            g7_live.validate(args)

    def test_relative_path_rejects_traversal(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "unsafe"):
            validate_relative_path("../etc/passwd")

    def test_zip_extraction_rejects_traversal(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "unsafe.zip"
            with zipfile.ZipFile(archive, "w") as destination:
                destination.writestr("../escape", "bad")
            with self.assertRaisesRegex(RuntimeError, "unsafe"):
                safe_zip_extract(archive, root / "out")

    def test_installer_rejects_diverged_active_host_file(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            app = root / "app"
            shadow = root / "shadow"
            patch_root = root / "patches"
            relative = "modules/_bundled/sirsoft-board/example.php"
            active = "modules/sirsoft-board/example.php"
            for base, path, content in (
                (app, relative, "old bundled"),
                (app, active, "different active"),
                (shadow, relative, "patched"),
            ):
                target = base / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(content)
            patch_root.mkdir()
            (patch_root / "0001.patch").write_text(f"+++ b/{relative}\n")
            installer = Installer.__new__(Installer)
            installer.app_root = app
            installer.patch_root = patch_root
            with self.assertRaisesRegex(RuntimeError, "active and bundled"):
                installer._changed_targets(shadow)


class PackageTest(unittest.TestCase):
    """Keep the installed Python harness reproducible and minimal."""

    def test_zipapp_is_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            first = root / "first.pyz"
            second = root / "second.pyz"
            package_zipapp.build(first)
            package_zipapp.build(second)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with zipfile.ZipFile(first) as archive:
                names = archive.namelist()
            self.assertIn("__main__.py", names)
            self.assertIn("g7mb_harness/verify_g7_contract.py", names)
            self.assertFalse(any("__pycache__" in name for name in names))


if __name__ == "__main__":
    unittest.main()
