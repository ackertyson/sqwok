FROM rust:1.92 AS builder
WORKDIR /usr/src/sqwok
RUN apt-get update && apt-get install -y libssl-dev pkg-config && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo install --path .


FROM debian:trixie-slim
RUN apt-get update && apt-get install -y fish && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/sqwok /usr/local/bin/sqwok

# TODO add non-root user

CMD ["sqwok"]
