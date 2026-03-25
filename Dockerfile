FROM rust:1.92 AS builder
WORKDIR /usr/src/sqwok
RUN apt-get update && apt-get install -y libssl-dev pkg-config && rm -rf /var/lib/apt/lists/*

# fetch first so Docker can cache the layer
COPY Cargo.toml Cargo.lock ./
RUN cargo fetch

COPY src src
COPY assets assets
RUN cargo install --path .


#---
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y fish && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/sqwok /usr/local/bin/sqwok

# Create a non-root user/group for running the application.
RUN groupadd -r sqwok && useradd -r -g sqwok sqwok

# Pre-create volume directories and set ownership before the volume mounts replace them.
RUN mkdir -p /home/sqwok/.sqwok /home/sqwok/.local/share/sqwok \
    && chown -R sqwok:sqwok /home/sqwok

USER sqwok

ENTRYPOINT ["sqwok"]
