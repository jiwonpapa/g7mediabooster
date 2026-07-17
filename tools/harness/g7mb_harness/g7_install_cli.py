"""Validated command surface for the privileged G7 installer."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

from .g7_install import Installer

SAFE_NAME = re.compile(r"^[A-Za-z0-9._-]+$")
SAFE_USER = re.compile(r"^[A-Za-z_][A-Za-z0-9_-]*$")
SAFE_ABSOLUTE = re.compile(r"^/[A-Za-z0-9._/-]+$")


def add_arguments(parser: argparse.ArgumentParser) -> None:
    """Register installer arguments."""

    parser.add_argument("app_root", type=Path)
    parser.add_argument("app_user")
    parser.add_argument("module_zip", type=Path)
    parser.add_argument("module_sha256")
    parser.add_argument("deployment_id")
    parser.add_argument("confirm_id")


def validate(args: argparse.Namespace) -> None:
    """Reject unsafe privileged installer arguments."""

    if args.confirm_id != args.deployment_id:
        raise RuntimeError("deployment confirmation mismatch")
    if not SAFE_ABSOLUTE.fullmatch(str(args.app_root)) or ".." in args.app_root.parts:
        raise RuntimeError("unsafe application root")
    if not SAFE_USER.fullmatch(args.app_user):
        raise RuntimeError("unsafe application user")
    if not SAFE_NAME.fullmatch(args.deployment_id):
        raise RuntimeError("unsafe deployment identifier")
    if not re.fullmatch(r"[a-f0-9]{64}", args.module_sha256):
        raise RuntimeError("unsafe module SHA-256")


def main(args: argparse.Namespace) -> int:
    """CLI entry point."""

    try:
        validate(args)
        Installer(args).execute()
        return 0
    except Exception as error:
        print(f"g7-live-install-remote FAIL: {error}", file=sys.stderr)
        return 1
