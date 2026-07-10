FROM rust:1.97-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
    postgresql-client \
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

# Tools used by the porting/compatibility workflow. Keep failure-tolerant so the
# devcontainer can still build if crates.io is temporarily unavailable.
RUN cargo install sqlx-cli --no-default-features --features postgres || true
RUN cargo install cargo-watch || true

EXPOSE 8181
