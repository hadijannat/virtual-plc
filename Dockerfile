# syntax=docker/dockerfile:1
# Multi-architecture Dockerfile for Virtual PLC
# Supports: linux/amd64, linux/arm64

# Build stage
FROM --platform=$BUILDPLATFORM rust:1.83-slim AS build

# Build arguments for cross-compilation
ARG TARGETPLATFORM
ARG BUILDPLATFORM
ARG TARGETARCH

WORKDIR /src

# Install cross-compilation tools for ARM64 when building on x86_64
RUN case "$TARGETARCH" in \
      arm64) \
        apt-get update && apt-get install -y --no-install-recommends \
          gcc-aarch64-linux-gnu \
          libc6-dev-arm64-cross \
        && rm -rf /var/lib/apt/lists/* \
        && rustup target add aarch64-unknown-linux-gnu \
        ;; \
    esac

# Set up cross-compilation environment
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc

# Copy source
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build for target architecture
RUN case "$TARGETARCH" in \
      amd64) \
        cargo build -p plc-daemon --release \
        && cp target/release/plc-daemon /plc-daemon \
        ;; \
      arm64) \
        cargo build -p plc-daemon --release --target aarch64-unknown-linux-gnu \
        && cp target/aarch64-unknown-linux-gnu/release/plc-daemon /plc-daemon \
        ;; \
    esac

# Runtime stage
FROM debian:bookworm-slim

# Labels for container registry
LABEL org.opencontainers.image.source="https://github.com/hadijannat/virtual-plc"
LABEL org.opencontainers.image.description="Virtual PLC - Production-grade soft PLC runtime"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"

# Create non-root user for PLC runtime
RUN groupadd -r plc && useradd -r -g plc plc

# Install minimal runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary from build stage
COPY --from=build /plc-daemon /usr/local/bin/plc-daemon
RUN chmod +x /usr/local/bin/plc-daemon

# Create directories for config and programs
RUN mkdir -p /etc/vplc /var/lib/vplc && \
    chown -R plc:plc /etc/vplc /var/lib/vplc

# Switch to non-root user
USER plc

# Default configuration directory
ENV VPLC_CONFIG_DIR=/etc/vplc
ENV VPLC_DATA_DIR=/var/lib/vplc

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD plc-daemon --help > /dev/null || exit 1

ENTRYPOINT ["plc-daemon"]
