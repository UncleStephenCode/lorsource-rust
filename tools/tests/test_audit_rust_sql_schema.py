from __future__ import annotations

import gzip
import sys
import tempfile
import unittest
from pathlib import Path


TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS))

import audit_rust_sql_schema as audit  # noqa: E402


CONTRACT = """\
comments\tid\tint4\tNO
comments\ttopic\tint4\tNO
msgbase\tid\tint4\tNO
msgbase\tmessage\ttext\tNO
msgbase\tmarkup\tmarkup_type\tNO
topics\tid\tint4\tNO
topics\ttitle\tvarchar\tNO
topics\tstat1\tint4\tNO
user_events\tid\tint4\tNO
user_events\ttype\tevent_type\tNO
users\tid\tint4\tNO
users\tnick\tvarchar\tNO
"""


class RustSqlSchemaAuditTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "src").mkdir()
        self.contract = self.root / "schema-contract.tsv"
        self.contract.write_text(CONTRACT, encoding="utf-8")
        self.java_sql = self.root / "java-sql"
        (self.java_sql / "updates").mkdir(parents=True)
        with gzip.open(self.java_sql / "demo.db.gz", "wt", encoding="utf-8") as stream:
            stream.write("CREATE TYPE event_type AS ENUM ('WATCH', 'REPLY');\n")
        (self.java_sql / "updates/enums.xml").write_text(
            """<databaseChangeLog><changeSet><sql>
            CREATE TYPE markup_type AS ENUM ('PLAIN', 'MARKDOWN');
            ALTER TYPE event_type ADD VALUE IF NOT EXISTS 'WARNING';
            </sql></changeSet></databaseChangeLog>""",
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temp.cleanup()

    def write_rust(self, text: str) -> None:
        (self.root / "src/lib.rs").write_text(text, encoding="utf-8")

    def report(self) -> dict[str, object]:
        return audit.run_audit(self.root, self.contract, self.java_sql)

    def test_alias_insert_update_table_and_enum_violations(self) -> None:
        self.write_rust(
            r'''
fn alias_failures() {
    sqlx::query(r#"SELECT m.message, m.bbcode, t.stat2, t.title
                       FROM msgbase m JOIN topics t ON t.id=m.id"#);
    sqlx::query("INSERT INTO users(id, force_unlogin) VALUES($1,$2)");
    sqlx::query("UPDATE msgbase SET bbcode=true, markup='BROKEN' WHERE id=$1");
    sqlx::query("SELECT id FROM vanished_table");
    sqlx::query("SELECT stat2 FROM topics");
    sqlx::query("SELECT 'NOPE'::event_type FROM topics");
    sqlx::query("SELECT id FROM user_events WHERE type='INVALID'");
    sqlx::query("SELECT 'x'::gone_enum FROM topics");
    sqlx::query("INSERT INTO msgbase(id,markup) VALUES($1,'UNKNOWN')");
}
'''
        )
        report = self.report()
        keys = {(row["kind"], row["identifier"]) for row in report["findings"]}
        self.assertIn(("missing_column", "msgbase.bbcode"), keys)
        self.assertIn(("missing_column", "topics.stat2"), keys)
        self.assertIn(("missing_column", "users.force_unlogin"), keys)
        self.assertIn(("missing_table", "vanished_table"), keys)
        self.assertIn(("missing_unqualified_column", "stat2"), keys)
        self.assertIn(("missing_enum_label", "event_type.NOPE"), keys)
        self.assertIn(("missing_enum_label", "event_type.INVALID"), keys)
        self.assertIn(("missing_enum_type", "gone_enum"), keys)
        self.assertIn(("missing_enum_label", "markup_type.BROKEN"), keys)
        self.assertIn(("missing_enum_label", "markup_type.UNKNOWN"), keys)

    def test_valid_cte_subquery_catalog_and_enum_sql_do_not_false_positive(self) -> None:
        self.write_rust(
            r'''
fn valid_queries() {
    sqlx::query("SELECT t.id,t.title FROM topics AS t WHERE t.stat1 >= 0");
    sqlx::query("WITH visible AS (SELECT id,title FROM topics) SELECT visible.title FROM visible");
    sqlx::query("SELECT c.relname FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace");
    sqlx::query("UPDATE msgbase SET markup='MARKDOWN' WHERE id=$1");
    sqlx::query("INSERT INTO msgbase(id,markup) VALUES($1,'PLAIN')");
    sqlx::query("SELECT id FROM user_events WHERE type='WATCH'");
}
'''
        )
        report = self.report()
        self.assertEqual([], report["findings"])
        self.assertEqual(6, report["summary"]["clean_queries"])

    def test_legacy_catalog_tuples_are_probes_not_column_references(self) -> None:
        self.write_rust(
            r'''
fn fingerprint() {
    sqlx::query(r#"SELECT EXISTS (
        SELECT 1 FROM pg_catalog.pg_attribute a
        JOIN pg_catalog.pg_class c ON c.oid=a.attrelid
        WHERE (c.relname,a.attname) IN (
          ('users','force_unlogin'), ('topics','stat2'), ('topics','title')
        )
      ) AS has_legacy_rust_columns"#);
}
'''
        )
        report = self.report()
        self.assertEqual([], report["findings"])
        self.assertEqual(
            ["topics.stat2", "users.force_unlogin"],
            [row["identifier"] for row in report["intentional_absence_probes"]],
        )

    def test_sql_fragments_use_unique_file_alias_evidence(self) -> None:
        self.write_rust(
            r'''
const BASE: &str = "SELECT t.id FROM topics t";
fn condition() {
    let valid = "AND t.stat1 > 0";
    let invalid = "AND t.stat2 > 0";
    let bare = "t.stat1";
    let human_error = "offset too big";
}
'''
        )
        report = self.report()
        self.assertEqual(4, report["summary"]["sql_literals"])
        self.assertEqual(3, report["summary"]["sql_fragments"])
        self.assertEqual(
            [("missing_column", "topics.stat2")],
            [(row["kind"], row["identifier"]) for row in report["findings"]],
        )

    def test_java_enum_contract_combines_dump_and_liquibase(self) -> None:
        enums = audit.read_java_enums(self.java_sql)
        self.assertEqual({"WATCH", "REPLY", "WARNING"}, enums["event_type"])
        self.assertEqual({"PLAIN", "MARKDOWN"}, enums["markup_type"])

    def test_rust_lexer_ignores_commented_sql(self) -> None:
        self.write_rust(
            r'''
// sqlx::query("SELECT force_unlogin FROM users");
/* sqlx::query(r#"SELECT stat2 FROM topics"#); */
fn live() { sqlx::query("SELECT id FROM users"); }
'''
        )
        strings = audit.extract_sql_strings(self.root)
        self.assertEqual(["SELECT id FROM users"], [item.value for item in strings])


if __name__ == "__main__":
    unittest.main()
