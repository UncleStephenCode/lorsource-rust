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

The audit-time diagnostic dump (not vendored) was produced with PostgreSQL
17.10 using `pg_dump --schema-only --no-owner --no-privileges` and had SHA-256
`66495e69b0d9442861f4905f08546e06f7633072b61686ac73f960681803c326`.
It is intentionally not a second executable bootstrap authority.
