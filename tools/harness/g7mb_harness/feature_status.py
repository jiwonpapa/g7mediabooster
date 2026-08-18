"""Validate the canonical product feature and evidence registry."""

from __future__ import annotations

import json
import re
import tomllib
from datetime import date
from pathlib import Path
from typing import Any

ALLOWED_STATUSES = {
    "IMPLEMENTED",
    "VERIFIED_LOCAL",
    "VERIFIED_PROVIDER",
    "RELEASED",
    "BLOCKED",
    "EXCLUDED",
}
PUBLISHABLE_STATUSES = {"VERIFIED_LOCAL", "VERIFIED_PROVIDER", "RELEASED"}
REQUIRED_STATES = {
    "cloudflare_r2_profile": ("VERIFIED_PROVIDER", True),
    "gnuboard7_module": ("BLOCKED", False),
    "lightsail_object_storage_profile": ("IMPLEMENTED", False),
    "multi_node_postgresql": ("EXCLUDED", False),
}
SPEC_VERSION_PATTERN = re.compile(r"^- 스펙 버전: (?P<version>\S+)$", re.MULTILINE)


def _load_json(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise RuntimeError(f"JSON root must be an object: {path}")
    return data


def _required_text(data: dict[str, Any], key: str, label: str) -> str:
    value = data.get(key)
    if not isinstance(value, str) or not value.strip():
        raise RuntimeError(f"{label}.{key} must be a non-empty string")
    return value


def validate_repository(root: Path) -> dict[str, object]:
    """Validate feature states, evidence paths, and manifest version contracts."""

    root = root.resolve()
    registry = _load_json(root / "deploy" / "official-features-v1.json")
    if registry.get("schema_version") != 2:
        raise RuntimeError("official feature schema_version must be 2")
    reviewed = _required_text(registry, "last_reviewed", "registry")
    try:
        date.fromisoformat(reviewed)
    except ValueError as error:
        raise RuntimeError("registry.last_reviewed must be an ISO date") from error

    cargo = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    product_version = str(cargo["workspace"]["package"]["version"])
    if registry.get("product_version") != product_version:
        raise RuntimeError("server version drift between Cargo and feature registry")

    spec_text = (root / "SPEC.md").read_text(encoding="utf-8")
    match = SPEC_VERSION_PATTERN.search(spec_text)
    if match is None or registry.get("spec_version") != match.group("version"):
        raise RuntimeError("feature registry spec_version drift")

    module_root = root / "adapters" / "gnuboard7" / "jiwonpapa-g7mediabooster"
    module = _load_json(module_root / "module.json")
    package = _load_json(module_root / "package.json")
    module_status = registry.get("module")
    if not isinstance(module_status, dict):
        raise RuntimeError("feature registry module must be an object")
    expected_module = {
        "identifier": module.get("identifier"),
        "version": module.get("version"),
        "gnuboard7": module.get("g7_version"),
        "sirsoft_board": module.get("dependencies", {}).get("modules", {}).get("sirsoft-board"),
        "capability": "sirsoft-board.secure-external-attachments "
        + str(
            module.get("compatibility", {})
            .get("contracts", {})
            .get("sirsoft-board.secure-external-attachments", "")
        ),
    }
    if module_status != expected_module:
        raise RuntimeError("G7 module contract drift between module and feature manifests")
    if package.get("version") != module.get("version"):
        raise RuntimeError("G7 module version drift between module.json and package.json")

    features = registry.get("features")
    if not isinstance(features, list) or not features:
        raise RuntimeError("feature registry must contain a non-empty features list")
    by_id: dict[str, dict[str, Any]] = {}
    for index, feature in enumerate(features):
        if not isinstance(feature, dict):
            raise RuntimeError(f"feature[{index}] must be an object")
        feature_id = _required_text(feature, "id", f"feature[{index}]")
        _required_text(feature, "title", feature_id)
        if feature_id in by_id:
            raise RuntimeError(f"duplicate feature id: {feature_id}")
        status = feature.get("status")
        if status not in ALLOWED_STATUSES:
            raise RuntimeError(f"{feature_id}: unsupported status {status}")
        publishable = feature.get("publishable")
        if not isinstance(publishable, bool):
            raise RuntimeError(f"{feature_id}: publishable must be boolean")
        if publishable != (status in PUBLISHABLE_STATUSES):
            raise RuntimeError(f"{feature_id}: publishable conflicts with status {status}")
        evidence = feature.get("evidence")
        if not isinstance(evidence, list) or not all(isinstance(item, str) for item in evidence):
            raise RuntimeError(f"{feature_id}: evidence must be a string list")
        if status in {"VERIFIED_LOCAL", "VERIFIED_PROVIDER", "RELEASED"} and not evidence:
            raise RuntimeError(f"{feature_id}: verified status requires evidence")
        for relative in evidence:
            path = Path(relative)
            if path.is_absolute() or ".." in path.parts or not (root / path).is_file():
                raise RuntimeError(f"{feature_id}: missing or unsafe evidence path {relative}")
        if status in {"IMPLEMENTED", "BLOCKED", "EXCLUDED"}:
            _required_text(feature, "reason", feature_id)
        by_id[feature_id] = feature

    for feature_id, (status, publishable) in REQUIRED_STATES.items():
        feature = by_id.get(feature_id)
        if feature is None:
            raise RuntimeError(f"required feature is missing: {feature_id}")
        if (feature["status"], feature["publishable"]) != (status, publishable):
            raise RuntimeError(f"required feature state drift: {feature_id}")

    counts = {
        status: sum(feature["status"] == status for feature in features)
        for status in sorted(ALLOWED_STATUSES)
    }
    return {"status": "PASS", "features": len(features), "states": counts}
