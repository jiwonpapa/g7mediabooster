"""Canonical feature-state registry regression tests."""

from __future__ import annotations

import copy
import unittest
from pathlib import Path
from typing import cast
from unittest import mock

from tools.harness.g7mb_harness import feature_status


class FeatureStatusTest(unittest.TestCase):
    """Keep release claims aligned with manifests and checked evidence."""

    def setUp(self) -> None:
        self.root = Path(__file__).parents[3]

    def test_repository_feature_status_passes(self) -> None:
        result = feature_status.validate_repository(self.root)
        self.assertEqual(result["status"], "PASS")
        feature_count = cast(int, result["features"])
        self.assertIsInstance(feature_count, int)
        self.assertGreaterEqual(feature_count, 20)

    def test_publishable_cannot_exceed_verification_state(self) -> None:
        load_json = feature_status._load_json
        registry_path = self.root / "deploy" / "official-features-v1.json"

        def changed_registry(path: Path) -> dict[str, object]:
            data = load_json(path)
            if path == registry_path:
                data = copy.deepcopy(data)
                feature = next(item for item in data["features"] if item["id"] == "aws_s3_profile")
                feature["publishable"] = True
            return data

        with (
            mock.patch.object(feature_status, "_load_json", side_effect=changed_registry),
            self.assertRaisesRegex(RuntimeError, "publishable conflicts"),
        ):
            feature_status.validate_repository(self.root)

    def test_verified_evidence_must_exist(self) -> None:
        load_json = feature_status._load_json
        registry_path = self.root / "deploy" / "official-features-v1.json"

        def missing_evidence(path: Path) -> dict[str, object]:
            data = load_json(path)
            if path == registry_path:
                data = copy.deepcopy(data)
                feature = next(
                    item for item in data["features"] if item["id"] == "cloudflare_r2_profile"
                )
                feature["evidence"] = ["docs/evidence/DOES_NOT_EXIST.md"]
            return data

        with (
            mock.patch.object(feature_status, "_load_json", side_effect=missing_evidence),
            self.assertRaisesRegex(RuntimeError, "missing or unsafe evidence"),
        ):
            feature_status.validate_repository(self.root)


if __name__ == "__main__":
    unittest.main()
