# Canonical Java database bootstrap provenance

This directory vendors the database bootstrap inputs from the sibling
`lorsource-java` repository at commit:

```text
2ddf930005adac28077cb6ad74d1481485f44096
```

Vendored verbatim on 2026-08-08:

- `sql/demo.db.gz` (a gzip copy of Java `sql/demo.db`);
- `sql/main.xml`;
- every file under `sql/updates/`, including `.empty`.

The original relative paths are intentionally preserved. Liquibase records
those paths as logical filenames in `databasechangelog`; renaming them would
break checksum/history compatibility with an existing Java database.

Integrity facts:

- 116 update XML files;
- 187 Liquibase changesets;
- uncompressed `demo.db` SHA-256:
  `9871b6148f59786da7cd00e8601931869129ec1968035cb8a6c2a1e3592a038e`;
- terminal changeset: `2026080501`, author `Maxim Valyanskiy`, logical file
  `sql/updates/2026-08-05-userlog-userpic-idx.xml`;
- terminal Liquibase 4.17.2 checksum:
  `8:d52bfe13718eea6a248d7c3abc488f2d`.

`checksums.sha256` covers only the verbatim Java inputs under `sql/`.
`check-vendor.sh` also verifies the counts, terminal identity and uncompressed
demo hash. `schema-contract.tsv` is derived from a PostgreSQL 17 schema-only
dump after applying this bootstrap; it is a Rust runtime compatibility
contract, not a replacement migration source.

`liquibase-changesets.tsv` was exported from a fresh local application of the
same canonical bootstrap using the Java-pinned Liquibase 4.17.2. Its 187 rows
record execution ordinal, ID, author, original logical path, version-8
checksum and execution state. The ordinal/ID/author/path projection is also
regenerated directly from the 116 sorted vendored XML files by
`check-vendor.sh`; its full SHA-256 is
`af0e920c0fe922f87d41957583ace19ff7db669a75a7f7d1f1d292c7cee1a644`.
This is local canonical-bootstrap provenance, not a production-ledger export.

`schema-objects-contract.tsv` complements the column contract with the
catalog objects that a column-only inventory cannot prove. It was generated
on 2026-08-15 from a fresh application of the same vendored bootstrap using
PostgreSQL 16.14, through the read-only, bounded query in
`export-schema-objects.sql`, then verified through the `linuxweb` runtime role.
It contains 728 sorted records:

- 82 primary/foreign/unique constraints (and any canonical checks; currently
  none);
- 61 column defaults, including every canonical `nextval(...)` binding;
- 101 index definitions;
- 15 sequence definitions and `OWNED BY` links;
- 12 application function definitions and five trigger definitions;
- 168 effective `linuxweb` relation grants, 12 effective runtime function
  `EXECUTE` grants and six effective runtime schema/enum `USAGE` checks;
- 60 advisory direct relation/function ACL provenance records;
- 33 relation records covering relation kind/persistence,
  row-level-security/forced-row-level-security flags and access method;
- one schema and five enum-type semantic records;
- 167 table/index/sequence/function/schema/type owner records.

The contract SHA-256 is
`eaed5aacda3724e56f4508a98ebc98e45a48fec6acba3f9e35a342d72d9e84f0`;
the exporter SHA-256 is
`930539ad7662d66ff8037a979f0e2a89f560bb055573b81b40065a53d85ff3d7`.
`check-vendor.sh` pins both. To prove reproducibility against a freshly
bootstrapped disposable database, run:

```bash
JAVA_DATABASE_RUNTIME_URL=postgres://linuxweb:linuxweb@localhost:5432/lor \
  bash compat/java-db/check-schema-object-contract.sh
```

This exact comparison is a regeneration/evidence check. Application startup
uses the canonical records as a required subset. Additional constraints and
enabled triggers on canonical tables are rejected because they can change
write behavior; additional operator indexes, grants, direct ACL provenance
and owners are reported as fingerprint drift instead. A production clone and
other PostgreSQL major versions have not been validated by this local
derivation and remain cutover evidence requirements.

The read-only sequence collision/bounds query is pinned at SHA-256
`c156537595af3e703e975fec83ae6494fa8200bacf56e8c90628c66756967c31`.
It covers the nine canonical sequences with `OWNED BY` dependencies and four
unowned generators whose table/column mappings are proved by the current Java
DAOs. It also rejects non-canonical increments/cycling and exhausted next
values outside the configured bounds. It deliberately excludes the unowned,
unused `s_guid` and `s_msg` generators because neither has a canonical
application ID column to compare.

The audit-time diagnostic dump (not vendored) was produced with PostgreSQL
17.10 using `pg_dump --schema-only --no-owner --no-privileges` and had SHA-256
`66495e69b0d9442861f4905f08546e06f7633072b61686ac73f960681803c326`.
It is intentionally not a second executable bootstrap authority.
