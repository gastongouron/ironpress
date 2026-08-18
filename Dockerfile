# syntax=docker/dockerfile:1

# ---- build stage ----
FROM rust:1.88-bookworm AS builder
WORKDIR /app

# Copy the whole workspace (the server depends on the ironpress library crate)
# and build the server binary in release mode.
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release -p ironpress-server && \
    cp target/release/ironpress-server /usr/local/bin/ironpress-server

# ---- runtime stage ----
FROM debian:bookworm-slim
# ca-certificates is only needed when IRONPRESS_REMOTE_ENABLED=true (TLS trust
# for outbound https). It is tiny and harmless when remote fetching is off.
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Run as a non-root system user.
RUN useradd --system --uid 10001 --user-group ironpress
COPY --from=builder /usr/local/bin/ironpress-server /usr/local/bin/ironpress-server
USER ironpress

# Operator-controlled defaults (override at `docker run` with -e).
ENV IRONPRESS_PORT=3000 \
    IRONPRESS_SANITIZE=true \
    IRONPRESS_REMOTE_ENABLED=false
EXPOSE 3000

# Self-contained health check (no curl/wget needed).
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD ["/usr/local/bin/ironpress-server", "--health"]

ENTRYPOINT ["/usr/local/bin/ironpress-server"]
