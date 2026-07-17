"""Filesystem primitives shared by the privileged G7 installer."""

from __future__ import annotations

import hashlib
import stat
import zipfile
from pathlib import Path


def digest(path: Path) -> str:
    """Compute a streaming SHA-256 digest."""

    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def safe_zip_extract(archive: Path, destination: Path) -> None:
    """Extract a ZIP after rejecting absolute and traversal paths."""

    with zipfile.ZipFile(archive) as source:
        members = source.infolist()
        if len(members) > 10_000:
            raise RuntimeError("module ZIP contains too many entries")
        total_size = 0
        names: set[str] = set()
        for member in members:
            path = Path(member.filename)
            if not member.filename or path.is_absolute() or ".." in path.parts:
                raise RuntimeError(f"unsafe module ZIP path: {member.filename}")
            if member.filename in names:
                raise RuntimeError(f"duplicate module ZIP path: {member.filename}")
            names.add(member.filename)
            mode = member.external_attr >> 16
            if stat.S_ISLNK(mode):
                raise RuntimeError(f"symbolic link is forbidden in module ZIP: {member.filename}")
            total_size += member.file_size
            if member.file_size > 64 * 1024 * 1024 or total_size > 256 * 1024 * 1024:
                raise RuntimeError("module ZIP expanded size exceeds the safety limit")
        source.extractall(destination)
