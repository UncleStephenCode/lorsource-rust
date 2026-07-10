FROM rust:1.82-slim-bookworm AS build
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY templates ./templates
COPY db ./db
RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/target/release/lorsource-rust /usr/local/bin/lorsource-rust
COPY static ./static
COPY templates ./templates
COPY db ./db
ENV LOR_HOST=0.0.0.0 LOR_PORT=8080 STATIC_DIR=/app/static UPLOAD_DIR=/app/uploads RUN_MIGRATIONS=true
EXPOSE 8080
CMD ["/usr/local/bin/lorsource-rust"]
