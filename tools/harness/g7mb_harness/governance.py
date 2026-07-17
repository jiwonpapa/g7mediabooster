"""Language ownership and size ratchets for repository harnesses."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

from .process import require_programs, run

DEFAULT_SHELL_LIMIT = 100
SHELL_TOTAL_LIMIT = 2_700
DEFAULT_PYTHON_LIMIT = 300
SHELL_EXCEPTIONS = {
    "scripts/cgroup-smoke-inner.sh": 109,
    "scripts/g7-live-module-install-remote.sh": 20,
    "scripts/heavy-avif.sh": 180,
    "scripts/heavy-image.sh": 154,
    "scripts/live-storage-conformance.sh": 139,
    "scripts/live-storage-preflight-smoke.sh": 141,
    "scripts/load-100.sh": 185,
    "scripts/native-smoke.sh": 191,
    "scripts/package-g7-module.sh": 212,
    "scripts/package-server-bundle.sh": 152,
    "scripts/server-install-smoke.sh": 160,
    "scripts/verify-gnuboard7-media-contract.sh": 20,
}
PYTHON_EXCEPTIONS = {
    # Streamed as a self-contained program to the remote host.
    "tools/harness/g7mb_harness/g7_remote.py": 350,
    # Transactional remote installation deliberately keeps rollback close to apply.
    "tools/harness/g7mb_harness/g7_install.py": 320,
}


def repository_root() -> Path:
    """Resolve the repository root independently of the current directory."""

    return Path(__file__).resolve().parents[3]


def line_count(path: Path) -> int:
    """Count physical lines without loading arbitrary binary data."""

    with path.open("r", encoding="utf-8") as handle:
        return sum(1 for _ in handle)


def shell_budget(root: Path) -> dict[str, int]:
    """Enforce a total ratchet and forbid new large shell harnesses."""

    failures: list[str] = []
    counts: dict[str, int] = {}
    for path in sorted((root / "scripts").glob("*.sh")):
        relative = path.relative_to(root).as_posix()
        count = line_count(path)
        counts[relative] = count
        maximum = SHELL_EXCEPTIONS.get(relative, DEFAULT_SHELL_LIMIT)
        if count > maximum:
            failures.append(f"{relative}: {count} lines exceeds {maximum}")
    total = sum(counts.values())
    if total > SHELL_TOTAL_LIMIT:
        failures.append(f"shell total: {total} lines exceeds {SHELL_TOTAL_LIMIT}")
    if failures:
        raise RuntimeError("shell harness budget failed:\n" + "\n".join(failures))
    return {"files": len(counts), "lines": total}


def python_budget(root: Path) -> dict[str, int]:
    """Prevent Python orchestration from becoming a new monolith."""

    failures: list[str] = []
    counts: dict[str, int] = {}
    harness_root = root / "tools" / "harness" / "g7mb_harness"
    for path in sorted(harness_root.glob("*.py")):
        relative = path.relative_to(root).as_posix()
        count = line_count(path)
        counts[relative] = count
        maximum = PYTHON_EXCEPTIONS.get(relative, DEFAULT_PYTHON_LIMIT)
        if count > maximum:
            failures.append(f"{relative}: {count} lines exceeds {maximum}")
    if failures:
        raise RuntimeError("Python harness budget failed:\n" + "\n".join(failures))
    return {"files": len(counts), "lines": sum(counts.values())}


def python_static_gate(root: Path) -> None:
    """Compile Python and reject accidental shell execution APIs."""

    harness_root = root / "tools" / "harness"
    run([sys.executable, "-m", "compileall", "-q", str(harness_root)], cwd=root)
    forbidden = "shell" + "=True"
    for path in harness_root.rglob("*.py"):
        if forbidden in path.read_text(encoding="utf-8").replace(" ", ""):
            raise RuntimeError(f"subprocess shell mode is forbidden: {path.relative_to(root)}")


def shell_static_gate(root: Path, require_tools: bool) -> None:
    """Parse all shell wrappers and apply ShellCheck warning policy."""

    scripts = sorted(str(path) for path in (root / "scripts").glob("*.sh"))
    for script in scripts:
        run(["bash", "-n", script], cwd=root)
    try:
        require_programs(["shellcheck"])
    except RuntimeError:
        if require_tools:
            raise
        return
    run(["shellcheck", "-S", "warning", "-x", *scripts], cwd=root)


def run_unit_tests(root: Path) -> None:
    """Run the dependency-free Python harness unit suite."""

    run(
        [
            sys.executable,
            "-m",
            "unittest",
            "discover",
            "-s",
            "tools/harness/tests",
            "-p",
            "test_*.py",
        ],
        cwd=root,
    )


def execute(*, require_tools: bool = False) -> dict[str, object]:
    """Run all harness governance gates and return evidence."""

    root = repository_root()
    python_static_gate(root)
    shell_static_gate(root, require_tools)
    run_unit_tests(root)
    if require_tools:
        require_programs(["ruff", "mypy", "pytest"])
        run(["ruff", "check", "tools/harness"], cwd=root)
        run(
            ["mypy", "--config-file", "pyproject.toml"],
            cwd=root / "tools/harness",
        )
        run(["pytest", "-c", "tools/harness/pyproject.toml"], cwd=root)
    shell = shell_budget(root)
    python = python_budget(root)
    result: dict[str, object] = {
        "status": "PASS",
        "python": python,
        "shell": shell,
    }
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return result


def main(argv: list[str] | None = None) -> int:
    """CLI entry point used by xtask and CI."""

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--require-tools", action="store_true")
    args = parser.parse_args(argv)
    try:
        execute(require_tools=args.require_tools)
    except (RuntimeError, subprocess.SubprocessError) as error:
        print(f"harness-governance FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
