"""Native source change detection for the module-only installer."""

from __future__ import annotations

import subprocess
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

    def test_layout_gate_uses_active_templates_not_bundled_identifiers(self) -> None:
        controller = g7_remote.Controller(Path("/test/g7"), "g7", "jiwonpapa-g7mediabooster")
        response = subprocess.CompletedProcess([], 0,
            '{"user_mount":1,"user_handler":1,"admin_mount":1,"admin_handler":1}')
        with patch.object(controller, "artisan", return_value=response) as artisan:
            self.assertEqual(controller.layout_status()["user_mount"], 1)
            script = artisan.call_args.args[1]
            self.assertIn('findActiveByType("user")', script)
            self.assertIn('findActiveByType("admin")', script)
            self.assertIn('getLayout($userTemplate, "board/form", true)', script)
            self.assertNotIn('"sirsoft-basic"', script)

    def test_uploads_off_does_not_report_intentionally_absent_mount_as_an_error(self) -> None:
        controller = g7_remote.Controller(Path("/test/g7"), "g7", "jiwonpapa-g7mediabooster")
        with (patch.object(controller, "module_row", return_value="our-module active"),
              patch.object(controller, "module_active", return_value=True),
              patch.object(controller, "uploads_enabled", return_value=False),
              patch.object(controller, "service_state", return_value="active"),
              patch.object(controller, "api_ready", return_value=True),
              patch.object(controller, "layout_status") as layout):
            result = controller.status(require_layout=True)
            self.assertFalse(result["layout_applied"])
            self.assertNotIn("layout_error", result)
            layout.assert_not_called()

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
