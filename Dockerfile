# =============================================================================
# Sockudo WebSocket Server - Multi-stage Docker Build
# =============================================================================

# -----------------------------------------------------------------------------
# Build Stage: Compile the Rust application
# -----------------------------------------------------------------------------
FROM rust:latest AS builder

# Install nightly toolchain
RUN rustup toolchain install nightly
RUN rustup default nightly

# Install system dependencies required for building
RUN apt-get update && apt-get install -y \
    pkg-config \
    libpq-dev \
    libmariadb-dev \
    cmake \
    && rm -rf /var/lib/apt/lists/*

# Create app user and directory
RUN groupadd -r sockudo && useradd -r -g sockudo sockudo

# Set the working directory
WORKDIR /app

# Copy dependency files first for better layer caching
COPY Cargo.toml ./
# Copy Cargo.lock if it exists, otherwise create empty one
COPY Cargo.loc[k] ./
RUN test -f Cargo.lock || cargo generate-lockfile

# Create a dummy main.rs to build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies (this layer will be cached unless Cargo files change)
RUN cargo build --release && rm src/main.rs target/release/deps/sockudo*

# Copy source code
COPY src/ ./src/

# Build the application
RUN cargo build --release

# Strip the binary to reduce size (optional)
RUN strip target/release/sockudo

# -----------------------------------------------------------------------------
# Runtime Stage: Create minimal runtime image
# -----------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libpq5 \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create app user and directories
RUN groupadd -r sockudo && useradd -r -g sockudo sockudo \
    && mkdir -p /app/config /app/logs /app/ssl \
    && chown -R sockudo:sockudo /app

# Set working directory
WORKDIR /app

# Copy the compiled binary from builder stage
COPY --from=builder /app/target/release/sockudo /usr/local/bin/sockudo
RUN chmod +x /usr/local/bin/sockudo

# Copy configuration files (if they exist)
COPY --chown=sockudo:sockudo config/ ./config/

# Create default configuration if it doesn't exist
RUN if [ ! -f ./config/config.json ]; then \
    echo '{}' > ./config/config.json && \
    chown sockudo:sockudo ./config/config.json; \
    fi

# Switch to non-root user
USER sockudo

# Set environment variables with defaults
ENV RUST_LOG=info
ENV CONFIG_FILE=/app/config/config.json
ENV HOST=0.0.0.0
ENV PORT=6001
ENV METRICS_PORT=9601
# Path for health check, you can change this to match your app's health endpoint e.g. "up/my-app-id"
# Defaults to "up" which is a common health check endpoint
ENV HEALTHCHECK_PATH=up

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=30s --retries=3 \
    CMD curl -f http://localhost:${PORT}/${HEALTHCHECK_PATH} || exit 1

# Expose ports
EXPOSE 6001 9601

# Default command
CMD ["sockudo", "--config", "/app/config/config.json"]