"""Contract regressions for native resource-gate evidence."""

from __future__ import annotations

import unittest

from tools.harness.g7mb_harness.resource_gates import _expect_probe


class ResourceGateContractTest(unittest.TestCase):
    """Keep sandbox probe validation aligned with its nested JSON contract."""

    def test_nested_probe_contract_is_accepted(self) -> None:
        document: dict[str, object] = {
            "format": "jpeg",
            "probe": {"width": 25000, "height": 4000},
        }
        _expect_probe(document, "jpeg", 25000, 4000)

    def test_changed_dimensions_are_rejected(self) -> None:
        document: dict[str, object] = {
            "format": "jpeg",
            "probe": {"width": 1280, "height": 4000},
        }
        with self.assertRaisesRegex(RuntimeError, "probe contract changed"):
            _expect_probe(document, "jpeg", 25000, 4000)


if __name__ == "__main__":
    unittest.main()
