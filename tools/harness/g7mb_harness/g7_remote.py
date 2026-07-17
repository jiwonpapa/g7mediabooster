"""Standalone remote-side G7 service and layout controller.

This module is streamed to ``python3 -`` over SSH. Keep it standard-library only
and free of package-relative imports.
"""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

SAFE_NAME = re.compile(r"^[A-Za-z0-9._-]+$")
SAFE_USER = re.compile(r"^[A-Za-z_][A-Za-z0-9_-]*$")
SAFE_PATH = re.compile(r"^/[A-Za-z0-9._/-]+$")


def run(
    args: list[str],
    *,
    check: bool = True,
    capture: bool = False,
    cwd: Path | None = None,
) -> subprocess.CompletedProcess[str]:
    """Run a remote command without invoking a shell."""

    completed = subprocess.run(
        args,
        cwd=cwd,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
        check=False,
    )
    if check and completed.returncode != 0:
        detail = (completed.stderr or "").strip()
        raise RuntimeError(f"command failed ({completed.returncode}): {' '.join(args)}\n{detail}")
    return completed


class Controller:
    """Fail-closed remote controller for one installed G7 instance."""

    def __init__(self, app_root: Path, app_user: str, module_id: str) -> None:
        self.app_root = app_root
        self.app_user = app_user
        self.module_id = module_id
        self.artisan_path = app_root / "artisan"

    def artisan(self, *args: str, capture: bool = False) -> subprocess.CompletedProcess[str]:
        """Run Artisan as the application owner."""

        return run(
            ["sudo", "-u", self.app_user, "-H", "php", str(self.artisan_path), *args],
            capture=capture,
            cwd=self.app_root,
        )

    def module_row(self) -> str:
        """Return the module list row or an empty string."""

        completed = self.artisan("module:list", "--no-ansi", capture=True)
        return next((line for line in completed.stdout.splitlines() if self.module_id in line), "")

    def module_active(self) -> bool:
        """Report whether the module is active."""

        completed = self.artisan("module:list", "--status=active", "--no-ansi", capture=True)
        return self.module_id in completed.stdout

    @staticmethod
    def service_state() -> str:
        """Return the product target state."""

        completed = run(
            ["systemctl", "is-active", "g7mediabooster.target"],
            capture=True,
            check=False,
        )
        return completed.stdout.strip() or "inactive"

    @staticmethod
    def api_ready() -> bool:
        """Probe the loopback-only API without a shell or curl."""

        from urllib.error import URLError
        from urllib.request import urlopen

        try:
            with urlopen("http://127.0.0.1:8088/health/ready", timeout=2) as response:
                return 200 <= int(response.status) < 300
        except (OSError, URLError):
            return False

    def layout_status(self) -> dict[str, int]:
        """Resolve user/admin layouts from the database and count mounted overlays."""

        php = (
            "$service = app(\\App\\Services\\LayoutService::class); "
            '$user = json_encode($service->getLayout("sirsoft-basic", "board/form", true), '
            "JSON_UNESCAPED_SLASHES); "
            '$admin = json_encode($service->getLayout("sirsoft-admin_basic", '
            '"sirsoft-board.admin_board_post_form", true), JSON_UNESCAPED_SLASHES); '
            'echo json_encode(["user_mount"=>substr_count($user, "g7mb-user-uploader-mount"),'
            '"user_handler"=>substr_count($user, "jiwonpapa-g7mediabooster.mountUploader"),'
            '"admin_mount"=>substr_count($admin, "g7mb-admin-uploader-mount"),'
            '"admin_handler"=>substr_count($admin, "jiwonpapa-g7mediabooster.mountUploader")]);'
        )
        rendered = self.artisan("tinker", f"--execute={php}", capture=True).stdout.strip()
        try:
            payload = json.loads(rendered.splitlines()[-1])
        except (IndexError, json.JSONDecodeError) as error:
            raise RuntimeError(f"invalid layout evidence: {rendered}") from error
        result = {str(key): int(value) for key, value in payload.items()}
        required = ("user_mount", "user_handler", "admin_mount", "admin_handler")
        if any(result.get(key, 0) < 1 for key in required):
            raise RuntimeError(f"DB-resolved media uploader overlay is absent: {result}")
        return result

    def refresh_layouts(self) -> None:
        """Synchronize patched source layouts and active module overlays into the DB."""

        self.artisan("module:refresh-layout", "sirsoft-board", "--no-ansi")
        self.artisan("template:refresh-layout", "sirsoft-basic", "--no-ansi")
        self.artisan("module:refresh-layout", self.module_id, "--no-ansi")
        self.artisan("optimize:clear", "--no-ansi")

    def status(self, *, require_layout: bool) -> dict[str, Any]:
        """Return structured runtime status and optionally fail on a missing overlay."""

        status: dict[str, Any] = {
            "service": self.service_state(),
            "api": "ready" if self.api_ready() else "unavailable",
            "module": self.module_row() or "not-installed",
            "module_active": self.module_active() if self.module_row() else False,
        }
        if status["module_active"]:
            try:
                status["layout"] = self.layout_status()
                status["layout_applied"] = True
            except RuntimeError as error:
                status["layout_applied"] = False
                status["layout_error"] = str(error)
                if require_layout:
                    print(json.dumps(status, ensure_ascii=False, sort_keys=True))
                    raise
        return status

    def apply(self) -> dict[str, Any]:
        """Enable the product only when the live G7 layout overlay resolves correctly."""

        if not Path("/usr/local/bin/g7mbctl").is_file():
            raise RuntimeError("g7mbctl is not installed")
        run(["sudo", "test", "-f", "/etc/g7mediabooster/g7mb.toml"])
        if not self.module_row():
            raise RuntimeError(f"G7 module is not installed: {self.module_id}")
        run(["sudo", "systemctl", "enable", "--now", "g7mediabooster.target"])
        ready = False
        for _ in range(30):
            completed = run(
                ["sudo", "/usr/local/bin/g7mbctl", "status"],
                capture=True,
                check=False,
            )
            if completed.returncode == 0:
                ready = True
                break
            time.sleep(1)
        if not ready:
            run(
                ["sudo", "systemctl", "disable", "--now", "g7mediabooster.target"],
                check=False,
            )
            raise RuntimeError("service readiness failed; product target was stopped")
        if not self.module_active():
            self.artisan("module:activate", self.module_id)
        try:
            self.refresh_layouts()
            self.layout_status()
        except Exception:
            if self.module_active():
                self.artisan("module:deactivate", self.module_id, "--force")
            run(
                ["sudo", "systemctl", "disable", "--now", "g7mediabooster.target"],
                check=False,
            )
            raise
        return self.status(require_layout=True)

    def disable(self) -> dict[str, Any]:
        """Disable code paths while retaining configuration and data."""

        if self.module_active():
            self.artisan("module:deactivate", self.module_id)
        run(
            ["sudo", "systemctl", "disable", "--now", "g7mediabooster.target"],
            check=False,
        )
        if self.service_state() == "active":
            raise RuntimeError("product target remains active")
        return self.status(require_layout=False)

    def rollback(self, deployment_id: str) -> dict[str, Any]:
        """Restore a recorded deployment without deleting media or credentials."""

        backup_root = Path("/var/backups/g7mediabooster") / deployment_id
        for required in ("receipt.txt", "files", "created-paths.txt", "before.sha256"):
            run(["sudo", "test", "-e", str(backup_root / required)])
        receipt = run(["sudo", "cat", str(backup_root / "receipt.txt")], capture=True).stdout
        if f"deployment_id={deployment_id}\n" not in receipt:
            raise RuntimeError("rollback receipt id mismatch")
        if f"app_root={self.app_root}\n" not in receipt:
            raise RuntimeError("rollback app root mismatch")
        self._verify_backup_manifest(backup_root)
        if self.module_active():
            self.artisan("module:deactivate", self.module_id, "--force")
        if self.module_row():
            self.artisan("module:uninstall", self.module_id, "--force")
        run(["sudo", "rm", "-rf", str(self.app_root / "modules/_pending" / self.module_id)])
        run(["sudo", "rm", "-rf", str(self.app_root / "modules" / self.module_id)])
        created = run(
            ["sudo", "cat", str(backup_root / "created-paths.txt")], capture=True
        ).stdout.splitlines()
        for relative in created:
            if relative:
                validate_relative_path(relative)
                run(["sudo", "rm", "-f", str(self.app_root / relative)])
        run(["sudo", "rsync", "-a", f"{backup_root / 'files'}/", f"{self.app_root}/"])
        self.artisan("optimize:clear", "--no-ansi")
        run(["sudo", "systemctl", "reload", "php8.5-fpm.service"])
        run(
            ["sudo", "systemctl", "disable", "--now", "g7mediabooster.target"],
            check=False,
        )
        return self.status(require_layout=False)

    @staticmethod
    def _verify_backup_manifest(backup_root: Path) -> None:
        manifest = run(
            ["sudo", "cat", str(backup_root / "before.sha256")], capture=True
        ).stdout.splitlines()
        for line in manifest:
            digest, separator, relative = line.partition("  ")
            if not separator or not re.fullmatch(r"[a-f0-9]{64}", digest):
                raise RuntimeError("rollback checksum manifest is invalid")
            validate_relative_path(relative)
            actual = run(
                ["sudo", "sha256sum", str(backup_root / "files" / relative)], capture=True
            ).stdout.split()[0]
            if actual != digest:
                raise RuntimeError(f"rollback checksum mismatch: {relative}")


def validate_relative_path(value: str) -> None:
    """Reject absolute and parent-traversal paths."""

    path = Path(value)
    if path.is_absolute() or ".." in path.parts or not value or not SAFE_PATH.match(f"/{value}"):
        raise RuntimeError(f"unsafe relative path: {value}")


def validate_args(app_root: str, app_user: str, module_id: str, deployment_id: str) -> None:
    """Validate all values before they reach privileged commands."""

    if not SAFE_PATH.fullmatch(app_root) or "/../" in app_root or app_root.endswith("/.."):
        raise RuntimeError("unsafe application root")
    if not SAFE_USER.fullmatch(app_user):
        raise RuntimeError("unsafe application user")
    if not SAFE_NAME.fullmatch(module_id):
        raise RuntimeError("unsafe module identifier")
    if deployment_id != "-" and not SAFE_NAME.fullmatch(deployment_id):
        raise RuntimeError("unsafe deployment identifier")


def preflight(controller: Controller) -> dict[str, Any]:
    """Check required host tools without changing the system."""

    if not controller.artisan_path.is_file():
        raise RuntimeError(f"G7 root is invalid: {controller.app_root}")
    run(["id", controller.app_user])
    run(["sudo", "-n", "true"])
    names = ("php", "vips", "ffmpeg", "ffprobe", "python3")
    tools = {name: shutil.which(name) is not None for name in names}
    return {"tools": tools, **controller.status(require_layout=False)}


def main(argv: list[str] | None = None) -> int:
    """Remote process entry point."""

    values = list(sys.argv[1:] if argv is None else argv)
    if len(values) != 5:
        print(
            "remote controller requires ACTION APP_ROOT APP_USER MODULE_ID DEPLOYMENT_ID",
            file=sys.stderr,
        )
        return 64
    action, app_root, app_user, module_id, deployment_id = values
    try:
        validate_args(app_root, app_user, module_id, deployment_id)
        controller = Controller(Path(app_root), app_user, module_id)
        if action == "preflight":
            result = preflight(controller)
        elif action == "status":
            result = controller.status(require_layout=True)
        elif action == "apply":
            result = controller.apply()
        elif action == "disable":
            result = controller.disable()
        elif action == "rollback":
            if deployment_id == "-":
                raise RuntimeError("rollback requires a deployment identifier")
            result = controller.rollback(deployment_id)
        else:
            raise RuntimeError(f"unknown action: {action}")
        print(
            json.dumps(
                {"status": "PASS", "action": action, **result},
                ensure_ascii=False,
                sort_keys=True,
            )
        )
        return 0
    except Exception as error:
        print(
            json.dumps(
                {"status": "FAIL", "action": action, "error": str(error)},
                ensure_ascii=False,
                sort_keys=True,
            ),
            file=sys.stderr,
        )
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
