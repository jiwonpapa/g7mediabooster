"""Machine-readable evidence emitted by operational harnesses."""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import TypeAlias

JsonScalar: TypeAlias = str | int | float | bool | None
JsonValue: TypeAlias = JsonScalar | list["JsonValue"] | dict[str, "JsonValue"]


@dataclass
class Evidence:
    """Collect stable facts and emit one compact JSON result."""

    harness: str
    phase: str = "bootstrap"
    facts: dict[str, JsonValue] = field(default_factory=dict)

    def set_phase(self, phase: str) -> None:
        """Record the operation currently in progress."""

        self.phase = phase

    def add(self, key: str, value: JsonValue) -> None:
        """Add a non-secret evidence value."""

        self.facts[key] = value

    def render(self, status: str, error: str | None = None) -> str:
        """Serialize the final result with stable key ordering."""

        payload: dict[str, JsonValue] = {
            "harness": self.harness,
            "status": status,
            "phase": self.phase,
            "facts": self.facts,
        }
        if error:
            payload["error"] = error
        return json.dumps(payload, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
