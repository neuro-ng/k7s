# k7s — Multi-stage Docker build — Phase 14.5
#
# Stage 1: builder  — full Rust toolchain, compiles a statically-linked binary
# Stage 2: runtime  — minimal distroless image, binary only (~5 MB total)
#
# Build:
#   docker build -t k7s:latest .
#
# Run (mount kubeconfig):
#   docker run --rm -it \
#     -v "$HOME/.kube:/root/.kube:ro" \
#     -v "$HOME/.config/k7s:/root/.config/k7s:ro" \
#     k7s:latest

# ── Stage 1: builder ──────────────────────────────────────────────────────────
FROM rust:1.77-slim-bookworm AS builder

WORKDIR /build

# System dependencies needed for some crates (openssl, etc.)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies by copying manifests first.
# Docker will cache this layer as long as Cargo.toml/Cargo.lock don't change.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./

# Create a dummy lib and main so `cargo build --release` downloads & compiles deps.
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs && \
    echo "" > src/lib.rs && \
    cargo build --release --bin k7s && \
    rm -rf src

# Now copy the real source and rebuild (only k7s code is recompiled).
COPY src ./src

# Touch main.rs to force a rebuild even if mtime is unchanged inside Docker.
RUN touch src/main.rs && \
    cargo build --release --bin k7s

# ── Stage 2: runtime ──────────────────────────────────────────────────────────
# gcr.io/distroless/cc-debian12 provides the C runtime but no shell or package
# manager — minimal attack surface.
FROM gcr.io/distroless/cc-debian12:latest AS runtime

# Copy the compiled binary.
COPY --from=builder /build/target/release/k7s /usr/local/bin/k7s

# k7s reads kubeconfig and config from standard XDG paths.
# Mount these via docker run -v flags at runtime.
ENV HOME=/root

ENTRYPOINT ["/usr/local/bin/k7s"]
