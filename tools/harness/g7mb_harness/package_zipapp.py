"""Build a deterministic, dependency-free Python harness ZIP application."""

from __future__ import annotations

import argparse
import zipfile
from pathlib import Path

FIXED_TIMESTAMP = (1980, 1, 1, 0, 0, 0)


def write_member(archive: zipfile.ZipFile, name: str, payload: bytes) -> None:
    """Write one normalized executable-readable ZIP member."""

    info = zipfile.ZipInfo(name, FIXED_TIMESTAMP)
    info.compress_type = zipfile.ZIP_DEFLATED
    info.create_system = 3
    info.external_attr = 0o644 << 16
    archive.writestr(info, payload)


def build(destination: Path) -> None:
    """Package only tracked Python modules with a stable entry point."""

    package = Path(__file__).resolve().parent
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(destination.suffix + ".tmp")
    temporary.unlink(missing_ok=True)
    with zipfile.ZipFile(temporary, "w") as archive:
        write_member(
            archive,
            "__main__.py",
            b"from g7mb_harness.__main__ import main\nraise SystemExit(main())\n",
        )
        for source in sorted(package.glob("*.py")):
            write_member(archive, f"g7mb_harness/{source.name}", source.read_bytes())
    temporary.replace(destination)


def add_arguments(parser: argparse.ArgumentParser) -> None:
    """Register the deterministic output path."""

    parser.add_argument("destination", type=Path)


def main(args: argparse.Namespace) -> int:
    """Build the ZIP application."""

    build(args.destination)
    print(f"harness-zipapp PASS path={args.destination}")
    return 0
