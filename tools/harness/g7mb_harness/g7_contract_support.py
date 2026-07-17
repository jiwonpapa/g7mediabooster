"""Reusable source and packaged-module helpers for G7 contract verification."""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path

from .g7_install_support import safe_zip_extract
from .process import run


def patched_paths(patch_root: Path) -> list[str]:
    """Return unique target paths from the canonical patch set."""

    paths: set[str] = set()
    for patch in sorted(patch_root.glob("*.patch")):
        for line in patch.read_text(encoding="utf-8").splitlines():
            if line.startswith("+++ b/"):
                paths.add(line.removeprefix("+++ b/"))
    return sorted(paths)


def verify_module_host(root: Path, verifier: Path, module_root: Path) -> bool:
    """Run the canonical PHP host/module activation contract."""

    completed = run(
        ["php", str(verifier), str(root), str(module_root)],
        capture=True,
        check=False,
    )
    if completed.stdout:
        print(completed.stdout, end="")
    if completed.returncode != 0 and completed.stderr:
        print(completed.stderr, file=sys.stderr, end="")
    return completed.returncode == 0


def verify_packaged_module(root: Path, support_root: Path) -> bool:
    """Extract and verify the module shipped beside an installed harness."""

    verifier = support_root / "verify-gnuboard7-module-host.php"
    module_zip = support_root / "jiwonpapa-g7mediabooster.zip"
    if not verifier.is_file() or not module_zip.is_file():
        return False
    with tempfile.TemporaryDirectory(prefix="g7mb-module-contract.") as temporary:
        extracted = Path(temporary)
        safe_zip_extract(module_zip, extracted)
        roots = [path for path in extracted.iterdir() if (path / "module.json").is_file()]
        return len(roots) == 1 and verify_module_host(root, verifier, roots[0])
