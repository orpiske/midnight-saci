#!/usr/bin/env bash
# The namespace workflow (docs/namespace-workflows.md): install Saci with
# Helm into one namespace next to the service it tests and a scripted
# model, then pass, fail and cancel runs, restart the backend while a run
# is in progress, and check that reports are kept and no Jobs are left.
# Run it from the repository root against a cluster kubectl can reach:
#
#   examples/kubernetes/namespace-workflow.sh
#
# Settings (environment variables):
#   NAMESPACE      existing namespace to install into (default: saci-demo)
#   RELEASE        Helm release name (default: saci)
#   IMAGE          Saci image repository:tag (default: localhost/saci-worker:latest)
#   FIXTURES       fixtures image (default: localhost/saci-fixtures:latest)
#   PULL_POLICY    image pull policy (default: IfNotPresent)
#   SACI_BIN       directory with the saci CLI (default: target/debug)
#   PORT           local port for the API port-forward (default: 18080)
#   HELM_ARGS      extra arguments for helm, split on spaces
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
namespace=${NAMESPACE:-saci-demo}
release=${RELEASE:-saci}
image=${IMAGE:-localhost/saci-worker:latest}
fixtures=${FIXTURES:-localhost/saci-fixtures:latest}
pull=${PULL_POLICY:-IfNotPresent}
bin=$(cd "${SACI_BIN:-target/debug}" && pwd)
port=${PORT:-18080}
# The chart's resource names: the release name, or <release>-saci.
case $release in *saci*) name=$release ;; *) name=$release-saci ;; esac
work=$(mktemp -d -t saci-namespace.XXXXXX)
export SACI_BACKEND_URL=http://127.0.0.1:$port
forward=""

say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
fail() { printf '\nnamespace workflow failed: %s\n' "$*" >&2; exit 1; }
saci() { "$bin/saci" "$@"; }
k() { kubectl --namespace "$namespace" "$@"; }
api() { curl -fsS -H "Authorization: Bearer $SACI_TOKEN" "$SACI_BACKEND_URL/api/v1/$1"; }

cleanup() {
  [ -n "$forward" ] && kill "$forward" 2>/dev/null || true
  wait 2>/dev/null || true
}
trap cleanup EXIT

expect_exit() {
  local expected=$1; shift
  local code=0
  saci "$@" || code=$?
  [ "$code" -eq "$expected" ] || fail "saci $* exited $code, expected $expected"
}

# (Re)starts the port-forward to the backend Service and waits for the API.
connect() {
  [ -n "$forward" ] && kill "$forward" 2>/dev/null || true
  k port-forward "svc/$name" "$port:8080" >>"$work/port-forward.log" 2>&1 &
  forward=$!
  for _ in $(seq 1 100); do
    curl -fsS -o /dev/null "$SACI_BACKEND_URL/api/v1/health" 2>/dev/null && return 0
    sleep 0.2
  done
  fail "the backend did not answer through port-forward"
}

jobs_left() { k get jobs -l io.saci.managed=true -o name; }

say "Checking requirements"
for program in kubectl helm curl jq; do
  command -v "$program" >/dev/null || fail "$program is not on PATH"
done
[ -x "$bin/saci" ] || fail "$bin/saci is missing; run: cargo build"
k get serviceaccounts default >/dev/null || fail "namespace $namespace is not reachable"
kubectl version
echo "namespace: $namespace, image $image, fixtures $fixtures"

say "1. Starting the orders service and the scripted model in the namespace"
sed -e "s#image: localhost/saci-fixtures:latest#image: $fixtures#" \
    -e "s#imagePullPolicy: IfNotPresent#imagePullPolicy: $pull#" "$here/fixtures.yaml" | k apply -f -
k rollout status deployment/orders --timeout=3m
k rollout status deployment/model --timeout=3m

say "2. Installing Saci with Helm"
helm upgrade --install "$release" "$here/../../deploy/helm/saci" --namespace "$namespace" \
  --wait --timeout 3m -f "$here/values.yaml" \
  --set image.repository="${image%:*}" --set image.tag="${image##*:}" --set image.pullPolicy="$pull" ${HELM_ARGS:-}
SACI_TOKEN=$(k get secret "$name-auth" -o jsonpath='{.data.token}' | base64 -d)
export SACI_TOKEN
connect

k get pods -l app.kubernetes.io/component=backend \
  -o jsonpath='{range .items[*]}backend pod {.metadata.name} runs as UID {.spec.securityContext.runAsUser}{"\n"}{end}'

say "3. Creating a plan and running it: the service is healthy (exit 0)"
(cd "$work" && saci new test-plan service-health.md)
expect_exit 0 run "$work/service-health.md" --environment same-namespace --follow
health=$(saci list --json | jq -r '.items[0].id')

say "4. Running the orders plan: creating an order fails (exit 1)"
expect_exit 1 run "$here/../walkthrough/plans/orders.md" --environment same-namespace --follow
orders=$(saci list --json | jq -r '.items[0].id')

say "5. Starting the slow export as a Job"
export_run=$(saci run "$here/../walkthrough/plans/export.md" --environment same-namespace)
echo "submitted $export_run"
for _ in $(seq 1 240); do
  [ "$(api "runs/$export_run" | jq '.last_event_seq')" -ge 2 ] && break
  sleep 0.5
done
k get jobs -l "io.saci.run=$export_run"

say "6. Restarting the backend while the export runs"
k rollout restart "deployment/$name"
k rollout status "deployment/$name" --timeout=3m
connect
expect_exit 8 status "$export_run"

say "7. Cancelling the export after the restart (exit 7)"
expect_exit 7 cancel "$export_run" --wait

say "8. Checking retained reports and evidence"
expect_exit 0 report "$health"
expect_exit 1 report "$orders"
expect_exit 7 report "$export_run"
for run in "$health" "$orders" "$export_run"; do
  count=$(api "runs/$run/report" | jq '.evidence | length')
  [ "$count" -ge 1 ] || fail "run $run kept no evidence"
  echo "$run: $count evidence records"
done

say "9. Checking that every run's Job was removed"
for _ in $(seq 1 60); do
  left=$(jobs_left)
  [ -z "$left" ] && break
  sleep 1
done
[ -z "$left" ] || fail "Jobs are left: $left"
echo "no Jobs left"

say "Namespace workflow complete"
