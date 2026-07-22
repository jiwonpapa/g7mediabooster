"""Command-line entry point for repository infrastructure harnesses."""

from __future__ import annotations

import argparse
import sys

from . import (
    coverage_ratchet,
    full_stack,
    g7_install_cli,
    g7_live,
    governance,
    package_zipapp,
    verify_g7_contract,
)


def parser() -> argparse.ArgumentParser:
    """Build the stable, repository-local harness CLI."""

    root = argparse.ArgumentParser(prog="python3 -m tools.harness.g7mb_harness")
    commands = root.add_subparsers(dest="command", required=True)

    governance_parser = commands.add_parser("governance", help="run language and size gates")
    governance_parser.add_argument("--require-tools", action="store_true")

    coverage_ratchet.add_arguments(commands.add_parser("coverage-ratchet"))
    full_stack.add_arguments(commands.add_parser("full-stack-smoke"))
    g7_live.add_arguments(commands.add_parser("g7-live-control"))
    g7_install_cli.add_arguments(commands.add_parser("g7-live-install-remote"))
    verify_g7_contract.add_arguments(commands.add_parser("verify-g7-contract"))
    package_zipapp.add_arguments(commands.add_parser("package-zipapp"))
    return root


def main(argv: list[str] | None = None) -> int:
    """Dispatch a typed harness command."""

    args = parser().parse_args(argv)
    if args.command == "governance":
        return governance.main(["--require-tools"] if args.require_tools else [])
    if args.command == "coverage-ratchet":
        return coverage_ratchet.main(args)
    if args.command == "full-stack-smoke":
        return full_stack.main(args)
    if args.command == "g7-live-control":
        return g7_live.main(args)
    if args.command == "g7-live-install-remote":
        return g7_install_cli.main(args)
    if args.command == "verify-g7-contract":
        return verify_g7_contract.main(args)
    if args.command == "package-zipapp":
        return package_zipapp.main(args)
    raise AssertionError(f"unhandled command: {args.command}")


if __name__ == "__main__":
    sys.exit(main())
