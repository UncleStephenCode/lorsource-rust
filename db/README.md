# No application-owned migrations

This directory intentionally contains no active SQL migrations. Database
schema history belongs to the original Java Liquibase chain vendored at
`compat/java-db/sql/`.

Do not recreate `db/migrations` or add `sqlx::migrate!`. Application startup is
validate-only and must never mutate an existing Java database.

