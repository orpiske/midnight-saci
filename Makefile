.PHONY: check fmt fmt-check clippy test image image-check web web-check console

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

## Build the web console into web/dist.
web:
	cd web && npm ci && npm run build

## Run every web console check CI runs, including the browser tests.
web-check: web
	cd web && npm run check && npm run e2e
	SACI_WEB_DIST=$(CURDIR)/web/dist cargo test --locked --package saci-backend --test console

## Build the backend with the web console embedded.
console: web
	SACI_WEB_DIST=$(CURDIR)/web/dist cargo build --locked --release --package saci-backend
