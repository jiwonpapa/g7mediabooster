"""Stock G7 capability verification. This command never applies patches."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

from .g7_contract_support import verify_module_host, verify_packaged_module
from .g7_remote import Controller


def repository_root() -> Path:
    """Resolve the repository containing this harness."""
    return Path(__file__).resolve().parents[3]


def verify_source(root: Path, repo: Path, support_root: Path | None = None) -> list[str]:
    """Check the adapter against unchanged host PHP classes and manifests."""
    if support_root is not None:
        passed = verify_packaged_module(root, support_root)
    else:
        passed = verify_module_host(
            root, repo / "scripts/verify-gnuboard7-module-host.php",
            repo / "adapters/gnuboard7/jiwonpapa-g7mediabooster",
        )
    return [] if passed else ["stock G7 7.1 activation capability"]


def runtime_layout_status(root: Path) -> dict[str, int]:
    """Use the same DB-resolved partial contract as installation."""
    import getpass
    return Controller(root, getpass.getuser(), "jiwonpapa-g7mediabooster").layout_status()


def add_arguments(parser: argparse.ArgumentParser) -> None:
    """Register source and optional runtime verification arguments."""
    parser.add_argument("root", type=Path)
    parser.add_argument("--runtime", action="store_true")
    parser.add_argument("--support-root", type=Path)


def main(args: argparse.Namespace) -> int:
    """Fail closed for unsupported stock hosts without changing their files."""
    try:
        root = args.root.resolve()
        support = args.support_root.resolve() if args.support_root else None
        if verify_source(root, repository_root(), support):
            return 1
        if args.runtime:
            print(json.dumps(runtime_layout_status(root), sort_keys=True))
    except (RuntimeError, subprocess.SubprocessError) as error:
        print(f"Gnuboard7 core-free contract: FAIL ({error})", file=sys.stderr)
        return 1
    print("Gnuboard7 core-free contract: PASS")
    return 0
