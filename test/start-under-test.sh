#!/usr/bin/env bash
# Starts the Saci that the plans in test/plans/ test: a backend on
# 127.0.0.1:18080 with the walkthrough's scripted model and orders service
# (examples/walkthrough). It stays in the foreground until Ctrl-C, then
# stops everything it started. Run it from the repository root after
# `cargo build` and `make image`:
#
#   test/start-under-test.sh
#
# Settings (environment variables):
#   SACI_BIN    directory with saci, saci-fake-model and saci-fake-target
#               (default: target/debug)
#   IMAGE       worker image (default: localhost/saci-worker:latest)
#   RUNTIME     podman or docker (default: podman if found, else docker)
#   WORK        working directory, kept afterwards (default: a new temporary one)
#
# Workers use the host network, so this needs Linux; see
# docs/launchers.md#networking for other systems.
set -euo pipefail

walkthrough=$(cd "$(dirname "$0")/../examples/walkthrough" && pwd)
bin=$(cd "${SACI_BIN:-target/debug}" && pwd)
image=${IMAGE:-localhost/saci-worker:latest}
runtime=${RUNTIME:-$(command -v podman >/dev/null 2>&1 && echo podman || echo docker)}
work=${WORK:-$(mktemp -d -t saci-under-test.XXXXXX)}
mkdir -p "$work"
pids=()

fail() { printf 'start-under-test: %s\n' "$*" >&2; exit 1; }

cleanup() {
  for pid in ${pids[@]+"${pids[@]}"}; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
}
trap cleanup EXIT

wait_for() {
  for _ in $(seq 1 100); do
    curl -fsS -o /dev/null "$1" 2>/dev/null && return 0
    sleep 0.1
  done
  fail "$1 did not answer; see the logs in $work"
}

[ "$(uname -s)" = Linux ] || fail "workers need the host network, which needs Linux"
for program in saci saci-fake-model saci-fake-target; do
  [ -x "$bin/$program" ] || fail "$bin/$program is missing; run: cargo build"
done
"$runtime" image inspect "$image" >/dev/null 2>&1 || fail "image $image is missing; run: make image"

cp "$walkthrough/playbook.json" "$work/"
sed -e "s#image: localhost/saci-worker:latest#image: $image#" \
    -e "s#runtime: auto #runtime: $runtime #" "$walkthrough/saci.yaml" >"$work/saci.yaml"

"$bin/saci-fake-target" --listen 127.0.0.1:18081 >>"$work/target.log" 2>&1 &
pids+=("$!")
"$bin/saci-fake-model" --listen 127.0.0.1:11435 --playbook "$work/playbook.json" >>"$work/model.log" 2>&1 &
pids+=("$!")
wait_for http://127.0.0.1:18081/health
(cd "$work" && exec "$bin/saci" backend start --config saci.yaml) >>"$work/backend.log" 2>&1 &
pids+=("$!")
wait_for http://127.0.0.1:18080/api/v1/health

echo "Saci under test: http://127.0.0.1:18080 (environment local), logs in $work"
echo "Press Ctrl-C to stop it."
wait
