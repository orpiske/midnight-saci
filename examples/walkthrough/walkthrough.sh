#!/usr/bin/env bash
# The Saci walkthrough (docs/walkthrough.md), from plan creation to retained
# reports, with a real worker container, a scripted model and a local
# orders service. Run it from the repository root after `cargo build` and
# `make image`:
#
#   examples/walkthrough/walkthrough.sh
#
# Settings (environment variables):
#   SACI_BIN    directory with saci, saci-fake-model and saci-fake-target
#               (default: target/debug)
#   IMAGE       worker image (default: localhost/saci-worker:latest)
#   RUNTIME     podman or docker (default: podman if found, else docker)
#   WORK        working directory, kept afterwards (default: a new temporary one)
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
bin=$(cd "${SACI_BIN:-target/debug}" && pwd)
image=${IMAGE:-localhost/saci-worker:latest}
runtime=${RUNTIME:-$(command -v podman >/dev/null 2>&1 && echo podman || echo docker)}
work=${WORK:-$(mktemp -d -t saci-walkthrough.XXXXXX)}
mkdir -p "$work"
export SACI_BACKEND_URL=http://127.0.0.1:18080
pids=()

say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
fail() { printf '\nwalkthrough failed: %s\n' "$*" >&2; exit 1; }
saci() { "$bin/saci" "$@"; }

cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
}
trap cleanup EXIT

# Runs `saci "$@"` and checks its exit code is $expected.
expect_exit() {
  local expected=$1; shift
  local code=0
  saci "$@" || code=$?
  [ "$code" -eq "$expected" ] || fail "saci $* exited $code, expected $expected"
}

wait_for() {
  local url=$1
  for _ in $(seq 1 100); do
    curl -fsS -o /dev/null "$url" 2>/dev/null && return 0
    sleep 0.1
  done
  fail "$url did not answer"
}

start_backend() {
  (cd "$work" && exec "$bin/saci" backend start --config saci.yaml) >>"$work/backend.log" 2>&1 &
  backend=$!
  pids+=("$backend")
  wait_for "$SACI_BACKEND_URL/api/v1/health"
}

say "Checking requirements"
for program in saci saci-fake-model saci-fake-target; do
  [ -x "$bin/$program" ] || fail "$bin/$program is missing; run: cargo build"
done
command -v "$runtime" >/dev/null || fail "$runtime is not on PATH; install Podman or Docker"
for program in curl jq; do
  command -v "$program" >/dev/null || fail "$program is not on PATH"
done
"$runtime" image inspect "$image" >/dev/null 2>&1 || fail "image $image is missing; run: make image"
echo "binaries: $bin"
echo "runtime:  $runtime, image $image"
echo "work dir: $work"

say "Preparing the working directory"
cp "$here/playbook.json" "$work/"
cp -r "$here/plans" "$work/"
sed -e "s#image: localhost/saci-worker:latest#image: $image#" \
    -e "s#runtime: auto #runtime: $runtime #" "$here/saci.yaml" >"$work/saci.yaml"

say "Starting the orders service and the scripted model"
"$bin/saci-fake-target" --listen 127.0.0.1:18081 >>"$work/target.log" 2>&1 &
pids+=("$!")
"$bin/saci-fake-model" --listen 127.0.0.1:11435 --playbook "$work/playbook.json" >>"$work/model.log" 2>&1 &
pids+=("$!")
wait_for http://127.0.0.1:18081/health

say "Starting the backend"
start_backend

say "1. Creating a plan from the template"
(cd "$work" && saci new test-plan plans/service-health.md)
cat "$work/plans/service-health.md"

say "2. Running it: the service is healthy, so the run passes (exit 0)"
expect_exit 0 run "$work/plans/service-health.md" --environment local --follow
health=$(saci list --json | jq -r '.items[0].id')

say "3. Running the orders plan: creating an order fails (exit 1)"
expect_exit 1 run "$work/plans/orders.md" --environment local --follow
orders=$(saci list --json | jq -r '.items[0].id')

say "4. Running the slow export, then cancelling it (exit 7)"
export_run=$(saci run "$work/plans/export.md" --environment local)
echo "submitted $export_run"
# Wait until the worker has started the export step (the run and attempt
# events), then give it a moment to start the command, so the cancellation
# interrupts it.
for _ in $(seq 1 240); do
  events=$(curl -fsS "$SACI_BACKEND_URL/api/v1/runs/$export_run" | jq '.last_event_seq')
  [ "$events" -ge 2 ] && break
  sleep 0.5
done
sleep 2
expect_exit 8 status "$export_run"
expect_exit 7 cancel "$export_run" --wait

say "5. Restarting the backend: every report is retained"
kill "$backend"
wait "$backend" 2>/dev/null || true
start_backend
expect_exit 0 report "$health"
expect_exit 1 report "$orders"
expect_exit 7 report "$export_run"
saci list

say "6. Checking that no worker containers are left"
for _ in $(seq 1 60); do
  left=$("$runtime" ps --all --quiet --filter label=io.saci.managed=true)
  [ -z "$left" ] && break
  sleep 1
done
[ -z "$left" ] || fail "worker containers are left: $left"
echo "none left"

say "Walkthrough complete"
echo "Data, logs and plans are in $work"
