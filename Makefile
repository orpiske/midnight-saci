.PHONY: check fmt fmt-check clippy test image image-check

## Run every quality gate that CI runs.
check: fmt-check clippy test

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

clippy:
	cargo clippy --locked --workspace --all-targets -- -D warnings

test:
	cargo test --locked --workspace

RUNTIME ?= $(shell command -v podman >/dev/null 2>&1 && echo podman || echo docker)
IMAGE ?= localhost/saci-worker:latest

## Build the worker image.
image:
	$(RUNTIME) build -f Containerfile -t $(IMAGE) .

## Run a fixture-backed plan inside the worker image, directly and launched by the backend.
image-check: image
	SACI_WORKER_IMAGE=$(IMAGE) SACI_CONTAINER_RUNTIME=$(RUNTIME) cargo test --locked --package saci-worker --test worker_bootstrap
	SACI_WORKER_IMAGE=$(IMAGE) SACI_CONTAINER_RUNTIME=$(RUNTIME) cargo test --locked --package saci-backend --test container_launcher
