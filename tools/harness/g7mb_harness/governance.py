"""Language ownership and size ratchets for repository harnesses."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tomllib
from pathlib import Path

from .process import require_programs, run

DEFAULT_SHELL_LIMIT = 100
SHELL_TOTAL_LIMIT = 1_636
DEFAULT_PYTHON_LIMIT = 300
PYTHON_TOTAL_LIMIT = 3_182
ORCHESTRATION_TOTAL_LIMIT = 4_818
DEFAULT_SOURCE_LIMIT = 500
SHELL_EXCEPTIONS = {
    "scripts/cgroup-smoke-inner.sh": 109,
    "scripts/live-storage-conformance.sh": 139,
    "scripts/live-storage-preflight-smoke.sh": 141,
    "scripts/native-smoke.sh": 191,
    "scripts/package-g7-module.sh": 212,
    "scripts/package-server-bundle.sh": 152,
    "scripts/server-install-smoke.sh": 160,
}
PYTHON_EXCEPTIONS = {
    # Streamed as a self-contained program to the remote host.
    "tools/harness/g7mb_harness/g7_remote.py": 343,
}
SOURCE_EXCEPTIONS = {
    "crates/g7mb-persistence-sqlite/src/lib.rs": 3_525,
    "apps/g7mb-api/src/lib.rs": 2_398,
    "apps/g7mb-worker/src/lib.rs": 1_947,
    "crates/g7mb-media/src/lib.rs": 1_497,
    "crates/g7mb-object-store-s3/src/lib.rs": 1_341,
    "crates/g7mb-application/src/uploads.rs": 1_291,
    "crates/g7mb-config/src/lib.rs": 1_177,
    "apps/g7mbctl/src/installer.rs": 1_054,
    "apps/g7mbctl/src/main.rs": 1_039,
    "crates/g7mb-object-store-s3/tests/live_provider_conformance.rs": 876,
    "crates/g7mb-domain/src/lib.rs": 817,
    "apps/g7mb-worker/src/main.rs": 790,
    "apps/g7mb-sandbox/src/main.rs": 758,
    "crates/g7mb-application/src/delivery.rs": 753,
    "crates/g7mb-application/src/inventory.rs": 627,
    "crates/g7mb-application/src/lifecycle.rs": 618,
    "crates/g7mb-application/src/policies.rs": 580,
    "xtask/src/main.rs": 536,
    "adapters/gnuboard7/jiwonpapa-g7mediabooster/resources/js/upload/MultiUploader.ts": 507,
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
        elif relative in SHELL_EXCEPTIONS and count < maximum:
            failures.append(f"{relative}: lower its stale exception from {maximum} to {count}")
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
        elif relative in PYTHON_EXCEPTIONS and count < maximum:
            failures.append(f"{relative}: lower its stale exception from {maximum} to {count}")
    total = sum(counts.values())
    if total > PYTHON_TOTAL_LIMIT:
        failures.append(f"Python total: {total} lines exceeds {PYTHON_TOTAL_LIMIT}")
    if failures:
        raise RuntimeError("Python harness budget failed:\n" + "\n".join(failures))
    return {"files": len(counts), "lines": total}


def orchestration_budget(shell_lines: int, python_lines: int) -> dict[str, int]:
    """Keep the combined Bash and Python harness smaller after language migrations."""

    total = shell_lines + python_lines
    if total > ORCHESTRATION_TOTAL_LIMIT:
        raise RuntimeError(
            f"orchestration total: {total} lines exceeds {ORCHESTRATION_TOTAL_LIMIT}"
        )
    return {"lines": total}


def source_budget(root: Path) -> dict[str, int]:
    """Stop existing source monoliths growing and reject new files over 500 lines."""

    failures: list[str] = []
    counts: dict[str, int] = {}
    for source_root in ("apps", "crates", "adapters", "xtask"):
        directory = root / source_root
        if not directory.exists():
            continue
        for path in sorted(directory.rglob("*")):
            if not path.is_file() or path.suffix not in {".rs", ".php", ".ts"}:
                continue
            if {"dist", "node_modules", "vendor"}.intersection(path.parts):
                continue
            relative = path.relative_to(root).as_posix()
            count = line_count(path)
            counts[relative] = count
            maximum = SOURCE_EXCEPTIONS.get(relative, DEFAULT_SOURCE_LIMIT)
            if count > maximum:
                failures.append(f"{relative}: {count} lines exceeds {maximum}")
            elif relative in SOURCE_EXCEPTIONS and count < maximum:
                failures.append(f"{relative}: lower its stale exception from {maximum} to {count}")
    if failures:
        raise RuntimeError("source size ratchet failed:\n" + "\n".join(failures))
    return {"files": len(counts), "lines": sum(counts.values())}


def rust_harness_dependency_budget(root: Path) -> dict[str, int]:
    """Keep xtask independent from product crates and their transitive graph."""

    manifest_path = root / "xtask" / "Cargo.toml"
    manifest = tomllib.loads(manifest_path.read_text(encoding="utf-8"))
    dependencies = manifest.get("dependencies", {})
    product_dependencies = sorted(name for name in dependencies if name.startswith("g7mb-"))
    if product_dependencies:
        joined = ", ".join(product_dependencies)
        raise RuntimeError(f"xtask must not depend on product crates: {joined}")
    return {"direct_dependencies": len(dependencies), "product_dependencies": 0}


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
    orchestration = orchestration_budget(shell["lines"], python["lines"])
    sources = source_budget(root)
    rust_harness = rust_harness_dependency_budget(root)
    result: dict[str, object] = {
        "status": "PASS",
        "python": python,
        "shell": shell,
        "orchestration": orchestration,
        "sources": sources,
        "rust_harness": rust_harness,
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
