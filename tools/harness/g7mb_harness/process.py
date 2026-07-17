"""Safe subprocess primitives shared by infrastructure harnesses."""

from __future__ import annotations

import os
import shutil
import subprocess
from collections.abc import Iterable, Mapping, Sequence
from pathlib import Path


class HarnessCommandError(RuntimeError):
    """A child process failed without exposing secret environment values."""

    def __init__(self, args: Sequence[str], returncode: int, stderr: str = "") -> None:
        rendered = " ".join(args)
        detail = stderr.strip()
        message = f"command failed ({returncode}): {rendered}"
        if detail:
            message = f"{message}\n{detail}"
        super().__init__(message)
        self.args_safe = tuple(args)
        self.returncode = returncode


def merged_env(overrides: Mapping[str, str] | None = None) -> dict[str, str]:
    """Return the current environment with explicit, non-null overrides."""

    environment = dict(os.environ)
    if overrides:
        environment.update(overrides)
    return environment


def require_programs(programs: Iterable[str]) -> None:
    """Fail with one actionable message when required programs are unavailable."""

    missing = sorted(program for program in set(programs) if shutil.which(program) is None)
    if missing:
        raise RuntimeError(f"required programs are unavailable: {', '.join(missing)}")


def run(
    args: Sequence[str],
    *,
    cwd: Path | None = None,
    env: Mapping[str, str] | None = None,
    input_text: str | None = None,
    capture: bool = False,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    """Run a command without a shell and with deterministic UTF-8 text handling."""

    completed = subprocess.run(
        list(args),
        cwd=cwd,
        env=merged_env(env),
        input=input_text,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
        check=False,
    )
    if check and completed.returncode != 0:
        raise HarnessCommandError(args, completed.returncode, completed.stderr or "")
    return completed


def output(
    args: Sequence[str],
    *,
    cwd: Path | None = None,
    env: Mapping[str, str] | None = None,
    input_text: str | None = None,
) -> str:
    """Return stripped stdout from a successful command."""

    return run(args, cwd=cwd, env=env, input_text=input_text, capture=True).stdout.strip()
