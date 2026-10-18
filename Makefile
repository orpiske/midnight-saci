.PHONY: check fmt fmt-check clippy test image image-check web web-check console install uninstall

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
	cargo test --locked --package saci-backend --test console

## Build the backend and the saci CLI (which also starts the backend) with
## the web console embedded. Debug by default; CARGO_FLAGS=--release for a
## release build.
CARGO_FLAGS ?=
console: web
	cargo build --locked $(CARGO_FLAGS) --package saci-backend --bin saci-backend --package saci-cli --bin saci

PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
BINARIES := saci saci-backend saci-worker

## Build the binaries in release mode and install them in $(DESTDIR)$(BINDIR).
## Override the location with PREFIX=... or BINDIR=.... Run `make web` first
## to embed the web console.
install:
	cargo build --locked --release --package saci-cli --bin saci \
		--package saci-backend --bin saci-backend --package saci-worker --bin saci-worker
	install -d "$(DESTDIR)$(BINDIR)"
	for bin in $(BINARIES); do install -m 0755 "target/release/$$bin" "$(DESTDIR)$(BINDIR)/$$bin"; done

## Remove the installed binaries from $(DESTDIR)$(BINDIR).
uninstall:
	for bin in $(BINARIES); do rm -f "$(DESTDIR)$(BINDIR)/$$bin"; done
