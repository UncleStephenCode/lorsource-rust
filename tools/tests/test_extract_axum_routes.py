from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).parents[1] / "extract_axum_routes.py"
SPEC = importlib.util.spec_from_file_location("extract_axum_routes", MODULE_PATH)
assert SPEC and SPEC.loader
extractor = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(extractor)


class ExtractAxumRoutesTest(unittest.TestCase):
    def test_resolves_named_method_router_in_its_module(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            routes = root / "src" / "routes"
            routes.mkdir(parents=True)
            (routes / "mod.rs").write_text(
                '.route("/commit.jsp", topics::stCommitRoute())\n'
                '.route("/inline", get(inline).post(inline))\n',
                encoding="utf-8",
            )
            (routes / "topics.rs").write_text(
                """
                pub fn stCommitRoute() -> MethodRouter<AppState> {
                    get(show).options(options).fallback(not_allowed)
                }
                """,
                encoding="utf-8",
            )

            found = {row["path"]: row["methods"] for row in extractor.extract_routes(root)}
            self.assertEqual(["GET"], found["/commit.jsp"])
            self.assertEqual(["GET", "POST"], found["/inline"])

    def test_unresolved_builder_remains_conservatively_any(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            routes = root / "src" / "routes"
            routes.mkdir(parents=True)
            (routes / "mod.rs").write_text(
                '.route("/external", plugin::router())\n', encoding="utf-8"
            )

            found = extractor.extract_routes(root)
            self.assertEqual(["ANY"], found[0]["methods"])


if __name__ == "__main__":
    unittest.main()
