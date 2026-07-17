"""Transactional G7 module installer with runtime layout attestation."""

from __future__ import annotations

import argparse
import grp
import json
import os
import pwd
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

from .g7_contract_support import patched_paths
from .g7_install_support import digest, safe_zip_extract
from .g7_remote import Controller
from .process import run
from .verify_g7_contract import repository_root, verify_source

MODULE_ID = "jiwonpapa-g7mediabooster"


class Installer:
    """Apply G7 contracts, install the module, attest runtime, and roll back on failure."""

    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.repo = repository_root()
        self.patch_root = self.repo / "adapters/gnuboard7/upstream-contract"
        self.backup_root = Path("/var/backups/g7mediabooster") / args.deployment_id
        self.app_root = args.app_root
        self.controller = Controller(self.app_root, args.app_user, MODULE_ID)
        self.created: list[str] = []
        self.files_applied = False
        self.complete = False

    def artisan(self, *args: str, capture: bool = False) -> subprocess.CompletedProcess[str]:
        """Run Artisan through the shared remote controller."""

        return self.controller.artisan(*args, capture=capture)

    def execute(self) -> None:
        """Run the complete transactional installation."""

        self._preflight()
        with tempfile.TemporaryDirectory(prefix="g7mb-live-install.") as temporary:
            work = Path(temporary)
            shadow = self._patched_shadow(work)
            changed, targets = self._changed_targets(shadow)
            self._backup(targets, changed)
            try:
                self.files_applied = True
                self._install_patches(shadow, changed)
                self._refresh_upstream_layouts()
                failures = verify_source(self.app_root, self.repo)
                if failures:
                    raise RuntimeError(f"live G7 source contract failed: {failures}")
                self._install_module(work)
                self.controller.refresh_layouts()
                run(["systemctl", "reload", "php8.5-fpm.service"])
                layout = self.controller.layout_status()
                self.complete = True
                print(
                    json.dumps(
                        {
                            "status": "PASS",
                            "deployment_id": self.args.deployment_id,
                            "backup": str(self.backup_root),
                            "module": MODULE_ID,
                            "layout": layout,
                        },
                        sort_keys=True,
                    )
                )
            except Exception:
                if self.files_applied and not self.complete:
                    self.rollback()
                raise

    def _preflight(self) -> None:
        if os.geteuid() != 0:
            raise RuntimeError("remote G7 installer requires root")
        if self.backup_root.exists():
            raise RuntimeError(f"backup receipt already exists: {self.backup_root}")
        if not self.controller.artisan_path.is_file() or not self.patch_root.is_dir():
            raise RuntimeError("G7 root or contract patch directory is missing")
        if not self.args.module_zip.is_file():
            raise RuntimeError("module ZIP is missing")
        if digest(self.args.module_zip) != self.args.module_sha256:
            raise RuntimeError("module ZIP checksum mismatch")
        if self.controller.module_row():
            raise RuntimeError(f"module is already installed: {MODULE_ID}")
        pending = self.app_root / "modules/_pending" / MODULE_ID
        if pending.exists() or (self.app_root / "modules" / MODULE_ID).exists():
            raise RuntimeError("module filesystem path already exists")

    def _patched_shadow(self, work: Path) -> Path:
        shadow = work / "host"
        for relative in ("app", "tests"):
            shutil.copytree(self.app_root / relative, shadow / relative)
        shutil.copytree(
            self.app_root / "modules/_bundled/sirsoft-board",
            shadow / "modules/_bundled/sirsoft-board",
        )
        shutil.copytree(
            self.app_root / "templates/_bundled/sirsoft-basic",
            shadow / "templates/_bundled/sirsoft-basic",
        )
        failures = 0
        for patch in sorted(self.patch_root.glob("*.patch")):
            completed = run(
                ["patch", "--batch", "--forward", "-p1", "-d", str(shadow)],
                input_text=patch.read_text(encoding="utf-8"),
                capture=True,
                check=False,
            )
            failures += int(completed.returncode != 0)
        rejects = sorted(path.relative_to(shadow).as_posix() for path in shadow.rglob("*.rej"))
        expected = sorted(
            [
                "modules/_bundled/sirsoft-board/composer.json.rej",
                "modules/_bundled/sirsoft-board/module.json.rej",
                "modules/_bundled/sirsoft-board/package-lock.json.rej",
                "modules/_bundled/sirsoft-board/package.json.rej",
            ]
        )
        if failures != 1 or rejects != expected:
            raise RuntimeError(
                f"contract patch is incompatible: failures={failures} rejects={rejects}"
            )
        for reject in shadow.rglob("*.rej"):
            reject.unlink()
        source_failures = verify_source(shadow, self.repo)
        if source_failures:
            raise RuntimeError(f"shadow G7 contract failed: {source_failures}")
        return shadow

    def _changed_targets(self, shadow: Path) -> tuple[list[str], list[str]]:
        changed: list[str] = []
        targets: set[str] = set()
        for relative in patched_paths(self.patch_root):
            source = shadow / relative
            target = self.app_root / relative
            if target.is_symlink():
                raise RuntimeError(f"refusing symbolic-link deployment target: {relative}")
            if source.is_file():
                changed_on_disk = not target.is_file() or source.read_bytes() != target.read_bytes()
                if changed_on_disk:
                    changed.append(relative)
                else:
                    continue
            else:
                continue
            targets.add(relative)
            active: str | None = None
            if relative.startswith("modules/_bundled/sirsoft-board/"):
                active = relative.replace("modules/_bundled", "modules", 1)
            elif relative.startswith("templates/_bundled/sirsoft-basic/"):
                active = relative.replace("templates/_bundled", "templates", 1)
            if active is not None:
                active_path = self.app_root / active
                if active_path.is_symlink():
                    raise RuntimeError(f"refusing symbolic-link deployment target: {active}")
                if target.exists() != active_path.exists():
                    raise RuntimeError(f"active and bundled G7 file presence differs: {active}")
                if target.is_file() and target.read_bytes() != active_path.read_bytes():
                    raise RuntimeError(f"active and bundled G7 files differ: {active}")
                targets.add(active)
        if not changed:
            raise RuntimeError("no contract changes were produced")
        return sorted(changed), sorted(targets)

    def _backup(self, targets: list[str], changed: list[str]) -> None:
        files_root = self.backup_root / "files"
        files_root.mkdir(parents=True, mode=0o700)
        manifest: list[str] = []
        for relative in targets:
            source = self.app_root / relative
            if source.is_symlink():
                raise RuntimeError(f"refusing symbolic-link target: {relative}")
            if source.is_file():
                destination = files_root / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, destination)
                manifest.append(f"{digest(destination)}  {relative}")
            elif not source.exists():
                self.created.append(relative)
            else:
                raise RuntimeError(f"deployment target is not a regular file: {relative}")
        (self.backup_root / "before.sha256").write_text("\n".join(manifest) + "\n")
        (self.backup_root / "target-paths.txt").write_text("\n".join(targets) + "\n")
        (self.backup_root / "patched-paths.txt").write_text("\n".join(changed) + "\n")
        (self.backup_root / "created-paths.txt").write_text("\n".join(self.created) + "\n")
        version = json.loads(
            (self.repo / "adapters/gnuboard7/jiwonpapa-g7mediabooster/module.json").read_text()
        )["version"]
        receipt = {
            "deployment_id": self.args.deployment_id,
            "module_id": MODULE_ID,
            "module_version": version,
            "module_zip_sha256": self.args.module_sha256,
            "app_root": str(self.app_root),
            "app_user": self.args.app_user,
        }
        (self.backup_root / "receipt.txt").write_text(
            "".join(f"{key}={value}\n" for key, value in receipt.items())
        )
        for path in [*self.backup_root.glob("*.txt"), self.backup_root / "before.sha256"]:
            path.chmod(0o600)

    def _install_patches(self, shadow: Path, changed: list[str]) -> None:
        user = pwd.getpwnam(self.args.app_user)
        web_group = grp.getgrnam("www-data")
        for relative in changed:
            source = shadow / relative
            destinations = [self.app_root / relative]
            if relative.startswith("modules/_bundled/sirsoft-board/"):
                installed = relative.replace("modules/_bundled", "modules", 1)
                destinations.append(self.app_root / installed)
            elif relative.startswith("templates/_bundled/sirsoft-basic/"):
                installed = relative.replace("templates/_bundled", "templates", 1)
                destinations.append(self.app_root / installed)
            for destination in destinations:
                destination.parent.mkdir(parents=True, exist_ok=True)
                os.chown(destination.parent, user.pw_uid, web_group.gr_gid)
                destination.parent.chmod(0o755)
                temporary = destination.with_name(
                    destination.name + f".g7mb-new-{self.args.deployment_id}"
                )
                shutil.copy2(source, temporary)
                os.chown(temporary, user.pw_uid, web_group.gr_gid)
                temporary.chmod(0o644)
                temporary.replace(destination)

    def _refresh_upstream_layouts(self) -> None:
        self.artisan("module:refresh-layout", "sirsoft-board", "--no-ansi")
        self.artisan("template:refresh-layout", "sirsoft-basic", "--no-ansi")
        self.artisan("optimize:clear", "--no-ansi")
        run(["systemctl", "reload", "php8.5-fpm.service"])

    def _install_module(self, work: Path) -> None:
        verifier = self.repo / "scripts/verify-gnuboard7-module-zip.php"
        metadata_path = self.repo / "adapters/gnuboard7/jiwonpapa-g7mediabooster/module.json"
        module_version = json.loads(metadata_path.read_text(encoding="utf-8"))["version"]
        if not isinstance(module_version, str) or not re.fullmatch(
            r"[0-9]+\.[0-9]+\.[0-9]+", module_version
        ):
            raise RuntimeError("module manifest version is invalid")
        run(
            [
                "sudo",
                "-u",
                self.args.app_user,
                "-H",
                "php",
                str(verifier),
                str(self.app_root),
                str(self.args.module_zip),
                str(module_version),
            ]
        )
        stage = work / "module-stage"
        safe_zip_extract(self.args.module_zip, stage)
        source = stage / MODULE_ID
        if not (source / "module.json").is_file():
            raise RuntimeError("module ZIP root is invalid")
        pending = self.app_root / "modules/_pending" / MODULE_ID
        pending.parent.mkdir(parents=True, exist_ok=True)
        run(["rsync", "-a", f"--chown={self.args.app_user}:www-data", f"{source}/", f"{pending}/"])
        self.artisan("module:install", MODULE_ID, "--vendor-mode=auto")
        self.artisan("module:activate", MODULE_ID)

    def rollback(self) -> None:
        """Restore files and module state after a failed installation."""

        if self.controller.module_active():
            self.artisan("module:deactivate", MODULE_ID, "--force")
        if self.controller.module_row():
            self.artisan("module:uninstall", MODULE_ID, "--force")
        shutil.rmtree(self.app_root / "modules/_pending" / MODULE_ID, ignore_errors=True)
        shutil.rmtree(self.app_root / "modules" / MODULE_ID, ignore_errors=True)
        for relative in self.created:
            (self.app_root / relative).unlink(missing_ok=True)
        run(["rsync", "-a", f"{self.backup_root / 'files'}/", f"{self.app_root}/"])
        self.artisan("optimize:clear", "--no-ansi")
        run(["systemctl", "reload", "php8.5-fpm.service"])
