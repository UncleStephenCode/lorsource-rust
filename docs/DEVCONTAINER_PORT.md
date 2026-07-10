# Devcontainer port

The `.devcontainer` directory was adapted from the Java original for the Rust/Axum port.

## Services

- `app`: Rust development container with rustfmt, clippy, PostgreSQL client and optional SQLx CLI/cargo-watch.
- `postgres`: PostgreSQL 16 with database `lor`.
- `opensearch`: OpenSearch 3.6.0, matching the original devcontainer service shape.

## Initialization

`postCreateCommand` runs `.devcontainer/init-db.sh`:

1. waits for PostgreSQL;
2. creates `hstore` and `fuzzystrmatch` extensions;
3. runs `sqlx migrate run --source /workspace/db/migrations` when SQLx CLI exists;
4. falls back to applying SQL files through `psql`;
5. creates upload directories.

## Usage

Open the project in VS Code / Dev Containers and run:

```bash
cargo fmt
cargo clippy
cargo run
```

The application listens on `http://localhost:8181`.
