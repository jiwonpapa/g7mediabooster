"""Keep a Changelog and Semantic Versioning release-policy gate."""

from __future__ import annotations

import json
import re
import tomllib
from dataclasses import dataclass
from datetime import date
from itertools import pairwise
from pathlib import Path
from typing import Any

from . import feature_status

SEMVER_PATTERN = re.compile(
    r"^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)"
    r"(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)"
    r"(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?"
    r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$"
)
SECTION_PATTERN = re.compile(
    r"^## \[(?P<label>Unreleased|[^]]+)\]"
    r"(?: - (?P<date>\d{4}-\d{2}-\d{2})(?: \[YANKED\])?)?$",
    re.MULTILINE,
)
CATEGORY_PATTERN = re.compile(r"^### (?P<category>[^\n]+)$", re.MULTILINE)
LINK_PATTERN = re.compile(r"^\[(?P<label>Unreleased|[^]]+)\]: (?P<url>https://\S+)$", re.MULTILINE)
ALLOWED_CATEGORIES = {"Added", "Changed", "Deprecated", "Removed", "Fixed", "Security"}
KEEP_A_CHANGELOG_URL = "https://keepachangelog.com/ko/1.1.0/"
SEMVER_URL = "https://semver.org/lang/ko/"


@dataclass(frozen=True)
class SemVer:
    """Parsed SemVer precedence fields; build metadata is intentionally ignored."""

    major: int
    minor: int
    patch: int
    prerelease: tuple[str, ...] | None


@dataclass(frozen=True)
class Product:
    """One independently versioned product and its changelog contract."""

    name: str
    version: str
    tag_prefix: str
    changelog: Path


def parse_semver(value: str) -> SemVer:
    """Parse an exact Semantic Versioning 2.0.0 string."""

    match = SEMVER_PATTERN.fullmatch(value)
    if match is None:
        raise RuntimeError(f"invalid Semantic Version: {value}")
    prerelease = tuple(match.group(4).split(".")) if match.group(4) else None
    return SemVer(int(match.group(1)), int(match.group(2)), int(match.group(3)), prerelease)


def compare_semver(left: SemVer, right: SemVer) -> int:
    """Return negative, zero, or positive according to SemVer precedence."""

    left_core = (left.major, left.minor, left.patch)
    right_core = (right.major, right.minor, right.patch)
    if left_core != right_core:
        return (left_core > right_core) - (left_core < right_core)
    if left.prerelease is None or right.prerelease is None:
        return (left.prerelease is None) - (right.prerelease is None)
    for left_item, right_item in zip(left.prerelease, right.prerelease, strict=False):
        if left_item == right_item:
            continue
        left_numeric = left_item.isdigit()
        right_numeric = right_item.isdigit()
        if left_numeric and right_numeric:
            return (int(left_item) > int(right_item)) - (int(left_item) < int(right_item))
        if left_numeric != right_numeric:
            return -1 if left_numeric else 1
        return (left_item > right_item) - (left_item < right_item)
    return (len(left.prerelease) > len(right.prerelease)) - (
        len(left.prerelease) < len(right.prerelease)
    )


def _section_body(text: str, sections: list[re.Match[str]], index: int) -> str:
    end = sections[index + 1].start() if index + 1 < len(sections) else len(text)
    body = text[sections[index].end() : end]
    link = LINK_PATTERN.search(body)
    return body[: link.start()] if link is not None else body


def validate_changelog(product: Product) -> dict[str, Any]:
    """Validate one product changelog against the repository release contract."""

    text = product.changelog.read_text(encoding="utf-8")
    if not text.startswith("# Changelog\n"):
        raise RuntimeError(f"{product.name}: changelog title is not canonical")
    if KEEP_A_CHANGELOG_URL not in text or SEMVER_URL not in text:
        raise RuntimeError(f"{product.name}: changelog policy links are missing")
    sections = list(SECTION_PATTERN.finditer(text))
    if not sections or sections[0].group("label") != "Unreleased":
        raise RuntimeError(f"{product.name}: [Unreleased] must be the first release section")

    versions: list[str] = []
    parsed_versions: list[SemVer] = []
    for index, section in enumerate(sections):
        label = section.group("label")
        release_date = section.group("date")
        if label == "Unreleased":
            if release_date is not None:
                raise RuntimeError(f"{product.name}: [Unreleased] cannot have a date")
        else:
            if release_date is None:
                raise RuntimeError(f"{product.name}: release {label} has no ISO date")
            try:
                date.fromisoformat(release_date)
            except ValueError as error:
                message = f"{product.name}: release {label} has an invalid date"
                raise RuntimeError(message) from error
            versions.append(label)
            parsed_versions.append(parse_semver(label))

        body = _section_body(text, sections, index)
        categories = list(CATEGORY_PATTERN.finditer(body))
        if label != "Unreleased" and not categories:
            raise RuntimeError(f"{product.name}: release {label} has no change category")
        for category_index, category in enumerate(categories):
            name = category.group("category")
            if name not in ALLOWED_CATEGORIES:
                raise RuntimeError(f"{product.name}: unsupported change category {name}")
            category_end = (
                categories[category_index + 1].start()
                if category_index + 1 < len(categories)
                else len(body)
            )
            if re.search(r"^- ", body[category.end() : category_end], re.MULTILINE) is None:
                raise RuntimeError(f"{product.name}: empty change category {name}")

    if len(versions) != len(set(versions)):
        raise RuntimeError(f"{product.name}: duplicate release version")
    for newer, older in pairwise(parsed_versions):
        if compare_semver(newer, older) <= 0:
            raise RuntimeError(f"{product.name}: releases are not newest-first")
    if not versions or versions[0] != product.version:
        raise RuntimeError(
            f"{product.name}: current version {product.version} is not the latest release entry"
        )

    link_matches = list(LINK_PATTERN.finditer(text))
    links = {match.group("label"): match.group("url") for match in link_matches}
    if len(links) != len(link_matches):
        raise RuntimeError(f"{product.name}: duplicate changelog link")
    for label in ["Unreleased", *versions]:
        if label not in links:
            raise RuntimeError(f"{product.name}: missing changelog link for {label}")
    return {"version": product.version, "releases": len(versions)}


def products(root: Path) -> tuple[Product, Product]:
    """Load server and G7-module versions from their canonical manifests."""

    cargo = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    server_version = str(cargo["workspace"]["package"]["version"])
    module_root = root / "adapters" / "gnuboard7" / "jiwonpapa-g7mediabooster"
    module = json.loads((module_root / "module.json").read_text(encoding="utf-8"))
    package = json.loads((module_root / "package.json").read_text(encoding="utf-8"))
    features_path = root / "deploy" / "official-features-v1.json"
    features = json.loads(features_path.read_text(encoding="utf-8"))
    module_version = str(module.get("version", ""))
    related_versions = {
        module_version,
        str(package.get("version", "")),
        str(features["module"]["version"]),
    }
    if len(related_versions) != 1:
        raise RuntimeError("G7 module version drift across module, package, and feature manifests")
    parse_semver(server_version)
    parse_semver(module_version)
    return (
        Product("server", server_version, "server-v", root / "CHANGELOG.md"),
        Product("g7-module", module_version, "g7-module-v", module_root / "CHANGELOG.md"),
    )


def product_for_tag(root: Path, tag: str) -> Product:
    """Resolve and validate the product selected by an exact release tag."""

    for product in products(root):
        if tag.startswith(product.tag_prefix):
            version = tag.removeprefix(product.tag_prefix)
            parse_semver(version)
            if version != product.version:
                message = f"{tag}: tag version does not match {product.name} {product.version}"
                raise RuntimeError(message)
            return product
    raise RuntimeError(f"unsupported release tag: {tag}")


def validate_repository(root: Path, expected_tag: str | None = None) -> dict[str, object]:
    """Validate all independently versioned products and an optional release tag."""

    loaded = products(root)
    result = {product.name: validate_changelog(product) for product in loaded}
    features = feature_status.validate_repository(root)
    if expected_tag is not None:
        product_for_tag(root, expected_tag)
    return {"status": "PASS", "products": result, "features": features}


def release_notes(root: Path, tag: str) -> str:
    """Extract human-authored release notes from the matching changelog section."""

    product = product_for_tag(root, tag)
    validate_changelog(product)
    text = product.changelog.read_text(encoding="utf-8")
    sections = list(SECTION_PATTERN.finditer(text))
    for index, section in enumerate(sections):
        if section.group("label") == product.version:
            body = _section_body(text, sections, index).strip()
            if not body:
                break
            return f"## {product.name} {product.version}\n\n{body}\n"
    raise RuntimeError(f"{product.name}: no release notes for {product.version}")
