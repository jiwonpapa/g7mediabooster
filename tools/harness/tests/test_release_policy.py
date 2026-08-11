"""Keep a Changelog and Semantic Versioning policy regression tests."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from tools.harness.g7mb_harness import release_policy


def changelog(*, current: str = "1.2.0", older: str = "1.1.0") -> str:
    """Build a minimal valid changelog fixture."""

    return f"""# Changelog

Keep a Changelog: {release_policy.KEEP_A_CHANGELOG_URL}
Semantic Versioning: {release_policy.SEMVER_URL}

## [Unreleased]

### Added

- Pending capability.

## [{current}] - 2026-08-11

### Changed

- Current behavior.

## [{older}] - 2026-08-10

### Added

- Initial behavior.

[Unreleased]: https://example.invalid/compare/server-v{current}...HEAD
[{current}]: https://example.invalid/compare/server-v{older}...server-v{current}
[{older}]: https://example.invalid/releases/tag/server-v{older}
"""


class ReleasePolicyTest(unittest.TestCase):
    """Reject release metadata that would make versions or notes ambiguous."""

    def test_repository_release_policy_passes(self) -> None:
        result = release_policy.validate_repository(Path(__file__).parents[3])
        self.assertEqual(result["status"], "PASS")

    def test_semver_precedence_and_leading_zero_rules(self) -> None:
        alpha = release_policy.parse_semver("1.0.0-alpha.1")
        stable = release_policy.parse_semver("1.0.0")
        self.assertLess(release_policy.compare_semver(alpha, stable), 0)
        with self.assertRaisesRegex(RuntimeError, "invalid Semantic Version"):
            release_policy.parse_semver("01.0.0")

    def test_changelog_rejects_wrong_order_and_unknown_category(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "CHANGELOG.md"
            path.write_text(
                changelog(current="1.1.0", older="1.2.0").replace("### Changed", "### Updates"),
                encoding="utf-8",
            )
            product = release_policy.Product("server", "1.1.0", "server-v", path)
            with self.assertRaisesRegex(RuntimeError, "unsupported change category"):
                release_policy.validate_changelog(product)
            path.write_text(changelog(current="1.1.0", older="1.2.0"), encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "newest-first"):
                release_policy.validate_changelog(product)

    def test_release_notes_come_from_human_changelog(self) -> None:
        root = Path(__file__).parents[3]
        notes = release_policy.release_notes(root, "server-v0.1.1")
        self.assertIn("server 0.1.1", notes)
        self.assertIn("### Fixed", notes)

    def test_release_tag_must_match_the_current_product_version(self) -> None:
        root = Path(__file__).parents[3]
        with self.assertRaisesRegex(RuntimeError, "does not match"):
            release_policy.product_for_tag(root, "server-v0.1.2")


if __name__ == "__main__":
    unittest.main()
