# Saci worker image (docs/worker.md).
#
#   podman build -t localhost/saci-worker:latest .
#   docker build -f Containerfile -t localhost/saci-worker:latest .
#
# The worker runs as a non-root user that may be replaced by any UID in
# group 0 (OpenShift's restricted policy). Extend the tool set with a
# derived image; see docs/worker.md#adding-utilities.

ARG RUST_IMAGE=docker.io/library/rust:1.96-slim-trixie
ARG BASE_IMAGE=docker.io/library/debian:trixie-slim

FROM ${RUST_IMAGE} AS build
WORKDIR /src
COPY . .
RUN cargo build --locked --release --package saci-worker --bin saci-worker \
    && install -D target/release/saci-worker /out/saci-worker

FROM ${BASE_IMAGE}
# The utilities test plans most often need. `bash` runs every shell action.
RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
        bash ca-certificates curl jq git openssh-client procps iproute2 dnsutils netcat-openbsd \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /out/saci-worker /usr/local/bin/saci-worker
# The workspace is writable by group 0 so an arbitrary UID can use it.
RUN mkdir -p /workspace \
    && chgrp 0 /workspace \
    && chmod 2775 /workspace
ENV SACI_WORKSPACE=/workspace \
    HOME=/workspace
WORKDIR /workspace
USER 10001:0
ENTRYPOINT ["/usr/local/bin/saci-worker"]

LABEL org.opencontainers.image.title="saci-worker" \
      org.opencontainers.image.description="Saci per-run worker: executes one run specification" \
      org.opencontainers.image.source="https://github.com/orpiske/midnight-saci" \
      org.opencontainers.image.licenses="Apache-2.0"
