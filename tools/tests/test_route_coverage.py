from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).parents[1] / "route_coverage.py"
SPEC = importlib.util.spec_from_file_location("route_coverage", MODULE_PATH)
assert SPEC and SPEC.loader
coverage = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(coverage)


class RouteCoverageTest(unittest.TestCase):
    def test_unrestricted_spring_mapping_is_not_full_get_parity(self) -> None:
        self.assertEqual("partial", coverage.method_relation(["ANY"], ["GET"]))
        self.assertEqual("full", coverage.method_relation(["ANY"], ["ANY"]))

    def test_all_original_explicit_methods_must_be_declared(self) -> None:
        self.assertEqual("full", coverage.method_relation(["GET"], ["GET", "POST"]))
        self.assertEqual("partial", coverage.method_relation(["GET", "HEAD"], ["GET"]))
        self.assertEqual("none", coverage.method_relation(["POST"], ["GET"]))


if __name__ == "__main__":
    unittest.main()
