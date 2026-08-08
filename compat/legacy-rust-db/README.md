# Offline legacy Rust SQL reference

`offline-sql/` contains the superseded Rust/SQLx development migrations. They
are retained only for forensic review of earlier port behavior.

They are deliberately outside `db/migrations`, are not compiled with
`sqlx::migrate!`, and must not be executed. Several files reference columns
removed by current Java Liquibase migrations, invent parallel schemas, seed
production-like data, or otherwise diverge from the canonical Java database.

The active fresh-database workflow is `compat/java-db/manage.sh bootstrap`.

