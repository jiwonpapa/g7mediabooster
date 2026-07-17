"""SSH transport for the fail-closed G7 remote controller."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

from .process import run

SAFE_HOST = re.compile(r"^[A-Za-z0-9._-]+$")
SAFE_USER = re.compile(r"^[A-Za-z_][A-Za-z0-9_-]*$")
SAFE_PATH = re.compile(r"^/[A-Za-z0-9._/-]+$")
MUTATING_ACTIONS = {"apply", "disable", "rollback"}


def remote_source() -> str:
    """Load the standalone script streamed to the target host."""

    return Path(__file__).with_name("g7_remote.py").read_text(encoding="utf-8")


def validate(args: argparse.Namespace) -> None:
    """Validate transport arguments and explicit mutation confirmation."""

    if not SAFE_HOST.fullmatch(args.host):
        raise RuntimeError("unsafe SSH host")
    if not SAFE_USER.fullmatch(args.app_user):
        raise RuntimeError("unsafe G7 application user")
    if not SAFE_PATH.fullmatch(args.app_root) or "/../" in args.app_root:
        raise RuntimeError("unsafe G7 application root")
    if args.action in MUTATING_ACTIONS and args.confirm != args.host:
        raise RuntimeError(f"refusing live mutation: use --confirm {args.host}")
    if args.action == "rollback" and not SAFE_HOST.fullmatch(args.deployment_id or ""):
        raise RuntimeError("rollback requires a safe --deployment-id")


def add_arguments(parser: argparse.ArgumentParser) -> None:
    """Register transport options."""

    parser.add_argument("action", choices=("preflight", "status", "apply", "disable", "rollback"))
    parser.add_argument("--host", default="g7devops")
    parser.add_argument("--app-root", default="/home/g7devops/public_html")
    parser.add_argument("--app-user", default="g7devops")
    parser.add_argument("--module-id", default="jiwonpapa-g7mediabooster")
    parser.add_argument("--deployment-id")
    parser.add_argument("--confirm")


def main(args: argparse.Namespace) -> int:
    """Stream and execute the controller without creating remote files."""

    try:
        validate(args)
        deployment_id = args.deployment_id or "-"
        completed = run(
            [
                "ssh",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                args.host,
                "python3",
                "-",
                args.action,
                args.app_root,
                args.app_user,
                args.module_id,
                deployment_id,
            ],
            input_text=remote_source(),
            check=False,
        )
        return completed.returncode
    except (RuntimeError, subprocess.SubprocessError) as error:
        print(f"g7-live-control FAIL: {error}", file=sys.stderr)
        return 1
