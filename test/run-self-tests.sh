#!/usr/bin/env bash
# Runs the plans in test/plans/ end to end: starts the Saci under test
# (test/start-under-test.sh), starts the Saci that runs the plans with
# test/saci.yaml, runs each plan in turn and writes its report. It exits 0
# when every plan passed and 1 otherwise. CI runs it in
# .github/workflows/self-test.yml. Run it from the repository root after
# `cargo build --package saci-cli --package saci-testkit` and `make image`:
#
#   TYPESAFE_API_KEY=... test/run-self-tests.sh
#
# Settings (environment variables):
#   SACI_BIN           directory with saci, saci-fake-model and saci-fake-target
#                      (default: target/debug)
#   IMAGE              worker image (default: localhost/saci-worker:latest)
#   RUNTIME            podman or docker (default: podman if found, else docker)
#   WORK               working directory with logs and reports, kept afterwards
#                      (default: a new temporary one)
#   PLANS              plan file names in test/plans/, in order
#                      (default: cli-authoring.md backend-api.md cli-runs.md)
#   SACI_LLM_API_KEY   the model provider's key; without it, judge only
#   SACI_LLM_BASE_URL  replaces llm.base_url of test/saci.yaml when set
#   SACI_LLM_MODEL     replaces llm.model of test/saci.yaml when set
#   TYPESAFE_API_KEY   when set, the TypeSafe judge assesses the units first
#   SACI_JUDGE_THRESHOLD  judge.threshold_percent (default: 90)
#
# Needs curl and jq. Workers use the host network, so this needs Linux.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
bin=$(cd "${SACI_BIN:-target/debug}" && pwd)
image=${IMAGE:-localhost/saci-worker:latest}
runtime=${RUNTIME:-$(command -v podman >/dev/null 2>&1 && echo podman || echo docker)}
work=${WORK:-$(mktemp -d -t saci-self-test.XXXXXX)}
plans=${PLANS:-cli-authoring.md backend-api.md cli-runs.md}
mkdir -p "$work/runner" "$work/reports"
work=$(cd "$work" && pwd)
pids=()

fail() { printf 'run-self-tests: %s\n' "$*" >&2; exit 1; }

cleanup() {
  for pid in ${pids[@]+"${pids[@]}"}; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
}
trap cleanup EXIT

wait_for() {
  for _ in $(seq 1 300); do
    curl -fsS -o /dev/null "$1" 2>/dev/null && return 0
    sleep 0.2
  done
  fail "$1 did not answer; see the logs in $work"
}

[ -x "$bin/saci" ] || fail "$bin/saci is missing; run: cargo build"
[ -n "${SACI_LLM_API_KEY:-}${TYPESAFE_API_KEY:-}" ] || fail "set SACI_LLM_API_KEY, TYPESAFE_API_KEY or both"

# The Saci under test, on 127.0.0.1:18080.
SACI_BIN="$bin" IMAGE="$image" RUNTIME="$runtime" WORK="$work/under-test" \
  "$root/test/start-under-test.sh" >"$work/under-test.log" 2>&1 &
pids+=("$!")
wait_for http://127.0.0.1:18080/api/v1/health

# The Saci that runs the plans, on 127.0.0.1:8080.
config="$work/runner/saci.yaml"
sed -e "s#image: localhost/saci-worker:latest#image: $image#" \
    -e "s#runtime: auto #runtime: $runtime #" \
    "$root/test/saci.yaml" >"$config"
edit() {  # edit <sed expression>: changes the configuration in place
  sed -e "$1" "$config" >"$config.new"
  mv "$config.new" "$config"
}
if [ -n "${SACI_LLM_API_KEY:-}" ]; then
  [ -z "${SACI_LLM_BASE_URL:-}" ] || edit 's#^  base_url: .*#  base_url: env:SACI_LLM_BASE_URL#'
  [ -z "${SACI_LLM_MODEL:-}" ] || edit 's#^  model: .*#  model: env:SACI_LLM_MODEL#'
else
  # Judge only: a scripted model with an empty playbook answers every
  # request with an error, so a unit the judge does not pass ends `error`.
  [ -x "$bin/saci-fake-model" ] || fail "$bin/saci-fake-model is missing; run: cargo build"
  echo '{"units": {}}' >"$work/runner/no-model.json"
  "$bin/saci-fake-model" --listen 127.0.0.1:11436 --playbook "$work/runner/no-model.json" \
    >>"$work/no-model.log" 2>&1 &
  pids+=("$!")
  edit 's#^  base_url: .*#  base_url: http://127.0.0.1:11436/v1#'
  edit 's#^  model: .*#  model: no-model-judge-only#'
  edit '/^  api_key: env:SACI_LLM_API_KEY/d'
  awk '{ print } /^limits:/ { print "  model_request_retries: 0" }' "$config" >"$config.new"
  mv "$config.new" "$config"
fi
if [ -n "${TYPESAFE_API_KEY:-}" ]; then
  printf '\njudge:\n  provider: typesafe\n  api_key: env:TYPESAFE_API_KEY\n' >>"$config"
  [ -z "${SACI_JUDGE_THRESHOLD:-}" ] || printf '  threshold_percent: %s\n' "$SACI_JUDGE_THRESHOLD" >>"$config"
fi
(cd "$work/runner" && exec "$bin/saci" backend start --config saci.yaml) >>"$work/runner.log" 2>&1 &
pids+=("$!")
wait_for http://127.0.0.1:8080/api/v1/health

# The units that did not pass, with their expected results and the output
# of their commands, so the reason shows in the log.
not_passed='.evidence as $ev | .units[] | select(.attempts[-1].outcome != "passed")
  | "--- \(.key) \(.title): \(.attempts[-1].outcome)",
    ((.criteria // [])[] | "expected: \(.text)"),
    (.attempts[-1].evidence[] as $id | $ev[] | select(.id == $id)
      | "$ \(.command // .tool)", (.stdout.text // ""), (.stderr.text // ""))'

# Keeps the log of the worker container of run $1 in file $2, since the
# backend removes the container when the run ends.
keep_worker_log() {
  for _ in $(seq 1 300); do
    if "$runtime" container inspect "saci-$1" >/dev/null 2>&1; then
      exec "$runtime" logs -f "saci-$1" >"$2" 2>&1
    fi
    sleep 0.2
  done
}

export SACI_BACKEND_URL=http://127.0.0.1:8080
failed=0
summary=""
for plan in $plans; do
  name=${plan%.md}
  [ -z "${GITHUB_ACTIONS:-}" ] || echo "::group::$plan"
  run=$("$bin/saci" run "$root/test/plans/$plan" --environment saci-under-test) \
    || fail "$plan was not accepted"
  keep_worker_log "$run" "$work/reports/$name.worker.log" &
  log_pid=$!
  status=0
  "$bin/saci" follow "$run" || status=$?
  wait "$log_pid" 2>/dev/null || true
  "$bin/saci" report "$run" >"$work/reports/$name.txt" || true
  "$bin/saci" report "$run" --json >"$work/reports/$name.json" || true
  [ "$status" -eq 0 ] || jq -r "$not_passed" "$work/reports/$name.json" || true
  [ "$status" -eq 0 ] || grep -i "judge" "$work/reports/$name.worker.log" || true
  [ -z "${GITHUB_ACTIONS:-}" ] || echo "::endgroup::"
  [ "$status" -eq 0 ] || failed=1
  summary="$summary$plan $run exit $status"$'\n'
done

printf '\n%s' "$summary"
echo "Reports and logs: $work"
exit "$failed"
