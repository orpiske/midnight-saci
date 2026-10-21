# Saci image (docs/worker.md): the per-run worker, and the backend for
# cluster installs (docs/helm.md), which runs `saci-backend` instead of
# the default entry point.
#
#   podman build -t localhost/saci-worker:latest .
#   docker build -f Containerfile -t localhost/saci-worker:latest .
#
# The worker runs as a non-root user that may be replaced by any UID in
# group 0 (OpenShift's restricted policy). Extend the tool set with a
# derived image; see docs/worker.md#adding-utilities.

ARG RUST_IMAGE=docker.io/library/rust:1.96-slim-trixie
ARG BASE_IMAGE=registry.fedoraproject.org/fedora:44
ARG NODE_IMAGE=docker.io/library/node:22-trixie-slim

# The web console (docs/web-ui.md), embedded in saci-backend below.
FROM ${NODE_IMAGE} AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
COPY docs/templates/ /src/docs/templates/
RUN npm run build

FROM ${RUST_IMAGE} AS build
# The commit the binaries report (docs/development.md#build-commit). The
# build context has no .git, so pass it: --build-arg SACI_BUILD_COMMIT=...
ARG SACI_BUILD_COMMIT=
WORKDIR /src
COPY . .
COPY --from=web /src/web/dist /src/web/dist
ENV SACI_WEB_DIST=/src/web/dist
RUN cargo build --locked --release --package saci-worker --bin saci-worker \
        --package saci-backend --bin saci-backend --package saci-cli --bin saci \
    && install -D -t /out target/release/saci-worker target/release/saci-backend target/release/saci

FROM ${BASE_IMAGE}
# The utilities test plans most often need. `bash` runs every shell action.
RUN dnf install --assumeyes --setopt=install_weak_deps=False \
        bash ca-certificates curl jq git openssh-clients procps-ng iproute bind-utils nmap-ncat \
    && dnf clean all
COPY --from=build /out/ /usr/local/bin/
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
      org.opencontainers.image.description="Saci: the per-run worker (default entry point) and the backend" \
      org.opencontainers.image.source="https://github.com/orpiske/midnight-saci" \
      org.opencontainers.image.licenses="Apache-2.0"
