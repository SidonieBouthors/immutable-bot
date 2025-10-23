# Build stage
FROM rust:1.90 as builder

WORKDIR /app

# Copy everything and build
COPY . .
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/immutable-bot /app/bot

RUN mkdir -p /app/data

VOLUME ["/app/data"]

CMD ["/app/bot"]