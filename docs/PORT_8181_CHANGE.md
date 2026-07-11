# Default application port changed to 8181

This change sets the Rust port default web listener to `8181` across Docker and compose entry points.

Updated surfaces:

- `Dockerfile`: `LOR_PORT=8181`, `EXPOSE 8181`.
- `docker-compose.yml`: app container listens on `8181`, host mapping is `8181:8181`, `PUBLIC_URL=http://localhost:8181`.
- `.devcontainer/docker-compose.yml`: forwarded port is `127.0.0.1:8181:8181`, `LOR_PORT=8181`, `PUBLIC_URL=http://localhost:8181`.
- `.env.example`: `LOR_PORT=8181`, `PUBLIC_URL=http://localhost:8181`.
- `src/config.rs`: fallback `LOR_PORT` is now `8181`, fallback `PUBLIC_URL` is now `http://localhost:8181`.
- README, compatibility scripts and docs now use `NEW_BASE_URL=http://localhost:8181` for the Rust application.

PostgreSQL and OpenSearch ports are unchanged.
