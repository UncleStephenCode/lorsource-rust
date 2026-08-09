FROM rust:1.97.1-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
    postgresql-client \
    maven \
    gzip \
    ripgrep \
    git \
    curl \
    ca-certificates \
    pkg-config \
    libssl-dev \
    build-essential \
    imagemagick \
    file \
    && rm -rf /var/lib/apt/lists/*

RUN rustup component add rustfmt clippy

# Tool used by the development reload loop. Keep failure-tolerant so the
# devcontainer can still build if crates.io is temporarily unavailable.
RUN cargo install cargo-watch || true

EXPOSE 8181
