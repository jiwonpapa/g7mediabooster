"""Native source change detection for the module-only installer."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tools.harness.g7mb_harness import g7_remote
from tools.harness.g7mb_harness.g7_install import native_snapshot


class CoreFreeSnapshotTest(unittest.TestCase):
    """Detect core, bundled and installed-source mutations; ignore our module."""

    def test_native_change_and_removal_are_detected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = root / "modules/_bundled/sirsoft-board/module.php"
            original.parent.mkdir(parents=True)
            original.write_text("original")
            before = native_snapshot(root)
            original.write_text("patched")
            self.assertNotEqual(before, native_snapshot(root))
            original.unlink()
            self.assertNotEqual(before, native_snapshot(root))

    def test_bootstrap_config_and_dependency_contracts_are_protected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            before = native_snapshot(root)
            for relative in ("bootstrap/app.php", "config/filesystems.php", "composer.json"):
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("changed")
                self.assertNotEqual(before, native_snapshot(root))


class CoreFreeDisableTest(unittest.TestCase):
    """Upload rollback must never take existing remote delivery offline."""

    def test_disable_changes_only_upload_setting(self) -> None:
        controller = g7_remote.Controller(Path("/test/g7"), "g7", "jiwonpapa-g7mediabooster")
        with (patch.object(controller, "module_active", return_value=True),
              patch.object(controller, "set_uploads_enabled") as settings,
              patch.object(controller, "artisan") as artisan,
              patch.object(controller, "status", return_value={"uploads_enabled": False}),
              patch.object(g7_remote, "run") as system):
            self.assertEqual(controller.disable(), {"uploads_enabled": False})
            settings.assert_called_once_with(False)
            artisan.assert_not_called()
            system.assert_not_called()

    def test_only_our_extension_is_excluded(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            own = root / "modules/_pending/jiwonpapa-g7mediabooster/module.php"
            own.parent.mkdir(parents=True)
            before = native_snapshot(root)
            own.write_text("our module")
            self.assertEqual(before, native_snapshot(root))
            template = root / "templates/_bundled/sirsoft-basic/layout.json"
            template.parent.mkdir(parents=True)
            template.write_text("{}")
            self.assertNotEqual(before, native_snapshot(root))


if __name__ == "__main__":
    unittest.main()
