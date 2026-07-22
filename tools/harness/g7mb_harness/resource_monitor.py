"""Bounded subprocess execution for native integration gates."""

from __future__ import annotations

import os
import signal
import subprocess
import time
from collections.abc import Callable, Mapping, Sequence
from contextlib import suppress
from dataclasses import dataclass
from pathlib import Path
from typing import IO

from .process import HarnessCommandError, merged_env, run


class ResourceLimitError(RuntimeError):
    """A child process exceeded a declared wall, memory, or disk budget."""


@dataclass(frozen=True)
class ResourceUsage:
    """Peak resources observed while one process group was running."""

    elapsed_ms: int
    peak_rss_kib: int
    peak_disk_kib: int = 0


def positive_int_env(name: str, default: int) -> int:
    """Read a strictly positive integer environment setting."""

    raw = os.environ.get(name, str(default))
    try:
        value = int(raw)
    except ValueError as error:
        raise ResourceLimitError(f"{name} must be a positive integer") from error
    if value <= 0:
        raise ResourceLimitError(f"{name} must be a positive integer")
    return value


def process_tree_rss(rows: Sequence[tuple[int, int, int]], root_pid: int) -> int:
    """Sum RSS for a process and every recursively discovered descendant."""

    selected = {root_pid}
    changed = True
    while changed:
        changed = False
        for pid, parent, _rss in rows:
            if parent in selected and pid not in selected:
                selected.add(pid)
                changed = True
    return sum(rss for pid, _parent, rss in rows if pid in selected)


def process_tree_rss_kib(root_pid: int) -> int:
    """Read the current process table and calculate tree RSS in KiB."""

    completed = run(["ps", "-axo", "pid=,ppid=,rss="], capture=True, check=False)
    if completed.returncode != 0:
        return 0
    rows: list[tuple[int, int, int]] = []
    for line in completed.stdout.splitlines():
        fields = line.split()
        if len(fields) == 3 and all(field.isdigit() for field in fields):
            rows.append((int(fields[0]), int(fields[1]), int(fields[2])))
    return process_tree_rss(rows, root_pid)


def directory_size_kib(path: Path) -> int:
    """Return logical file bytes below a temporary directory, rounded to KiB."""

    total = 0
    try:
        for parent, _directories, filenames in os.walk(path):
            for filename in filenames:
                try:
                    total += (Path(parent) / filename).stat(follow_symlinks=False).st_size
                except FileNotFoundError:
                    continue
    except FileNotFoundError:
        return 0
    return (total + 1023) // 1024


def _terminate_group(process: subprocess.Popen[str]) -> None:
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    with suppress(subprocess.TimeoutExpired):
        process.wait(timeout=2)
    with suppress(ProcessLookupError):
        os.killpg(process.pid, signal.SIGKILL)
    process.wait()


def _tail(path: Path, lines: int = 120) -> str:
    try:
        return "\n".join(path.read_text(encoding="utf-8", errors="replace").splitlines()[-lines:])
    except FileNotFoundError:
        return ""


def run_monitored(
    args: Sequence[str],
    *,
    cwd: Path,
    log_path: Path,
    timeout_seconds: float,
    max_rss_kib: int,
    env: Mapping[str, str] | None = None,
    stdout_path: Path | None = None,
    disk_path: Path | None = None,
    max_disk_kib: int | None = None,
    sample_seconds: float = 0.05,
    rss_reader: Callable[[int], int] = process_tree_rss_kib,
    disk_reader: Callable[[Path], int] = directory_size_kib,
) -> ResourceUsage:
    """Run an argument-array command and enforce process-tree resource limits."""

    log_path.parent.mkdir(parents=True, exist_ok=True)
    output: IO[str]
    started = time.monotonic()
    peak_rss = 0
    peak_disk = 0
    process: subprocess.Popen[str] | None = None
    with log_path.open("a", encoding="utf-8") as log:
        output = log if stdout_path is None else stdout_path.open("w", encoding="utf-8")
        try:
            process = subprocess.Popen(
                list(args),
                cwd=cwd,
                env=merged_env(env),
                text=True,
                encoding="utf-8",
                errors="replace",
                stdout=output,
                stderr=log,
                start_new_session=True,
            )
            while process.poll() is None:
                peak_rss = max(peak_rss, rss_reader(process.pid))
                if disk_path is not None:
                    peak_disk = max(peak_disk, disk_reader(disk_path))
                if time.monotonic() - started > timeout_seconds:
                    raise ResourceLimitError(
                        f"command exceeded {timeout_seconds:g}s: {' '.join(args)}"
                    )
                time.sleep(sample_seconds)
            returncode = process.wait()
            if returncode != 0:
                raise HarnessCommandError(args, returncode, _tail(log_path))
        except BaseException:
            if process is not None:
                _terminate_group(process)
            raise
        finally:
            if output is not log:
                output.close()
    usage = ResourceUsage(round((time.monotonic() - started) * 1000), peak_rss, peak_disk)
    if usage.peak_rss_kib > max_rss_kib:
        raise ResourceLimitError(f"peak RSS {usage.peak_rss_kib} KiB exceeded {max_rss_kib} KiB")
    if max_disk_kib is not None and usage.peak_disk_kib > max_disk_kib:
        raise ResourceLimitError(
            f"peak temp disk {usage.peak_disk_kib} KiB exceeded {max_disk_kib} KiB"
        )
    return usage
