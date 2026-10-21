#!/usr/bin/env bash
# Makes sure that a Saci backend answers at the backend URL. If a backend
# already answers, it does nothing. If not, it starts
# `saci backend start --config <file>` in the background, in the current
# directory, and waits until the backend answers.
#
#   bash ensure-backend.sh [--config saci.yaml] [--backend URL]
#                          [--log saci-backend.log] [--timeout 60]
#
# Defaults: --backend is SACI_BACKEND_URL, else http://127.0.0.1:8080.
# The pid file is the log file with the extension .pid. Set SACI to use a
# saci binary that is not on PATH.
#
# Exit codes, as for saci: 0 the backend answers, 2 invalid input (missing
# saci binary or configuration, bad option), 3 the backend did not start or
# is remote and does not answer.
set -euo pipefail

config=saci.yaml
backend=${SACI_BACKEND_URL:-http://127.0.0.1:8080}
log=saci-backend.log
timeout=60

fail() {
  printf 'ensure-backend: %s\n' "$2" >&2
  exit "$1"
}

while [ $# -gt 0 ]; do
  case "$1" in
    --config) [ $# -ge 2 ] || fail 2 "--config needs a value"; config=$2; shift 2 ;;
    --backend) [ $# -ge 2 ] || fail 2 "--backend needs a value"; backend=$2; shift 2 ;;
    --log) [ $# -ge 2 ] || fail 2 "--log needs a value"; log=$2; shift 2 ;;
    --timeout) [ $# -ge 2 ] || fail 2 "--timeout needs a value"; timeout=$2; shift 2 ;;
    -h | --help) sed -n '2,16p' "$0"; exit 0 ;;
    *) fail 2 "unknown option $1; see --help" ;;
  esac
done
case "$timeout" in
  '' | *[!0-9]*) fail 2 "--timeout must be a number of seconds" ;;
esac
backend=${backend%/}
pidfile=${log%.log}.pid

answers() {
  curl -fsS --max-time 2 "$backend/api/v1/health" 2>/dev/null | grep -q '"ok"'
}

if answers; then
  echo "A Saci backend already answers at $backend."
  exit 0
fi

case "$backend" in
  http://127.0.0.1:* | http://localhost:* | http://\[::1\]:*) ;;
  *) fail 3 "no backend answers at $backend, and this script starts only local backends (http://127.0.0.1:<port>)" ;;
esac

saci=${SACI:-$(command -v saci || true)}
[ -n "$saci" ] && [ -x "$saci" ] || fail 2 "cannot find the saci binary; install Saci (make install) or set SACI to its path"
[ -f "$config" ] || fail 2 "the configuration $config does not exist; give it with --config"

nohup "$saci" backend start --config "$config" >>"$log" 2>&1 </dev/null &
pid=$!
echo "$pid" >"$pidfile"

waited=0
while ! answers; do
  if ! kill -0 "$pid" 2>/dev/null; then
    rm -f "$pidfile"
    tail -n 20 "$log" >&2
    fail 3 "the backend stopped before it answered; the full log is in $log"
  fi
  if [ "$waited" -ge $((timeout * 2)) ]; then
    kill "$pid" 2>/dev/null || true
    rm -f "$pidfile"
    tail -n 20 "$log" >&2
    fail 3 "no answer at $backend after ${timeout}s; stopped the backend; the full log is in $log"
  fi
  sleep 0.5
  waited=$((waited + 1))
done

echo "Started a Saci backend at $backend (pid $pid, log $log)."
echo "To stop it: kill \$(cat $pidfile)"
