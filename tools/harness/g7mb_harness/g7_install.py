"""Install only G7MediaBooster; attest zero native source changes."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import tempfile
from pathlib import Path

from .g7_install_support import digest, safe_zip_extract
from .g7_remote import Controller
from .process import run
from .verify_g7_contract import repository_root, verify_source

MODULE_ID = "jiwonpapa-g7mediabooster"


def native_snapshot(root: Path) -> dict[str, str]:
    """Hash native source trees, excluding only this independently installed module."""
    result: dict[str, str] = {}
    for directory in ("app", "bootstrap", "config", "routes", "resources/js/core",
                      "resources/views", "modules", "templates"):
        for path in sorted((root / directory).rglob("*")):
            relative = path.relative_to(root)
            if (MODULE_ID in relative.parts
                    or {"vendor", "node_modules"}.intersection(relative.parts)):
                continue
            if relative.parts[:2] == ("bootstrap", "cache"):
                continue
            if path.is_symlink():
                result[relative.as_posix()] = "link:" + str(path.readlink())
            elif path.is_file():
                with path.open("rb") as source:
                    result[relative.as_posix()] = hashlib.file_digest(source, "sha256").hexdigest()
    for name in ("artisan", "composer.json", "composer.lock", "package.json", "package-lock.json"):
        path = root / name
        if path.is_file():
            with path.open("rb") as source:
                result[name] = hashlib.file_digest(source, "sha256").hexdigest()
    return result


class Installer:
    """Transactional module-only installation; no core/module/template patch path."""

    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.repo = repository_root()
        self.app_root = args.app_root
        self.backup_root = Path("/var/backups/g7mediabooster") / args.deployment_id
        self.controller = Controller(self.app_root, args.app_user, MODULE_ID)

    def execute(self) -> None:
        """Validate, install our module, verify native hashes and roll back on failure."""
        if os.geteuid() != 0 or self.backup_root.exists():
            raise RuntimeError("root is required and deployment receipt must be new")
        if (not self.args.module_zip.is_file()
                or digest(self.args.module_zip) != self.args.module_sha256):
            raise RuntimeError("module ZIP checksum mismatch")
        if self.controller.module_row() or verify_source(self.app_root, self.repo):
            raise RuntimeError("module already exists or stock G7 capability is unsupported")
        pending = self.app_root / "modules/_pending" / MODULE_ID
        if pending.exists() or (self.app_root / "modules" / MODULE_ID).exists():
            raise RuntimeError("module destination already exists")
        before = native_snapshot(self.app_root)
        self.backup_root.mkdir(parents=True, mode=0o700)
        (self.backup_root / "native-before.json").write_text(json.dumps(before, sort_keys=True))
        try:
            with tempfile.TemporaryDirectory(prefix="g7mb-module-install.") as temporary:
                stage = Path(temporary)
                safe_zip_extract(self.args.module_zip, stage)
                source = stage / MODULE_ID
                if not (source / "module.json").is_file():
                    raise RuntimeError("invalid module ZIP root")
                pending.parent.mkdir(parents=True, exist_ok=True)
                run(["rsync", "-a", f"--chown={self.args.app_user}:www-data",
                     f"{source}/", f"{pending}/"])
                self.controller.artisan("module:install", MODULE_ID, "--vendor-mode=auto")
                self.controller.artisan("module:activate", MODULE_ID)
                self.controller.refresh_layouts()
                if native_snapshot(self.app_root) != before:
                    raise RuntimeError("CORE_FREE_VIOLATION: native source changed")
                receipt = {"status": "PASS", "module": MODULE_ID, "native_changes": 0,
                           "deployment_id": self.args.deployment_id, "app_root": str(self.app_root)}
                (self.backup_root / "receipt.json").write_text(json.dumps(receipt, sort_keys=True))
                print(json.dumps(receipt, sort_keys=True))
        except Exception:
            self.rollback()
            raise

    def rollback(self) -> None:
        """Remove only this module after failure; never restore/overwrite native files."""
        if self.controller.module_active():
            self.controller.artisan("module:deactivate", MODULE_ID, "--force")
        if self.controller.module_row():
            self.controller.artisan("module:uninstall", MODULE_ID, "--force")
        for relative in (f"modules/_pending/{MODULE_ID}", f"modules/{MODULE_ID}"):
            shutil.rmtree(self.app_root / relative, ignore_errors=True)
        self.controller.artisan("optimize:clear", "--no-ansi")
