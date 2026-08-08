# Devcontainer port

The `.devcontainer` directory was adapted from the Java original for the Rust/Axum port.

## Services

- `app`: Rust development container with rustfmt, clippy, Maven/Liquibase, PostgreSQL client and optional cargo-watch.
- `postgres`: PostgreSQL 16; the canonical Java workflow creates database `lor`.
- `opensearch`: OpenSearch 3.6.0, matching the original devcontainer service shape.

## Initialization

`postCreateCommand` runs `.devcontainer/init-db.sh`:

1. waits for PostgreSQL;
2. classifies the target schema and fails closed for mixed/legacy/unknown state;
3. bootstraps a missing/empty disposable database from the vendored Java demo and Liquibase chain;
4. validates an existing Java database without updating it;
5. creates upload directories.

No SQLx migration is compiled or executed. The application connects as the
Java `linuxweb` runtime role.

## Usage

Open the project in VS Code / Dev Containers and run:

```bash
cargo fmt
cargo clippy
cargo run
```

The application listens on `http://localhost:8181`.
