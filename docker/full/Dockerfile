# docker/full/Dockerfile — Phase 38: Batteries-Included Container Image
#
# Multi-stage build: builder → tools → runtime
#
#   builder   rust:slim  Compiles k7s release binary
#   tools     debian:bookworm-slim  Downloads + SHA-256 verifies kubectl, helm, vela, trivy
#   runtime   debian:bookworm-slim  Assembles final image; installs gcloud via apt; non-root user
#
# Build (single arch):
#   docker build -t ghcr.io/neuro-ng/k7s:full -f docker/full/Dockerfile .
#
# Build (multi-arch via buildx):
#   docker buildx build --platform linux/amd64,linux/arm64 \
#     -t ghcr.io/neuro-ng/k7s:full -f docker/full/Dockerfile --push .
#
# Run:
#   docker run --rm -it \
#     -v "$HOME/.kube:/home/k7s/.kube:ro" \
#     -v "$HOME/.config/k7s:/home/k7s/.config/k7s" \
#     -v "$HOME/.local/state/k7s:/home/k7s/.local/state/k7s" \
#     ghcr.io/neuro-ng/k7s:full

# ── Stage 1: builder ──────────────────────────────────────────────────────────
FROM rust:slim-bookworm AS builder

WORKDIR /build

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies by copying manifests first.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
RUN mkdir -p src benches && \
    echo "fn main() {}" > src/main.rs && \
    echo "" > src/lib.rs && \
    echo "" > benches/sanitizer.rs && \
    cargo build --release --bin k7s && \
    rm -rf src benches

# Compile the real binary (only k7s code is recompiled).
COPY src ./src
COPY benches ./benches
RUN touch src/main.rs && cargo build --release --bin k7s

# ── Stage 2: tools ────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS tools

ARG TARGETARCH
ARG KUBECTL_VERSION=v1.30.2
ARG HELM_VERSION=v3.15.2
ARG VELA_VERSION=v1.9.7
ARG TRIVY_VERSION=v0.52.0

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    tar \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /tools

# ── kubectl ──────────────────────────────────────────────────────────────────
RUN set -eux; \
    case "${TARGETARCH}" in \
        amd64) ARCH=amd64 ;; \
        arm64) ARCH=arm64 ;; \
        *) echo "unsupported arch: ${TARGETARCH}" && exit 1 ;; \
    esac; \
    curl -fsSL \
        "https://dl.k8s.io/release/${KUBECTL_VERSION}/bin/linux/${ARCH}/kubectl" \
        -o kubectl; \
    curl -fsSL \
        "https://dl.k8s.io/release/${KUBECTL_VERSION}/bin/linux/${ARCH}/kubectl.sha256" \
        -o kubectl.sha256; \
    echo "$(cat kubectl.sha256)  kubectl" | sha256sum --check; \
    chmod 0755 kubectl; \
    mv kubectl /usr/local/bin/kubectl

# ── helm ─────────────────────────────────────────────────────────────────────
RUN set -eux; \
    case "${TARGETARCH}" in \
        amd64) ARCH=amd64 ;; \
        arm64) ARCH=arm64 ;; \
        *) echo "unsupported arch: ${TARGETARCH}" && exit 1 ;; \
    esac; \
    HELM_TAR="helm-${HELM_VERSION}-linux-${ARCH}.tar.gz"; \
    curl -fsSL "https://get.helm.sh/${HELM_TAR}" -o helm.tar.gz; \
    curl -fsSL "https://get.helm.sh/${HELM_TAR}.sha256sum" -o helm.sha256sum; \
    sed -i "s|${HELM_TAR}|helm.tar.gz|g" helm.sha256sum; \
    sha256sum --check helm.sha256sum; \
    tar xzf helm.tar.gz --strip-components=1 "linux-${ARCH}/helm"; \
    chmod 0755 helm; \
    mv helm /usr/local/bin/helm; \
    rm helm.tar.gz helm.sha256sum

# ── vela (KubeVela) ──────────────────────────────────────────────────────────
# Releases use filename pattern: vela-v<VERSION>-linux-<ARCH>.tar.gz
# Checksums are in a single sha256sums.txt file per release.
# Tarball layout: linux-<ARCH>/vela (strip one component on extract).
RUN set -eux; \
    case "${TARGETARCH}" in \
        amd64) ARCH=amd64 ;; \
        arm64) ARCH=arm64 ;; \
        *) echo "unsupported arch: ${TARGETARCH}" && exit 1 ;; \
    esac; \
    VELA_TAR="vela-${VELA_VERSION}-linux-${ARCH}.tar.gz"; \
    curl -fsSL \
        "https://github.com/kubevela/kubevela/releases/download/${VELA_VERSION}/${VELA_TAR}" \
        -o vela.tar.gz; \
    curl -fsSL \
        "https://github.com/kubevela/kubevela/releases/download/${VELA_VERSION}/sha256sums.txt" \
        -o sha256sums.txt; \
    grep "  ${VELA_TAR}$" sha256sums.txt | sed "s|${VELA_TAR}|vela.tar.gz|" | sha256sum --check; \
    tar xzf vela.tar.gz --strip-components=1 "linux-${ARCH}/vela"; \
    chmod 0755 vela; \
    mv vela /usr/local/bin/vela; \
    rm vela.tar.gz sha256sums.txt

# ── trivy ────────────────────────────────────────────────────────────────────
# Since ~v0.55 Trivy ships a single checksums.txt instead of per-file .sha256sum.
RUN set -eux; \
    TRIVY_VER="${TRIVY_VERSION#v}"; \
    case "${TARGETARCH}" in \
        amd64) TRIVY_ARCH="Linux-64bit" ;; \
        arm64) TRIVY_ARCH="Linux-ARM64" ;; \
        *) echo "unsupported arch: ${TARGETARCH}" && exit 1 ;; \
    esac; \
    TRIVY_TAR="trivy_${TRIVY_VER}_${TRIVY_ARCH}.tar.gz"; \
    TRIVY_CHECKSUMS="trivy_${TRIVY_VER}_checksums.txt"; \
    curl -fsSL \
        "https://github.com/aquasecurity/trivy/releases/download/${TRIVY_VERSION}/${TRIVY_TAR}" \
        -o trivy.tar.gz; \
    curl -fsSL \
        "https://github.com/aquasecurity/trivy/releases/download/${TRIVY_VERSION}/${TRIVY_CHECKSUMS}" \
        -o checksums.txt; \
    grep "  ${TRIVY_TAR}$" checksums.txt | sed "s|${TRIVY_TAR}|trivy.tar.gz|" | sha256sum --check; \
    tar xzf trivy.tar.gz trivy; \
    chmod 0755 trivy; \
    mv trivy /usr/local/bin/trivy; \
    rm trivy.tar.gz checksums.txt

# ── Stage 3: runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

ARG GCLOUD_VERSION=481.0.0

# Minimal runtime packages + gcloud SDK via apt (handles arch automatically,
# brings Python runtime that gcloud requires, and provides
# gke-gcloud-auth-plugin for Kubernetes ADC authentication).
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    gnupg \
    && curl -fsSL https://packages.cloud.google.com/apt/doc/apt-key.gpg \
        | gpg --dearmor -o /usr/share/keyrings/cloud.google.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/cloud.google.gpg] \
        https://packages.cloud.google.com/apt cloud-sdk main" \
        > /etc/apt/sources.list.d/google-cloud-sdk.list \
    && apt-get update && apt-get install -y --no-install-recommends \
        "google-cloud-cli=${GCLOUD_VERSION}-0" \
        google-cloud-sdk-gke-gcloud-auth-plugin \
    && apt-get purge -y --auto-remove gnupg \
    && rm -rf /var/lib/apt/lists/*

# Copy k7s binary from builder.
COPY --from=builder /build/target/release/k7s /usr/local/bin/k7s

# Copy tool binaries from tools stage.
COPY --from=tools /usr/local/bin/kubectl  /usr/local/bin/kubectl
COPY --from=tools /usr/local/bin/helm     /usr/local/bin/helm
COPY --from=tools /usr/local/bin/vela     /usr/local/bin/vela
COPY --from=tools /usr/local/bin/trivy    /usr/local/bin/trivy

# Create non-root user (UID 65532 matches distroless convention).
RUN groupadd -g 65532 k7s && \
    useradd -u 65532 -g k7s -m -s /bin/sh -d /home/k7s k7s

# Declare mount points so users know where to attach volumes.
VOLUME ["/home/k7s/.kube", "/home/k7s/.config/k7s", "/home/k7s/.local/state/k7s"]

# OCI image labels (https://github.com/opencontainers/image-spec).
ARG VERSION=dev
ARG REVISION=unknown
ARG BUILD_DATE=unknown
LABEL org.opencontainers.image.title="k7s" \
      org.opencontainers.image.description="Security-first Kubernetes TUI with AI-powered cluster analysis — batteries-included variant" \
      org.opencontainers.image.url="https://github.com/neuro-ng/k7s" \
      org.opencontainers.image.source="https://github.com/neuro-ng/k7s" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${BUILD_DATE}" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.vendor="neuro-ng"

USER k7s
ENV HOME=/home/k7s \
    PATH="/usr/local/bin:/usr/lib/google-cloud-sdk/bin:${PATH}"

ENTRYPOINT ["/usr/local/bin/k7s"]
