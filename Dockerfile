# Build stage
FROM rust:1.75 as builder

WORKDIR /app

# Copy everything and build
COPY . .
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/immutable-bot /app/bot

RUN mkdir -p /data

VOLUME ["/data"]

CMD ["/app/bot"]