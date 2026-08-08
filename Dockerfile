FROM rust:1.97-slim-bookworm AS build
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY templates ./templates
COPY compat/java-db/schema-contract.tsv ./compat/java-db/schema-contract.tsv
COPY static ./static
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release --locked && \
    cp /app/target/release/lorsource-rust /app/lorsource-rust

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl gosu && \
    rm -rf /var/lib/apt/lists/* && \
    groupadd --system --gid 8181 lorsource && \
    useradd --system --uid 8181 --gid lorsource --home-dir /app --shell /usr/sbin/nologin lorsource && \
    mkdir -p /app/uploads/photos /app/uploads/gallery /app/uploads/images && \
    chown -R lorsource:lorsource /app/uploads
COPY --from=build /app/lorsource-rust /usr/local/bin/lorsource-rust
COPY static ./static
COPY templates ./templates
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod 0755 /usr/local/bin/docker-entrypoint.sh
ENV LOR_HOST=0.0.0.0 LOR_PORT=8181 STATIC_DIR=/app/static UPLOAD_DIR=/app/uploads
EXPOSE 8181
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl --fail --silent --show-error http://127.0.0.1:8181/healthz >/dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["/usr/local/bin/lorsource-rust"]
