"""Enforce component line-coverage floors from a Cargo LCOV report."""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import Mapping

COMPONENT_THRESHOLDS: dict[str, float] = {
    "apps/g7mb-api/": 80.0,
    "apps/g7mb-worker/": 79.0,
    "crates/g7mb-application/": 87.0,
    "crates/g7mb-auth/": 98.0,
    "crates/g7mb-config/": 85.0,
    "crates/g7mb-domain/": 96.0,
    "crates/g7mb-media/": 91.0,
    "crates/g7mb-object-store-s3/": 45.0,
    "crates/g7mb-persistence-sqlite/": 89.0,
}


def component_for(source: str, components: Mapping[str, float]) -> str | None:
    """Map an LCOV source path to its configured workspace component."""

    normalized = source.replace("\\", "/")
    return next((component for component in components if component in normalized), None)


def parse_lcov(path: Path, components: Mapping[str, float]) -> dict[str, tuple[int, int]]:
    """Return covered and instrumented line counts for configured components."""

    lines: dict[str, dict[tuple[str, int], int]] = {component: {} for component in components}
    source = ""
    component: str | None = None
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        if raw_line.startswith("SF:"):
            source = raw_line[3:]
            component = component_for(source, components)
        elif component is not None and raw_line.startswith("DA:"):
            fields = raw_line[3:].split(",", maxsplit=2)
            if len(fields) < 2:
                raise RuntimeError(f"malformed LCOV line: {raw_line}")
            line_number = int(fields[0])
            executions = int(fields[1])
            key = (source, line_number)
            lines[component][key] = max(lines[component].get(key, 0), executions)

    return {
        component: (sum(executions > 0 for executions in entries.values()), len(entries))
        for component, entries in lines.items()
    }


def enforce(path: Path, thresholds: Mapping[str, float] = COMPONENT_THRESHOLDS) -> dict[str, float]:
    """Reject missing components and coverage below their committed floors."""

    counts = parse_lcov(path, thresholds)
    percentages: dict[str, float] = {}
    failures: list[str] = []
    for component, minimum in thresholds.items():
        covered, instrumented = counts[component]
        if instrumented == 0:
            failures.append(f"{component}: missing from LCOV report")
            continue
        percentage = covered * 100 / instrumented
        percentages[component] = percentage
        if percentage < minimum:
            failures.append(f"{component}: {percentage:.2f}% is below {minimum:.2f}%")
    if failures:
        raise RuntimeError("component coverage ratchet failed:\n" + "\n".join(failures))
    return percentages


def add_arguments(parser: argparse.ArgumentParser) -> None:
    """Register the coverage-ratchet command arguments."""

    parser.add_argument("lcov", type=Path)


def main(args: argparse.Namespace) -> int:
    """Run the component coverage ratchet and print stable evidence."""

    percentages = enforce(args.lcov)
    for component, percentage in percentages.items():
        print(f"{component} {percentage:.2f}%")
    return 0
